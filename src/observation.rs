//! What a simulation run reports: occurrences and state. Views and tests consume
//! these instead of reaching into material-flow objects or engine events.

use crate::flows::ProcessState;
use crate::statistics::ObjectStatistics;
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
    /// A failure or pause began. The object is stopped while any interruption is active, so
    /// overlapping ones are one stop.
    InterruptionBegan { object_id: u32, cause: StopCause },
    InterruptionEnded { object_id: u32, cause: StopCause },
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

/// Why a material-flow object is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    Failure,
    Pause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Source,
    Station,
    Buffer,
    Drain,
}

/// What one material-flow object reports about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectState {
    pub id: u32,
    pub kind: ObjectKind,
    /// Ids of the MUs the object currently holds.
    pub contents: Vec<u32>,
    pub process: ProcessState,
    pub stop: Option<StopCause>,
    /// Time left until the object's current work completes, frozen during a stop.
    pub remaining: Option<SimTime>,
    pub blocked_predecessors: Vec<u32>,
    pub successors: Vec<u32>,
    /// How many MUs a Station or Buffer can hold. 0 for a Source or Drain.
    pub capacity: u64,
    pub statistics: ObjectStatistics,
}

/// The full current state of a simulation run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunState {
    pub time: SimTime,
    pub initialised: bool,
    /// When the objects' statistics began: t=0, or the last time they were cleared.
    pub statistics_since: SimTime,
    pub objects: Vec<ObjectState>,
}
