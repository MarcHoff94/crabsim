//! A Station's process time by MU (ADR 0030) at the run's interface: when/then lines read as an
//! MU enters, each kind of condition, the step, a plan step's own time winning, the Station's own
//! time as the default an intervention changes, a Station with capacity deciding at entry, lines
//! reading the giver and other objects, a stretch, and Reset and a copy giving the same again.

use crabsim::figures::Figure;
use crabsim::flows::NO_DESTINATION;
use crabsim::random_numbers::Distribution;
use crabsim::routing::PlanStep;
use crabsim::rules::{Compare, Condition, Operand, Reading, RuleScope, StateName, TimeLine, TimeRules};
use crabsim::{EventType, FailureProfile, ModelDescription, MuKind, Occurrence, OccurrenceKind, Recording, SimTime, SimulationRun, StartMode};

fn never() -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0)
}

fn secs(secs: u64) -> SimTime {
    SimTime::from_secs(secs)
}

fn line(when: Vec<Vec<Condition>>, then: Option<u64>) -> TimeLine {
    TimeLine { when, then: then.map(secs) }
}

fn run(model: &ModelDescription, until: u64) -> (SimulationRun, Vec<Occurrence>) {
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    (run, occurrences)
}

/// How long `at` worked on each MU, as (MU, seconds), in order.
fn work_at(occurrences: &[Occurrence], at: u32) -> Vec<(u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { mu_id, at: station, duration } if station == at => Some((mu_id, duration.as_secs())),
            _ => None,
        })
        .collect()
}

/// Two Sources into one Station of 20 s, plain MUs from the first and frames from the second,
/// each every 100 s, offered at the same instants, then a Drain. Plain MUs have odd ids and frames
/// even ones.
struct Line {
    model: ModelDescription,
    plain: u32,
    frames: u32,
    station: u32,
    frame: crabsim::MuKindId,
}

fn line_of_two() -> Line {
    let mut model = ModelDescription::new();
    let plain = model.add_source(secs(100));
    let frames = model.add_source(secs(100));
    let station = model.add_station(secs(20), never());
    let drain = model.add_drain();
    let frame = model.add_mu_kind(MuKind { length: 1.2, ..MuKind::default() });
    model.set_mu_kind(frames, frame);
    model.connect(plain, station);
    model.connect(frames, station);
    model.connect(station, drain);
    Line { model, plain, frames, station, frame }
}

#[test]
fn the_mus_class_gives_its_time_and_the_station_its_own_to_the_rest() {
    let mut l = line_of_two();
    l.model.set_time_rules(l.station, TimeRules { lines: vec![line(vec![vec![Condition::Class { kinds: vec![l.frame], equal: true }]], Some(45))] });
    let (_, occurrences) = run(&l.model, 1000);
    let work = work_at(&occurrences, l.station);
    assert!(work.len() >= 10, "{work:?}");
    // Both Sources offer at the same instants: whichever goes first, a frame takes 45 s and a
    // plain MU the Station's own 20 s.
    let frames: Vec<u64> = work.iter().filter(|(mu, _)| mu % 2 == 0).map(|(_, time)| *time).collect();
    let plain: Vec<u64> = work.iter().filter(|(mu, _)| mu % 2 == 1).map(|(_, time)| *time).collect();
    assert!(frames.iter().all(|&time| time == 45) && plain.iter().all(|&time| time == 20), "frames {frames:?}, plain {plain:?}");
    let _ = (l.plain, l.frames);
}

#[test]
fn default_and_no_line_holding_give_the_stations_own_time_which_an_intervention_changes() {
    let mut l = line_of_two();
    let frames = Condition::Class { kinds: vec![l.frame], equal: true };
    l.model.set_time_rules(l.station, TimeRules { lines: vec![line(vec![vec![frames]], None), line(Vec::new(), Some(5))] });
    let mut run = l.model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(500)).unwrap());
    run.set_process_time(l.station, secs(30)).unwrap();
    occurrences.extend(run.advance_to(secs(1000)).unwrap());
    let starts: Vec<(SimTime, u32, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { mu_id, at, duration } if at == l.station => Some((o.time, mu_id, duration.as_secs())),
            _ => None,
        })
        .collect();
    // `default` is the Station's own time, 20 s and then 30 s; `otherwise` 5 s for the rest.
    for (time, mu, duration) in starts {
        let expected = match (mu % 2 == 0, time < secs(500)) {
            (true, true) => 20,
            (true, false) => 30,
            (false, _) => 5,
        };
        assert_eq!(duration, expected, "MU {mu} at {time:?}");
    }
}

#[test]
fn a_plan_steps_own_time_wins_and_the_lines_decide_by_step_otherwise() {
    // Source -> Saw -> Mill -> Drain: MUs whose plan is Mill for 7 s, then Drain; Saw's lines
    // give 40 s to an MU whose step is Mill, which is every MU there, and Mill's lines would give
    // 90 s, but its plan step's own time wins.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let saw = model.add_station(secs(10), never());
    let mill = model.add_station(secs(20), never());
    let drain = model.add_drain();
    for (from, to) in [(source, saw), (saw, mill), (mill, drain)] {
        model.connect(from, to);
    }
    let at_mill = model.add_destination(&[mill]);
    let at_drain = model.add_destination(&[drain]);
    let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: at_mill, process_time: Some(secs(7)) }, PlanStep { at: at_drain, process_time: None }]);
    model.set_mu_kind(source, kind);
    let to_mill = Condition::Step { destinations: vec![at_mill], equal: true };
    model.set_time_rules(saw, TimeRules { lines: vec![line(vec![vec![to_mill.clone()]], Some(40))] });
    model.set_time_rules(mill, TimeRules { lines: vec![line(vec![vec![to_mill]], Some(90)), line(Vec::new(), Some(90))] });
    let (ran, occurrences) = run(&model, 1000);
    assert!(work_at(&occurrences, saw).iter().all(|&(_, time)| time == 40), "{:?}", work_at(&occurrences, saw));
    assert!(work_at(&occurrences, mill).iter().all(|&(_, time)| time == 7), "{:?}", work_at(&occurrences, mill));
    assert_eq!(ran.state().objects[mill as usize].plan.unwrap().off_plan, 0);

    // An MU without a plan has no step: `step != Mill` holds for it, `step = Mill` doesn't.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let saw = model.add_station(secs(10), never());
    let drain = model.add_drain();
    model.connect(source, saw);
    model.connect(saw, drain);
    let somewhere = model.add_destination(&[drain]);
    model.set_time_rules(
        saw,
        TimeRules {
            lines: vec![line(vec![vec![Condition::Step { destinations: vec![somewhere], equal: true }]], Some(1)), line(vec![vec![Condition::Step { destinations: vec![somewhere], equal: false }]], Some(3))],
        },
    );
    let (_, occurrences) = run(&model, 500);
    assert!(work_at(&occurrences, saw).iter().all(|&(_, time)| time == 3));
    assert_eq!(NO_DESTINATION, u16::MAX);
}

#[test]
fn the_time_of_day_and_another_objects_state_and_figures_decide() {
    // An MU every 60 s into a Station of 10 s by day, 30 s from 12:00.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(60));
    let station = model.add_station(secs(10), never());
    let other = model.add_station(secs(1), never());
    let drain = model.add_drain();
    model.connect(source, station);
    model.connect(station, drain);
    let afternoon = Condition::TimeOfDay { compare: Compare::AtLeast, time: secs(12 * 3600) };
    model.set_time_rules(station, TimeRules { lines: vec![line(vec![vec![afternoon]], Some(30))] });
    let (_, occurrences) = run(&model, 24 * 3600 - 1);
    let starts: Vec<(SimTime, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { at, duration, .. } if at == station => Some((o.time, duration.as_secs())),
            _ => None,
        })
        .collect();
    assert!(starts.iter().all(|&(time, duration)| duration == if time >= secs(12 * 3600) { 30 } else { 10 }));

    // Another object's state now: `other` idle the whole time.
    let idle = Condition::State { object: other, state: StateName::Idle, equal: true };
    model.set_time_rules(station, TimeRules { lines: vec![line(vec![vec![idle]], Some(2))] });
    let (_, occurrences) = run(&model, 600);
    assert!(work_at(&occurrences, station).iter().all(|&(_, time)| time == 2));

    // The giver's own figure: the Source has counted the MU it hands over as out, so from the
    // third MU on.
    let given = Condition::Compare { left: Reading { object: source, figure: Figure::MusOut, scope: RuleScope::SinceStart }, compare: Compare::AtLeast, right: Operand::Number(3.0) };
    model.set_time_rules(station, TimeRules { lines: vec![line(vec![vec![given]], Some(4))] });
    let (_, occurrences) = run(&model, 600);
    let times: Vec<u64> = work_at(&occurrences, station).iter().map(|&(_, time)| time).collect();
    assert_eq!(&times[..4], [10, 10, 4, 4], "{times:?}");
}

#[test]
fn a_conveyor_handing_over_is_read_as_the_giver() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(20));
    let belt = model.add_conveyor(2.0, 0.6, 1.0);
    let station = model.add_station(secs(10), never());
    let drain = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, station);
    model.connect(station, drain);
    let given = Condition::Compare { left: Reading { object: belt, figure: Figure::MusOut, scope: RuleScope::SinceStart }, compare: Compare::AtLeast, right: Operand::Number(2.0) };
    model.set_time_rules(station, TimeRules { lines: vec![line(vec![vec![given]], Some(5))] });
    let (_, occurrences) = run(&model, 600);
    let times: Vec<u64> = work_at(&occurrences, station).iter().map(|&(_, time)| time).collect();
    assert_eq!(&times[..3], [10, 5, 5], "{times:?}");
}

#[test]
fn a_station_with_capacity_decides_each_mus_time_as_it_enters_even_in_a_batch() {
    let mut l = line_of_two();
    l.model.set_station_capacity(l.station, 2, StartMode::Batch);
    l.model.set_time_rules(l.station, TimeRules { lines: vec![line(vec![vec![Condition::Class { kinds: vec![l.frame], equal: true }]], Some(45))] });
    let mut run = l.model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    // The first pair comes at 100 s and fills the batch at once; change the Station's own time
    // before the next pair: a frame's 45 s was decided as it entered, the plain MU's own time is
    // the one when work starts.
    occurrences.extend(run.advance_to(secs(150)).unwrap());
    run.set_process_time(l.station, secs(15)).unwrap();
    occurrences.extend(run.advance_to(secs(400)).unwrap());
    let work = work_at(&occurrences, l.station);
    for (mu, time) in &work {
        let expected = if mu % 2 == 0 { 45 } else if *mu <= 2 { 20 } else { 15 };
        assert_eq!(*time, expected, "MU {mu}: {work:?}");
    }
}

#[test]
fn a_stretch_is_read_from_buckets_the_run_keeps() {
    // `other` works an MU every 60 s until its feed is paused at 1 h, and from then on is paused
    // and resumed every 30 s, so its state keeps changing while it takes no MU. The Station's MUs
    // take 5 s while `other` took any in the last 10 minutes, and 50 s once it hasn't, which only
    // the buckets the run keeps can tell.
    let mut model = ModelDescription::new();
    let feed = model.add_source(secs(60));
    let other = model.add_station(secs(1), never());
    let source = model.add_source(secs(120));
    let station = model.add_station(secs(50), never());
    let drain = model.add_drain();
    model.connect(feed, other);
    model.connect(other, drain);
    model.connect(source, station);
    model.connect(station, drain);
    let busy_lately = Condition::Compare { left: Reading { object: other, figure: Figure::MusIn, scope: RuleScope::Last(secs(600)) }, compare: Compare::AtLeast, right: Operand::Number(1.0) };
    model.set_time_rules(station, TimeRules { lines: vec![line(vec![vec![busy_lately]], Some(5))] });
    let check = |run: &mut SimulationRun| {
        run.set_recording(Recording::ALL);
        let mut occurrences = run.init().unwrap();
        run.schedule_event(secs(3600), EventType::PauseOn, feed).unwrap();
        for half_minute in 0..120 {
            let eventtype = if half_minute % 2 == 0 { EventType::PauseOn } else { EventType::PauseOff };
            run.schedule_event(secs(3610 + 30 * half_minute), eventtype, other).unwrap();
        }
        occurrences.extend(run.advance_to(secs(2 * 3600)).unwrap());
        occurrences
    };
    let mut first = model.build(0);
    let occurrences = check(&mut first);
    let starts: Vec<(SimTime, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { at, duration, .. } if at == station => Some((o.time, duration.as_secs())),
            _ => None,
        })
        .collect();
    assert!(starts.iter().filter(|(time, _)| *time > secs(3600 + 700)).count() > 20);
    for &(time, duration) in &starts {
        if time > secs(60) && time < secs(3600) {
            assert_eq!(duration, 5, "at {time:?}");
        }
        if time > secs(3600 + 700) {
            assert_eq!(duration, 50, "at {time:?}: up to a bucket longer than 10 minutes");
        }
    }
    // Reset and a run again give the same.
    first.reset();
    assert_eq!(check(&mut first), occurrences);
}

#[test]
fn a_copy_continues_as_the_original() {
    let mut l = line_of_two();
    l.model.set_time_rules(l.station, TimeRules { lines: vec![line(vec![vec![Condition::Class { kinds: vec![l.frame], equal: true }]], Some(45))] });
    let mut original = l.model.build(3);
    original.set_recording(Recording::ALL);
    original.init().unwrap();
    original.advance_to(secs(530)).unwrap();
    let mut copy = original.clone();
    assert_eq!(original.advance_to(secs(5000)).unwrap(), copy.advance_to(secs(5000)).unwrap());
    assert_eq!(l.model.time_rules(l.station).unwrap().lines.len(), 1);
    assert!(l.model.time_rules(l.frames).is_none());
}

#[test]
fn a_model_without_time_rules_runs_as_before_and_a_rule_for_every_mu_is_its_own_time() {
    let l = line_of_two();
    let (plain_run, plain) = run(&l.model, 3000);
    let mut same = line_of_two();
    same.model.set_time_rules(same.station, TimeRules { lines: vec![line(Vec::new(), Some(20))] });
    let (ruled_run, ruled) = run(&same.model, 3000);
    assert_eq!(plain, ruled);
    assert_eq!(plain_run.state().objects, ruled_run.state().objects);
}
