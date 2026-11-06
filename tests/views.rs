//! The views that consume what a simulation run reports.

use crabsim::visualization::terminal::render_table;
use crabsim::visualization::{advance_with_view, RunView};
use crabsim::{FailureProfile, ModelDescription, Occurrence, RunError, RunState, SimTime, SimulationRun};
use crabsim::random_numbers::Distribution;

fn line() -> SimulationRun {
    let fp = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut model = ModelDescription::new();
    let src = model.add_source(SimTime::from_secs(10));
    let st = model.add_station(SimTime::from_secs(60), fp);
    let dr = model.add_drain();
    model.connect(src, st);
    model.connect(st, dr);
    model.build(0)
}

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
fn a_view_receives_the_same_occurrences_as_a_plain_advance() {
    let until = SimTime::from_secs(200);
    let mut plain = line();
    plain.init().unwrap();
    let expected = plain.advance_to(until).unwrap();

    let mut run = line();
    run.init().unwrap();
    let mut recorder = Recorder::default();
    advance_with_view(&mut run, until, &mut recorder).unwrap();

    assert_eq!(recorder.occurrences, expected);
    assert_eq!(run.time(), until);
    assert_eq!(recorder.frames.last(), Some(&until));
    assert!(recorder.frames.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn a_view_is_shown_nothing_when_nothing_happened_before_the_error() {
    let mut run = line();
    let mut recorder = Recorder::default();
    let error = advance_with_view(&mut run, SimTime::from_secs(10), &mut recorder).unwrap_err();
    assert_eq!(error, RunError::NotInitialised);
    assert!(recorder.frames.is_empty());
}

#[test]
fn the_terminal_table_shows_one_row_per_object_with_its_state() {
    let mut run = line();
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(30)).unwrap();
    let table = render_table(&run.state());

    assert!(table.contains("State at time 30.000"), "{table}");
    let station_row = table.lines().find(|l| l.contains("Station")).expect("a station row");
    assert!(station_row.contains("MU1"), "{station_row}");
    assert!(station_row.contains("Process"), "{station_row}");
    assert!(station_row.contains("40.000"), "{station_row}");
    assert!(table.lines().any(|l| l.contains("Source")));
    assert!(table.lines().any(|l| l.contains("Drain")));
}
