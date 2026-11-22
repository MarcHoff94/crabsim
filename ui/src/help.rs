//! What each state means, for the inspector's help on an object: the states its type can be in,
//! crabsim's own meaning of each, worded from GLOSSARY.md, and beside it the model's own note, if
//! the class or instance has one. A note never replaces the meaning, so what crabsim measures as
//! Blocked stays clear whatever a model says about it. Plain Rust, tested without a window.

use crabsim::ObjectKind;
use crabsim_model::State;
use serde_json::Value;

/// One state as the help shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct StateHelp {
    pub state: State,
    /// What crabsim means by it, for this type of object.
    pub meaning: &'static str,
    /// The model's own note on it, from `state_notes`.
    pub note: Option<String>,
}

/// The built-in class a kind of object is, as model files name it.
pub fn built_in(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Source => "Source",
        ObjectKind::Station => "Station",
        ObjectKind::Buffer => "Buffer",
        ObjectKind::Drain => "Drain",
        ObjectKind::Conveyor => "Conveyor",
    }
}

/// Every state an object of `kind` can be in, in a fixed order, with what it means and the note
/// from `notes`, the instance's `state_notes` as a model file writes them.
pub fn states(kind: ObjectKind, notes: Option<&Value>) -> Vec<StateHelp> {
    State::of(built_in(kind))
        .iter()
        .map(|&state| StateHelp {
            state,
            meaning: meaning(kind, state),
            note: notes.and_then(|notes| notes.get(state.name())).and_then(Value::as_str).map(str::to_string),
        })
        .collect()
}

/// What crabsim means by `state` for an object of `kind`.
pub fn meaning(kind: ObjectKind, state: State) -> &'static str {
    match (kind, state) {
        (ObjectKind::Source, State::Busy) => "Making its next MU: waiting out its interval before offering it on.",
        (ObjectKind::Source, State::Blocked) => "Holding an MU that is ready to leave, but no successor can take it.",
        (ObjectKind::Buffer, State::Blocked) => "Holding an MU that may leave, but no successor can take it.",
        (ObjectKind::Buffer, State::Idle) => "Empty, waiting for an MU.",
        (_, State::Holding) => "Holding at least one MU, none of which is kept from leaving: each is still in its dwelling time.",
        (ObjectKind::Drain, State::Idle) => "Ready to take MUs out of the model. It takes each one at once, so it is idle unless interrupted.",
        (ObjectKind::Source, State::Failed) => "Interrupted by a random failure until its repair is done. Its interval waits, so its next MU comes later.",
        (ObjectKind::Source, State::Stopped) => "Stopped by hand, until someone ends it. Its interval waits. It counts as a failure in the statistics.",
        (ObjectKind::Source, State::Paused) => "Paused until someone resumes it. Its interval waits, so its next MU comes later.",
        (ObjectKind::Buffer, State::Failed) => "Interrupted by a random failure until its repair is done. It takes and releases no MU.",
        (ObjectKind::Buffer, State::Stopped) => "Stopped by hand, until someone ends it. It takes and releases no MU. It counts as a failure in the statistics.",
        (ObjectKind::Buffer, State::Paused) => "Paused until someone resumes it. It takes and releases no MU.",
        (ObjectKind::Drain, State::Failed) => "Interrupted by a random failure until its repair is done. It takes no MU, so what feeds it waits.",
        (ObjectKind::Drain, State::Stopped) => "Stopped by hand, until someone ends it. It takes no MU. It counts as a failure in the statistics.",
        (ObjectKind::Drain, State::Paused) => "Paused until someone resumes it. It takes no MU, so what feeds it waits.",
        (ObjectKind::Conveyor, State::Busy) => "Moving at least one MU along its path.",
        (ObjectKind::Conveyor, State::Blocked) => "Holding MUs, none of which moves: the one at its end waits for a successor to take it, and the rest have closed up behind it.",
        (ObjectKind::Conveyor, State::Idle) => "Empty: no MU is on it.",
        (ObjectKind::Conveyor, State::Failed) => "Interrupted by a random failure until its repair is done. Every MU on it stands, and it takes and passes on none.",
        (ObjectKind::Conveyor, State::Stopped) => "Stopped by hand, until someone ends it. Every MU on it stands, and it takes and passes on none. It counts as a failure in the statistics.",
        (ObjectKind::Conveyor, State::Paused) => "Paused until someone resumes it. Every MU on it stands, and it takes and passes on none.",
        (_, State::Receiving) => "A place is taken for an MU a conveyor is handing over, until the MU's booking point arrives, and nothing else is going on. Once begun, the MU always arrives, even if it is interrupted meanwhile.",
        (_, State::Busy) => "Working on an MU.",
        (_, State::Blocked) => "Holding an MU whose work is done, but no successor can take it.",
        (_, State::Idle) => "Not busy, blocked or interrupted: waiting for an MU to arrive.",
        (_, State::Failed) => "Interrupted by a random failure until its repair is done. The work on its MU waits.",
        (_, State::Stopped) => "Stopped by hand, until someone ends it. The work on its MU waits. It counts as a failure in the statistics.",
        (_, State::Paused) => "Paused until someone resumes it. The work on its MU waits.",
    }
}
