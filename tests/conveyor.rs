//! A straight conveyor at constant speed between objects without geometry (spec issue 133, PR 2),
//! tested at the run's seam with times worked out by hand. The built-in MU is 0.6 m long and
//! booked at its centre, so it enters with its front 0.3 m past the start, and its booking point
//! crosses the end a conveyor's length later, however long it is: 10 m at 0.5 m/s is 20 s.

use crabsim::live_session::SessionOutput;
use crabsim::random_numbers::Distribution;
use crabsim::{
    EventType, FailureProfile, ModelDescription, MuKind, Occurrence, OccurrenceKind, Picture, ProcessState, Recording, RunState, SimTime, SimulationRun,
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn millis(ms: u64) -> SimTime {
    SimTime::from_millis(ms)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

/// The conveyor of these tests: 10 m long, 0.6 m wide, at 0.5 m/s.
fn conveyor(model: &mut ModelDescription) -> u32 {
    model.add_conveyor(10.0, 0.6, 0.5)
}

fn recorded(mut run: SimulationRun) -> SimulationRun {
    run.set_recording(Recording::ALL);
    run
}

/// Everything a run records from Init up to `until`.
fn run_until(run: &mut SimulationRun, until: SimTime) -> Vec<Occurrence> {
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(until).unwrap());
    occurrences
}

/// When MUs moved from `from` to `to`, in seconds.
fn moves(occurrences: &[Occurrence], from: u32, to: u32) -> Vec<f64> {
    occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { from: f, to: t, .. } if f == from && t == to))
        .map(|o| o.time.as_secs_f64())
        .collect()
}

fn object(state: &RunState, id: u32) -> &crabsim::ObjectState {
    &state.objects[id as usize]
}

/// Source (an MU every 10 s) -> Conveyor -> Drain.
fn conveyor_to_drain() -> (ModelDescription, u32, u32, u32) {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, out);
    (model, source, belt, out)
}

#[test]
fn an_mu_crosses_an_empty_conveyor_in_its_length_over_its_speed() {
    let (model, source, belt, out) = conveyor_to_drain();
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(55));

    assert_eq!(moves(&occurrences, source, belt), [10.0, 20.0, 30.0, 40.0, 50.0]);
    assert_eq!(moves(&occurrences, belt, out), [30.0, 40.0, 50.0]);
}

#[test]
fn a_successor_receives_from_the_front_reaching_the_end_until_the_booking_point_crosses_it() {
    let (model, _, belt, out) = conveyor_to_drain();
    let mut run = recorded(model.build(0));

    // The front reaches the end at 29.4 s, its booking point 0.3 m later.
    let occurrences = run_until(&mut run, millis(29_700));

    assert!(occurrences.iter().any(|o| o.time == millis(29_400) && o.kind == OccurrenceKind::Reserved { object_id: out, mu_id: 1 }));
    let state = run.state();
    assert_eq!(object(&state, out).process, ProcessState::Receiving);
    assert_eq!(object(&state, belt).contents, [1, 2], "it still belongs to the conveyor");
    run.advance_to(secs(31)).unwrap();
    let statistics = object(&run.state(), out).statistics;
    assert_eq!(statistics.time_receiving, millis(600));
    assert_eq!(statistics.mus_in, 1);
}

#[test]
fn a_state_says_where_each_mu_on_a_conveyor_is_and_how_it_moves() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = recorded(model.build(0));

    run_until(&mut run, secs(25));

    let state = run.state();
    let positions = &object(&state, belt).positions;
    assert_eq!(positions.len(), 2);
    assert_eq!((positions[0].mu_id, positions[0].speed), (1, 0.5));
    assert!((positions[0].front - 7.8).abs() < 1e-9, "0.3 m in at 10 s, then 15 s at 0.5 m/s: {}", positions[0].front);
    assert!((positions[1].front - 2.8).abs() < 1e-9, "{}", positions[1].front);
}

#[test]
fn where_the_booking_point_is_sets_how_long_a_station_receives() {
    for (booking_point, receiving) in [(0.0, SimTime::ZERO), (0.5, millis(600)), (1.0, millis(1_200))] {
        let mut model = ModelDescription::new();
        let kind = model.add_mu_kind(MuKind { length: 0.6, width: 0.4, booking_point });
        let source = model.add_source(secs(10));
        model.set_mu_kind(source, kind);
        let belt = conveyor(&mut model);
        let mill = station(&mut model, 1_000);
        model.connect(source, belt);
        model.connect(belt, mill);
        let mut run = recorded(model.build(0));

        let occurrences = run_until(&mut run, secs(35));

        // Entering by its booking point, an MU's booking point crosses the end 20 s later, wherever it is.
        assert_eq!(moves(&occurrences, belt, mill), [30.0], "booking point {booking_point}");
        assert_eq!(object(&run.state(), mill).statistics.time_receiving, receiving, "booking point {booking_point}");
    }
}

#[test]
fn mus_close_up_behind_one_that_waits_at_the_end_until_the_conveyor_is_full() {
    for (gap, held) in [(0.0, 17), (0.1, 14)] {
        let mut model = ModelDescription::new();
        let source = model.add_source(secs(1));
        let belt = conveyor(&mut model);
        model.set_conveyor_gap(belt, gap);
        let mill = station(&mut model, 1_000);
        model.connect(source, belt);
        model.connect(belt, mill);
        let mut run = recorded(model.build(0));

        run_until(&mut run, secs(300));

        let state = run.state();
        let on_belt = object(&state, belt);
        assert_eq!(on_belt.contents.len(), held, "gap {gap}");
        assert_eq!(on_belt.process, ProcessState::Blocked, "nothing on it moves");
        assert_eq!(object(&state, source).process, ProcessState::Blocked, "the Source waits for room");
        // Front first, standing, each closed up to the one ahead with the gap.
        let fronts: Vec<f64> = on_belt.positions.iter().map(|position| position.front).collect();
        assert!((fronts[0] - 10.0).abs() < 1e-9);
        for pair in fronts.windows(2) {
            assert!((pair[0] - 0.6 - gap - pair[1]).abs() < 1e-6, "gap {gap}: {fronts:?}");
        }
        assert!(on_belt.positions.iter().all(|position| position.speed == 0.0));
    }
}

#[test]
fn an_mu_refused_at_the_end_waits_and_is_taken_when_its_successor_has_room() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 30);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(100));

    // MU 1 works 30 to 60 s. MU 2 reaches the end at 39.4 s, waits, is taken at 60 s and handed
    // over 0.6 s later, when its booking point crosses the end. MU 3 closed up behind it, so it
    // reaches the end at 61.2 s and waits for MU 2's work to end at 90.6 s.
    assert_eq!(moves(&occurrences, belt, mill), [30.0, 60.6, 91.2]);
    assert_eq!(moves(&occurrences, mill, out), [60.0, 90.6]);
}

#[test]
fn a_transfer_once_begun_finishes_though_the_station_is_stopped() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 30);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = recorded(model.build(0));
    let mut occurrences = run.init().unwrap();
    // Stopped by hand while it receives MU 2, from 60 s to 60.6 s, and long after.
    run.schedule_event(millis(60_300), EventType::StopOn, mill).unwrap();
    run.schedule_event(secs(100), EventType::StopOff, mill).unwrap();

    occurrences.extend(run.advance_to(secs(140)).unwrap());

    assert_eq!(moves(&occurrences, belt, mill)[..2], [30.0, 60.6], "it arrives while the Station is stopped");
    // Its 30 s of work wait for the stop to end.
    assert_eq!(moves(&occurrences, mill, out), [60.0, 130.0]);
}

#[test]
fn an_mu_too_wide_for_a_conveyor_is_refused_and_waits() {
    let (mut model, source, belt, _) = conveyor_to_drain();
    let wide = model.add_mu_kind(MuKind { length: 0.6, width: 0.8, booking_point: 0.5 });
    model.set_mu_kind(source, wide);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(100));

    assert!(moves(&occurrences, source, belt).is_empty());
    assert_eq!(object(&run.state(), source).process, ProcessState::Blocked);
}

#[test]
fn a_paused_conveyor_stops_its_mus_for_as_long() {
    let (model, _, belt, out) = conveyor_to_drain();
    let mut run = recorded(model.build(0));
    let mut occurrences = run.init().unwrap();
    run.schedule_event(secs(15), EventType::PauseOn, belt).unwrap();
    run.schedule_event(secs(20), EventType::PauseOff, belt).unwrap();

    occurrences.extend(run.advance_to(secs(46)).unwrap());

    // MU 1 was on it, 5 s later. MU 2 enters as the pause ends.
    assert_eq!(moves(&occurrences, belt, out), [35.0, 40.0]);
    assert_eq!(object(&run.state(), belt).statistics.time_paused, secs(5));
}

#[test]
fn reset_empties_a_conveyor_and_the_same_seed_gives_the_same_run() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 7);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = recorded(model.build(0));
    let first = run_until(&mut run, secs(500));

    run.reset();
    let state = run.state();
    assert!(object(&state, belt).contents.is_empty() && object(&state, belt).positions.is_empty());
    assert_eq!(object(&state, mill).process, ProcessState::Idle);
    let again = run_until(&mut run, secs(500));

    assert_eq!(first, again);
    assert_eq!(first, run_until(&mut recorded(model.build(0)), secs(500)));
}

#[test]
fn the_picture_follows_the_mus_on_a_conveyor_and_who_receives() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 7);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = recorded(model.build(0));
    let mut picture = Picture::new();
    let init = run.init().unwrap();
    picture.observe(&SessionOutput::State(run.state()));
    let _ = init;

    // Every 0.1 s for a while, the picture agrees with the run.
    for step in 1..=1_200u64 {
        let now = millis(step * 100);
        let occurrences = run.advance_to(now).unwrap();
        picture.observe(&SessionOutput::Occurrences(occurrences));
        picture.observe(&SessionOutput::ShownTime(now));
        let state = run.state();
        let pictured = picture.object(belt).unwrap();
        assert_eq!(pictured.contents(), object(&state, belt).contents.as_slice(), "at {now:?}");
        assert_eq!(pictured.process(), object(&state, belt).process, "at {now:?}");
        for (motion, position) in pictured.motion().iter().zip(&object(&state, belt).positions) {
            assert_eq!(motion.mu_id, position.mu_id);
            assert!((motion.front_at(now) - position.front).abs() < 1e-3, "MU {} at {now:?}: {} against {}", motion.mu_id, motion.front_at(now), position.front);
        }
        assert_eq!(picture.object(mill).unwrap().process(), object(&state, mill).process, "at {now:?}");
    }
}

#[test]
fn a_reserved_place_is_taken_so_another_predecessor_is_refused_until_the_mu_arrives() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    // Its first MU is ready at 29.7 s, while the Buffer receives from the conveyor.
    let late = model.add_source(millis(29_700));
    let store = model.add_buffer(crabsim::Capacity::Limited(1));
    model.set_dwelling_time(store, secs(50));
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, store);
    model.connect(late, store);
    model.connect(store, out);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(81));

    // Reserved at 29.4 s, the conveyor's MU arrives at 30 s and dwells until 80 s. The late
    // Source has waited since 29.7 s, longer than the conveyor's next MU, which reached the end
    // at 39.4 s, so it goes in first.
    assert_eq!(moves(&occurrences, belt, store), [30.0]);
    assert_eq!(moves(&occurrences, late, store), [80.0]);
}

#[test]
fn predecessors_waiting_for_room_on_a_conveyor_go_in_the_order_their_mus_were_ready() {
    let mut model = ModelDescription::new();
    let first = model.add_source(secs(1));
    let second = model.add_source(secs(1));
    let belt = conveyor(&mut model);
    let out = model.add_drain();
    model.connect(first, belt);
    model.connect(second, belt);
    model.connect(belt, out);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(60));

    // Room comes every 1.2 s, for one MU at a time, and the two take turns.
    let entries: Vec<u32> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { from, to, .. } if to == belt => Some(from),
            _ => None,
        })
        .collect();
    assert!(entries.len() > 20);
    for pair in entries.windows(2) {
        assert_ne!(pair[0], pair[1], "{entries:?}");
    }
    let times: Vec<f64> = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { to, .. } if to == belt)).map(|o| o.time.as_secs_f64()).collect();
    for pair in times.windows(2) {
        assert!((pair[1] - pair[0] - 1.2).abs() < 1e-9, "{times:?}");
    }
}
