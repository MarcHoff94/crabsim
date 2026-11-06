pub mod terminal;
pub mod topology;

use crate::error::{AdvanceFailure, RunError};
use crate::observation::{Occurrence, RunState};
use crate::simulation_run::SimulationRun;
use std::time::Duration;

/// Something that shows a simulation run as it advances.
pub trait RunView {
    fn show(&mut self, occurrences: &[Occurrence], state: &RunState);
}

/// Advances a run up to and including `until`, showing the view each event's result. If that
/// ends in an error, the view has first been shown whatever happened before it.
pub fn advance_with_view(
    run: &mut SimulationRun,
    until: Duration,
    view: &mut dyn RunView,
) -> Result<(), RunError> {
    while run.next_event_time().is_some_and(|time| time <= until) {
        let occurrences = run.step().map_err(|failure| show_failure(run, view, failure))?;
        view.show(&occurrences, &run.state());
    }
    let occurrences = run.advance_to(until).map_err(|failure| show_failure(run, view, failure))?;
    view.show(&occurrences, &run.state());
    Ok(())
}

fn show_failure(run: &SimulationRun, view: &mut dyn RunView, failure: AdvanceFailure) -> RunError {
    if !failure.occurrences.is_empty() {
        view.show(&failure.occurrences, &run.state());
    }
    failure.error
}
