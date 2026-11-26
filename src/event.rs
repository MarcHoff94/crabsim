#![allow(dead_code)]

use std::cmp::Ordering;

use crate::report::{Occurrence, OccurrenceKind, Recording};
use crate::time::SimTime;

/// Names the MU an `MUExit` is for and how many times its object had been stopped
/// when that exit was scheduled. The event heap can't cancel events, so an exit
/// scheduled before a stop stays queued and is recognised as stale by this count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExitTicket {
    pub mu_id: u32,
    pub stop_count: u32,
}

impl ExitTicket {
    pub fn new(mu_id: u32) -> ExitTicket {
        ExitTicket { mu_id, stop_count: 0 }
    }
}

/// What a material-flow object gets while handling an event: the current simulation
/// time, a place to record occurrences, new MU identities, and the model's work plans and
/// destinations.
#[derive(Clone)]
pub struct EventContext {
    pub time: SimTime,
    pub seq: u64,
    /// The model's work plans and destinations (spec issue 148), which never change during a run.
    /// Shared, since a checkpoint clones the context and the table can be large.
    pub routing: std::sync::Arc<crate::routing::Routing>,
    next_mu_id: u32,
    recording: Recording,
    occurrences: Vec<Occurrence>,
    /// The events the object in hand schedules, in order. The run moves them into its queue
    /// after each event and keeps the memory, so scheduling allocates nothing once it has grown.
    scheduled: Vec<EventTemplate>,
    /// Which objects have which hooks (ADR 0029), or `None` in a model without user code, which
    /// then pays one check at each entry and exit.
    hooks: Option<std::sync::Arc<crate::hooks::HookTable>>,
    /// The calls of user code noted since the run last made them, first noted first. The run
    /// makes them right after the event that noted them.
    calls: std::collections::VecDeque<crate::hooks::HookCall>,
    /// The calls at the rear of the MU that moved last, until the conveyor that tracks its rear
    /// takes them (`take_rears`).
    rears: std::collections::VecDeque<crate::hooks::HookCall>,
    /// Whether anything watches MUs move or come into being: recording, or user code. One check
    /// at each move and creation, the same a run recording nothing always paid.
    watching: bool,
}

impl EventContext {
    pub fn new(first_mu_id: u32, recording: Recording) -> EventContext {
        EventContext {
            time: SimTime::ZERO,
            seq: 0,
            routing: Default::default(),
            next_mu_id: first_mu_id,
            recording,
            occurrences: Vec::new(),
            scheduled: Vec::new(),
            hooks: None,
            calls: std::collections::VecDeque::new(),
            rears: std::collections::VecDeque::new(),
            watching: recording != Recording::NOTHING,
        }
    }

    /// The same, noting calls of user code by this table of hooks.
    pub fn with_hooks(mut self, hooks: Option<std::sync::Arc<crate::hooks::HookTable>>) -> EventContext {
        self.hooks = hooks;
        self.watching = self.recording != Recording::NOTHING || self.hooks.is_some();
        self
    }

    /// Records the MU moving from `from` into `to`, about to enter it, and notes the hooks on exit
    /// and entry it sets off (ADR 0029). Calls run after the event, so noting the entry before the
    /// MU is in changes nothing.
    #[inline(always)]
    pub fn moved(&mut self, mu: &crate::flows::MoveableUnit, from: u32, to: u32) {
        if self.watching {
            self.record(OccurrenceKind::MuMoved { mu_id: mu.id, from, to });
            if self.hooks.is_some() {
                self.moved_hooked(mu, from, to, false);
            }
        }
    }

    /// The same for a move user code made: the MU leaves its object whole at once, whatever
    /// geometry that object has.
    pub(crate) fn moved_by_code(&mut self, mu: &crate::flows::MoveableUnit, from: u32, to: u32) {
        self.record(OccurrenceKind::MuMoved { mu_id: mu.id, from, to });
        if self.hooks.is_some() {
            self.moved_hooked(mu, from, to, true);
        }
    }

    /// Notes the hooks a move sets off at the trigger points that are now, and keeps those at the
    /// MU's rear for later (`take_rears`), where a conveyor's geometry puts it later: from a conveyor
    /// the move is its booking point crossing the end, its front having crossed as the place was
    /// reserved (`front_crossed`); onto a conveyor from anything else it enters with its booking
    /// point at the start, its front ahead of it.
    #[cold]
    #[inline(never)]
    fn moved_hooked(&mut self, mu: &crate::flows::MoveableUnit, from: u32, to: u32, by_code: bool) {
        use crate::hooks::{ALL_POINTS, BOOKING_POINT, FRONT, REAR};
        let Some(table) = self.hooks.clone() else { return };
        // Moved by code, it leaves its object whole at once.
        let entry_now = if table.is_belt(from) && !by_code {
            BOOKING_POINT
        } else if table.is_belt(to) {
            FRONT | BOOKING_POINT
        } else {
            ALL_POINTS
        };
        let exit_now = if by_code { ALL_POINTS } else { entry_now };
        let later = |now: crate::hooks::Points| ALL_POINTS & !now & REAR;
        self.note_exit(&table, from, mu, to, exit_now, later(exit_now));
        let mut arrived = mu.clone();
        if arrived.is_routed() {
            // The MU as it comes in: reaching its next destination moves its plan on.
            self.routing.arrive(&mut arrived, to);
        }
        self.note_entry(&table, to, &arrived, entry_now, later(entry_now));
    }

    /// The front of the MU a conveyor offers has crossed its end: `to` has reserved a place for it
    /// (ADR 0023). Notes the hooks at the front, of the conveyor's exit and `to`'s entry.
    #[cold]
    #[inline(never)]
    pub(crate) fn front_crossed(&mut self, mu: &crate::flows::MoveableUnit, from: u32, to: u32) {
        let Some(table) = self.hooks.clone() else { return };
        self.note_exit(&table, from, mu, to, crate::hooks::FRONT, 0);
        let mut arrived = mu.clone();
        if arrived.is_routed() {
            self.routing.arrive(&mut arrived, to);
        }
        self.note_entry(&table, to, &arrived, crate::hooks::FRONT, 0);
    }

    /// Records a Source creating the MU, and notes the Source's hook on entry, its MU coming into
    /// being (ADR 0029).
    #[inline(always)]
    pub fn created(&mut self, mu: &crate::flows::MoveableUnit, at: u32, duration: SimTime) {
        if self.watching {
            self.record(OccurrenceKind::MuCreated { mu_id: mu.id, at, duration, kind: mu.kind() });
            if self.hooks.is_some() {
                self.entered_with_hooks(at, mu);
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn entered_with_hooks(&mut self, object: u32, mu: &crate::flows::MoveableUnit) {
        if let Some(table) = self.hooks.clone() {
            self.note_entry(&table, object, mu, crate::hooks::ALL_POINTS, 0);
        }
    }

    /// Whether the run has user code, which a conveyor asks before doing anything for its hooks.
    #[inline]
    pub fn has_hooks(&self) -> bool {
        self.hooks.is_some()
    }

    /// Notes the calls of an object's hook on entry for the MU that came in at the trigger points
    /// in `now`, and keeps those in `later` for when its rear crosses, with the distribution where
    /// its moment has moved to the MU's entry: the hook on entry first at each point.
    fn note_entry(&mut self, table: &crate::hooks::HookTable, object: u32, mu: &crate::flows::MoveableUnit, now: crate::hooks::Points, later: crate::hooks::Points) {
        use crate::hooks::{CallKind, HookCall};
        let hooks = table.object(object);
        for at in crate::hooks::TriggerPoint::ALL {
            let bit = at.bit();
            if (now | later) & bit == 0 {
                continue;
            }
            let calls = if now & bit != 0 { &mut self.calls } else { &mut self.rears };
            if let Some(hook) = hooks.on_entry.filter(|hook| hook.triggers.points().any(|point| point == at)) {
                calls.push_back(HookCall { function: hook.function, kind: CallKind::Entry { object, mu: mu.clone(), at } });
            }
            if let (Some(function), Some(moment)) = (hooks.distribution, hooks.distribution_on_entry) {
                if moment == at {
                    calls.push_back(HookCall { function, kind: CallKind::Distribution { object, mu: mu.clone() } });
                }
            }
        }
    }

    /// The same for an object's hook on exit, the MU leaving it for `to`.
    fn note_exit(&mut self, table: &crate::hooks::HookTable, object: u32, mu: &crate::flows::MoveableUnit, to: u32, now: crate::hooks::Points, later: crate::hooks::Points) {
        use crate::hooks::{CallKind, HookCall};
        let Some(hook) = table.object(object).on_exit else { return };
        for at in hook.triggers.points() {
            let bit = at.bit();
            let calls = if now & bit != 0 {
                &mut self.calls
            } else if later & bit != 0 {
                &mut self.rears
            } else {
                continue;
            };
            calls.push_back(HookCall { function: hook.function, kind: CallKind::Exit { object, mu: mu.clone(), at, to } });
        }
    }

    /// The calls at the rear of the MU that moved last, which the conveyor its geometry belongs to
    /// keeps until its rear crosses, then hands back through `note_calls`. Empty without hooks.
    #[inline]
    pub(crate) fn take_rears(&mut self) -> std::collections::VecDeque<crate::hooks::HookCall> {
        if self.rears.is_empty() { std::collections::VecDeque::new() } else { std::mem::take(&mut self.rears) }
    }

    /// Notes calls kept for later, now that their moment has come.
    pub(crate) fn note_calls(&mut self, calls: std::collections::VecDeque<crate::hooks::HookCall>) {
        self.calls.extend(calls);
    }

    /// Whether a call asking about this MU's successor at `object` is already waiting to be made,
    /// noted at the MU's entry where the distribution's moment has moved there.
    pub(crate) fn is_asking(&self, object: u32, mu: u32) -> bool {
        self.calls.iter().any(|call| matches!(&call.kind, crate::hooks::CallKind::Distribution { object: asked, mu: asked_mu } if *asked == object && asked_mu.id == mu))
    }

    /// Notes a call asking which successor gets the MU that wants to leave `object`.
    pub fn ask(&mut self, object: u32, mu: crate::flows::MoveableUnit, function: crate::hooks::HookFunction) {
        self.calls.push_back(crate::hooks::HookCall { function, kind: crate::hooks::CallKind::Distribution { object, mu } });
    }

    /// Notes any call.
    pub(crate) fn note(&mut self, call: crate::hooks::HookCall) {
        self.calls.push_back(call);
    }

    /// Whether calls of user code are waiting to be made.
    #[inline]
    pub fn has_calls(&self) -> bool {
        !self.calls.is_empty()
    }

    /// The next call to make, first noted first.
    pub(crate) fn next_call(&mut self) -> Option<crate::hooks::HookCall> {
        self.calls.pop_front()
    }

    /// Drops the calls not made, after a run error.
    pub(crate) fn drop_calls(&mut self) {
        self.calls.clear();
        self.rears.clear();
    }

    /// The same, reading this model's work plans and destinations.
    pub fn with_routing(mut self, routing: std::sync::Arc<crate::routing::Routing>) -> EventContext {
        self.routing = routing;
        self
    }

    pub fn set_recording(&mut self, recording: Recording) {
        self.recording = recording;
        self.watching = recording != Recording::NOTHING || self.hooks.is_some();
    }

    /// Keeps this occurrence if the run is recording its kind.
    #[inline]
    pub fn record(&mut self, kind: OccurrenceKind) {
        if self.recording.includes(&kind) {
            self.occurrences.push(Occurrence { time: self.time, kind });
        }
    }

    /// Hands out an MU id that is unique across the whole model.
    pub fn new_mu_id(&mut self) -> u32 {
        let id = self.next_mu_id;
        self.next_mu_id += 1;
        id
    }

    pub fn take_occurrences(&mut self) -> Vec<Occurrence> {
        std::mem::take(&mut self.occurrences)
    }

    /// Schedules an event, `exec_time` from now. Events at one time run in the order they were
    /// scheduled, so the order of these calls is part of the result.
    #[inline]
    pub fn schedule(&mut self, event: EventTemplate) {
        self.scheduled.push(event);
    }

    /// The events scheduled since this was last called, leaving none. The run hands the list back
    /// through `return_scheduled` once it has queued them, so its memory is kept.
    pub fn take_scheduled(&mut self) -> Vec<EventTemplate> {
        std::mem::take(&mut self.scheduled)
    }

    /// Takes back the list from `take_scheduled`, emptied, to schedule into again.
    pub fn return_scheduled(&mut self, mut list: Vec<EventTemplate>) {
        list.clear();
        self.scheduled = list;
    }
}

/// Which of an object's failure profiles an event is about.
pub type ProfileIndex = u8;

#[derive(Debug, Clone)]
pub enum EventType {
    MUEntrance,
    MUExit(ExitTicket),
    /// A successor has room: `from` is that successor, which the predecessor may now try.
    BlockedRetry { from: u32 },
    Finished,
    /// A random failure of the object's failure profile at this index begins, or ends.
    StartFailure(ProfileIndex),
    EndFailure(ProfileIndex),
    PauseOn,
    PauseOff,
    /// Someone stops a Station by hand, or ends that.
    StopOn,
    StopOff,
    /// One of a conveyor's own events, for the plan it was queued in.
    Conveyor(crate::materialflow_objects::conveyor::ConveyorTicket),
    /// User code answered which successor gets the object's MU (ADR 0029): it tries to pass it on.
    /// The run hands it over at once, never through the queue.
    Decided,
    /// A timer user code started: the run calls `function` with `data`. Queued for no object.
    Timer { function: u32, data: u32 },
}

/// The object id a timer is queued for, which no object has.
pub const NO_OBJECT: u32 = u32::MAX;

/// An event an object wants to happen, `exec_time` from now.
#[derive(Debug, Clone)]
pub struct EventTemplate {
    pub eventtype: EventType,
    pub exec_time: SimTime,
    pub object_id: u32,
}

/// An event in the queue. It is on the per-event path, so it stays at 32 bytes.
#[derive(Clone, Debug)]
pub struct ScheduledEvent {
    pub time: SimTime,
    pub seq: u64,
    pub object_id: u32,
    pub eventtype: EventType,
}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.seq == other.seq
    }
}
impl Eq for ScheduledEvent {}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    /// The earliest time comes first. At one time, retries come before everything else, so
    /// those already waiting get a free successor before anything else can reach it. Within
    /// each of those two groups, events run in the order they were queued.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .time
            .cmp(&self.time)
            .then_with(|| self.is_retry().cmp(&other.is_retry()))
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl ScheduledEvent {
    fn is_retry(&self) -> bool {
        matches!(self.eventtype, EventType::BlockedRetry { .. })
    }
}

