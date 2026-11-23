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

// ---------- Conveyor to conveyor, non-accumulating and interruptions (spec issue 133, PR 3) ----------

/// Source (an MU every `interval`) -> a conveyor of 10 m at `first` m/s -> one at `second` m/s -> Drain.
fn two_conveyors(interval: SimTime, first: f64, second: f64) -> (ModelDescription, u32, u32, u32) {
    let mut model = ModelDescription::new();
    let source = model.add_source(interval);
    let a = model.add_conveyor(10.0, 0.6, first);
    let b = model.add_conveyor(10.0, 0.6, second);
    let out = model.add_drain();
    model.connect(source, a);
    model.connect(a, b);
    model.connect(b, out);
    (model, a, b, out)
}

/// What a millimetre of rounding to a millisecond can do.
const TOLERANCE: f64 = 1e-3;

/// Nothing overlaps another MU on a conveyor, nor, across the two, the last MU of the second, whose
/// rear may be on the first. Every MU is 0.6 m long.
fn assert_nothing_overlaps(state: &RunState, a: u32, b: u32, gap: f64, at: SimTime) {
    for id in [a, b] {
        for pair in object(state, id).positions.windows(2) {
            assert!(pair[0].front - 0.6 - pair[1].front >= gap - TOLERANCE, "on {id} at {at:?}: {pair:?}");
        }
    }
    if let (Some(first_on_a), Some(last_on_b)) = (object(state, a).positions.first(), object(state, b).positions.last()) {
        // In the first conveyor's metres, the second one's start is at 10.
        assert!(first_on_a.front <= 10.0 + (last_on_b.front - 0.6) - gap + TOLERANCE, "between the conveyors at {at:?}: {first_on_a:?} and {last_on_b:?}");
    }
}

#[test]
fn an_mu_passes_between_conveyors_of_one_speed_without_stopping() {
    let (model, a, b, out) = two_conveyors(secs(10), 0.5, 0.5);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(75));

    // 20 s on each, by its booking point.
    assert_eq!(moves(&occurrences, a, b), [30.0, 40.0, 50.0, 60.0, 70.0]);
    assert_eq!(moves(&occurrences, b, out), [50.0, 60.0, 70.0]);
}

#[test]
fn an_mu_on_two_conveyors_moves_at_the_slower_speed() {
    // Entering at 100 s. On the first, at 1 m/s, its front reaches the end 9.7 s later, and its
    // booking point crosses 0.3 m after that, at 0.5 m/s since it is on the second too, 0.6 s later.
    // Then 10 m to its booking point crossing the end of the second, 20 s.
    let (model, a, b, out) = two_conveyors(secs(100), 1.0, 0.5);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(145));

    assert_eq!(moves(&occurrences, a, b), [110.3]);
    assert_eq!(moves(&occurrences, b, out), [130.3]);
}

#[test]
fn an_mu_behind_one_on_two_conveyors_closes_up_to_its_rear_and_follows_it() {
    let (model, a, b, _) = two_conveyors(secs(1), 1.0, 0.5);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(25));

    // MU 1, entering at 1 s, is handed over at 11.3 s. MU 2, 1 m behind, closes up to its rear 0.2 s
    // later and follows it at 0.5 m/s, so its front reaches the end at 11.9 s, as MU 1's rear
    // clears it, and its booking point crosses 0.6 s after that.
    assert_eq!(moves(&occurrences, a, b)[..2], [11.3, 12.5]);
}

#[test]
fn mus_never_overlap_across_conveyors_of_different_speeds_with_a_gap_and_a_slow_end() {
    for (first, second, gap) in [(1.0, 0.5, 0.0), (0.5, 1.0, 0.1), (0.8, 0.3, 0.2), (0.4, 0.4, 0.05)] {
        let mut model = ModelDescription::new();
        let source = model.add_source(millis(700));
        let a = model.add_conveyor(10.0, 0.6, first);
        let b = model.add_conveyor(10.0, 0.6, second);
        model.set_conveyor_gap(a, gap);
        model.set_conveyor_gap(b, gap);
        let mill = station(&mut model, 6);
        let out = model.add_drain();
        model.connect(source, a);
        model.connect(a, b);
        model.connect(b, mill);
        model.connect(mill, out);
        let mut run = recorded(model.build(0));
        run.init().unwrap();
        let mut at = SimTime::ZERO;
        while at < secs(400) {
            at += millis(100);
            run.advance_to(at).unwrap();
            assert_nothing_overlaps(&run.state(), a, b, gap, at);
        }
        assert!(object(&run.state(), mill).statistics.mus_in > 30, "speeds {first} and {second}");
    }
}

#[test]
fn a_full_conveyor_holds_the_mu_that_has_reserved_a_place_on_it_until_it_fits() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let a = conveyor(&mut model);
    let b = model.add_conveyor(2.0, 0.6, 0.5);
    let mill = station(&mut model, 1_000);
    model.connect(source, a);
    model.connect(a, b);
    model.connect(b, mill);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(200));

    // The first MU goes through to the Station, the second stops at the end of the short conveyor,
    // and the others close up behind it: three on the second, a fourth with its front on it but
    // belonging to the first, which waits there.
    assert_eq!(moves(&occurrences, b, mill), [29.0]);
    let state = run.state();
    let on_b: Vec<f64> = object(&state, b).positions.iter().map(|position| (position.front * 1e3).round() / 1e3).collect();
    assert_eq!(on_b, [2.0, 1.4, 0.8]);
    let first_on_a = &object(&state, a).positions[0];
    assert!((first_on_a.front - 10.2).abs() < 1e-6, "{first_on_a:?}");
    for position in object(&state, a).positions.iter().chain(object(&state, b).positions.iter()) {
        assert_eq!(position.speed, 0.0, "{position:?}");
    }
    assert_nothing_overlaps(&state, a, b, 0.0, secs(200));
}

#[test]
fn a_non_accumulating_conveyor_stops_as_a_whole_when_its_first_mu_cannot_leave() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let belt = conveyor(&mut model);
    model.set_conveyor_accumulating(belt, false);
    let mill = station(&mut model, 1_000);
    model.connect(source, belt);
    model.connect(belt, mill);
    let mut run = recorded(model.build(0));

    run_until(&mut run, secs(40));

    // MU 1 went into the Station at 25 s. MU 2 reached the end at 29.4 s and was refused, and the
    // others stopped where they were, as MU 6 did when it entered at 30 s.
    let state = run.state();
    let positions = &object(&state, belt).positions;
    let fronts: Vec<f64> = positions.iter().map(|position| (position.front * 1000.0).round() / 1000.0).collect();
    assert_eq!(object(&state, belt).contents, [2, 3, 4, 5, 6]);
    assert_eq!(fronts, [10.0, 7.5, 5.0, 2.5, 0.3]);
    assert!(positions.iter().all(|position| position.speed == 0.0));
    assert_eq!(object(&state, belt).process, ProcessState::Blocked);
}

#[test]
fn an_accumulating_conveyor_lets_the_mus_behind_close_up_where_a_non_accumulating_one_stops() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 1_000);
    model.connect(source, belt);
    model.connect(belt, mill);
    let mut run = recorded(model.build(0));

    run_until(&mut run, secs(40));

    let state = run.state();
    let fronts: Vec<f64> = object(&state, belt).positions.iter().map(|position| (position.front * 1000.0).round() / 1000.0).collect();
    assert_eq!(fronts[..4], [10.0, 9.4, 8.8, 7.8], "MU 5 entered at 25 s and hasn't closed up yet");
}

#[test]
fn a_non_accumulating_conveyor_starts_all_its_mus_together_when_the_first_can_leave() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let belt = conveyor(&mut model);
    model.set_conveyor_accumulating(belt, false);
    let mill = station(&mut model, 40);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = recorded(model.build(0));

    let occurrences = run_until(&mut run, secs(66));

    // The Station takes MU 1 at 25 s and is free at 65 s, when MU 2, waiting since 29.4 s, goes in,
    // its booking point crossing 0.6 s later, and the whole belt moves on together, as far apart
    // as it was.
    assert_eq!(moves(&occurrences, belt, mill)[..2], [25.0, 65.6]);
    let state = run.state();
    let positions = &object(&state, belt).positions;
    assert!(positions.iter().all(|position| position.speed == 0.5), "{positions:?}");
    let fronts: Vec<f64> = positions.iter().take(3).map(|position| position.front).collect();
    assert!(fronts.iter().zip([8.0, 5.5, 3.0]).all(|(front, expected)| (front - expected).abs() < 1e-6), "{fronts:?}");
}

#[test]
fn a_straddling_mu_carries_on_at_its_own_conveyors_speed_onto_a_stopped_one() {
    // The second conveyor is paused from 29.6 s to 40 s. MU 1, entering at 10 s, reached the end of
    // the first at 29.4 s and belongs to it until its booking point crosses at 30 s, going on at
    // 0.5 m/s though the second is stopped. Then it belongs to the stopped one, and stands there.
    let (model, a, b, out) = two_conveyors(secs(10), 0.5, 0.5);
    let mut run = recorded(model.build(0));
    let mut occurrences = run.init().unwrap();
    run.schedule_event(millis(29_600), EventType::PauseOn, b).unwrap();
    run.schedule_event(secs(40), EventType::PauseOff, b).unwrap();

    occurrences.extend(run.advance_to(secs(35)).unwrap());

    assert_eq!(moves(&occurrences, a, b), [30.0]);
    let state = run.state();
    let on_b = &object(&state, b).positions;
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0].speed, 0.0);
    assert!((on_b[0].front - 0.3).abs() < 1e-6, "{on_b:?}");

    // Paused 10 s of its 20 s on the second.
    occurrences.extend(run.advance_to(secs(70)).unwrap());
    assert_eq!(moves(&occurrences, b, out)[0], 60.0);
}

#[test]
fn a_straddling_mu_that_belongs_to_the_second_conveyor_carries_on_over_a_stopped_first_one() {
    // The first conveyor is paused from 30.4 s to 40 s. MU 1's booking point crossed at 30 s, with
    // 0.3 m of its rear still on the first: it goes on at the second's speed over the stopped one.
    let (model, a, b, _) = two_conveyors(secs(10), 0.5, 0.5);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.schedule_event(millis(30_400), EventType::PauseOn, a).unwrap();
    run.schedule_event(secs(40), EventType::PauseOff, a).unwrap();

    run.advance_to(secs(35)).unwrap();

    let state = run.state();
    let on_b = &object(&state, b).positions[0];
    assert_eq!(on_b.speed, 0.5);
    assert!((on_b.front - (0.3 + 0.5 * 5.0)).abs() < 1e-6, "{on_b:?}");
    // MU 2 entered the first conveyor at 20 s and stood where it was when it stopped.
    let on_a = &object(&state, a).positions[0];
    assert_eq!(on_a.speed, 0.0);
    assert!((on_a.front - 5.5).abs() < 1e-6, "{on_a:?}");
}

#[test]
fn a_straddling_mu_belonging_to_a_stopped_conveyor_stands_and_so_do_those_behind() {
    // The first conveyor stops at 20.7 s, while MU 1, entering at 1 s, has its front 0.15 m over the
    // second and its booking point not yet across: it stands, and the MU behind it stops.
    let (model, a, b, _) = two_conveyors(secs(1), 0.5, 0.5);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.schedule_event(millis(20_700), EventType::PauseOn, a).unwrap();
    run.schedule_event(secs(40), EventType::PauseOff, a).unwrap();

    run.advance_to(secs(30)).unwrap();

    let state = run.state();
    for position in object(&state, a).positions.iter() {
        assert_eq!(position.speed, 0.0, "{position:?}");
    }
    assert!((object(&state, a).positions[0].front - 10.15).abs() < 1e-6, "MU 1 stands with its front on the second conveyor");
    assert_nothing_overlaps(&state, a, b, 0.0, secs(30));
}

#[test]
fn mus_never_overlap_while_either_of_two_conveyors_is_stopped() {
    for stopped in [0, 1] {
        let (model, a, b, _) = two_conveyors(millis(1_300), 0.5, 0.5);
        let mut run = recorded(model.build(0));
        run.init().unwrap();
        let belt = [a, b][stopped];
        run.schedule_event(secs(30), EventType::PauseOn, belt).unwrap();
        run.schedule_event(secs(55), EventType::PauseOff, belt).unwrap();
        run.schedule_event(secs(70), EventType::PauseOn, belt).unwrap();
        run.schedule_event(secs(75), EventType::PauseOff, belt).unwrap();
        let mut at = SimTime::ZERO;
        while at < secs(200) {
            at += millis(100);
            run.advance_to(at).unwrap();
            assert_nothing_overlaps(&run.state(), a, b, 0.0, at);
        }
        assert!(run.state().objects[3].statistics.mus_in > 50);
    }
}

#[test]
fn reset_forgets_what_two_conveyors_were_doing_and_replays_the_same() {
    let (model, a, b, _) = two_conveyors(secs(1), 1.0, 0.5);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.schedule_event(secs(30), EventType::PauseOn, b).unwrap();
    run.schedule_event(secs(40), EventType::PauseOff, b).unwrap();
    run.advance_to(secs(35)).unwrap();

    run.reset();
    let state = run.state();
    assert!(object(&state, a).positions.is_empty() && object(&state, b).positions.is_empty());
    let mut again = run.init().unwrap();
    again.extend(run.advance_to(secs(100)).unwrap());

    assert_eq!(again, run_until(&mut recorded(model.build(0)), secs(100)));
}

// ---------- Conveyor statistics (spec issue 133, PR 4) ----------

fn belt_figures(run: &SimulationRun, belt: u32) -> crabsim::BeltStatistics {
    **object(&run.state(), belt).belt.as_ref().expect("a conveyor reports its belt statistics")
}

#[test]
fn a_conveyors_busy_time_is_full_while_its_start_is_taken_and_not_full_after() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = model.build(0);

    run.init().unwrap();
    run.advance_to(secs(55)).unwrap();

    let statistics = object(&run.state(), belt).statistics;
    // Empty for the first 10 s, then an MU on it throughout. Each of the five MUs that entered
    // keeps its rear behind the start for 0.3 m at 0.5 m/s, 0.6 s, which is when no MU could enter.
    assert_eq!(statistics.time_idle, secs(10));
    assert_eq!(statistics.time_busy, secs(45));
    assert_eq!(statistics.time_busy_full(), millis(3_000));
    assert_eq!(statistics.time_busy_not_full, millis(42_000));
    assert_eq!(statistics.time_busy_partly_blocked, SimTime::ZERO);
    assert_eq!(statistics.time_blocked, SimTime::ZERO);
    assert_eq!(statistics.measured_time(), secs(55));
}

#[test]
fn a_conveyors_start_coming_free_between_events_ends_busy_and_full_where_it_happened() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = model.build(0);
    run.init().unwrap();
    // Busy since 10 s, and full for 0.6 s after each MU enters. The MU entering at 50 s has its
    // rear clear of the start at 50.6 s, and the conveyor has no event of its own until 60 s.
    run.advance_to(millis(50_300)).unwrap();
    let statistics = object(&run.state(), belt).statistics;
    assert_eq!((statistics.time_busy_full(), statistics.time_busy_not_full), (millis(2_700), millis(37_600)));
    run.advance_to(secs(52)).unwrap();
    let statistics = object(&run.state(), belt).statistics;
    assert_eq!((statistics.time_busy_full(), statistics.time_busy_not_full), (millis(3_000), millis(39_000)));

    // Cleared after the start came free, it measures busy with room from then on.
    run.clear_statistics();
    run.advance_to(secs(55)).unwrap();
    let statistics = object(&run.state(), belt).statistics;
    assert_eq!((statistics.time_busy_full(), statistics.time_busy_not_full), (SimTime::ZERO, secs(3)));
    assert_eq!(statistics.measured_time(), secs(3));
    run.advance_to(secs(65)).unwrap();
    let statistics = object(&run.state(), belt).statistics;
    // The MU entering at 60 s keeps its start taken until 60.6 s.
    assert_eq!((statistics.time_busy_full(), statistics.time_busy_not_full), (millis(600), millis(12_400)));
}

#[test]
fn a_conveyor_that_stopped_with_its_start_taken_is_full_until_its_last_mu_moves_clear_after_it_starts() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let belt = conveyor(&mut model);
    model.set_conveyor_accumulating(belt, false);
    let mill = station(&mut model, 40);
    let out = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, mill);
    model.connect(mill, out);
    let mut run = model.build(0);
    run.init().unwrap();

    // MUs enter every 5 s from 5 s, each keeping the start taken for 0.6 s while the belt moves.
    // It stops as a whole at 29.4 s, and MU 6 enters at 30 s standing, its rear 0.3 m short of the
    // start. The belt moves again at 65 s, when the Station takes MU 2, so MU 6's rear clears the
    // start at 65.6 s, not 0.6 s after it entered.
    run.advance_to(millis(65_300)).unwrap();
    assert_eq!(object(&run.state(), belt).statistics.time_busy_full(), millis(5 * 600 + 300));
    run.advance_to(secs(66)).unwrap();
    let statistics = object(&run.state(), belt).statistics;
    assert_eq!(statistics.time_busy_full(), millis(6 * 600));
    assert_eq!(statistics.time_blocked, millis(35_600), "standing from 29.4 s to 65 s");
    let _ = source;
}

#[test]
fn a_conveyor_is_partly_blocked_while_its_first_mu_waits_and_others_still_move_then_blocked() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 1_000);
    model.connect(source, belt);
    model.connect(belt, mill);
    let mut run = model.build(0);
    run.init().unwrap();

    // The mill takes the first MU at once and works on it for 1,000 s. The second reaches the end at
    // 39.4 s and waits, while the third, entering at 30 s, is still moving.
    run.advance_to(millis(39_400)).unwrap();
    assert_eq!(object(&run.state(), belt).statistics.time_busy_partly_blocked, SimTime::ZERO);
    run.advance_to(secs(45)).unwrap();
    assert_eq!(object(&run.state(), belt).statistics.time_busy_partly_blocked, millis(5_600));
    run.advance_to(secs(900)).unwrap();

    let statistics = object(&run.state(), belt).statistics;
    assert!(statistics.time_blocked > secs(500), "every MU closed up and stopped: {statistics:?}");
    assert_eq!(statistics.time_busy, statistics.time_busy_full() + statistics.time_busy_not_full + statistics.time_busy_partly_blocked);
    assert_eq!(statistics.measured_time(), secs(900));
    let _ = source;
}

#[test]
fn a_conveyor_reports_how_full_it_was_in_mus_and_in_length() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(55)).unwrap();

    let state = run.state();
    let fill = object(&state, belt).fill.as_ref().expect("a conveyor has a fill histogram as a Buffer does");
    // None for 10 s, one MU for 10 s, then two.
    assert_eq!(fill.histogram.bars[..3], [secs(10), secs(10), secs(35)]);
    assert_eq!(fill.lowest, 0);
    assert_eq!(object(&state, belt).statistics.max_contents, 3, "for an instant at 30 s: the MU leaving is still on it when the next enters");
    let figures = belt_figures(&run, belt);
    // 0.6 m for 10 s and 1.2 m for 35 s, of 10 m over the 55 s.
    assert!((figures.length_time - 0.6 * 10_000.0 - 1.2 * 35_000.0).abs() < 1e-6, "{figures:?}");
    assert!((figures.mean_share_taken(secs(55)).unwrap() - 48.0 / 550.0).abs() < 1e-9);
    assert!((figures.max_length_taken - 1.2).abs() < 1e-9);
    assert!((figures.max_share_taken() - 0.12).abs() < 1e-9);
}

#[test]
fn a_conveyor_reports_how_long_mus_took_to_cross_it() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(55)).unwrap();

    let figures = belt_figures(&run, belt);
    // Booking point to booking point, 10 m at 0.5 m/s, for the three that left.
    assert_eq!(figures.crossings, 3);
    assert_eq!(figures.crossing_mean(), Some(secs(20)));
    assert_eq!(figures.crossing_max, secs(20));
}

#[test]
fn the_longest_crossing_includes_an_mu_that_has_been_on_the_conveyor_longer_than_any_that_left() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = conveyor(&mut model);
    let mill = station(&mut model, 1_000);
    model.connect(source, belt);
    model.connect(belt, mill);
    let mut run = model.build(0);
    run.init().unwrap();

    run.advance_to(secs(500)).unwrap();

    let figures = belt_figures(&run, belt);
    assert_eq!(figures.crossings, 1, "only the first left");
    assert_eq!(figures.crossing_max, secs(480), "the second has been on since 20 s");
    let _ = source;
}

#[test]
fn clearing_statistics_and_reset_start_a_conveyors_figures_afresh() {
    let (model, _, belt, _) = conveyor_to_drain();
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(35)).unwrap();

    run.clear_statistics();
    run.advance_to(secs(55)).unwrap();
    let figures = belt_figures(&run, belt);
    assert_eq!(figures.crossings, 2, "those leaving at 40 s and 50 s");
    assert!((figures.length_time - 1.2 * 20_000.0).abs() < 1e-6, "two MUs throughout: {figures:?}");
    let fill = object(&run.state(), belt).fill.clone().unwrap();
    assert_eq!(fill.histogram.total(), secs(20));

    run.reset();
    run.init().unwrap();
    let figures = belt_figures(&run, belt);
    assert_eq!((figures.crossings, figures.length_time, figures.max_length_taken), (0, 0.0, 0.0));
}
