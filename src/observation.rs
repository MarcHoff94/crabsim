//! What a simulation run reports: occurrences and state. Views and tests consume
//! these instead of reaching into material-flow objects or engine events.

use crate::flows::ProcessState;
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
    MuCreated { mu_id: u32, at: u32 },
    MuMoved { mu_id: u32, from: u32, to: u32 },
    MuLeft { mu_id: u32, at: u32 },
    WorkStarted { mu_id: u32, at: u32 },
    WorkFinished { mu_id: u32, at: u32 },
    Blocked { object_id: u32 },
    Unblocked { object_id: u32 },
    StopBegan { object_id: u32, cause: StopCause },
    StopEnded { object_id: u32, cause: StopCause },
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
    /// How many MUs have left the model through this object.
    pub mus_left: usize,
}

/// The full current state of a simulation run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunState {
    pub time: SimTime,
    pub initialised: bool,
    pub objects: Vec<ObjectState>,
}
