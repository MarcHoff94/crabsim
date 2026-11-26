use crate::report::Occurrence;
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
    /// Only a Station can do what was asked, such as change its process time.
    NotAStation(u32),
    ObjectFailed { object_id: u32, time: SimTime },
    /// Too many events in a row happened without simulation time moving.
    ZeroTimeLoop { time: SimTime },
    /// The run was asked to go beyond the latest time an MU can hold as its departure, about
    /// 8,900 years (ADR 0027).
    BeyondTimeLimit { time: SimTime },
    /// User code failed, or answered something the run can't do (ADR 0029). Boxed, since every
    /// event returns a `Result` with this in it, and that must stay small.
    Hook(Box<HookFailure>),
}

/// Which call of user code failed, when, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct HookFailure {
    pub call: crate::hooks::HookCall,
    pub time: SimTime,
    pub message: String,
}

impl RunError {
    pub(crate) fn hook(call: &crate::hooks::HookCall, time: SimTime, message: String) -> RunError {
        RunError::Hook(Box::new(HookFailure { call: call.clone(), time, message }))
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::AlreadyInitialised => write!(f, "the simulation run is already initialised; reset it first"),
            RunError::NotInitialised => write!(f, "the simulation run has not been initialised"),
            RunError::EventInThePast => write!(f, "an event cannot be scheduled before the current simulation time"),
            RunError::UnknownObject(id) => write!(f, "an event names object {id}, which is not in the model"),
            RunError::NotAStation(id) => write!(f, "object {id} is not a Station, and only a Station can do this"),
            RunError::ObjectFailed { object_id, time } => {
                write!(f, "object {object_id} could not handle an event at {:.3}s", time.as_secs_f64())
            }
            RunError::ZeroTimeLoop { time } => {
                write!(f, "simulation time is stuck at {:.3}s: events keep happening without it advancing", time.as_secs_f64())
            }
            RunError::Hook(failure) => {
                write!(f, "user code failed at {} in {}: {}", crate::time::format_time_exact(failure.time), crate::hooks::describe(&failure.call), failure.message)
            }
            RunError::BeyondTimeLimit { time } => write!(
                f,
                "a run can't go beyond {} (about 8,900 years), the latest time an MU can hold as when it left its Source, and was asked to reach {}",
                crate::time::format_time(crate::flows::TIME_LIMIT),
                crate::time::format_time(*time)
            ),
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
