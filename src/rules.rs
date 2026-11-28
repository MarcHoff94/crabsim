//! Rules (ADR 0030): a decision written as when/then lines that an object reads in order when it
//! passes an MU on, the first line whose condition holds deciding. Between the built-in strategies
//! and user code: readable and checkable as the built-in ones are, without a program. A model file
//! writes them as text (`crabsim_model::rules`), which this module never sees: it holds them by
//! id, ready to run.
//!
//! A rule reads the MU it decides for (its class and next destination), the time of day, and
//! other objects' figures now, since statistics began or over a stretch before now, kept in buckets
//! by the objects it names (`StatisticsKeeper::watch`). Its answer is an order of exits to offer
//! the MU in, waiting in each one's line, as the built-in strategies' are: the best by a figure
//! first among those free, or the best alone to wait for, a named successor, round robin among
//! some, or the object's own strategy. Only objects with rules pay for them, and they pay their
//! lines per decision.

use crate::figures::{Figure, Unit};
use crate::materialflow_objects::flows::{DestinationId, MoveableUnit, MuKindId};
use crate::materialflow_objects::{ModelObject, OtherObjects};
use crate::statistics::{MachineState, ObjectStatistics};
use crate::time::SimTime;
use crate::ObjectKind;
use std::sync::Arc;

/// A comparison in a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    AtLeast,
    AtMost,
    Above,
    Below,
    Equal,
    NotEqual,
}

impl Compare {
    pub fn holds(self, left: f64, right: f64) -> bool {
        match self {
            Compare::AtLeast => left >= right,
            Compare::AtMost => left <= right,
            Compare::Above => left > right,
            Compare::Below => left < right,
            Compare::Equal => left == right,
            Compare::NotEqual => left != right,
        }
    }
}

/// Over what time a figure is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    /// As it is now: the MUs held, or the state.
    Now,
    /// Since statistics began, at Init or after the warm-up.
    SinceStart,
    /// Over the stretch of this length before now, kept in buckets: up to one bucket longer.
    Last(SimTime),
}

/// A figure of an object, as a rule reads it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    pub object: u32,
    pub figure: Figure,
    pub scope: RuleScope,
}

/// One side of a comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operand {
    Figure(Reading),
    /// In the figure's own unit: a share from 0 to 1, MUs an hour, seconds, a count.
    Number(f64),
}

/// A state a rule compares an object's with, as the statistics keep it: a stop by hand is failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateName {
    Busy,
    Blocked,
    Idle,
    Receiving,
    Failed,
    Paused,
}

impl StateName {
    fn of(state: MachineState) -> StateName {
        match state {
            MachineState::Busy | MachineState::BusyNotFull | MachineState::BusyPartlyBlocked => StateName::Busy,
            MachineState::Blocked => StateName::Blocked,
            MachineState::Idle => StateName::Idle,
            MachineState::Receiving => StateName::Receiving,
            MachineState::Failed => StateName::Failed,
            MachineState::Paused => StateName::Paused,
        }
    }
}

/// One condition of a rule.
#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    /// The MU's class, by its kind, is (or isn't) one of these.
    Class { kinds: Vec<MuKindId>, equal: bool },
    /// The MU's next destination is (or isn't) this one.
    Destination { destination: DestinationId, equal: bool },
    /// The time of day, from the run's start, against a time between 0 and a day.
    TimeOfDay { compare: Compare, time: SimTime },
    /// A figure against a value or another figure. A figure with no value, such as a share
    /// measured over no time, holds nothing.
    Compare { left: Reading, compare: Compare, right: Operand },
    /// An object's state now is (or isn't) this one.
    State { object: u32, state: StateName, equal: bool },
}

/// What a rule decides.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Offers the MU to the successor at this exit alone, waiting for it.
    To(usize),
    /// Offers the MU to the exits in `among`, or all, by a figure each reads, the smallest or
    /// `largest` first, ties by exit order. With `wait`, only the best, which the MU waits for;
    /// otherwise every one in that order, the first free taking it.
    Best { figure: Figure, scope: RuleScope, largest: bool, among: Vec<usize>, wait: bool },
    /// Round robin among the exits in `among`, the object's turn going on as by its own strategy.
    RoundRobin { among: Vec<usize> },
    /// The object's own ranking among the exits in `among`: it must be weighted.
    Weighted { among: Vec<usize> },
    /// The object's own strategy.
    Default,
}

/// One line: `when` the conditions hold, in groups any of which holding with all its conditions,
/// `then` the action. A line with no condition always holds.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleLine {
    /// Conditions joined by `or` between groups and `and` within one.
    pub when: Vec<Vec<Condition>>,
    pub then: Action,
}

/// An object's rules, the first line that holds deciding, and when none holds, its own strategy.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rules {
    pub lines: Vec<RuleLine>,
}

impl Rules {
    /// Every object and stretch the rules read over a stretch, so those objects keep them.
    pub fn stretches(&self, successors: &[u32]) -> Vec<(u32, SimTime)> {
        let mut found = Vec::new();
        for line in &self.lines {
            for condition in line.when.iter().flatten() {
                if let Condition::Compare { left, right, .. } = condition {
                    for reading in std::iter::once(left).chain(match right {
                        Operand::Figure(right) => Some(right),
                        Operand::Number(_) => None,
                    }) {
                        if let RuleScope::Last(length) = reading.scope {
                            found.push((reading.object, length));
                        }
                    }
                }
            }
            if let Action::Best { scope: RuleScope::Last(length), among, .. } = &line.then {
                let exits: Vec<usize> = if among.is_empty() { (0..successors.len()).collect() } else { among.clone() };
                for exit in exits {
                    if let Some(&object) = successors.get(exit) {
                        found.push((object, *length));
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        found
    }
}

/// An object's rules as it runs them, with its own strategy for `default` and for when no line
/// holds.
#[derive(Debug, Clone)]
pub struct Ruled {
    pub rules: Arc<Rules>,
    pub builtin: crate::materialflow_objects::flows::Distributing,
    /// Each exit's place in each line's `among` for round robin and weighted, kept so a decision
    /// allocates nothing: a mask of the exits it may take, by line.
    pub(crate) masks: Vec<Vec<bool>>,
    /// The exits a decision offers the MU to, in order, reused from one decision to the next.
    pub(crate) order: Vec<usize>,
    /// Each exit's value while a decision sorts them, reused likewise.
    values: Vec<(usize, Option<f64>)>,
    /// The stretches its lines read, each the statistics of its object at its last boundaries,
    /// which the run keeps (`roll`). Kept here, not by the objects read, so no other object or
    /// model pays for them.
    pub(crate) stretches: Vec<Stretch>,
}

/// How many buckets a stretch is kept in: its value is read over the stretch's length and up to
/// one bucket more.
pub const STRETCH_BUCKETS: u64 = 60;

/// One object's statistics at the last boundaries of a stretch, a bucket's width apart from time
/// 0, oldest first.
#[derive(Debug, Clone)]
pub(crate) struct Stretch {
    object: u32,
    length: SimTime,
    width: SimTime,
    /// Up to `STRETCH_BUCKETS + 1` boundaries with the statistics then.
    kept: std::collections::VecDeque<(SimTime, ObjectStatistics)>,
}

impl Stretch {
    fn new(object: u32, length: SimTime) -> Stretch {
        let width = SimTime::from_millis((length.as_millis() / STRETCH_BUCKETS).max(1));
        Stretch { object, length, width, kept: std::collections::VecDeque::new() }
    }

    /// Keeps the statistics of its object at each boundary after the last kept, up to `now`,
    /// giving the next. Exact while the object's state hasn't changed since the boundary, as it
    /// hasn't before the event that reaches it.
    fn roll(&mut self, keeper: &crate::statistics::StatisticsKeeper, now: SimTime) -> SimTime {
        let width = self.width.as_millis();
        let first = self.kept.back().map_or(0, |(at, _)| at.as_millis() / width + 1);
        let end = now.as_millis() / width;
        for bucket in first.max(end.saturating_sub(STRETCH_BUCKETS))..=end {
            let at = SimTime::from_millis(bucket * width);
            if at >= keeper.since() && first <= end {
                self.kept.push_back((at, keeper.read(at)));
                while self.kept.len() as u64 > STRETCH_BUCKETS + 1 {
                    self.kept.pop_front();
                }
            }
        }
        SimTime::from_millis((end + 1) * width)
    }

    /// The statistics at the start of the stretch before `now`: at the last boundary at or before
    /// it, or since statistics began when that is later.
    fn at_start(&self, keeper: &crate::statistics::StatisticsKeeper, now: SimTime) -> ObjectStatistics {
        let width = self.width.as_millis();
        let start = SimTime::from_millis(now.saturating_sub(self.length).as_millis() / width * width);
        if start >= keeper.since() {
            return keeper.read(start);
        }
        let zero = || ObjectStatistics { drain: keeper.read(now).drain.map(|_| Default::default()), ..ObjectStatistics::default() };
        match self.kept.iter().rev().find(|(at, _)| *at <= start) {
            Some((_, statistics)) => *statistics,
            None => zero(),
        }
    }
}

impl Ruled {
    pub fn new(rules: Arc<Rules>, builtin: crate::materialflow_objects::flows::Distributing, successors: &[u32]) -> Ruled {
        let exits = successors.len();
        let masks = rules
            .lines
            .iter()
            .map(|line| match &line.then {
                Action::RoundRobin { among } | Action::Weighted { among } => (0..exits).map(|exit| among.is_empty() || among.contains(&exit)).collect(),
                _ => Vec::new(),
            })
            .collect();
        let stretches = rules.stretches(successors).into_iter().map(|(object, length)| Stretch::new(object, length)).collect();
        Ruled { rules, builtin, masks, order: Vec::new(), values: Vec::new(), stretches }
    }

    /// Keeps every stretch's buckets up to `now`, before the event at `now` changes anything,
    /// giving when the next boundary is.
    pub(crate) fn roll(&mut self, objects: &[ModelObject], now: SimTime) -> SimTime {
        let mut next = SimTime::MAX;
        for stretch in &mut self.stretches {
            next = next.min(stretch.roll(objects[stretch.object as usize].statistics_keeper(), now));
        }
        next
    }

    /// Forgets what the stretches kept, at Reset or when statistics start afresh.
    pub(crate) fn clear_stretches(&mut self) {
        for stretch in &mut self.stretches {
            stretch.kept.clear();
        }
    }
}

impl Ruled {
    /// The exits the line at `at` allows, for round robin or weighted among some.
    pub(crate) fn rules_mask(&self, at: usize) -> &[bool] {
        &self.masks[at]
    }

    /// The mask of the line at `at` as a bit per exit, for a cross conveyor's at most 8.
    pub(crate) fn bits(&self, at: usize) -> u8 {
        self.masks[at].iter().take(8).enumerate().fold(0, |bits, (exit, &allowed)| if allowed { bits | 1 << exit } else { bits })
    }
}

/// What the rules decided for one MU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decided {
    /// Offer it to the exits in `Ruled::order`, in that order.
    Listed,
    /// Round robin among the exits the line's mask allows, by the line's place.
    RoundRobin(usize),
    /// The object's own ranking among the exits the line's mask allows.
    Weighted(usize),
    /// The object's own strategy.
    Default,
}

impl Ruled {
    /// Decides where the MU goes: reads the lines in order, the first that holds deciding, at
    /// `now`, reading the objects in `others` and the deciding object's successors.
    pub(crate) fn decide(&mut self, mu: &MoveableUnit, destination: DestinationId, successors: &[u32], others: &OtherObjects<'_>, now: SimTime) -> Decided {
        let stretches = &self.stretches;
        let reading = |reading: &Reading| read(others.get(reading.object), reading.object, reading.figure, reading.scope, now, stretches);
        let holds = |condition: &Condition| match condition {
            Condition::Class { kinds, equal } => kinds.contains(&mu.kind()) == *equal,
            Condition::Destination { destination: wanted, equal } => (destination == *wanted) == *equal,
            Condition::TimeOfDay { compare, time } => compare.holds((now.as_millis() % DAY) as f64, time.as_millis() as f64),
            Condition::Compare { left, compare, right } => {
                let right = match right {
                    Operand::Number(value) => Some(*value),
                    Operand::Figure(right) => reading(right),
                };
                matches!((reading(left), right), (Some(left), Some(right)) if compare.holds(left, right))
            }
            Condition::State { object, state, equal } => (StateName::of(others.get(*object).statistics_keeper().state()) == *state) == *equal,
        };
        let Some(at) = self.rules.lines.iter().position(|line| line.when.is_empty() || line.when.iter().any(|group| group.iter().all(holds))) else {
            return Decided::Default;
        };
        match &self.rules.lines[at].then {
            Action::To(exit) => {
                self.order.clear();
                self.order.push(*exit);
                Decided::Listed
            }
            Action::Best { figure, scope, largest, among, wait } => {
                self.order.clear();
                if among.is_empty() {
                    self.order.extend(0..successors.len());
                } else {
                    self.order.extend(among.iter().copied().filter(|&exit| exit < successors.len()));
                }
                // Each exit's value, read once; those with none go last, ties keep exit order.
                let value = |exit: usize| read(others.get(successors[exit]), successors[exit], *figure, *scope, now, &self.stretches);
                self.values.clear();
                self.values.extend(self.order.iter().map(|&exit| (exit, value(exit))));
                self.values.sort_by(|(a, x), (b, y)| match (x, y) {
                    (Some(x), Some(y)) => {
                        let by = if *largest { y.total_cmp(x) } else { x.total_cmp(y) };
                        by.then(a.cmp(b))
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.cmp(b),
                });
                self.order.clear();
                self.order.extend(self.values.iter().map(|(exit, _)| *exit).take(if *wait { 1 } else { usize::MAX }));
                Decided::Listed
            }
            Action::RoundRobin { .. } => Decided::RoundRobin(at),
            Action::Weighted { .. } => Decided::Weighted(at),
            Action::Default => Decided::Default,
        }
    }
}

const DAY: u64 = 86_400_000;

/// A figure of `object` at `now`, over `scope`, or `None` where it has no value.
fn read(object: &ModelObject, id: u32, figure: Figure, scope: RuleScope, now: SimTime, stretches: &[Stretch]) -> Option<f64> {
    let keeper = object.statistics_keeper();
    let kind = object.kind();
    match (figure, scope) {
        (Figure::Mus, _) => Some(keeper.contents() as f64),
        (_, RuleScope::Now) => None,
        (_, RuleScope::SinceStart) => of_statistics(figure, kind, None, &keeper.read(now), now),
        (_, RuleScope::Last(length)) => {
            let start = stretches.iter().find(|stretch| stretch.object == id && stretch.length == length).map(|stretch| stretch.at_start(keeper, now));
            of_statistics(figure, kind, start.as_ref(), &keeper.read(now), now)
        }
    }
}

/// The figures a rule can read, from statistics alone: since they began, or over a stretch from
/// `earlier` to `later`.
pub fn readable(figure: Figure) -> bool {
    matches!(
        figure,
        Figure::MusIn
            | Figure::MusOut
            | Figure::WorkingShare
            | Figure::BlockedShare
            | Figure::IdleShare
            | Figure::ReceivingShare
            | Figure::FailedShare
            | Figure::PausedShare
            | Figure::WorkingFullShare
            | Figure::WorkingNotFullShare
            | Figure::WorkingPartlyBlockedShare
            | Figure::Failures
            | Figure::Availability
            | Figure::ThroughputPerHour
            | Figure::MeanLeadTime
            | Figure::MeanMus
            | Figure::Mus
            | Figure::State
    )
}

fn of_statistics(figure: Figure, kind: ObjectKind, earlier: Option<&ObjectStatistics>, later: &ObjectStatistics, now: SimTime) -> Option<f64> {
    if !figure.applies_to(kind) {
        return None;
    }
    let zero = ObjectStatistics { drain: later.drain.map(|_| Default::default()), ..ObjectStatistics::default() };
    let a = earlier.unwrap_or(&zero);
    let b = later;
    let measured = b.measured_time().saturating_sub(a.measured_time()).as_millis() as f64;
    let difference = |from: SimTime, to: SimTime| to.saturating_sub(from);
    let share = |from: SimTime, to: SimTime| (measured > 0.0).then(|| difference(from, to).as_millis() as f64 / measured);
    let count = |from: u64, to: u64| Some(to.saturating_sub(from) as f64);
    let _ = now;
    match figure {
        Figure::MusIn => count(a.mus_in, b.mus_in),
        Figure::MusOut => count(a.mus_out, b.mus_out),
        Figure::WorkingShare => share(a.time_busy, b.time_busy),
        Figure::BlockedShare => share(a.time_blocked, b.time_blocked),
        Figure::IdleShare => share(a.time_idle, b.time_idle),
        Figure::ReceivingShare => share(a.time_receiving, b.time_receiving),
        Figure::FailedShare => share(a.time_failed, b.time_failed),
        Figure::PausedShare => share(a.time_paused, b.time_paused),
        Figure::WorkingFullShare => share(a.time_busy_full(), b.time_busy_full()),
        Figure::WorkingNotFullShare => share(a.time_busy_not_full, b.time_busy_not_full),
        Figure::WorkingPartlyBlockedShare => share(a.time_busy_partly_blocked, b.time_busy_partly_blocked),
        Figure::Failures => count(a.failures, b.failures),
        Figure::Availability => (measured > 0.0).then(|| 1.0 - difference(a.time_failed, b.time_failed).as_millis() as f64 / measured),
        Figure::ThroughputPerHour => (measured > 0.0).then(|| b.mus_in.saturating_sub(a.mus_in) as f64 * 3_600_000.0 / measured),
        Figure::MeanLeadTime => {
            let mus = b.mus_in.saturating_sub(a.mus_in);
            let total = difference(a.drain?.lead_time_total, b.drain?.lead_time_total);
            (mus > 0).then(|| total.as_secs_f64() / mus as f64)
        }
        Figure::MeanMus => {
            let mu_time = |s: &ObjectStatistics| s.mu_time_busy + s.mu_time_blocked + s.mu_time_waiting + s.mu_time_interrupted;
            (measured > 0.0).then(|| difference(mu_time(a), mu_time(b)).as_millis() as f64 / measured)
        }
        _ => None,
    }
}

/// Whether a figure's values compare with another's: the same unit.
pub fn same_unit(a: Figure, b: Figure) -> bool {
    a.unit() == b.unit() && a.unit() != Unit::State
}
