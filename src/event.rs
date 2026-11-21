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
/// time, a place to record occurrences, and new MU identities.
#[derive(Clone)]
pub struct EventContext {
    pub time: SimTime,
    pub seq: u64,
    next_mu_id: u32,
    recording: Recording,
    occurrences: Vec<Occurrence>,
    /// The events the object in hand schedules, in order. The run moves them into its queue
    /// after each event and keeps the memory, so scheduling allocates nothing once it has grown.
    scheduled: Vec<EventTemplate>,
}

impl EventContext {
    pub fn new(first_mu_id: u32, recording: Recording) -> EventContext {
        EventContext { time: SimTime::ZERO, seq: 0, next_mu_id: first_mu_id, recording, occurrences: Vec::new(), scheduled: Vec::new() }
    }

    pub fn set_recording(&mut self, recording: Recording) {
        self.recording = recording;
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
}

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

