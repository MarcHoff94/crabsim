//! Simulation time, as a count of whole milliseconds.

use std::{fmt, ops::Add};

/// A point in simulation time, or a length of it. Wall time stays a `std::time::Duration`, so the
/// compiler keeps the two apart. 64 bits of milliseconds cover over 500 million years, and a
/// queue entry only needs half the room a `Duration` would take.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimTime(u64);

impl SimTime {
    pub const ZERO: SimTime = SimTime(0);
    pub const MAX: SimTime = SimTime(u64::MAX);

    pub const fn from_millis(millis: u64) -> SimTime {
        SimTime(millis)
    }

    pub const fn from_secs(secs: u64) -> SimTime {
        SimTime(secs * 1_000)
    }

    /// The nearest whole millisecond, which is how sampled times enter the simulation. Anything
    /// below zero becomes zero, and anything too large becomes `MAX`.
    pub fn from_millis_f64(millis: f64) -> SimTime {
        SimTime(millis.round() as u64)
    }

    pub fn from_secs_f64(secs: f64) -> SimTime {
        SimTime::from_millis_f64(secs * 1_000.0)
    }

    pub const fn as_millis(self) -> u64 {
        self.0
    }

    pub const fn as_secs(self) -> u64 {
        self.0 / 1_000
    }

    pub const fn subsec_millis(self) -> u64 {
        self.0 % 1_000
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1_000.0
    }

    pub const fn saturating_add(self, other: SimTime) -> SimTime {
        SimTime(self.0.saturating_add(other.0))
    }

    pub const fn saturating_sub(self, other: SimTime) -> SimTime {
        SimTime(self.0.saturating_sub(other.0))
    }
}

impl Add for SimTime {
    type Output = SimTime;

    fn add(self, other: SimTime) -> SimTime {
        SimTime(self.0 + other.0)
    }
}

/// Seconds with three decimals, such as `30.000`.
impl fmt::Display for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&format!("{}.{:03}", self.as_secs(), self.subsec_millis()))
    }
}

impl fmt::Debug for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_round_to_the_nearest_millisecond() {
        assert_eq!(SimTime::from_millis_f64(1_500.4), SimTime::from_millis(1_500));
        assert_eq!(SimTime::from_millis_f64(1_500.6), SimTime::from_millis(1_501));
        assert_eq!(SimTime::from_secs_f64(1.5), SimTime::from_millis(1_500));
    }

    #[test]
    fn times_out_of_range_saturate() {
        assert_eq!(SimTime::from_millis_f64(-1.0), SimTime::ZERO);
        assert_eq!(SimTime::from_millis_f64(f64::NAN), SimTime::ZERO);
        assert_eq!(SimTime::from_secs_f64(f64::MAX), SimTime::MAX);
        assert_eq!(SimTime::MAX.saturating_add(SimTime::from_secs(1)), SimTime::MAX);
        assert_eq!(SimTime::ZERO.saturating_sub(SimTime::from_secs(1)), SimTime::ZERO);
    }

    #[test]
    fn it_prints_as_seconds_with_three_decimals() {
        assert_eq!(SimTime::from_millis(30_005).to_string(), "30.005");
        assert_eq!(format!("{:8}|", SimTime::from_secs(1)), "1.000   |");
        assert_eq!(format!("{:?}", SimTime::from_millis(1_500)), "1.500s");
    }
}
