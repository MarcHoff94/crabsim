#![allow(dead_code)]

use std::{
    cmp::Ordering,
    time::Duration,
};

// EcTime removed: use std::time::Duration everywhere

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

#[derive(Debug, Clone)]
pub enum EventType {
    MUEntrance,
    MUExit(ExitTicket),
    BlockedRetry,
    Finished,
    StartFailure,
    EndFailure,
}

#[derive(Debug, Clone)]
pub struct EventTemplate {
    pub eventtype: EventType,
    pub exec_time: Duration,
    pub object_id: u32,
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct ScheduledEvent {
    pub time: Duration,
    pub template: EventTemplate,
    pub seq: u64,
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
    fn cmp(&self, other: &Self) -> Ordering {
        let tcmp = other.time.cmp(&self.time);
        if tcmp != Ordering::Equal {
            return tcmp;
        }
        other.seq.cmp(&self.seq)
    }
}

