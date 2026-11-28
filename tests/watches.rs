//! Which MUs an object takes (ADR 0030), at the run's interface: a run told to watch objects
//! counts the MUs entering each that break its limit, by class or current step, `in` or `not in`,
//! and keeps the first with its time, MU, kind and step; a warm-up and Reset start it afresh, a
//! copy carries it on, and a run that watches nothing reports nothing and runs as before.

use crabsim::flows::NO_DESTINATION;
use crabsim::random_numbers::Distribution;
use crabsim::routing::PlanStep;
use crabsim::rules::{Action, Condition, RuleLine, Rules};
use crabsim::watch::{Break, Watch, WatchState, Watched};
use crabsim::{FailureProfile, ModelDescription, MuKind, MuKindId, Recording, SimTime, SimulationRun};

fn never() -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0)
}

fn secs(secs: u64) -> SimTime {
    SimTime::from_secs(secs)
}

/// Plain MUs and frames, each every 100 s, into Mill, then Rework, then a Drain. Frames follow a
/// plan of Mill then Drain; plain MUs have none. Mill sends every MU it works on to Rework.
struct Line {
    model: ModelDescription,
    mill: u32,
    rework: u32,
    drain: u32,
    frame: MuKindId,
    at_mill: u16,
    at_rework: u16,
}

fn line() -> Line {
    let mut model = ModelDescription::new();
    let plain = model.add_source(secs(100));
    let frames = model.add_source(secs(100));
    let mill = model.add_station(secs(10), never());
    let rework = model.add_station(secs(5), never());
    let drain = model.add_drain();
    for (from, to) in [(plain, mill), (frames, mill), (mill, rework), (rework, drain)] {
        model.connect(from, to);
    }
    let at_mill = model.add_destination(&[mill]);
    let at_rework = model.add_destination(&[rework]);
    let at_drain = model.add_destination(&[drain]);
    let frame = model.add_mu_kind_with_plan(MuKind { length: 1.2, ..MuKind::default() }, vec![PlanStep { at: at_mill, process_time: None }, PlanStep { at: at_drain, process_time: None }]);
    model.set_mu_kind(frames, frame);
    model.set_detour(mill, at_rework);
    Line { model, mill, rework, drain, frame, at_mill, at_rework }
}

fn watched(model: &ModelDescription, watches: Vec<Watch>, until: u64) -> SimulationRun {
    let mut run = model.build(0);
    run.watch(watches);
    run.init().unwrap();
    run.advance_to(secs(until)).unwrap();
    run
}

fn mus_in(run: &SimulationRun, id: u32) -> u64 {
    run.state().objects[id as usize].statistics.mus_in
}

#[test]
fn a_class_limit_counts_the_mus_of_other_classes_and_keeps_the_first() {
    let l = line();
    let only_frames = Watch { object: l.mill, watched: Watched::Class(vec![l.frame]), inside: true };
    let no_frames = Watch { object: l.mill, watched: Watched::Class(vec![l.frame]), inside: false };
    let run = watched(&l.model, vec![only_frames, no_frames], 1000);
    let [only, none] = run.state().watches[..] else { panic!("two watches") };
    let total = mus_in(&run, l.mill);
    assert!(total >= 18, "{total}");
    assert_eq!(only.breaks + none.breaks, total, "every MU breaks exactly one of them");
    // Both Sources offer at 100 s: the plain MU, from the first, goes first.
    assert_eq!(only.first, Some(Break { time: secs(100), object: l.mill, mu: 1, kind: 0, step: NO_DESTINATION }));
    assert_eq!(none.first.map(|first| (first.mu, first.kind, first.step)), Some((2, l.frame, l.at_mill)));
}

#[test]
fn a_step_limit_reads_the_step_an_mu_has_as_it_enters_and_breaks_without_one() {
    let l = line();
    // At Mill, frames are on their step Mill; plain MUs have no step, which breaks `in` and `not
    // in` alike.
    let at_mill = Watch { object: l.mill, watched: Watched::Step(vec![l.at_mill]), inside: true };
    // At Rework, every MU is on the detour Mill gave it: frames and plain MUs alike.
    let at_rework = Watch { object: l.rework, watched: Watched::Step(vec![l.at_rework]), inside: true };
    let not_rework = Watch { object: l.rework, watched: Watched::Step(vec![l.at_rework]), inside: false };
    // At Mill, not on the way to Rework: frames meet it, plain MUs break it without a step.
    let mill_not_rework = Watch { object: l.mill, watched: Watched::Step(vec![l.at_rework]), inside: false };
    let run = watched(&l.model, vec![at_mill, at_rework, not_rework, mill_not_rework], 1000);
    let states = run.state().watches;
    let plain_at_mill = mus_in(&run, l.mill).div_ceil(2);
    assert_eq!(states[0].breaks, plain_at_mill, "the plain MUs, without a step");
    assert_eq!(states[0].first.map(|first| first.step), Some(NO_DESTINATION));
    assert_eq!(states[1], WatchState::default(), "every MU on its detour");
    assert_eq!(states[2].breaks, mus_in(&run, l.rework));
    assert_eq!(states[2].first.map(|first| first.step), Some(l.at_rework));
    assert_eq!(states[3].breaks, plain_at_mill, "an MU without a step breaks `not in` too");
    let _ = l.drain;
}

#[test]
fn a_warm_up_and_reset_start_it_afresh_and_a_copy_carries_it_on() {
    let l = line();
    let only_frames = Watch { object: l.mill, watched: Watched::Class(vec![l.frame]), inside: true };
    let mut run = watched(&l.model, vec![only_frames], 1000);
    assert!(run.state().watches[0].breaks > 0);
    let mut copy = run.clone();
    run.clear_statistics();
    assert_eq!(run.state().watches[0], WatchState::default());
    run.advance_to(secs(2000)).unwrap();
    let after = run.state().watches[0];
    assert!(after.first.is_some_and(|first| first.time > secs(1000)));
    copy.advance_to(secs(2000)).unwrap();
    assert!(copy.state().watches[0].breaks > after.breaks, "the copy kept what it saw before");

    run.reset();
    assert_eq!(run.state().watches[0], WatchState::default());
    run.init().unwrap();
    run.advance_to(secs(1000)).unwrap();
    assert_eq!(run.state().watches, watched(&l.model, vec![Watch { object: l.mill, watched: Watched::Class(vec![l.frame]), inside: true }], 1000).state().watches);
}

#[test]
fn a_run_that_watches_nothing_reports_nothing_and_watching_changes_no_result() {
    let l = line();
    let mut plain = l.model.build(0);
    plain.set_recording(Recording::ALL);
    let mut occurrences = plain.init().unwrap();
    occurrences.extend(plain.advance_to(secs(3000)).unwrap());
    assert!(plain.state().watches.is_empty());

    let mut watching = l.model.build(0);
    watching.set_recording(Recording::ALL);
    watching.watch(vec![Watch { object: l.rework, watched: Watched::Class(vec![l.frame]), inside: false }]);
    let mut seen = watching.init().unwrap();
    seen.extend(watching.advance_to(secs(3000)).unwrap());
    assert_eq!(occurrences, seen);
    assert_eq!(plain.state().objects, watching.state().objects);
}

#[test]
fn a_distribution_rule_reads_the_step_too() {
    // A dummy Station sends MUs whose step is Mill to Mill and the rest to Rework.
    let mut model = ModelDescription::new();
    let plain = model.add_source(secs(100));
    let frames = model.add_source(secs(100));
    let split = model.add_station(SimTime::ZERO, never());
    let mill = model.add_station(secs(10), never());
    let rework = model.add_station(secs(10), never());
    let drain = model.add_drain();
    for (from, to) in [(plain, split), (frames, split), (split, mill), (split, rework), (mill, drain), (rework, drain)] {
        model.connect(from, to);
    }
    let at_mill = model.add_destination(&[mill]);
    let at_drain = model.add_destination(&[drain]);
    let frame = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: at_mill, process_time: None }, PlanStep { at: at_drain, process_time: None }]);
    model.set_mu_kind(frames, frame);
    model.set_rules(split, Rules { lines: vec![RuleLine { when: vec![vec![Condition::Step { destinations: vec![at_mill], equal: true }]], then: Action::To(0) }, RuleLine { when: Vec::new(), then: Action::To(1) }] });
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(2000)).unwrap();
    let state = run.state();
    let (to_mill, to_rework) = (state.objects[mill as usize].statistics.mus_in, state.objects[rework as usize].statistics.mus_in);
    assert!(to_mill >= 19 && to_rework >= 19 && to_mill.abs_diff(to_rework) <= 1, "{to_mill} and {to_rework}");
}
