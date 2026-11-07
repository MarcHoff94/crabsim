//! Simulation time, as a count of whole milliseconds.

use std::{
    fmt,
    ops::{Add, AddAssign},
};

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

    /// This length of time, `count` times over.
    pub const fn times(self, count: u64) -> SimTime {
        SimTime(self.0 * count)
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

impl AddAssign for SimTime {
    fn add_assign(&mut self, other: SimTime) {
        self.0 += other.0;
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

/// Simulation time as hours, minutes and seconds, with days in front once there are any.
pub fn format_time(time: SimTime) -> String {
    let secs = time.as_secs();
    let (days, hours, minutes, seconds) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60, secs % 60);
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// Simulation time typed as `format_time` writes it: seconds, minutes and seconds, or hours,
/// minutes and seconds, with days in front as `2d`, and a fraction of a second at the end.
/// Minutes and seconds after the first field are below 60. `None` if it isn't a time.
pub fn parse_time(typed: &str) -> Option<SimTime> {
    let typed = typed.trim();
    let (days, clock) = match typed.split_once('d') {
        Some((days, rest)) => (days.trim().parse::<u64>().ok()?, rest.trim()),
        None => (0, typed),
    };
    let mut millis = days.checked_mul(86_400_000)?;
    if clock.is_empty() {
        // Days alone, but not nothing at all.
        return (typed != clock).then_some(SimTime::from_millis(millis));
    }
    let fields: Vec<&str> = clock.split(':').collect();
    if fields.len() > 3 {
        return None;
    }
    let (whole, last) = fields.split_at(fields.len() - 1);
    let seconds: f64 = last[0].parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.0)?;
    if !whole.is_empty() && seconds >= 60.0 {
        return None;
    }
    for (index, field) in whole.iter().enumerate() {
        let value: u64 = field.parse().ok()?;
        if index > 0 && value >= 60 {
            return None;
        }
        // The field two from the end is hours, the one before the seconds minutes.
        let unit = if whole.len() - index == 2 { 3_600_000 } else { 60_000 };
        millis = millis.checked_add(value.checked_mul(unit)?)?;
    }
    millis.checked_add((seconds * 1000.0).round() as u64).map(SimTime::from_millis)
}

/// As `format_time`, with milliseconds when there are any.
pub fn format_time_exact(time: SimTime) -> String {
    match time.subsec_millis() {
        0 => format_time(time),
        millis => format!("{}.{millis:03}", format_time(time)),
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

    #[test]
    fn shown_time_counts_hours_and_days() {
        assert_eq!(format_time(SimTime::from_millis(59_999)), "00:00:59");
        assert_eq!(format_time(SimTime::from_secs(3_600 * 5 + 61)), "05:01:01");
        assert_eq!(format_time(SimTime::from_secs(86_400 * 400 + 3_661)), "400d 01:01:01");
    }

    #[test]
    fn times_are_typed_as_they_are_shown() {
        let secs = |s: u64| Some(SimTime::from_secs(s));
        assert_eq!(parse_time("90"), secs(90));
        assert_eq!(parse_time("1:30"), secs(90));
        assert_eq!(parse_time(" 01:02:03 "), secs(3_723));
        assert_eq!(parse_time("2d 01:00:00"), secs(2 * 86_400 + 3_600));
        assert_eq!(parse_time("3d"), secs(3 * 86_400));
        assert_eq!(parse_time("00:00:12.5"), Some(SimTime::from_millis(12_500)));
        assert_eq!(parse_time(&format_time(SimTime::from_secs(400 * 86_400 + 59))), secs(400 * 86_400 + 59));
        assert_eq!(parse_time("1:60"), None, "60 seconds is a minute");
        assert_eq!(parse_time("1:2:3:4"), None);
        assert_eq!(parse_time("-5"), None);
        assert_eq!(parse_time(""), None);
    }
}
