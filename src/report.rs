//! What a simulation run reports: occurrences and state. Views and tests consume
//! these instead of reaching into material-flow objects or engine events.

use crate::event::ProfileIndex;
use crate::flows::ProcessState;
use crate::statistics::{FillStatistics, MeasuringPeriods, ObjectStatistics};
use crate::time::SimTime;

/// Something that happened in the model at a point in simulation time.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub time: SimTime,
    pub kind: OccurrenceKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OccurrenceKind {
    RunInitialised,
    /// `duration` is how long until the MU is ready to leave the Source.
    MuCreated { mu_id: u32, at: u32, duration: SimTime },
    MuMoved { mu_id: u32, from: u32, to: u32 },
    /// `lead_time` runs from the MU leaving its Source until it entered this Drain.
    MuLeft { mu_id: u32, at: u32, lead_time: SimTime },
    /// `duration` is the process time, not counting any stop.
    WorkStarted { mu_id: u32, at: u32, duration: SimTime },
    WorkFinished { mu_id: u32, at: u32 },
    Blocked { object_id: u32 },
    Unblocked { object_id: u32 },
    /// A failure, stop or pause began. The object is interrupted while any interruption is active, so
    /// overlapping ones are one stop.
    /// `profile` is the failure profile's index in the object's list, for a random failure.
    InterruptionBegan { object_id: u32, cause: Interruption, profile: Option<ProfileIndex> },
    InterruptionEnded { object_id: u32, cause: Interruption, profile: Option<ProfileIndex> },
}

/// The kinds of occurrence, for choosing which to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccurrenceType {
    RunInitialised,
    MuCreated,
    MuMoved,
    MuLeft,
    WorkStarted,
    WorkFinished,
    Blocked,
    Unblocked,
    InterruptionBegan,
    InterruptionEnded,
}

impl OccurrenceKind {
    pub fn occurrence_type(&self) -> OccurrenceType {
        match self {
            OccurrenceKind::RunInitialised => OccurrenceType::RunInitialised,
            OccurrenceKind::MuCreated { .. } => OccurrenceType::MuCreated,
            OccurrenceKind::MuMoved { .. } => OccurrenceType::MuMoved,
            OccurrenceKind::MuLeft { .. } => OccurrenceType::MuLeft,
            OccurrenceKind::WorkStarted { .. } => OccurrenceType::WorkStarted,
            OccurrenceKind::WorkFinished { .. } => OccurrenceType::WorkFinished,
            OccurrenceKind::Blocked { .. } => OccurrenceType::Blocked,
            OccurrenceKind::Unblocked { .. } => OccurrenceType::Unblocked,
            OccurrenceKind::InterruptionBegan { .. } => OccurrenceType::InterruptionBegan,
            OccurrenceKind::InterruptionEnded { .. } => OccurrenceType::InterruptionEnded,
        }
    }
}

/// Which kinds of occurrence a run keeps. A run keeps none unless asked, because data per MU is
/// the only thing that grows with simulated length. It can change at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recording(u16);

impl Recording {
    pub const NOTHING: Recording = Recording(0);
    pub const ALL: Recording = Recording(u16::MAX);

    pub fn only(types: &[OccurrenceType]) -> Recording {
        Recording(types.iter().fold(0, |bits, &occurrence_type| bits | 1 << occurrence_type as u16))
    }

    /// Costs one predictable check on the per-event path when nothing is recorded.
    #[inline]
    pub fn includes(self, kind: &OccurrenceKind) -> bool {
        self.0 != 0 && self.0 & (1 << kind.occurrence_type() as u16) != 0
    }
}

/// Why a material-flow object is interrupted: one kind of interruption (ADR 0020). Listed from
/// most to least serious, which is how state picks one to report when several are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Interruption {
    /// A random failure.
    Failed,
    /// Stopped by hand, and ended by hand. It counts as a failure in statistics.
    Stopped,
    /// A planned interruption, which doesn't count against availability.
    Paused,
}

impl Interruption {
    /// Every cause, most serious first.
    pub const ALL: [Interruption; 3] = [Interruption::Failed, Interruption::Stopped, Interruption::Paused];

    /// The cause's place in `ALL`.
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// One active interruption of an object, and when it began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveInterruption {
    pub cause: Interruption,
    pub since: SimTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Source,
    Station,
    Buffer,
    Drain,
}

/// How many MUs an object can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capacity {
    Limited(u64),
    /// A Buffer that takes every MU. It grows as far as it's filled (ADR 0019).
    Unlimited,
}

impl Capacity {
    /// The number of MUs at which the object is full, which an unlimited one never reaches.
    pub fn limit(self) -> u64 {
        match self {
            Capacity::Limited(limit) => limit,
            Capacity::Unlimited => u64::MAX,
        }
    }
}

/// An MU in work in a Station, and how much work is left on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkLeft {
    pub mu_id: u32,
    pub left: SimTime,
}

/// What an object reports about one of its failure profiles.
#[derive(Debug, Clone, PartialEq)]
pub struct FailureProfileState {
    /// Its place in the object's list, which failure events and occurrences name it by. Profiles
    /// that never fail (100% available) are left out, so this can skip numbers.
    pub index: u8,
    pub name: String,
    pub availability: f32,
    /// How many failures of this profile began since statistics began.
    pub failures: u64,
    /// How long it has failed since statistics began, a failure in progress included.
    pub time_failed: SimTime,
    /// Since when it has been failing, if it is.
    pub failing_since: Option<SimTime>,
}

/// What one material-flow object reports about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectState {
    pub id: u32,
    pub kind: ObjectKind,
    /// Ids of the MUs the object currently holds, for a Buffer in the order they came in.
    pub contents: Vec<u32>,
    /// What it is doing. A Buffer is `Idle` when empty, `Processing` while it holds MUs, and
    /// `Blocked` when an MU that may leave can't.
    pub process: ProcessState,
    /// The most serious active interruption, if the object is interrupted.
    pub interrupted: Option<Interruption>,
    /// Every active interruption, most serious first, with when it began. A view needs them all,
    /// so that it still knows the object is interrupted after the most serious one ends.
    pub interruptions: Vec<ActiveInterruption>,
    /// Time left until the object's current work completes, frozen during a stop. For a Station
    /// with several MUs in work, the soonest.
    pub remaining: Option<SimTime>,
    /// Each MU in work in a Station and the work left on it, frozen during a stop, in the order
    /// they came in. Empty for other objects.
    pub work: Vec<WorkLeft>,
    pub blocked_predecessors: Vec<u32>,
    pub successors: Vec<u32>,
    /// How many MUs a Station or Buffer can hold. 0 for a Source or Drain.
    pub capacity: Capacity,
    /// How long a Station works on one MU, now. `None` for other objects.
    pub process_time: Option<SimTime>,
    /// The process time in the Station's default state, which Reset returns to.
    pub configured_process_time: Option<SimTime>,
    pub statistics: ObjectStatistics,
    /// Each failure profile's figures, in the order of the object's list. Empty for an object with
    /// none, which then allocates nothing for it.
    pub failure_profiles: Vec<FailureProfileState>,
    /// How full a Buffer was: the fewest MUs it held and its histogram. `None` for other objects,
    /// which then allocate nothing for it.
    pub fill: Option<Box<FillStatistics>>,
}

/// The full current state of a simulation run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunState {
    pub time: SimTime,
    pub initialised: bool,
    /// When the objects' statistics began: t=0, or the last time they were cleared.
    pub statistics_since: SimTime,
    pub objects: Vec<ObjectState>,
    /// Each object's figures over the complete measuring periods since statistics began, when
    /// the run measures in periods. See `SimulationRun::set_measuring_period`.
    pub periods: Option<MeasuringPeriods>,
}
