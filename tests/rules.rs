//! Rules (ADR 0030) at the run's interface: when/then lines an object reads in order as it passes
//! an MU on, each kind of condition and action, the best by a figure among those free against
//! waiting for it, a stretch read from buckets, a conveyor and a cross conveyor deciding by rules,
//! and Reset giving the same again.

use crabsim::figures::Figure;
use crabsim::random_numbers::Distribution;
use crabsim::rules::{Action, Compare, Condition, Operand, Reading, RuleLine, RuleScope, Rules};
use crabsim::{FailureProfile, ModelDescription, MuKind, SimTime, SimulationRun};

fn never() -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0)
}

fn secs(secs: u64) -> SimTime {
    SimTime::from_secs(secs)
}

/// A Source every `interval`, a dummy Station deciding, A and B working `a` and `b`, one Drain.
struct Split {
    model: ModelDescription,
    source: u32,
    split: u32,
    a: u32,
    b: u32,
}

fn split(interval: u64, a: u64, b: u64) -> Split {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(interval));
    let split = model.add_station(SimTime::ZERO, never());
    let station_a = model.add_station(secs(a), never());
    let station_b = model.add_station(secs(b), never());
    let drain = model.add_drain();
    model.connect(source, split);
    model.connect(split, station_a);
    model.connect(split, station_b);
    model.connect(station_a, drain);
    model.connect(station_b, drain);
    Split { model, source, split, a: station_a, b: station_b }
}

fn line(when: Vec<Vec<Condition>>, then: Action) -> RuleLine {
    RuleLine { when, then }
}

fn run_for(model: &ModelDescription, length: SimTime) -> SimulationRun {
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(length).unwrap();
    run
}

fn mus_in(run: &SimulationRun, id: u32) -> u64 {
    run.state().objects[id as usize].statistics.mus_in
}

#[test]
fn a_named_successor_takes_every_mu_and_a_line_that_doesnt_hold_leaves_it_to_the_next() {
    let mut s = split(10, 1, 1);
    s.model.set_rules(s.split, Rules { lines: vec![line(Vec::new(), Action::To(1))] });
    let run = run_for(&s.model, secs(3600));
    assert_eq!(mus_in(&run, s.a), 0);
    assert!(mus_in(&run, s.b) > 300);

    // A line whose condition never holds hands on to the next, and with none left, the object's
    // own strategy, round robin, decides.
    let never_true = Condition::TimeOfDay { compare: Compare::Below, time: SimTime::ZERO };
    s.model.set_rules(s.split, Rules { lines: vec![line(vec![vec![never_true]], Action::To(1))] });
    let run = run_for(&s.model, secs(3600));
    let (a, b) = (mus_in(&run, s.a), mus_in(&run, s.b));
    assert!(a.abs_diff(b) <= 1, "round robin: {a} and {b}");
}

#[test]
fn the_mus_class_and_next_destination_choose() {
    let mut s = split(10, 1, 1);
    let frame = s.model.add_mu_kind(MuKind { length: 1.2, ..MuKind::default() });
    // A second Source of frames into the split.
    let frames = s.model.add_source(secs(30));
    s.model.set_mu_kind(frames, frame);
    s.model.connect(frames, s.split);
    s.model.set_rules(
        s.split,
        Rules {
            lines: vec![
                line(vec![vec![Condition::Class { kinds: vec![frame], equal: true }]], Action::To(0)),
                line(Vec::new(), Action::To(1)),
            ],
        },
    );
    let run = run_for(&s.model, secs(3600));
    let (a, b) = (mus_in(&run, s.a), mus_in(&run, s.b));
    assert!((115..=121).contains(&a), "only the frames, one every 30 s: {a}");
    assert!((355..=361).contains(&b), "every other MU, one every 10 s: {b}");
    let _ = s.source;
}

#[test]
fn the_time_of_day_chooses() {
    let mut s = split(60, 1, 1);
    let morning = Condition::TimeOfDay { compare: Compare::Below, time: secs(12 * 3600) };
    s.model.set_rules(s.split, Rules { lines: vec![line(vec![vec![morning]], Action::To(0)), line(Vec::new(), Action::To(1))] });
    let mut run = s.model.build(0);
    run.init().unwrap();
    run.advance_to(secs(12 * 3600 - 1)).unwrap();
    let a_by_noon = mus_in(&run, s.a);
    assert_eq!(mus_in(&run, s.b), 0, "every MU before noon to A");
    run.advance_to(secs(24 * 3600 - 1)).unwrap();
    assert_eq!(mus_in(&run, s.a), a_by_noon, "from noon on only to B, the MU at noon itself included");
    assert!(mus_in(&run, s.b) > 700);
    // The next morning, A again.
    run.advance_to(secs(30 * 3600)).unwrap();
    assert!(mus_in(&run, s.a) > a_by_noon + 300);
}

/// The best by a figure: A works 8 s, B 12 s, an MU every 5 s, so both are busy, and the split
/// waits whenever the one it wants isn't free.
fn best(wait: bool) -> SimulationRun {
    let mut s = split(5, 8, 12);
    let action = Action::Best { figure: Figure::WorkingShare, scope: RuleScope::SinceStart, largest: false, among: Vec::new(), wait };
    s.model.set_rules(s.split, Rules { lines: vec![line(Vec::new(), action)] });
    run_for(&s.model, secs(8 * 3600))
}

#[test]
fn the_best_by_a_figure_goes_to_the_first_free_among_those_free_and_waits_for_the_best_with_wait() {
    let free = best(false);
    let waiting = best(true);
    let blocked = |run: &SimulationRun| {
        let statistics = run.state().objects[1].statistics;
        statistics.time_blocked.as_secs_f64() / statistics.measured_time().as_secs_f64()
    };
    let out = |run: &SimulationRun| run.state().objects[4].statistics.mus_in;
    assert!(blocked(&waiting) > blocked(&free) + 0.05, "waiting for the best blocks more: {} against {}", blocked(&waiting), blocked(&free));
    assert!(out(&free) > out(&waiting), "and ships less: {} against {}", out(&free), out(&waiting));
    // Among those free, both Stations stay about equally busy, the least busy taking when free.
    let share = |run: &SimulationRun, id: usize| {
        let statistics = run.state().objects[id].statistics;
        statistics.time_busy.as_secs_f64() / statistics.measured_time().as_secs_f64()
    };
    assert!((share(&free, 2) - share(&free, 3)).abs() < 0.05, "{} and {}", share(&free, 2), share(&free, 3));
}

#[test]
fn a_figure_over_the_last_stretch_follows_a_change_that_since_start_lags_behind() {
    // A watched line: Source every 1 s into X, working 0.5 s, then 0.9 s from 2 h on.
    let rule = |scope: RuleScope| {
        let mut s = split(10, 1, 1);
        let feed = s.model.add_source(secs(1));
        let x = s.model.add_station(SimTime::from_millis(500), never());
        let drain = s.model.add_drain();
        s.model.connect(feed, x);
        s.model.connect(x, drain);
        let busy = Condition::Compare { left: Reading { object: x, figure: Figure::WorkingShare, scope }, compare: Compare::AtLeast, right: Operand::Number(0.8) };
        s.model.set_rules(s.split, Rules { lines: vec![line(vec![vec![busy]], Action::To(1)), line(Vec::new(), Action::To(0))] });
        let mut run = s.model.build(0);
        run.init().unwrap();
        run.advance_to(secs(2 * 3600)).unwrap();
        assert_eq!(mus_in(&run, s.b), 0, "X works half the time");
        run.set_process_time(x, SimTime::from_millis(900)).unwrap();
        run.advance_to(secs(3 * 3600)).unwrap();
        let a_at_3h = mus_in(&run, s.a);
        run.advance_to(secs(4 * 3600)).unwrap();
        (mus_in(&run, s.a) - a_at_3h, mus_in(&run, s.b))
    };
    let (a_late, b) = rule(RuleScope::Last(secs(3600)));
    assert_eq!(a_late, 0, "over the last hour X works 90%, so from 3 h every MU goes to B");
    assert!(b > 400, "{b}");
    let (_, b) = rule(RuleScope::SinceStart);
    assert_eq!(b, 0, "since start X works about 63% at 3 h and 70% at 4 h, never 80%");
}

#[test]
fn round_robin_among_some_and_another_objects_state_choose() {
    let mut s = split(10, 1, 1);
    let c = s.model.add_station(secs(1), never());
    let drain = s.model.add_drain();
    s.model.connect(s.split, c);
    s.model.connect(c, drain);
    s.model.set_rules(s.split, Rules { lines: vec![line(Vec::new(), Action::RoundRobin { among: vec![1, 2] })] });
    let run = run_for(&s.model, secs(3600));
    assert_eq!(mus_in(&run, s.a), 0);
    assert!(mus_in(&run, s.b).abs_diff(mus_in(&run, c)) <= 1);
    // A Station in its state: B is idle almost always, so the rule holds.
    let idle = Condition::State { object: s.b, state: crabsim::rules::StateName::Idle, equal: true };
    s.model.set_rules(s.split, Rules { lines: vec![line(vec![vec![idle]], Action::To(0)), line(Vec::new(), Action::To(2))] });
    let run = run_for(&s.model, secs(3600));
    assert_eq!((mus_in(&run, s.b), mus_in(&run, c)), (0, 0));
}

#[test]
fn a_conveyor_and_a_cross_conveyor_decide_by_rules() {
    // A conveyor handing to two Stations.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = model.add_conveyor(4.0, 0.6, 0.5);
    let a = model.add_station(secs(1), never());
    let b = model.add_station(secs(1), never());
    let drain = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, a);
    model.connect(belt, b);
    model.connect(a, drain);
    model.connect(b, drain);
    model.set_rules(belt, Rules { lines: vec![line(Vec::new(), Action::To(1))] });
    let run = run_for(&model, secs(3600));
    assert_eq!(mus_in(&run, a), 0);
    assert!(mus_in(&run, b) > 300);

    // The cross line's Fork sends every MU to its first exit, whatever its exits' lists say.
    let mut model = crabsim::demo::cross_line();
    let fork = (0..model.object_count() as u32).find(|&id| model.name(id) == Some("Fork")).unwrap();
    let successors = model.successors(fork).to_vec();
    let free = run_for(&model, secs(4 * 3600));
    assert!(successors.iter().all(|&id| mus_in(&free, id) > 0), "by its exits' lists, both get MUs");
    model.set_rules(fork, Rules { lines: vec![line(Vec::new(), Action::To(0))] });
    let ruled = run_for(&model, secs(4 * 3600));
    assert!(mus_in(&ruled, successors[0]) > 0);
    assert_eq!(mus_in(&ruled, successors[1]), 0);
}

#[test]
fn reset_and_a_copy_of_a_run_give_the_same_again() {
    let mut s = split(5, 8, 12);
    let action = Action::Best { figure: Figure::WorkingShare, scope: RuleScope::Last(secs(600)), largest: false, among: Vec::new(), wait: false };
    s.model.set_rules(s.split, Rules { lines: vec![line(Vec::new(), action)] });
    let mut run = run_for(&s.model, secs(3600));
    let first = run.state();
    let mut copy = s.model.build(0);
    copy.init().unwrap();
    copy.advance_to(secs(1800)).unwrap();
    let mut clone = copy.clone();
    clone.advance_to(secs(3600)).unwrap();
    assert_eq!(clone.state(), first, "a copy halfway carries its buckets");
    run.reset();
    run.init().unwrap();
    run.advance_to(secs(3600)).unwrap();
    assert_eq!(run.state(), first, "Reset clears the buckets");
}
