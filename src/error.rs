use crate::observation::Occurrence;
use crate::time::SimTime;
use std::fmt;

#[derive(Debug)]
pub struct InitError;

#[derive(Debug)]
pub struct RunTimeError;

#[derive(Debug)]
pub enum MaterialflowError {
    EntranceDenied,
    NoMU,
}

/// Why a simulation run refused a control or could not continue.
#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    AlreadyInitialised,
    NotInitialised,
    EventInThePast,
    UnknownObject(u32),
    ObjectFailed { object_id: u32, time: SimTime },
    /// Too many events in a row happened without simulation time moving.
    ZeroTimeLoop { time: SimTime },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::AlreadyInitialised => write!(f, "the simulation run is already initialised; reset it first"),
            RunError::NotInitialised => write!(f, "the simulation run has not been initialised"),
            RunError::EventInThePast => write!(f, "an event cannot be scheduled before the current simulation time"),
            RunError::UnknownObject(id) => write!(f, "an event names object {id}, which is not in the model"),
            RunError::ObjectFailed { object_id, time } => {
                write!(f, "object {object_id} could not handle an event at {:.3}s", time.as_secs_f64())
            }
            RunError::ZeroTimeLoop { time } => {
                write!(f, "simulation time is stuck at {:.3}s: events keep happening without it advancing", time.as_secs_f64())
            }
        }
    }
}

impl std::error::Error for RunError {}

/// An advance that ended in an error, with everything it recorded before the error, so that
/// no caller loses what happened.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvanceFailure {
    pub error: RunError,
    pub occurrences: Vec<Occurrence>,
}

impl fmt::Display for AdvanceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for AdvanceFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
