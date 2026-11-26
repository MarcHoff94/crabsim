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
                self.moved_hooked(mu, from, to);
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn moved_hooked(&mut self, mu: &crate::flows::MoveableUnit, from: u32, to: u32) {
        self.left_with_hooks(from, mu, to);
        if let Some(hook) = self.entry_hook(to) {
            // The MU as it comes in: reaching its next destination moves its plan on.
            let mut arrived = mu.clone();
            if arrived.is_routed() {
                self.routing.arrive(&mut arrived, to);
            }
            self.note_entry(to, &arrived, hook);
        }
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

    /// The object's hook on entry, if it has one.
    #[inline]
    pub fn entry_hook(&self, object: u32) -> Option<crate::hooks::Hook> {
        self.hooks.as_ref().and_then(|table| table.object(object).on_entry)
    }

    /// Notes the calls of an object's hook on entry for the MU that came in, one for each of its
    /// trigger points.
    #[cold]
    pub fn note_entry(&mut self, object: u32, mu: &crate::flows::MoveableUnit, hook: crate::hooks::Hook) {
        for at in hook.triggers.points() {
            self.calls.push_back(crate::hooks::HookCall { function: hook.function, kind: crate::hooks::CallKind::Entry { object, mu: mu.clone(), at } });
        }
    }

    #[cold]
    #[inline(never)]
    fn entered_with_hooks(&mut self, object: u32, mu: &crate::flows::MoveableUnit) {
        if let Some(hook) = self.entry_hook(object) {
            self.note_entry(object, mu, hook);
        }
    }

    #[cold]
    #[inline(never)]
    fn left_with_hooks(&mut self, object: u32, mu: &crate::flows::MoveableUnit, to: u32) {
        if let Some(hook) = self.hooks.as_ref().and_then(|table| table.object(object).on_exit) {
            self.note_exit(object, mu, to, hook);
        }
    }

    fn note_exit(&mut self, object: u32, mu: &crate::flows::MoveableUnit, to: u32, hook: crate::hooks::Hook) {
        for at in hook.triggers.points() {
            self.calls.push_back(crate::hooks::HookCall { function: hook.function, kind: crate::hooks::CallKind::Exit { object, mu: mu.clone(), at, to } });
        }
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

