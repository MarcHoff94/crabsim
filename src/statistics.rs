//! The fixed-size numbers every material-flow object keeps about a run. They are always on, need
//! no allocation and don't depend on occurrences. See `docs/design/statistics.md`.

use crate::flows::{InterruptionState, ProcessState};
use crate::time::SimTime;

/// What a material-flow object has done since statistics began, in raw whole numbers, so that
/// same-seed runs can be compared exactly. Fields that don't apply to a kind of object stay at
/// zero. The machine-level times add up exactly to the time since `RunState::statistics_since`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObjectStatistics {
    /// A Source's are the MUs it created. A starting MU counts at Init.
    pub mus_in: u64,
    pub mus_out: u64,
    pub time_busy: SimTime,
    pub time_blocked: SimTime,
    pub time_idle: SimTime,
    pub time_failed: SimTime,
    pub time_paused: SimTime,
    /// Time spent at capacity. It says how full the object is rather than what it is doing, so
    /// it doesn't add up with the times above.
    pub time_full: SimTime,
    pub failures: u64,
    pub pauses: u64,
    /// A failure overlapping a pause is one stop.
    pub stops: u64,
    pub max_contents: u64,
    pub mu_time_busy: SimTime,
    pub mu_time_blocked: SimTime,
    pub mu_time_waiting: SimTime,
    pub mu_time_stopped: SimTime,
    /// Only a Drain has these.
    pub drain: Option<DrainStatistics>,
}

/// What only a Drain keeps. Its count of MUs is `ObjectStatistics::mus_in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainStatistics {
    pub lead_time_total: SimTime,
    pub lead_time_min: Option<SimTime>,
    pub lead_time_max: Option<SimTime>,
    pub first_exit: Option<SimTime>,
    pub last_exit: Option<SimTime>,
    pub exit_interval_min: Option<SimTime>,
    /// Includes the interval still in progress when read, so a jam shows here too.
    pub exit_interval_max: Option<SimTime>,
}

impl ObjectStatistics {
    /// The time these numbers cover.
    pub fn measured_time(&self) -> SimTime {
        self.time_busy + self.time_blocked + self.time_idle + self.time_failed + self.time_paused
    }

    /// How many MUs the object held on average. None until any time has been measured.
    pub fn average_contents(&self) -> Option<f64> {
        let mu_time = self.mu_time_busy + self.mu_time_blocked + self.mu_time_waiting + self.mu_time_stopped;
        ratio(mu_time.as_millis() as f64, self.measured_time().as_millis() as f64)
    }

    /// Up time over up time plus failed time, on the Simulation time basis: the only time basis
    /// built so far. None until any time has been measured.
    pub fn achieved_availability(&self) -> Option<f64> {
        let total = self.measured_time();
        ratio(total.saturating_sub(self.time_failed).as_millis() as f64, total.as_millis() as f64)
    }

    /// Mean lead time of the MUs that have entered this Drain. None for anything else, or
    /// before the first MU.
    pub fn lead_time_mean(&self) -> Option<SimTime> {
        let drain = self.drain?;
        (self.mus_in > 0).then(|| SimTime::from_millis_f64(drain.lead_time_total.as_millis() as f64 / self.mus_in as f64))
    }

    /// Throughput from this Drain's first exit up to `now`, so the rate falls while no MUs arrive
    /// and a jammed line can't look healthy. None for anything else, or before two exits.
    pub fn mus_per_hour(&self, now: SimTime) -> Option<f64> {
        let first_exit = self.drain?.first_exit?;
        if self.mus_in < 2 {
            return None;
        }
        ratio((self.mus_in - 1) as f64 * 3_600_000.0, now.saturating_sub(first_exit).as_millis() as f64)
    }
}

fn ratio(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| part / whole)
}

/// The one thing an object is doing at any moment, by the priority in the statistics design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineState {
    Busy,
    Blocked,
    Idle,
    Failed,
    Paused,
}

impl MachineState {
    /// For a Station or Source: failed, paused, busy, blocked, idle.
    pub fn of(interruption: &InterruptionState, process: &ProcessState) -> MachineState {
        match (interruption, process) {
            (InterruptionState::Failed, _) => MachineState::Failed,
            (InterruptionState::Paused, _) => MachineState::Paused,
            (InterruptionState::None, ProcessState::Processing) => MachineState::Busy,
            (InterruptionState::None, ProcessState::Blocked) => MachineState::Blocked,
            (InterruptionState::None, ProcessState::Idle) => MachineState::Idle,
        }
    }
}

/// Keeps one object's statistics. Each object tells it the state it ended up in after everything
/// it handles. Time doesn't move within an event, so this catches every change of state, and
/// all time goes through one place, which keeps the buckets from drifting.
#[derive(Debug, Clone)]
pub struct StatisticsKeeper {
    /// Up to `since`. Reading adds the state still in progress.
    totals: ObjectStatistics,
    state: MachineState,
    contents: u64,
    /// 0 for an object that has no MU-level statistics.
    capacity: u64,
    since: SimTime,
}

impl StatisticsKeeper {
    pub fn new(capacity: u64) -> StatisticsKeeper {
        StatisticsKeeper { totals: ObjectStatistics::default(), state: MachineState::Idle, contents: 0, capacity, since: SimTime::ZERO }
    }

    pub fn for_drain() -> StatisticsKeeper {
        let mut keeper = StatisticsKeeper::new(0);
        keeper.totals.drain = Some(DrainStatistics::default());
        keeper
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Takes note of what the object is doing as of `now`. Costs one comparison if nothing changed.
    #[inline]
    pub fn observe(&mut self, now: SimTime, state: MachineState, contents: u64) {
        if state == self.state && contents == self.contents {
            return;
        }
        self.add_time_up_to(now);
        self.state = state;
        self.contents = contents;
        self.totals.max_contents = self.totals.max_contents.max(contents);
    }

    pub fn count_mu_in(&mut self) {
        self.totals.mus_in += 1;
    }

    pub fn count_mu_out(&mut self) {
        self.totals.mus_out += 1;
    }

    pub fn count_failure(&mut self) {
        self.totals.failures += 1;
    }

    pub fn count_pause(&mut self) {
        self.totals.pauses += 1;
    }

    /// A failure overlapping a pause is one stop, so this counts separately from them.
    pub fn count_stop(&mut self) {
        self.totals.stops += 1;
    }

    /// An MU entered this Drain at `now`, `lead_time` after it left its Source.
    pub fn count_exit(&mut self, now: SimTime, lead_time: SimTime) {
        self.totals.mus_in += 1;
        let Some(drain) = &mut self.totals.drain else { return };
        drain.lead_time_total += lead_time;
        drain.lead_time_min = Some(drain.lead_time_min.map_or(lead_time, |shortest| shortest.min(lead_time)));
        drain.lead_time_max = Some(drain.lead_time_max.map_or(lead_time, |longest| longest.max(lead_time)));
        if let Some(last_exit) = drain.last_exit {
            let interval = now.saturating_sub(last_exit);
            drain.exit_interval_min = Some(drain.exit_interval_min.map_or(interval, |shortest| shortest.min(interval)));
            drain.exit_interval_max = Some(drain.exit_interval_max.map_or(interval, |longest| longest.max(interval)));
        }
        drain.first_exit.get_or_insert(now);
        drain.last_exit = Some(now);
    }

    /// The numbers as of `now`, including the state in progress, without changing anything. So
    /// they are the same however an advance was split up.
    pub fn read(&self, now: SimTime) -> ObjectStatistics {
        let mut statistics = self.totals;
        add_time(&mut statistics, self.state, self.contents, self.capacity, now.saturating_sub(self.since));
        if let Some(drain) = &mut statistics.drain {
            if let Some(last_exit) = drain.last_exit {
                let in_progress = now.saturating_sub(last_exit);
                drain.exit_interval_max = Some(drain.exit_interval_max.map_or(in_progress, |longest| longest.max(in_progress)));
            }
        }
        statistics
    }

    /// Starts measuring afresh from `now`. What the object is doing carries on. A Drain forgets
    /// its exits, so no exit interval spans the time before this.
    pub fn clear(&mut self, now: SimTime) {
        self.totals = ObjectStatistics {
            max_contents: self.contents,
            drain: self.totals.drain.map(|_| DrainStatistics::default()),
            ..ObjectStatistics::default()
        };
        self.since = now;
    }

    /// Back to how it was before Init.
    pub fn reset(&mut self) {
        self.state = MachineState::Idle;
        self.contents = 0;
        self.clear(SimTime::ZERO);
    }

    fn add_time_up_to(&mut self, now: SimTime) {
        add_time(&mut self.totals, self.state, self.contents, self.capacity, now.saturating_sub(self.since));
        self.since = now;
    }
}

fn add_time(statistics: &mut ObjectStatistics, state: MachineState, contents: u64, capacity: u64, time: SimTime) {
    let (machine, mu) = match state {
        MachineState::Busy => (&mut statistics.time_busy, &mut statistics.mu_time_busy),
        MachineState::Blocked => (&mut statistics.time_blocked, &mut statistics.mu_time_blocked),
        MachineState::Idle => (&mut statistics.time_idle, &mut statistics.mu_time_waiting),
        MachineState::Failed => (&mut statistics.time_failed, &mut statistics.mu_time_stopped),
        MachineState::Paused => (&mut statistics.time_paused, &mut statistics.mu_time_stopped),
    };
    *machine += time;
    *mu += time.times(contents);
    if capacity > 0 && contents >= capacity {
        statistics.time_full += time;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn achieved_availability_on_the_simulation_time_basis() {
        // Over 1000s: failed 100s, paused 200s, no overlap. The design's example gives 90%.
        let statistics = ObjectStatistics {
            time_busy: SimTime::from_secs(700),
            time_failed: SimTime::from_secs(100),
            time_paused: SimTime::from_secs(200),
            ..ObjectStatistics::default()
        };
        assert_eq!(statistics.achieved_availability(), Some(0.9));
        assert_eq!(ObjectStatistics::default().achieved_availability(), None);
    }

    #[test]
    fn a_capacity_two_object_counts_each_mu_at_mu_level() {
        let mut keeper = StatisticsKeeper::new(2);
        keeper.observe(SimTime::ZERO, MachineState::Busy, 1);
        keeper.observe(SimTime::from_secs(10), MachineState::Busy, 2);
        let statistics = keeper.read(SimTime::from_secs(30));
        assert_eq!(statistics.time_busy, SimTime::from_secs(30));
        assert_eq!(statistics.mu_time_busy, SimTime::from_secs(50));
        assert_eq!(statistics.time_full, SimTime::from_secs(20));
        assert_eq!(statistics.max_contents, 2);
        assert_eq!(statistics.average_contents(), Some(50.0 / 30.0));
    }

    #[test]
    fn stopped_time_goes_to_the_cause_and_mu_time_to_stopped() {
        let mut keeper = StatisticsKeeper::new(1);
        keeper.observe(SimTime::ZERO, MachineState::Failed, 1);
        keeper.observe(SimTime::from_secs(10), MachineState::Paused, 1);
        let statistics = keeper.read(SimTime::from_secs(15));
        assert_eq!(statistics.time_failed, SimTime::from_secs(10));
        assert_eq!(statistics.time_paused, SimTime::from_secs(5));
        assert_eq!(statistics.mu_time_stopped, SimTime::from_secs(15));
    }

    #[test]
    fn exits_give_the_rates_in_the_design_example() {
        let mut keeper = StatisticsKeeper::for_drain();
        for at in [70, 130, 190] {
            keeper.count_exit(SimTime::from_secs(at), SimTime::from_secs(60));
        }
        let at = |secs| keeper.read(SimTime::from_secs(secs)).mus_per_hour(SimTime::from_secs(secs)).unwrap().round();
        assert_eq!(at(190), 60.0);
        assert_eq!(at(200), 55.0);
        assert_eq!(at(250), 40.0);
        assert_eq!(at(3_600), 2.0);
    }

    #[test]
    fn there_is_no_rate_before_two_exits() {
        let mut keeper = StatisticsKeeper::for_drain();
        let now = SimTime::from_secs(100);
        assert_eq!(keeper.read(now).mus_per_hour(now), None);
        keeper.count_exit(SimTime::from_secs(70), SimTime::from_secs(60));
        assert_eq!(keeper.read(now).mus_per_hour(now), None);
    }

    #[test]
    fn the_longest_exit_interval_includes_the_one_in_progress_and_the_shortest_does_not() {
        let mut keeper = StatisticsKeeper::for_drain();
        keeper.count_exit(SimTime::from_secs(70), SimTime::from_secs(1));
        keeper.count_exit(SimTime::from_secs(130), SimTime::from_secs(1));
        let drain = keeper.read(SimTime::from_secs(500)).drain.unwrap();
        assert_eq!(drain.exit_interval_max, Some(SimTime::from_secs(370)));
        assert_eq!(drain.exit_interval_min, Some(SimTime::from_secs(60)));
    }
}
