use std::collections::VecDeque;

use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::materialflow_objects::OtherObjects;
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{FillKeeper, MachineState, StatisticsKeeper};
use crate::time::SimTime;
use super::interruptions::{Interruptible, Interruptions};
use super::flows::{
    FailureProfile, list_blocked, relist_blocked, retries_for_blocked, retry_move_to_successor, try_to_move_to_successor, waiting_ids, BaseObject, Distributing, EventInterface,
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
    /// How full it was: only a Buffer keeps this, so no other object grows. Boxed, since it is
    /// half a Buffer's size and every object in a run takes the size of the largest.
    fill: Box<FillKeeper>,
    /// The width of its histogram's bars, or automatic.
    pub histogram_bar_width: Option<u64>,
    /// Its failures and pauses (ADR 0020). Interrupted, it takes and releases no MU.
    pub interruptions: Interruptions,
    /// An interruption freezes the dwelling time of the MUs dwelling when it began, rather than
    /// letting it run on, which is the default.
    pub stop_freezes_dwelling: bool,
    /// Bumped when an interruption freezes dwelling, so the exits queued before it are stale.
    stop_count: u32,
    /// When the interruption in progress began.
    interrupted_at: SimTime,
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
            fill: Box::new(FillKeeper::new(None)),
            histogram_bar_width: None,
            interruptions: Interruptions::new(None),
            stop_freezes_dwelling: false,
            stop_count: 0,
            interrupted_at: SimTime::ZERO,
        }
    }

    /// Gives it failures, or none with `None`.
    pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
        self.interruptions.set_failures(failures);
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
        EventTemplate { eventtype: EventType::MUExit(ExitTicket { mu_id, stop_count: self.stop_count }), exec_time: after, object_id: self.base.object_id }
    }

    fn handle_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<(), RunTimeError> {
        let interrupted = self.interruptions.is_interrupted();
        match *eventtype {
            // Interrupted, it releases nothing: every MU that became ready meanwhile is passed on
            // when the interruption ends. An exit queued before dwelling froze is stale.
            EventType::MUExit(ticket) if interrupted || ticket.stop_count != self.stop_count => Ok(()),
            // An MU's dwelling time is over. A blocked Buffer already waits in line with an
            // earlier MU and keeps that place, so there is nothing to try.
            EventType::MUExit(_) if self.blocked => Ok(()),
            EventType::MUExit(_) => self.pass_on(context, others, None),
            // Only a Buffer that was refused is waiting. A retry can also arrive after its MU
            // went elsewhere, and then an MU that became ready since must take its own turn.
            EventType::BlockedRetry { .. } if !self.blocked => Ok(()),
            // Interrupted, it keeps its place in the line, as if refused (spec issue 122).
            EventType::BlockedRetry { from } if interrupted => {
                if let Some(since) = self.offered_since(context.time) {
                    others.get_mut(from).add_blocked(self.base.object_id, since, true);
                }
                Ok(())
            }
            EventType::BlockedRetry { from } => self.pass_on(context, others, Some(from)),
            _ => self.handle_interruption(eventtype, context, others).unwrap_or(Err(RunTimeError {})),
        }
    }

    /// Passes on every MU that may leave, one at a time, until one is refused: to any successor,
    /// or in answer to a retry only to the successor that sent it. Each successor's event is
    /// scheduled as its MU goes in, and the retries for those waiting for room after them all.
    fn pass_on(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, only: Option<u32>) -> Result<(), RunTimeError> {
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
            if let Some(entered) = entered {
                context.schedule(entered);
            }
        }
        if moved {
            // It has room again.
            retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
        }
        Ok(())
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
        let state = MachineState::of(self.interruptions.most_serious(), &self.process_state());
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
        let room = match self.capacity {
            Capacity::Unlimited => true,
            Capacity::Limited(limit) => (self.content.len() as u64) < limit,
        };
        room && !self.interruptions.is_interrupted()
    }
    fn is_exit_allowed(&self) -> bool {
        self.blocked && !self.interruptions.is_interrupted()
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
    fn notify_blocked_predecessors(&mut self, context: &mut EventContext) -> Result<(), MaterialflowError> {
        retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
        Ok(())
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
        let mut events: Vec<EventTemplate> = self.interruptions.first_failure(self.base.object_id).into_iter().collect();
        // One exit is enough: it passes on every MU that may leave by then.
        events.extend(self.content.front().map(|held| self.exit_event(held.mu.id, self.dwelling_time)));
        Ok(events)
    }
    fn reset(&mut self) {
        self.set_starting_mus(self.starting_mus.clone());
        self.succ_pointer = 0;
        self.blockedlist.clear();
        self.blocked = false;
        self.statistics.reset();
        self.fill.clear();
        self.interruptions.reset();
        self.stop_count = 0;
        self.interrupted_at = SimTime::ZERO;
    }
    fn clear_statistics(&mut self, now: SimTime) {
        self.interruptions.clear_statistics(now, &self.statistics);
		self.statistics.clear(now);
        self.fill.clear();
    }
    fn report(&self, now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Buffer,
            contents: self.content.iter().map(|held| held.mu.id).collect(),
            process: self.process_state(),
            interrupted: self.interruptions.most_serious(),
            interruptions: self.interruptions.list(),
			failure_profiles: self.interruptions.profile_states(now, &self.statistics),
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
        self.interruptions.reseed(seed);
    }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<(), RunTimeError> {
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

impl Interruptible for Buffer {
    fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32) {
        (&mut self.interruptions, &mut self.statistics, self.base.object_id)
    }
    /// It takes and releases nothing, which its entrance and exit rules already say. Freezing
    /// dwelling voids the exits queued, which the end of the interruption schedules afresh.
    fn stop_began(&mut self, context: &mut EventContext) {
        self.interrupted_at = context.time;
        if self.stop_freezes_dwelling {
            self.stop_count += 1;
        }
    }
    /// With dwelling frozen, every MU still dwelling when the interruption began becomes ready
    /// later by its length, each with an exit then. Every MU that may leave now is passed on,
    /// and those waiting for room hear of it.
    fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
        if self.stop_freezes_dwelling {
            let (began, now) = (self.interrupted_at, context.time);
            let length = now.saturating_sub(began);
            for at in 0..self.content.len() {
                if self.content[at].ready_at > began {
                    self.content[at].ready_at = self.content[at].ready_at + length;
                    let (mu_id, ready_at) = (self.content[at].mu.id, self.content[at].ready_at);
                    context.schedule(self.exit_event(mu_id, ready_at.saturating_sub(now)));
                }
            }
        }
        self.pass_on(context, others, None)?;
        if self.is_entrance_allowed() {
            retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
        }
        Ok(())
    }
}
