#![allow(dead_code)]

use std::{
    cmp::Ordering,
    time::Duration,
};

// EcTime removed: use std::time::Duration everywhere

#[derive(Debug, Clone)]
pub enum EventType {
    MUEntrance,
    MUExit,
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

