use std::collections::VecDeque;

use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::materialflow_objects::OtherObjects;
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{FillKeeper, MachineState, StatisticsKeeper};
use crate::time::SimTime;
use super::flows::{
    list_blocked, relist_blocked, retries_for_blocked, retry_move_to_successor, try_to_move_to_successor, waiting_ids, BaseObject, Distributing, EventInterface,
    MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, Waiting,
};

/// Which of the MUs that may leave a Buffer it passes on next. The Buffer picks, and the code
/// that moves MUs never knows how, so another strategy is added here alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExitStrategy {
    /// The MU that came in first.
    #[default]
    FirstInFirstOut,
    /// The MU that came in last, as from a stack.
    LastInFirstOut,
}

/// An MU in a Buffer, and when its dwelling time is over.
#[derive(Debug, Clone)]
struct Held {
    mu: MoveableUnit,
    ready_at: SimTime,
}

/// Holds MUs, up to its capacity or without limit, each for the dwelling time from its own
/// entry, and passes on one at a time, chosen by its exit strategy among those whose dwelling
/// time is over. It is empty, holding, or blocked when an MU that may leave can't.
///
/// An unlimited Buffer grows as far as it's filled, 24 bytes for each MU held: the one case
/// where a run's memory isn't flat (ADR 0019).
#[derive(Clone)]
pub struct Buffer {
    pub base: BaseObject,
    pub capacity: Capacity,
    /// How long each MU stays before it may leave, counted from its entry.
    pub dwelling_time: SimTime,
    pub exit_strategy: ExitStrategy,
    /// The ids of the MUs it holds in its default state, in the order they count as entered.
    pub starting_mus: Vec<u32>,
    /// In the order they came in. With one dwelling time for all, that is also the order in
    /// which they become ready, which `offered` relies on.
    content: VecDeque<Held>,
    pub succ: Vec<u32>,
    pub succ_pointer: usize,
    /// How it picks the successor for its MU. Set by the run, which ranks the successors.
    pub distribution: Distributing,
    pub blockedlist: Vec<Waiting>,
    /// An MU that may leave was refused by every successor, so the Buffer waits in their lines.
    blocked: bool,
    pub statistics: StatisticsKeeper,
    /// How full it was: only a Buffer keeps this, so no other object grows.
    fill: FillKeeper,
    /// The width of its histogram's bars, or automatic.
    pub histogram_bar_width: Option<u64>,
}

impl Buffer {
    pub fn new(object_id: u32, capacity: Capacity, succ: Vec<u32>) -> Buffer {
        Buffer {
            base: BaseObject { object_id, random_seed: 0 },
            capacity,
            dwelling_time: SimTime::ZERO,
            exit_strategy: ExitStrategy::FirstInFirstOut,
            starting_mus: Vec::new(),
            content: VecDeque::new(),
            succ,
            succ_pointer: 0,
            distribution: Distributing::RoundRobin,
            blockedlist: Vec::new(),
            blocked: false,
            statistics: StatisticsKeeper::new(capacity.limit()),
            fill: FillKeeper::new(None),
            histogram_bar_width: None,
        }
    }

    /// How many MUs it holds.
    pub fn held(&self) -> usize {
        self.content.len()
    }

    /// Where in `content` the MU is that the exit strategy passes on at `now`, if any may leave.
    fn offered(&self, now: SimTime) -> Option<usize> {
        match self.exit_strategy {
            ExitStrategy::FirstInFirstOut => self.content.front().is_some_and(|held| held.ready_at <= now).then_some(0),
            ExitStrategy::LastInFirstOut => {
                // Without a dwelling time every MU may leave, which is the usual case.
                if self.content.back().is_some_and(|held| held.ready_at <= now) {
                    return Some(self.content.len() - 1);
                }
                self.content.partition_point(|held| held.ready_at <= now).checked_sub(1)
            }
        }
    }

    fn exit_event(&self, mu_id: u32, after: SimTime) -> EventTemplate {
        EventTemplate { eventtype: EventType::MUExit(ExitTicket::new(mu_id)), exec_time: after, object_id: self.base.object_id }
    }

    fn handle_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match *eventtype {
            // An MU's dwelling time is over. A blocked Buffer already waits in line with an
            // earlier MU and keeps that place, so there is nothing to try.
            EventType::MUExit(_) if self.blocked => Ok(None),
            EventType::MUExit(_) => self.pass_on(context, others, None),
            // Only a Buffer that was refused is waiting. A retry can also arrive after its MU
            // went elsewhere, and then an MU that became ready since must take its own turn.
            EventType::BlockedRetry { .. } if !self.blocked => Ok(None),
            EventType::BlockedRetry { from } => self.pass_on(context, others, Some(from)),
            _ => Err(RunTimeError {}),
        }
    }

    /// Passes on every MU that may leave, one at a time, until one is refused: to any successor,
    /// or in answer to a retry only to the successor that sent it.
    fn pass_on(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, only: Option<u32>) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        let mut events = Vec::new();
        let mut moved = false;
        while self.offered(context.time).is_some() {
            let held = self.content.len();
            let entered = match only {
                Some(from) => retry_move_to_successor(self, context, others, from),
                None => try_to_move_to_successor(self, context, others),
            }
            .map_err(|_| RunTimeError {})?;
            if self.content.len() == held {
                self.set_blocked(true, context);
                break;
            }
            moved = true;
            self.set_blocked(false, context);
            events.extend(entered);
        }
        if moved {
            // It has room again.
            events.extend(retries_for_blocked(&mut self.blockedlist, self.base.object_id).unwrap_or_default());
        }
        Ok((!events.is_empty()).then_some(events))
    }

    /// Notes whether it is blocked, reporting when that begins or ends.
    fn set_blocked(&mut self, blocked: bool, context: &mut EventContext) {
        let object_id = self.base.object_id;
        match (self.blocked, blocked) {
            (false, true) => context.record(OccurrenceKind::Blocked { object_id }),
            (true, false) => context.record(OccurrenceKind::Unblocked { object_id }),
            _ => {}
        }
        self.blocked = blocked;
    }

    /// Blocked, holding or empty. Holding is reported as `Processing`, and counts as busy time.
    fn process_state(&self) -> ProcessState {
        if self.blocked {
            ProcessState::Blocked
        } else if self.content.is_empty() {
            ProcessState::Idle
        } else {
            ProcessState::Processing
        }
    }

    /// Tells the statistics what the Buffer ended up doing.
    fn update_statistics(&mut self, now: SimTime) {
        let state = MachineState::of(None, &self.process_state());
        if let Some((held, time)) = self.statistics.observe_stretch(now, state, self.content.len() as u64) {
            self.fill.add(held, time);
        }
    }

    /// About how much memory its MUs take, for a checkpoint holding them.
    pub(crate) fn content_bytes(&self) -> usize {
        self.content.len() * size_of::<Held>()
    }
}

impl MaterialFlow for Buffer {
    fn is_entrance_allowed(&self) -> bool {
        match self.capacity {
            Capacity::Unlimited => true,
            Capacity::Limited(limit) => (self.content.len() as u64) < limit,
        }
    }
    fn is_exit_allowed(&self) -> bool {
        self.blocked
    }
    /// Takes the MU and schedules the end of its dwelling time, when it tries to leave.
    fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
        log::info!("MU {} entered Buffer {}", mu.id, self.base.object_id);
        let exit = self.exit_event(mu.id, self.dwelling_time);
        self.content.push_back(Held { mu, ready_at: context.time + self.dwelling_time });
        self.statistics.count_mu_in();
        self.update_statistics(context.time);
        Ok(Some(exit))
    }
    fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool) {
        list_blocked(&mut self.blockedlist, id_blocked, since, answering_retry);
    }
    fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>) {
        relist_blocked(&mut self.blockedlist, id_blocked, next);
    }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        Ok(retries_for_blocked(&mut self.blockedlist, self.base.object_id))
    }
}

impl SuccessorContainer for Buffer {
    fn distribution_mut(&mut self) -> &mut Distributing {
        &mut self.distribution
    }
    fn successors(&self) -> &[u32] {
        &self.succ
    }
    fn succ_pointer(&self) -> usize {
        self.succ_pointer
    }
    fn succ_pointer_mut(&mut self) -> &mut usize {
        &mut self.succ_pointer
    }
    fn clear_content_and_idle(&mut self) {
        self.content.clear();
        self.blocked = false;
    }
    fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
        &mut self.statistics
    }
    fn take_content(&mut self, now: SimTime) -> Option<MoveableUnit> {
        let at = self.offered(now)?;
        self.content.remove(at).map(|held| held.mu)
    }
    fn offered_since(&self, now: SimTime) -> Option<SimTime> {
        self.offered(now).map(|at| self.content[at].ready_at)
    }
}

impl EventInterface for Buffer {
    /// Its starting MUs count as entered now, in their order, and dwell from now.
    fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
        for _ in &self.content {
            self.statistics.count_mu_in();
        }
        self.update_statistics(context.time);
        // One exit is enough: it passes on every MU that may leave by then.
        Ok(self.content.front().map(|held| self.exit_event(held.mu.id, self.dwelling_time)).into_iter().collect())
    }
    fn reset(&mut self) {
        self.set_starting_mus(self.starting_mus.clone());
        self.succ_pointer = 0;
        self.blockedlist.clear();
        self.blocked = false;
        self.statistics.reset();
        self.fill.clear();
    }
    fn clear_statistics(&mut self, now: SimTime) {
        self.statistics.clear(now);
        self.fill.clear();
    }
    fn report(&self, now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Buffer,
            contents: self.content.iter().map(|held| held.mu.id).collect(),
            process: self.process_state(),
            // A Buffer can't be interrupted yet: its stops aren't built.
            stop: None,
            interruptions: Vec::new(),
            remaining: None,
            work: Vec::new(),
            blocked_predecessors: waiting_ids(&self.blockedlist),
            successors: self.succ.clone(),
            capacity: self.capacity,
            process_time: None,
            configured_process_time: None,
            statistics: self.statistics.read(now),
            fill: Some(Box::new(self.fill.read(self.statistics.in_progress(now)))),
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.base.object_id
    }
    fn set_object_id(&mut self, id: u32) {
        self.base.object_id = id;
    }
    fn set_random_seed(&mut self, seed: u64) {
        self.base.random_seed = seed;
    }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        let outcome = self.handle_event(context, eventtype, others);
        self.update_statistics(context.time);
        outcome
    }
}

impl Buffer {
    /// Sets the MUs it holds in its default state, by id, in the order they count as entered.
    /// More than its capacity is the caller's mistake: a model file is checked for it.
    pub fn set_starting_mus(&mut self, mus: Vec<u32>) {
        let (object_id, dwelling_time) = (self.base.object_id, self.dwelling_time);
        self.content = mus
            .iter()
            .map(|&id| Held { mu: MoveableUnit { id, location_id: object_id, left_source: SimTime::ZERO }, ready_at: dwelling_time })
            .collect();
        self.starting_mus = mus;
    }

    /// Sets the width of its fill histogram's bars, or `None` for automatic.
    pub fn set_histogram_bar_width(&mut self, width: Option<u64>) {
        self.histogram_bar_width = width;
        self.fill.set_width(width);
    }

    /// Sets how long each MU stays. In the default state, its starting MUs dwell that long too.
    pub fn set_dwelling_time(&mut self, dwelling_time: SimTime) {
        self.dwelling_time = dwelling_time;
        for held in &mut self.content {
            held.ready_at = dwelling_time;
        }
    }
}
