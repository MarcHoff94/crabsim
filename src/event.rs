#![allow(dead_code)]

use std::{
    cmp::Ordering,
    ops::{Add, Div, Sub},
};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EcTime(pub u128);

impl std::iter::Sum for EcTime {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(EcTime(0), |acc, x| EcTime(acc.0 + x.0))
    }
}

impl<'a> std::iter::Sum<&'a EcTime> for EcTime {
    fn sum<I: Iterator<Item = &'a EcTime>>(iter: I) -> Self {
        iter.fold(EcTime(0), |acc, x| EcTime(acc.0 + x.0))
    }
}

impl Add<u128> for EcTime {
    type Output = EcTime;
    fn add(self, other: u128) -> EcTime {
        EcTime(self.0 + other)
    }
}

impl Add<EcTime> for EcTime {
    type Output = EcTime;
    fn add(self, other: EcTime) -> EcTime {
        EcTime(self.0 + other.0)
    }
}
impl Sub<EcTime> for EcTime {
    type Output = EcTime;
    fn sub(self, other: EcTime) -> EcTime {
        EcTime(self.0 - other.0)
    }
}
impl Sub<u128> for EcTime {
    type Output = EcTime;
    fn sub(self, other: u128) -> EcTime {
        EcTime(self.0 - other)
    }
}
impl Div<u128> for EcTime {
    type Output = EcTime;
    fn div(self, other: u128) -> EcTime {
        EcTime(self.0 / other)
    }
}

impl Ord for EcTime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for EcTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<u128> for EcTime {
    fn from(value: u128) -> Self {
        EcTime(value)
    }
}

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
    pub exec_time: EcTime,
    pub object_id: u32,
    pub seq: u64,
}

#[derive(Clone, Debug)]
pub struct ScheduledEvent {
    pub time: EcTime,
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

// Errors moved to `src/error.rs` to centralize error types for the crate.
