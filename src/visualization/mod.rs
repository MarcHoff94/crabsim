pub mod terminal;
pub mod topology;

use crate::error::{AdvanceFailure, RunError};
use crate::observation::{Occurrence, RunState};
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;

/// Something that shows a simulation run as it advances.
pub trait RunView {
    fn show(&mut self, occurrences: &[Occurrence], state: &RunState);
}

/// Advances a run up to and including `until`, showing the view each event's result. If that
/// ends in an error, the view has first been shown whatever happened before it.
pub fn advance_with_view(
    run: &mut SimulationRun,
    until: SimTime,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materialflow_objects::records_then_fails::RecordsThenFails;
    use crate::materialflow_objects::ModelObject;
    use crate::observation::OccurrenceKind;

    #[derive(Default)]
    struct Recorder {
        occurrences: Vec<Occurrence>,
        frames: Vec<SimTime>,
    }

    impl RunView for Recorder {
        fn show(&mut self, occurrences: &[Occurrence], state: &RunState) {
            self.occurrences.extend_from_slice(occurrences);
            self.frames.push(state.time);
        }
    }

    #[test]
    fn a_view_sees_what_an_event_recorded_before_it_failed() {
        let at = SimTime::from_secs(5);
        let mut run = SimulationRun::new(vec![ModelObject::RecordsThenFails(RecordsThenFails::new(at))], 0);
        run.init().unwrap();
        let mut recorder = Recorder::default();

        let error = advance_with_view(&mut run, SimTime::from_secs(10), &mut recorder).unwrap_err();

        assert_eq!(error, RunError::ObjectFailed { object_id: 0, time: at });
        assert_eq!(recorder.occurrences, vec![Occurrence { time: at, kind: OccurrenceKind::Blocked { object_id: 0 } }]);
        assert_eq!(recorder.frames, vec![at]);
    }
}
