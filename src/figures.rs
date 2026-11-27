//! The one vocabulary of figures (ADR 0030): everything crabsim measures about an object, the
//! model or an MU, each with its name, meaning, unit and the time scopes it can be read over, for
//! expectations, the results store and, later, rules and user code. A figure is read from a run's
//! state, never on the per-event path: a run pays nothing for the vocabulary.
//!
//! The names are the results store's, so a figure an expectation names is the one a run stores.

use crate::flows::ProcessState;
use crate::report::{Interruption, ObjectKind, ObjectState};
use crate::time::SimTime;
use crate::{Capacity, Side};

/// What a figure's value is counted in, which an expectation writes explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// A whole number of MUs or of things that happened, written plainly, such as `20`.
    Count,
    /// MUs on average, written plainly, such as `3.5`.
    Mus,
    /// A share of the time measured, from 0 to 1, written as a percentage, such as `60%`.
    Share,
    /// MUs an hour, written with its rate, such as `4000/h`, `70/min` or `96000/d`.
    PerHour,
    /// A length of simulated time, written as the UI writes time or with a unit, such as `1:30`,
    /// `90s`, `30min`, `2h` or `1d`.
    Time,
    /// One of the states an object can be in, such as `busy` or `failed`.
    State,
    /// A name, such as an MU class or a destination.
    Name,
}

impl Unit {
    pub fn name(self) -> &'static str {
        match self {
            Unit::Count => "count",
            Unit::Mus => "MUs",
            Unit::Share => "share",
            Unit::PerHour => "per hour",
            Unit::Time => "time",
            Unit::State => "state",
            Unit::Name => "name",
        }
    }
}

/// Over which time a figure is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// At the moment it is read, such as the MUs an object holds at a run's end.
    Now,
    /// Since statistics began: from Init, or from the end of the warm-up.
    SinceStart,
    /// Over the last stretch of this length before it is read.
    Last(SimTime),
    /// At every moment since statistics began: a limit, such as "never more than 20 MUs".
    Always,
}

/// The kinds of scope, for saying which a figure can be read over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Now,
    SinceStart,
    Last,
    Always,
}

impl ScopeKind {
    /// How an expectation writes it.
    pub fn written(self) -> &'static str {
        match self {
            ScopeKind::Now => "now",
            ScopeKind::SinceStart => "since start",
            ScopeKind::Last => "last <time>",
            ScopeKind::Always => "always",
        }
    }
}

impl Scope {
    pub fn kind(self) -> ScopeKind {
        match self {
            Scope::Now => ScopeKind::Now,
            Scope::SinceStart => ScopeKind::SinceStart,
            Scope::Last(_) => ScopeKind::Last,
            Scope::Always => ScopeKind::Always,
        }
    }
}

/// What a figure is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// One material-flow object, or each of a class's.
    Object,
    /// The MU an object is deciding about, for rules and user code. An expectation can't name it.
    Mu,
}

/// One figure of the vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Figure {
    MusIn,
    MusOut,
    WorkingShare,
    BlockedShare,
    IdleShare,
    ReceivingShare,
    FailedShare,
    PausedShare,
    WorkingFullShare,
    WorkingNotFullShare,
    WorkingPartlyBlockedShare,
    Failures,
    Availability,
    ThroughputPerHour,
    MeanLeadTime,
    MeanMus,
    MinMus,
    MaxMus,
    MedianMus,
    FullShare,
    MusOnPlan,
    MusOffPlan,
    MeanLengthTakenShare,
    MaxLengthTakenShare,
    MeanCrossingTime,
    MaxCrossingTime,
    SwitchingShare,
    MusOutFront,
    MusOutBack,
    MusOutLeft,
    MusOutRight,
    /// The MUs an object holds.
    Mus,
    /// Each MU's lead time as it enters a Drain.
    LeadTime,
    /// What an object is doing.
    State,
    /// The MU's class.
    MuClass,
    /// Where the MU goes next.
    NextDestination,
    /// The step of its work plan the MU is on.
    PlanStep,
}

use Figure::*;

const ALL_KINDS: &[ObjectKind] = &[ObjectKind::Source, ObjectKind::Station, ObjectKind::Buffer, ObjectKind::Drain, ObjectKind::Conveyor, ObjectKind::CrossConveyor];
const HOLDING_KINDS: &[ObjectKind] = &[ObjectKind::Station, ObjectKind::Buffer, ObjectKind::Conveyor, ObjectKind::CrossConveyor];
const FILL_KINDS: &[ObjectKind] = &[ObjectKind::Buffer, ObjectKind::Conveyor, ObjectKind::CrossConveyor];
const DRAIN: &[ObjectKind] = &[ObjectKind::Drain];
const STATION: &[ObjectKind] = &[ObjectKind::Station];
const BELTS: &[ObjectKind] = &[ObjectKind::Conveyor, ObjectKind::CrossConveyor];
const CROSS: &[ObjectKind] = &[ObjectKind::CrossConveyor];
const PARTS: &[ObjectKind] = &[ObjectKind::Station, ObjectKind::Conveyor];
const NONE: &[ObjectKind] = &[];

const SINCE_AND_LAST: &[ScopeKind] = &[ScopeKind::SinceStart, ScopeKind::Last];
const SINCE: &[ScopeKind] = &[ScopeKind::SinceStart];
const NOW: &[ScopeKind] = &[ScopeKind::Now];
const NOW_AND_ALWAYS: &[ScopeKind] = &[ScopeKind::Now, ScopeKind::Always];
const ALWAYS: &[ScopeKind] = &[ScopeKind::Always];

impl Figure {
    /// Every figure, in the order the results store writes them.
    pub const ALL: [Figure; 37] = [
        MusIn,
        MusOut,
        WorkingShare,
        BlockedShare,
        IdleShare,
        ReceivingShare,
        FailedShare,
        PausedShare,
        WorkingFullShare,
        WorkingNotFullShare,
        WorkingPartlyBlockedShare,
        Failures,
        Availability,
        ThroughputPerHour,
        MeanLeadTime,
        MeanMus,
        MinMus,
        MaxMus,
        MedianMus,
        FullShare,
        MusOnPlan,
        MusOffPlan,
        MeanLengthTakenShare,
        MaxLengthTakenShare,
        MeanCrossingTime,
        MaxCrossingTime,
        SwitchingShare,
        MusOutFront,
        MusOutBack,
        MusOutLeft,
        MusOutRight,
        Mus,
        LeadTime,
        State,
        MuClass,
        NextDestination,
        PlanStep,
    ];

    /// The figure called `name`.
    pub fn named(name: &str) -> Option<Figure> {
        Figure::ALL.into_iter().find(|figure| figure.name() == name)
    }

    /// Its name, the same in expectations, the results store and code.
    pub fn name(self) -> &'static str {
        match self {
            MusIn => "mus_in",
            MusOut => "mus_out",
            WorkingShare => "working_share",
            BlockedShare => "blocked_share",
            IdleShare => "idle_share",
            ReceivingShare => "receiving_share",
            FailedShare => "failed_share",
            PausedShare => "paused_share",
            WorkingFullShare => "working_full_share",
            WorkingNotFullShare => "working_not_full_share",
            WorkingPartlyBlockedShare => "working_partly_blocked_share",
            Failures => "failures",
            Availability => "availability",
            ThroughputPerHour => "throughput_per_hour",
            MeanLeadTime => "mean_lead_time_s",
            MeanMus => "mean_mus",
            MinMus => "min_mus",
            MaxMus => "max_mus",
            MedianMus => "median_mus",
            FullShare => "full_share",
            MusOnPlan => "mus_on_plan",
            MusOffPlan => "mus_off_plan",
            MeanLengthTakenShare => "mean_length_taken_share",
            MaxLengthTakenShare => "max_length_taken_share",
            MeanCrossingTime => "mean_crossing_time_s",
            MaxCrossingTime => "max_crossing_time_s",
            SwitchingShare => "switching_share",
            MusOutFront => "mus_out_front",
            MusOutBack => "mus_out_back",
            MusOutLeft => "mus_out_left",
            MusOutRight => "mus_out_right",
            Mus => "mus",
            LeadTime => "lead_time_s",
            State => "state",
            MuClass => "class",
            NextDestination => "next_destination",
            PlanStep => "plan_step",
        }
    }

    /// What it means, in the glossary's words.
    pub fn meaning(self) -> &'static str {
        match self {
            MusIn => "MUs that entered it; a Source's are the MUs it created, a starting MU counting at Init",
            MusOut => "MUs that left it",
            WorkingShare => "share of the time it was busy (a Buffer: holding), in three parts for a Station or Conveyor: full, not full and partly blocked",
            BlockedShare => "share of the time an MU that was ready couldn't leave",
            IdleShare => "share of the time it had nothing to do",
            ReceivingShare => "share of the time it had a place reserved for an MU a conveyor was handing over, and nothing else going on",
            FailedShare => "share of the time it was failed or stopped by hand",
            PausedShare => "share of the time it was paused",
            WorkingFullShare => "share of the time it was busy with every place in work; for a Conveyor, with its start taken so that no MU could enter",
            WorkingNotFullShare => "share of the time it was busy with free places and nothing waiting to leave; for a Conveyor, with room at its start",
            WorkingPartlyBlockedShare => "share of the time it was busy while a finished MU couldn't leave; for a Conveyor, while the MU at its end waited",
            Failures => "failures that began, of every failure profile together, a stop by hand counting as one",
            Availability => "up time over up and failed time, from 0 to 1",
            ThroughputPerHour => "MUs an hour into this Drain, since start from its first MU; for the model, of every Drain together",
            MeanLeadTime => "mean time from an MU leaving its Source to entering this Drain; for the model, over every Drain",
            MeanMus => "MUs it held on average, weighted by time",
            MinMus => "the fewest MUs it held for any time",
            MaxMus => "the most MUs it held",
            MedianMus => "the median of the MUs it held, weighted by time: the fewest of the histogram bar holding it",
            FullShare => "share of the time a Buffer with a limit was full",
            MusOnPlan => "MUs it worked on for their work plan's step or their detour",
            MusOffPlan => "MUs it worked on that were on their way somewhere else",
            MeanLengthTakenShare => "the share of its length its MUs took on average",
            MaxLengthTakenShare => "the most share of its length its MUs took",
            MeanCrossingTime => "mean time from an MU's booking point crossing its start to crossing its end",
            MaxCrossingTime => "the longest crossing, including the MU on it longest",
            SwitchingShare => "share of the time an MU stood at its centre to turn, or the release time after one ran",
            MusOutFront => "MUs out of its front side",
            MusOutBack => "MUs out of its back side",
            MusOutLeft => "MUs out of its left side",
            MusOutRight => "MUs out of its right side",
            Mus => "the MUs it holds; always: the most at any moment, for an upper limit",
            LeadTime => "each MU's time from leaving its Source to entering this Drain, as a limit; for the model, at every Drain",
            State => "what it is doing: busy, holding, blocked, idle, receiving, failed, stopped or paused",
            MuClass => "the MU's class",
            NextDestination => "where the MU goes next: its detour, or else its work plan's step",
            PlanStep => "the step of its work plan the MU is on, from 1",
        }
    }

    pub fn unit(self) -> Unit {
        match self {
            MusIn | MusOut | Failures | MinMus | MaxMus | MedianMus | MusOnPlan | MusOffPlan | MusOutFront | MusOutBack | MusOutLeft | MusOutRight | Mus | PlanStep => Unit::Count,
            MeanMus => Unit::Mus,
            WorkingShare | BlockedShare | IdleShare | ReceivingShare | FailedShare | PausedShare | WorkingFullShare | WorkingNotFullShare | WorkingPartlyBlockedShare | Availability | FullShare
            | MeanLengthTakenShare | MaxLengthTakenShare | SwitchingShare => Unit::Share,
            ThroughputPerHour => Unit::PerHour,
            MeanLeadTime | MeanCrossingTime | MaxCrossingTime | LeadTime => Unit::Time,
            State => Unit::State,
            MuClass | NextDestination => Unit::Name,
        }
    }

    pub fn subject(self) -> Subject {
        match self {
            MuClass | NextDestination | PlanStep => Subject::Mu,
            _ => Subject::Object,
        }
    }

    /// The kinds of object it is a figure of. Empty for an MU's figures.
    pub fn kinds(self) -> &'static [ObjectKind] {
        match self {
            MusIn | MusOut | WorkingShare | BlockedShare | IdleShare | ReceivingShare | FailedShare | PausedShare | Failures | Availability | State => ALL_KINDS,
            WorkingFullShare | WorkingNotFullShare | WorkingPartlyBlockedShare => PARTS,
            ThroughputPerHour | MeanLeadTime | LeadTime => DRAIN,
            MeanMus | MinMus | MaxMus | MedianMus => FILL_KINDS,
            FullShare => &[ObjectKind::Buffer],
            MusOnPlan | MusOffPlan => STATION,
            MeanLengthTakenShare | MaxLengthTakenShare | MeanCrossingTime | MaxCrossingTime => BELTS,
            SwitchingShare | MusOutFront | MusOutBack | MusOutLeft | MusOutRight => CROSS,
            Mus => HOLDING_KINDS,
            MuClass | NextDestination | PlanStep => NONE,
        }
    }

    pub fn applies_to(self, kind: ObjectKind) -> bool {
        self.kinds().contains(&kind)
    }

    /// Whether the model as a whole has it: its Drains' together.
    pub fn of_model(self) -> bool {
        matches!(self, ThroughputPerHour | MeanLeadTime | LeadTime)
    }

    /// The scopes it can be read over.
    pub fn scopes(self) -> &'static [ScopeKind] {
        match self {
            MinMus | MaxMus | MedianMus | MaxLengthTakenShare | MaxCrossingTime => SINCE,
            Mus => NOW_AND_ALWAYS,
            LeadTime => ALWAYS,
            State | MuClass | NextDestination | PlanStep => NOW,
            _ => SINCE_AND_LAST,
        }
    }

    pub fn has_scope(self, scope: ScopeKind) -> bool {
        self.scopes().contains(&scope)
    }

    /// Its value for `object` since statistics began, or for a figure read now, at the state's
    /// time `now`. `None` where it doesn't apply or can't be worked out yet, such as a mean lead
    /// time before the first MU, or a full share for a Buffer without a limit.
    pub fn value(self, object: &ObjectState, now: SimTime) -> Option<f64> {
        if !self.applies_to(object.kind) {
            return None;
        }
        let statistics = &object.statistics;
        let measured = statistics.measured_time().as_millis() as f64;
        let share = |time: SimTime| (measured > 0.0).then(|| time.as_millis() as f64 / measured);
        match self {
            MusIn => Some(statistics.mus_in as f64),
            MusOut => Some(statistics.mus_out as f64),
            WorkingShare => share(statistics.time_busy),
            BlockedShare => share(statistics.time_blocked),
            IdleShare => share(statistics.time_idle),
            ReceivingShare => share(statistics.time_receiving),
            FailedShare => share(statistics.time_failed),
            PausedShare => share(statistics.time_paused),
            WorkingFullShare => share(statistics.time_busy_full()),
            WorkingNotFullShare => share(statistics.time_busy_not_full),
            WorkingPartlyBlockedShare => share(statistics.time_busy_partly_blocked),
            Failures => Some(statistics.failures as f64),
            Availability => statistics.achieved_availability(),
            ThroughputPerHour => statistics.mus_per_hour(now),
            MeanLeadTime => statistics.lead_time_mean().map(SimTime::as_secs_f64),
            MeanMus => object.fill.as_ref().and(statistics.average_contents()),
            MinMus => object.fill.as_ref().map(|fill| fill.lowest as f64),
            MaxMus => object.fill.as_ref().map(|_| statistics.max_contents as f64),
            // The fewest MUs of the bar holding the median: exact while bars are 1 MU wide.
            MedianMus => object.fill.as_ref().and_then(|fill| fill.median()).map(|(from, _)| from as f64),
            FullShare => (object.capacity != Capacity::Unlimited).then(|| share(statistics.time_full)).flatten(),
            MusOnPlan => object.plan.map(|plan| plan.on_plan as f64),
            MusOffPlan => object.plan.map(|plan| plan.off_plan as f64),
            MeanLengthTakenShare => object.belt.as_ref().and_then(|belt| belt.mean_share_taken(statistics.measured_time())),
            MaxLengthTakenShare => object.belt.as_ref().map(|belt| belt.max_share_taken()),
            MeanCrossingTime => object.belt.as_ref().and_then(|belt| belt.crossing_mean()).map(SimTime::as_secs_f64),
            MaxCrossingTime => object.belt.as_ref().map(|belt| belt.crossing_max.as_secs_f64()),
            SwitchingShare => object.cross.as_ref().and_then(|cross| share(cross.time_switching)),
            MusOutFront | MusOutBack | MusOutLeft | MusOutRight => object.cross.as_ref().map(|cross| cross.mus_out[self.side().index()] as f64),
            Mus => Some(object.contents.len() as f64),
            LeadTime | State | MuClass | NextDestination | PlanStep => None,
        }
    }

    /// Its value as the results store keeps it for a run, or `None` where the store keeps nothing:
    /// beyond what `value` leaves out, a Station of capacity 1 has no parts of its busy time, which
    /// would only repeat it, a Station that worked on no MU going somewhere no plan counts, and a
    /// cross conveyor no count for a side no MU left by, so a model using none of them stores no
    /// more.
    pub fn stored(self, object: &ObjectState, now: SimTime) -> Option<f64> {
        let kept = match self {
            WorkingFullShare | WorkingNotFullShare | WorkingPartlyBlockedShare => {
                (object.kind == ObjectKind::Station && object.capacity.limit() > 1) || object.kind == ObjectKind::Conveyor
            }
            MusOnPlan | MusOffPlan => object.plan.is_some_and(|plan| plan.on_plan + plan.off_plan > 0),
            MusOutFront | MusOutBack | MusOutLeft | MusOutRight => object.cross.as_ref().is_some_and(|cross| cross.mus_out[self.side().index()] > 0),
            Mus | LeadTime | State | MuClass | NextDestination | PlanStep => false,
            _ => true,
        };
        if kept { self.value(object, now) } else { None }
    }

    /// Its value over the stretch from `earlier` to `later`, two states of the same object, such
    /// as the last hour of a run: from the difference of their statistics, which add up. `None`
    /// for a figure that can't be read over a stretch, such as the most MUs held, or where it
    /// can't be worked out, such as a mean lead time with no MU in the stretch.
    pub fn value_over(self, earlier: &ObjectState, later: &ObjectState) -> Option<f64> {
        if !self.applies_to(later.kind) || !self.has_scope(ScopeKind::Last) {
            return None;
        }
        let (a, b) = (&earlier.statistics, &later.statistics);
        let measured = b.measured_time().saturating_sub(a.measured_time()).as_millis() as f64;
        let difference = |from: SimTime, to: SimTime| to.saturating_sub(from);
        let share = |from: SimTime, to: SimTime| (measured > 0.0).then(|| difference(from, to).as_millis() as f64 / measured);
        let count = |from: u64, to: u64| Some(to.saturating_sub(from) as f64);
        match self {
            MusIn => count(a.mus_in, b.mus_in),
            MusOut => count(a.mus_out, b.mus_out),
            WorkingShare => share(a.time_busy, b.time_busy),
            BlockedShare => share(a.time_blocked, b.time_blocked),
            IdleShare => share(a.time_idle, b.time_idle),
            ReceivingShare => share(a.time_receiving, b.time_receiving),
            FailedShare => share(a.time_failed, b.time_failed),
            PausedShare => share(a.time_paused, b.time_paused),
            WorkingFullShare => share(a.time_busy_full(), b.time_busy_full()),
            WorkingNotFullShare => share(a.time_busy_not_full, b.time_busy_not_full),
            WorkingPartlyBlockedShare => share(a.time_busy_partly_blocked, b.time_busy_partly_blocked),
            Failures => count(a.failures, b.failures),
            Availability => (measured > 0.0).then(|| 1.0 - difference(a.time_failed, b.time_failed).as_millis() as f64 / measured),
            ThroughputPerHour => (measured > 0.0).then(|| b.mus_in.saturating_sub(a.mus_in) as f64 * 3_600_000.0 / measured),
            MeanLeadTime => {
                let mus = b.mus_in.saturating_sub(a.mus_in);
                let total = difference(a.drain?.lead_time_total, b.drain?.lead_time_total);
                (mus > 0).then(|| mean_time(total, mus))
            }
            MeanMus => {
                later.fill.as_ref()?;
                let mu_time = |s: &crate::statistics::ObjectStatistics| s.mu_time_busy + s.mu_time_blocked + s.mu_time_waiting + s.mu_time_interrupted;
                (measured > 0.0).then(|| difference(mu_time(a), mu_time(b)).as_millis() as f64 / measured)
            }
            FullShare => (later.capacity != Capacity::Unlimited).then(|| share(a.time_full, b.time_full)).flatten(),
            MusOnPlan => count(earlier.plan?.on_plan, later.plan?.on_plan),
            MusOffPlan => count(earlier.plan?.off_plan, later.plan?.off_plan),
            MeanLengthTakenShare => {
                let (from, to) = (earlier.belt.as_ref()?, later.belt.as_ref()?);
                (measured > 0.0 && to.length > 0.0).then(|| ((to.length_time - from.length_time) / (measured * to.length)).clamp(0.0, 1.0))
            }
            MeanCrossingTime => {
                let (from, to) = (earlier.belt.as_ref()?, later.belt.as_ref()?);
                let crossings = to.crossings.saturating_sub(from.crossings);
                (crossings > 0).then(|| mean_time(difference(from.crossing_total, to.crossing_total), crossings))
            }
            SwitchingShare => share(earlier.cross.as_ref()?.time_switching, later.cross.as_ref()?.time_switching),
            MusOutFront | MusOutBack | MusOutLeft | MusOutRight => {
                let side = self.side().index();
                count(earlier.cross.as_ref()?.mus_out[side], later.cross.as_ref()?.mus_out[side])
            }
            _ => None,
        }
    }

    /// The extreme a limit compares with: the most (`upper`) or fewest it reached at any moment
    /// since statistics began. `None` where nothing keeps it, such as the fewest MUs, which is
    /// `min_mus` since start, or before any lead time.
    pub fn extreme(self, object: &ObjectState, upper: bool) -> Option<f64> {
        if !self.applies_to(object.kind) {
            return None;
        }
        let statistics = &object.statistics;
        match (self, upper) {
            (Mus, true) => Some(statistics.max_contents as f64),
            (LeadTime, true) => statistics.drain.and_then(|drain| drain.lead_time_max).map(SimTime::as_secs_f64),
            (LeadTime, false) => statistics.drain.and_then(|drain| drain.lead_time_min).map(SimTime::as_secs_f64),
            _ => None,
        }
    }

    /// Whether a limit on it can be checked in this direction for this kind of object.
    pub fn limit_kept(self, kind: ObjectKind, upper: bool) -> bool {
        match self {
            // The fewest MUs held for any time is `min_mus` since start.
            Mus => self.applies_to(kind) && upper,
            LeadTime => kind == ObjectKind::Drain,
            _ => false,
        }
    }

    /// Its value for the model as a whole, from every object's state: its Drains' together.
    pub fn model_value(self, objects: &[ObjectState], now: SimTime) -> Option<f64> {
        let drains = || objects.iter().filter(|object| object.kind == ObjectKind::Drain);
        match self {
            ThroughputPerHour => {
                let rates: Vec<f64> = drains().filter_map(|drain| drain.statistics.mus_per_hour(now)).collect();
                (!rates.is_empty()).then(|| rates.iter().sum())
            }
            MeanLeadTime => {
                let mus: u64 = drains().map(|drain| drain.statistics.mus_in).sum();
                let total: f64 = drains().filter_map(|drain| drain.statistics.drain).map(|drain| drain.lead_time_total.as_secs_f64()).sum();
                (mus > 0).then(|| total / mus as f64)
            }
            _ => None,
        }
    }

    /// Its value for the model over the stretch between two states of the whole run.
    pub fn model_value_over(self, earlier: &[ObjectState], later: &[ObjectState]) -> Option<f64> {
        let drains = || earlier.iter().zip(later).filter(|(_, object)| object.kind == ObjectKind::Drain);
        match self {
            ThroughputPerHour => {
                let rates: Vec<f64> = drains().filter_map(|(a, b)| ThroughputPerHour.value_over(a, b)).collect();
                (!rates.is_empty()).then(|| rates.iter().sum())
            }
            MeanLeadTime => {
                let mus: u64 = drains().map(|(a, b)| b.statistics.mus_in.saturating_sub(a.statistics.mus_in)).sum();
                let total: f64 = drains()
                    .filter_map(|(a, b)| Some(b.statistics.drain?.lead_time_total.saturating_sub(a.statistics.drain?.lead_time_total).as_secs_f64()))
                    .sum();
                (mus > 0).then(|| total / mus as f64)
            }
            _ => None,
        }
    }

    /// The model's extreme for a limit: over every Drain.
    pub fn model_extreme(self, objects: &[ObjectState], upper: bool) -> Option<f64> {
        let values = objects.iter().filter_map(|object| self.extreme(object, upper));
        if upper { values.reduce(f64::max) } else { values.reduce(f64::min) }
    }

    fn side(self) -> Side {
        match self {
            MusOutBack => Side::Back,
            MusOutLeft => Side::Left,
            MusOutRight => Side::Right,
            _ => Side::Front,
        }
    }
}

/// A mean time in seconds, rounded to the millisecond as the statistics' own means are.
fn mean_time(total: SimTime, count: u64) -> f64 {
    SimTime::from_millis_f64(total.as_millis() as f64 / count as f64).as_secs_f64()
}

/// What the object is doing at its state's time, by the name of one of the model file's states:
/// busy, holding (a Buffer's busy), blocked, idle, receiving, failed, stopped or paused.
pub fn state_name(object: &ObjectState) -> &'static str {
    match object.interrupted {
        Some(Interruption::Failed) => return "failed",
        Some(Interruption::Stopped) => return "stopped",
        Some(Interruption::Paused) => return "paused",
        None => {}
    }
    match object.process {
        ProcessState::Processing if object.kind == ObjectKind::Buffer => "holding",
        ProcessState::Processing => "busy",
        ProcessState::Blocked => "blocked",
        ProcessState::Idle => "idle",
        ProcessState::Receiving => "receiving",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_figure_is_named_once_and_found_by_its_name() {
        for figure in Figure::ALL {
            assert_eq!(Figure::named(figure.name()), Some(figure));
            assert!(!figure.meaning().is_empty());
            assert!(!figure.scopes().is_empty());
            assert_eq!(figure.subject() == Subject::Mu, figure.kinds().is_empty(), "{figure:?}");
        }
    }
}
