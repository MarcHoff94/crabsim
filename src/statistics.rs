//! The fixed-size numbers every material-flow object keeps about a run. They are always on, need
//! no allocation and don't depend on occurrences. See `docs/design/statistics.md`.

use crate::flows::ProcessState;
use crate::report::Interruption;
use crate::time::SimTime;

/// What a material-flow object has done since statistics began, in raw whole numbers, so that
/// same-seed runs can be compared exactly. Fields that don't apply to a kind of object stay at
/// zero. The machine-level times add up exactly to the time since `RunState::statistics_since`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObjectStatistics {
    /// A Source's are the MUs it created. A starting MU counts at Init.
    pub mus_in: u64,
    pub mus_out: u64,
    /// All the time it worked, in three parts that don't overlap: full, not full and partly
    /// blocked. Only a Station with a capacity above 1 has the last two, and `time_busy_full`
    /// is what is left. For a Buffer, busy is holding.
    pub time_busy: SimTime,
    /// Busy with free places and nothing waiting to leave.
    pub time_busy_not_full: SimTime,
    /// Busy while at least one finished MU can't leave. Outranks full and not full.
    pub time_busy_partly_blocked: SimTime,
    pub time_blocked: SimTime,
    pub time_idle: SimTime,
    /// A place reserved for an MU a conveyor is handing over, and nothing else going on
    /// (ADR 0023).
    pub time_receiving: SimTime,
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
    pub mu_time_interrupted: SimTime,
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
        self.time_busy + self.time_blocked + self.time_idle + self.time_receiving + self.time_failed + self.time_paused
    }

    /// Busy with every place in work: all its busy time unless it has a capacity above 1.
    pub fn time_busy_full(&self) -> SimTime {
        self.time_busy.saturating_sub(self.time_busy_not_full).saturating_sub(self.time_busy_partly_blocked)
    }

    /// How many MUs the object held on average. None until any time has been measured.
    pub fn average_contents(&self) -> Option<f64> {
        let mu_time = self.mu_time_busy + self.mu_time_blocked + self.mu_time_waiting + self.mu_time_interrupted;
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
    /// Busy with every place in work, or for a Buffer, holding.
    Busy,
    /// Busy with free places, and nothing waiting to leave.
    BusyNotFull,
    /// Busy while a finished MU can't leave.
    BusyPartlyBlocked,
    Blocked,
    Idle,
    /// Receiving an MU a conveyor is handing over, with nothing else going on.
    Receiving,
    Failed,
    Paused,
}

impl MachineState {
    /// For a Station or Source: failed, paused, busy, blocked, idle.
    pub fn of(interruption: Option<Interruption>, process: &ProcessState) -> MachineState {
        match (interruption, process) {
            (Some(Interruption::Failed | Interruption::Stopped), _) => MachineState::Failed,
            (Some(Interruption::Paused), _) => MachineState::Paused,
            (None, ProcessState::Processing) => MachineState::Busy,
            (None, ProcessState::Blocked) => MachineState::Blocked,
            (None, ProcessState::Idle) => MachineState::Idle,
            (None, ProcessState::Receiving) => MachineState::Receiving,
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

    /// The same as `observe`, but hands back the stretch of time that just ended, with the MUs
    /// held through it, when one did. A Buffer feeds its fill histogram from these, so it needs no
    /// clock of its own: every stretch has one number of MUs.
    #[inline]
    pub(crate) fn observe_stretch(&mut self, now: SimTime, state: MachineState, contents: u64) -> Option<(u64, SimTime)> {
        if state == self.state && contents == self.contents {
            return None;
        }
        let stretch = (self.contents, now.saturating_sub(self.since));
        self.add_time_up_to(now);
        self.state = state;
        self.contents = contents;
        self.totals.max_contents = self.totals.max_contents.max(contents);
        Some(stretch)
    }

    /// The MUs held now, and for how long by `now`: the stretch still in progress.
    pub(crate) fn in_progress(&self, now: SimTime) -> (u64, SimTime) {
        (self.contents, now.saturating_sub(self.since))
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
        MachineState::BusyNotFull => {
            statistics.time_busy_not_full += time;
            (&mut statistics.time_busy, &mut statistics.mu_time_busy)
        }
        MachineState::BusyPartlyBlocked => {
            statistics.time_busy_partly_blocked += time;
            (&mut statistics.time_busy, &mut statistics.mu_time_busy)
        }
        MachineState::Blocked => (&mut statistics.time_blocked, &mut statistics.mu_time_blocked),
        MachineState::Idle => (&mut statistics.time_idle, &mut statistics.mu_time_waiting),
        MachineState::Receiving => (&mut statistics.time_receiving, &mut statistics.mu_time_waiting),
        MachineState::Failed => (&mut statistics.time_failed, &mut statistics.mu_time_interrupted),
        MachineState::Paused => (&mut statistics.time_paused, &mut statistics.mu_time_interrupted),
    };
    *machine += time;
    *mu += time.times(contents);
    if capacity > 0 && contents >= capacity {
        statistics.time_full += time;
    }
}

/// How many bars a Buffer's fill histogram has, whatever their width.
pub const HISTOGRAM_BARS: usize = 64;

/// How long a Buffer held each number of MUs, in `HISTOGRAM_BARS` bars, so it has the same size
/// however full the Buffer gets. Bar `i` covers `i × width` up to `(i + 1) × width − 1` MUs.
/// Automatic, the width starts at 1 and doubles whenever more MUs come than the bars reach, two
/// bars merging into one, so the bars always start at 0 and line up. Set by the model, the width
/// never changes and the last bar collects everything from `63 × width` up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    pub width: u64,
    /// Set by the model, so the last bar is open-ended.
    pub fixed: bool,
    pub bars: [SimTime; HISTOGRAM_BARS],
    /// An automatic width is a power of 2, so a bar is found by a shift rather than a division:
    /// this is on a Buffer's per-event path.
    shift: u32,
}

impl Histogram {
    /// One MU per bar to start with, doubling as needed, or `width` MUs per bar for good.
    pub fn new(width: Option<u64>) -> Histogram {
        Histogram { width: width.unwrap_or(1).max(1), fixed: width.is_some(), bars: [SimTime::ZERO; HISTOGRAM_BARS], shift: 0 }
    }

    /// Adds `time` spent holding `level` MUs.
    #[inline]
    pub(crate) fn add(&mut self, level: u64, time: SimTime) {
        // Automatic and within the bars, nearly always: one comparison.
        if !self.fixed {
            let bar = level >> self.shift;
            if bar < HISTOGRAM_BARS as u64 {
                self.bars[bar as usize] += time;
                return;
            }
        }
        let bar = self.bar_of(level);
        self.bars[bar] += time;
    }

    #[cold]
    fn bar_of(&mut self, level: u64) -> usize {
        if self.fixed {
            return (level / self.width).min(HISTOGRAM_BARS as u64 - 1) as usize;
        }
        while level >> self.shift >= HISTOGRAM_BARS as u64 {
            for bar in 0..HISTOGRAM_BARS / 2 {
                self.bars[bar] = self.bars[2 * bar] + self.bars[2 * bar + 1];
            }
            self.bars[HISTOGRAM_BARS / 2..].fill(SimTime::ZERO);
            self.width *= 2;
            self.shift += 1;
        }
        (level >> self.shift) as usize
    }

    /// The MUs bar `bar` covers, from and to, with no end for a set width's last bar.
    pub fn range(&self, bar: usize) -> (u64, Option<u64>) {
        let from = bar as u64 * self.width;
        if self.fixed && bar == HISTOGRAM_BARS - 1 {
            (from, None)
        } else {
            (from, Some(from + self.width - 1))
        }
    }

    /// All the time it covers.
    pub fn total(&self) -> SimTime {
        self.bars.iter().fold(SimTime::ZERO, |sum, &time| sum + time)
    }

    /// The bar holding the median, weighted by time: the first whose time, with all below it,
    /// reaches half the total. Exact to the bar's width. None before any time has passed.
    pub fn median_bar(&self) -> Option<usize> {
        let total = self.total();
        if total == SimTime::ZERO {
            return None;
        }
        let mut below = SimTime::ZERO;
        self.bars.iter().position(|&time| {
            below += time;
            below.as_millis() * 2 >= total.as_millis()
        })
    }

    /// The bars that have any time, each with its place, range and time, in order.
    pub fn used(&self) -> impl Iterator<Item = (usize, (u64, Option<u64>), SimTime)> + '_ {
        self.bars.iter().enumerate().filter(|(_, time)| **time > SimTime::ZERO).map(|(bar, &time)| (bar, self.range(bar), time))
    }
}

/// What only a Buffer keeps about how full it was, beyond its `ObjectStatistics`: the fewest MUs
/// it held for any length of time, and the histogram. Its mean and most MUs and its full share are
/// in `ObjectStatistics`, which every object has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillStatistics {
    /// The fewest MUs it held for some time, or holds now.
    pub lowest: u64,
    pub histogram: Histogram,
}

impl FillStatistics {
    /// The MUs the median lies between, weighted by time: one number while bars are 1 MU wide.
    pub fn median(&self) -> Option<(u64, Option<u64>)> {
        self.histogram.median_bar().map(|bar| self.histogram.range(bar))
    }
}

/// Keeps a Buffer's fill statistics from the stretches of time its `StatisticsKeeper` hands it,
/// each at one number of MUs (`observe_stretch`), so it keeps no clock of its own and costs a
/// Buffer's events only an addition when a stretch ends. Measured on the buffered line, a keeper
/// with its own clock cost 3.5% of a headless run.
#[derive(Debug, Clone)]
pub(crate) struct FillKeeper {
    width: Option<u64>,
    histogram: Histogram,
    /// `u64::MAX` until a level has been held for some time.
    lowest: u64,
}

impl FillKeeper {
    pub(crate) fn new(width: Option<u64>) -> FillKeeper {
        FillKeeper { width, histogram: Histogram::new(width), lowest: u64::MAX }
    }

    /// `time` spent holding `level` MUs.
    #[inline]
    pub(crate) fn add(&mut self, level: u64, time: SimTime) {
        if time > SimTime::ZERO {
            self.histogram.add(level, time);
            self.lowest = self.lowest.min(level);
        }
    }

    /// With the stretch `in_progress`, the MUs held now and for how long, changing nothing.
    pub(crate) fn read(&self, (level, time): (u64, SimTime)) -> FillStatistics {
        let mut histogram = self.histogram.clone();
        histogram.add(level, time);
        FillStatistics { lowest: self.lowest.min(level), histogram }
    }

    /// Starts afresh, as its `StatisticsKeeper` does when cleared or reset.
    pub(crate) fn clear(&mut self) {
        self.histogram = Histogram::new(self.width);
        self.lowest = u64::MAX;
    }

    pub(crate) fn set_width(&mut self, width: Option<u64>) {
        self.width = width;
        self.histogram = Histogram::new(width);
    }
}

/// The 97.5% quantile of Student's t distribution with `df` degrees of freedom, for a two-sided
/// 95% interval.
pub fn t_quantile(df: usize) -> f64 {
    const TABLE: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101,
        2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048, 2.045, 2.042,
    ];
    if (1..=30).contains(&df) {
        return TABLE[df - 1];
    }
    // Beyond the table, the Cornish-Fisher expansion around the normal quantile, accurate to
    // better than 0.001 from 30 degrees of freedom on.
    let z: f64 = 1.959_964;
    let n = df as f64;
    z + (z.powi(3) + z) / (4.0 * n) + (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / (96.0 * n * n)
        + (3.0 * z.powi(7) + 19.0 * z.powi(5) + 17.0 * z.powi(3) - 15.0 * z) / (384.0 * n.powi(3))
}

/// The mean and spread of one figure over complete measuring periods, kept as running sums by
/// Welford's method, so memory doesn't grow with the number of periods.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Spread {
    count: u64,
    mean: f64,
    /// The sum of squared differences from the mean.
    squares: f64,
}

impl Spread {
    pub fn add(&mut self, value: f64) {
        self.count += 1;
        let before = value - self.mean;
        self.mean += before / self.count as f64;
        self.squares += before * (value - self.mean);
    }

    /// How many periods it holds.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The mean over the periods. None before the first.
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then_some(self.mean)
    }

    /// The sample variance between periods. None before the second.
    pub fn variance(&self) -> Option<f64> {
        (self.count > 1).then(|| self.squares / (self.count - 1) as f64)
    }

    /// Half the width of the 95% confidence interval for the mean (Student's t), taking the
    /// periods as independent samples. None before the second period.
    pub fn half_width(&self) -> Option<f64> {
        let variance = self.variance()?;
        Some(t_quantile(self.count as usize - 1) * (variance / self.count as f64).sqrt())
    }
}

/// One object's figures over complete measuring periods, each period one sample. Shares are of
/// the period's length, and `mus_in_per_hour` is the MUs that came in during it, per hour.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PeriodFigures {
    pub mus_in_per_hour: Spread,
    pub busy: Spread,
    /// The three parts of busy, which add up to it.
    pub busy_full: Spread,
    pub busy_not_full: Spread,
    pub busy_partly_blocked: Spread,
    pub blocked: Spread,
    pub idle: Spread,
    pub receiving: Spread,
    pub failed: Spread,
    pub paused: Spread,
    /// Failed or paused.
    pub interrupted: Spread,
    /// Busy and full, or interrupted (failed, stopped or paused): what the likely-bottleneck rule ranks by, so a Station
    /// that works half empty doesn't look like a bottleneck.
    pub full_or_interrupted: Spread,
}

impl PeriodFigures {
    fn add(&mut self, before: &ObjectStatistics, after: &ObjectStatistics, length: SimTime) {
        let length = length.as_millis() as f64;
        let share = |of: fn(&ObjectStatistics) -> SimTime| (of(after).saturating_sub(of(before))).as_millis() as f64 / length;
        let (busy, failed, paused) = (share(|s| s.time_busy), share(|s| s.time_failed), share(|s| s.time_paused));
        let busy_full = share(|s| s.time_busy_full());
        self.mus_in_per_hour.add((after.mus_in - before.mus_in) as f64 * 3_600_000.0 / length);
        self.busy.add(busy);
        self.busy_full.add(busy_full);
        self.busy_not_full.add(share(|s| s.time_busy_not_full));
        self.busy_partly_blocked.add(share(|s| s.time_busy_partly_blocked));
        self.blocked.add(share(|s| s.time_blocked));
        self.idle.add(share(|s| s.time_idle));
        self.receiving.add(share(|s| s.time_receiving));
        self.failed.add(failed);
        self.paused.add(paused);
        self.interrupted.add(failed + paused);
        self.full_or_interrupted.add(busy_full + failed + paused);
    }
}

/// What a run measuring in periods reports: how long each is, how many are complete since
/// statistics began, and every object's figures over them, indexed by id.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuringPeriods {
    pub length: SimTime,
    pub complete: u64,
    pub objects: Vec<PeriodFigures>,
}

/// A run's measuring periods in progress: the statistics at the end of the last complete one,
/// and the figures so far. Its size doesn't grow with the number of periods.
#[derive(Debug, Clone)]
pub(crate) struct Periods {
    length: SimTime,
    /// The end of the period in progress.
    end: SimTime,
    complete: u64,
    last: Vec<ObjectStatistics>,
    figures: Vec<PeriodFigures>,
}

impl Periods {
    /// Periods of `length` from `since`, when every object's statistics were `statistics`.
    pub(crate) fn new(length: SimTime, since: SimTime, statistics: Vec<ObjectStatistics>) -> Periods {
        Periods { length, end: later(since, length), complete: 0, figures: vec![PeriodFigures::default(); statistics.len()], last: statistics }
    }

    /// The end of the period in progress.
    pub(crate) fn end(&self) -> SimTime {
        self.end
    }

    /// Completes the period in progress with every object's statistics at its end.
    pub(crate) fn close(&mut self, statistics: Vec<ObjectStatistics>) {
        for ((figures, before), after) in self.figures.iter_mut().zip(&self.last).zip(&statistics) {
            figures.add(before, after, self.length);
        }
        self.last = statistics;
        self.complete += 1;
        self.end = later(self.end, self.length);
    }

    /// About how much memory these take, for a checkpoint holding them.
    pub(crate) fn bytes(&self) -> usize {
        self.last.len() * (size_of::<ObjectStatistics>() + size_of::<PeriodFigures>())
    }

    pub(crate) fn report(&self) -> MeasuringPeriods {
        MeasuringPeriods { length: self.length, complete: self.complete, objects: self.figures.clone() }
    }
}

/// `length` after `time`, or the end of time.
fn later(time: SimTime, length: SimTime) -> SimTime {
    SimTime::from_millis(time.as_millis().saturating_add(length.as_millis()))
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
    fn busy_splits_into_full_not_full_and_partly_blocked() {
        let mut keeper = StatisticsKeeper::new(3);
        keeper.observe(SimTime::ZERO, MachineState::BusyNotFull, 1);
        keeper.observe(SimTime::from_secs(10), MachineState::Busy, 3);
        keeper.observe(SimTime::from_secs(30), MachineState::BusyPartlyBlocked, 3);
        keeper.observe(SimTime::from_secs(35), MachineState::Blocked, 1);
        let statistics = keeper.read(SimTime::from_secs(40));
        assert_eq!(statistics.time_busy, SimTime::from_secs(35));
        assert_eq!(statistics.time_busy_not_full, SimTime::from_secs(10));
        assert_eq!(statistics.time_busy_partly_blocked, SimTime::from_secs(5));
        assert_eq!(statistics.time_busy_full(), SimTime::from_secs(20));
        assert_eq!(statistics.time_blocked, SimTime::from_secs(5));
        assert_eq!(statistics.measured_time(), SimTime::from_secs(40));
    }

    /// A Buffer's two keepers, fed as a Buffer feeds them.
    struct Fill {
        keeper: StatisticsKeeper,
        fill: FillKeeper,
    }

    impl Fill {
        fn new(width: Option<u64>) -> Fill {
            Fill { keeper: StatisticsKeeper::new(0), fill: FillKeeper::new(width) }
        }
        fn observe(&mut self, now: SimTime, level: u64) {
            if let Some((held, time)) = self.keeper.observe_stretch(now, MachineState::Busy, level) {
                self.fill.add(held, time);
            }
        }
        fn read(&self, now: SimTime) -> FillStatistics {
            self.fill.read(self.keeper.in_progress(now))
        }
        fn clear(&mut self, now: SimTime) {
            self.keeper.clear(now);
            self.fill.clear();
        }
    }

    #[test]
    fn an_automatic_histogram_doubles_its_bars_and_a_set_one_collects_the_rest_in_its_last() {
        let mut fill = Fill::new(None);
        fill.observe(SimTime::ZERO, 3);
        fill.observe(SimTime::from_secs(10), 63);
        fill.observe(SimTime::from_secs(15), 64);
        let read = fill.read(SimTime::from_secs(20));
        assert_eq!(read.histogram.width, 2, "64 MUs don't fit 64 bars of one");
        assert_eq!(read.histogram.bars[1], SimTime::from_secs(10), "3 is in the bar of 2 and 3");
        assert_eq!(read.histogram.bars[31], SimTime::from_secs(5), "62 and 63");
        assert_eq!(read.histogram.bars[32], SimTime::from_secs(5), "64 and 65");
        assert_eq!(read.histogram.total(), SimTime::from_secs(20));
        assert_eq!(read.lowest, 3);
        assert_eq!(read.median(), Some((2, Some(3))), "half the time at 3, exact to the bar");

        let mut fill = Fill::new(Some(5));
        fill.observe(SimTime::ZERO, 7);
        fill.observe(SimTime::from_secs(4), 1_000);
        let read = fill.read(SimTime::from_secs(10));
        assert_eq!(read.histogram.width, 5);
        assert_eq!(read.histogram.range(63), (315, None), "315 or more");
        assert_eq!((read.histogram.bars[1], read.histogram.bars[63]), (SimTime::from_secs(4), SimTime::from_secs(6)));
        assert_eq!(read.median(), Some((315, None)));
    }

    #[test]
    fn the_lowest_fill_is_the_fewest_held_for_some_time() {
        let mut fill = Fill::new(None);
        fill.observe(SimTime::ZERO, 4);
        // Empty for no time at all, as when an MU leaves and another comes at one instant.
        fill.observe(SimTime::from_secs(10), 0);
        fill.observe(SimTime::from_secs(10), 2);
        assert_eq!(fill.read(SimTime::from_secs(12)).lowest, 2);
        assert_eq!(fill.read(SimTime::from_secs(10)).lowest, 2, "it holds 2 now");
        fill.clear(SimTime::from_secs(12));
        let cleared = fill.read(SimTime::from_secs(13));
        assert_eq!((cleared.lowest, cleared.histogram.total()), (2, SimTime::from_secs(1)));
        assert_eq!(Fill::new(None).read(SimTime::ZERO).median(), None);
    }

    #[test]
    fn interrupted_time_goes_to_the_cause_and_mu_time_to_interrupted() {
        let mut keeper = StatisticsKeeper::new(1);
        keeper.observe(SimTime::ZERO, MachineState::Failed, 1);
        keeper.observe(SimTime::from_secs(10), MachineState::Paused, 1);
        let statistics = keeper.read(SimTime::from_secs(15));
        assert_eq!(statistics.time_failed, SimTime::from_secs(10));
        assert_eq!(statistics.time_paused, SimTime::from_secs(5));
        assert_eq!(statistics.mu_time_interrupted, SimTime::from_secs(15));
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

    #[test]
    fn a_spread_gives_the_mean_variance_and_interval_of_its_values() {
        let mut spread = Spread::default();
        assert_eq!((spread.mean(), spread.variance(), spread.half_width()), (None, None, None));
        for value in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            spread.add(value);
        }
        assert_eq!(spread.count(), 8);
        assert!((spread.mean().unwrap() - 5.0).abs() < 1e-12);
        assert!((spread.variance().unwrap() - 32.0 / 7.0).abs() < 1e-12);
        let half = 2.365 * (32.0 / 7.0 / 8.0f64).sqrt();
        assert!((spread.half_width().unwrap() - half).abs() < 1e-12);
    }
}
