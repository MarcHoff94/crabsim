//! The views that consume what a simulation run reports.

use crabsim::visualization::terminal::render_table;
use crabsim::visualization::{advance_with_view, RunView};
use crabsim::{Drain, FailureProfile, Occurrence, RunState, SimulationRun, SingleStation, Source};
use crabsim::random_numbers::Distribution;
use std::{cell::RefCell, rc::Rc, time::Duration};

fn line() -> SimulationRun {
    let fp = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let st = Rc::new(RefCell::new(SingleStation::new(0, Duration::from_secs(60), fp, None, Vec::new(), Vec::new())));
    let src = Rc::new(RefCell::new(Source::new(0, Duration::from_secs(10), Vec::new())));
    let dr = Rc::new(RefCell::new(Drain::new(0, Vec::new())));
    src.borrow_mut().succ = vec![st.clone()];
    st.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(st);
    run.add_object(dr);
    run
}

#[derive(Default)]
struct Recorder {
    occurrences: Vec<Occurrence>,
    frames: Vec<Duration>,
}

impl RunView for Recorder {
    fn show(&mut self, occurrences: &[Occurrence], state: &RunState) {
        self.occurrences.extend_from_slice(occurrences);
        self.frames.push(state.time);
    }
}

#[test]
fn a_view_receives_the_same_occurrences_as_a_plain_advance() {
    let until = Duration::from_secs(200);
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
fn the_terminal_table_shows_one_row_per_object_with_its_state() {
    let mut run = line();
    run.init().unwrap();
    run.advance_to(Duration::from_secs(30)).unwrap();
    let table = render_table(&run.state());

    assert!(table.contains("State at time 30.000"), "{table}");
    let station_row = table.lines().find(|l| l.contains("Station")).expect("a station row");
    assert!(station_row.contains("MU1"), "{station_row}");
    assert!(station_row.contains("Process"), "{station_row}");
    assert!(station_row.contains("40.000"), "{station_row}");
    assert!(table.lines().any(|l| l.contains("Source")));
    assert!(table.lines().any(|l| l.contains("Drain")));
}
