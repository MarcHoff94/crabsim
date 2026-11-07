use super::RunView;
use crate::flows::ProcessState;
use crate::observation::{ObjectKind, ObjectState, Occurrence, RunState, StopCause};
use std::fmt::Write as _;
use std::io::Write as _;
use crate::time::SimTime;

fn row(object: &ObjectState) -> String {
    let kind = match object.kind {
        ObjectKind::Source => "Source",
        ObjectKind::Station => "Station",
        ObjectKind::Buffer => "Buffer",
        ObjectKind::Drain => "Drain",
    };
    let contents = if object.contents.is_empty() {
        "-".to_string()
    } else {
        object.contents.iter().map(|id| format!("MU{id}")).collect::<Vec<_>>().join(",")
    };
    let process = match (object.kind, &object.process) {
        (ObjectKind::Drain, _) => "-",
        (_, ProcessState::Processing) => "Process",
        (_, ProcessState::Blocked) => "Blocked",
        (_, ProcessState::Idle) => "Idle",
    };
    let stop = match object.stop {
        None => "-",
        Some(StopCause::Failure) => "Failed",
        Some(StopCause::ForcedFailure) => "Forced to fail",
        Some(StopCause::Pause) => "Paused",
    };
    let blocked = if object.blocked_predecessors.is_empty() {
        "-".to_string()
    } else {
        object.blocked_predecessors.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")
    };
    format!(
        "│ {:3} │ {:8} │ {:8} │ {:10} │ {:7} │ {:12} │ {:15} │",
        object.id,
        kind,
        contents,
        process,
        stop,
        object.remaining.unwrap_or(SimTime::ZERO),
        blocked
    )
}

/// The state of a run as a table of one row per material-flow object.
pub fn render_table(state: &RunState) -> String {
    let title = format!("State at time {}", state.time);
    let mut table = String::new();
    let _ = writeln!(table, "╔═════════════════════════════════════════════════════════════════════════════════╗");
    let _ = writeln!(table, "║ {:<80}║", title);
    let _ = writeln!(table, "╚═════════════════════════════════════════════════════════════════════════════════╝");
    let _ = writeln!(table, "\n┌─────┬──────────┬──────────┬────────────┬─────────┬──────────────┬─────────────────┐");
    let _ = writeln!(table, "│ {:3} │ {:8} │ {:8} │ {:10} │ {:7} │ {:12} │ {:15} │", "ID", "Type", "Content", "State", "Interr", "RemTime", "Blocked");
    let _ = writeln!(table, "├─────┼──────────┼──────────┼────────────┼─────────┼──────────────┼─────────────────┤");
    for object in &state.objects {
        let _ = writeln!(table, "{}", row(object));
    }
    let _ = writeln!(table, "└─────┴──────────┴──────────┴────────────┴─────────┴──────────────┴─────────────────┘");
    table
}

/// Redraws the topology and state table in place after every event.
pub struct InPlaceTerminalView {
    topology: String,
}

impl InPlaceTerminalView {
    pub fn new(topology: String) -> InPlaceTerminalView {
        InPlaceTerminalView { topology }
    }
}

impl RunView for InPlaceTerminalView {
    fn show(&mut self, _occurrences: &[Occurrence], state: &RunState) {
        print!("\x1B[2J\x1B[H{}{}", self.topology, render_table(state));
        let _ = std::io::stdout().flush();
    }
}
