//! The cross conveyor (spec issue 148, PR 2), tested at the run's seam with times worked out by
//! hand. Unless a test says otherwise it is 0.6 by 0.6 m at 0.5 m/s with a switch time of 2 s, and
//! its MUs are the built-in MU, 0.6 m long, 0.4 m wide and booked at its centre.
//!
//! Going straight through from an object without geometry, an MU enters with its booking point at
//! the entry side, so its front is 0.3 m in, reaches the far side 0.6 s later and is handed over
//! when its booking point crosses it, 0.6 s after that: 1.2 s in all. Turning, it reaches the
//! centre, its front 0.6 m in, 0.6 s after entering, stands for the switch time, then leaves
//! sideways lying crosswise, its 0.4 m width along its way: from its front 0.5 m in to the side is
//! 0.2 s, and its booking point, 0.2 m behind its front, crosses 0.4 s after that. 3.2 s in all.

use crabsim::distribution::{DistributionRule, Order, Rank, WeighBy, WeightedRule};
use crabsim::random_numbers::Distribution;
use crabsim::routing::PlanStep;
use crabsim::{EventType, FailureProfile, ModelDescription, MuKind, Occurrence, OccurrenceKind, ProcessState, Recording, RunState, Side, SimTime, SimulationRun};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn millis(ms: u64) -> SimTime {
    SimTime::from_millis(ms)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

fn run_until(model: &ModelDescription, until: SimTime) -> (SimulationRun, Vec<Occurrence>) {
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(until).unwrap());
    (run, occurrences)
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

/// A Source every `interval` into the back of a cross conveyor, a Drain "Ahead" at its front and a
/// Drain "Spur" at its left, which lists the Spur. MUs go to the Spur if `to_spur`, otherwise they
/// have no plan.
struct Cross {
    model: ModelDescription,
    cross: u32,
    ahead: u32,
    spur: u32,
}

fn cross(interval: SimTime, to_spur: bool) -> Cross {
    let mut model = ModelDescription::new();
    let source = model.add_source(interval);
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let ahead = model.add_drain();
    let spur = model.add_drain();
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Front), ahead, None);
    model.connect_at(cross, Some(Side::Left), spur, None);
    let to = model.add_destination(&[spur]);
    model.set_exit_destinations(cross, Side::Left, &[to]);
    if to_spur {
        let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: to, process_time: None }]);
        model.set_mu_kind(source, kind);
    }
    Cross { model, cross, ahead, spur }
}

#[test]
fn an_mu_whose_destination_no_side_lists_goes_straight_through_without_stopping() {
    let Cross { model, cross, ahead, spur } = cross(secs(10), false);

    let (_, occurrences) = run_until(&model, secs(35));

    assert_eq!(moves(&occurrences, cross, ahead), [11.2, 21.2, 31.2]);
    assert!(moves(&occurrences, cross, spur).is_empty());
    let standing = occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::MotionPlanned { at, speed, .. } if at == cross && speed == 0.0));
    assert!(!standing, "it never stops");
}

#[test]
fn an_mu_going_where_a_side_lists_turns_there_after_the_switch_time() {
    let Cross { model, cross, ahead, spur } = cross(secs(10), true);

    let (run, occurrences) = run_until(&model, secs(25));

    assert_eq!(moves(&occurrences, cross, spur), [13.2, 23.2]);
    assert!(moves(&occurrences, cross, ahead).is_empty());
    // At its centre from 10.6 s, standing, then leaving sideways from 12.6 s, crosswise, its way
    // starting from the right side, opposite the left one it leaves by.
    let motions: Vec<_> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MotionPlanned { mu_id: 1, at, front, speed, crosswise, from_side, .. } if at == cross => Some((o.time, front, speed, crosswise, from_side)),
            _ => None,
        })
        .collect();
    assert_eq!(
        motions,
        [
            (secs(10), 0.3, 0.5, false, Some(Side::Back)),
            (millis(10_600), 0.6, 0.0, false, Some(Side::Back)),
            (millis(12_600), 0.5, 0.5, true, Some(Side::Right)),
        ]
    );
    let figures = object(&run.state(), cross).cross.as_deref().copied().unwrap();
    assert_eq!(figures.time_switching, secs(4), "two MUs, 2 s each");
    assert_eq!(figures.mus_out[Side::Left.index()], 2);
}

#[test]
fn a_cross_conveyor_holds_one_mu_at_a_time() {
    // A Source with an MU every 0.5 s: each MU takes 1.2 s to cross, and the next enters as it
    // is handed over to the Drain, which takes it whole.
    let Cross { model, cross, ahead, .. } = cross(millis(500), false);

    let (_, occurrences) = run_until(&model, millis(5_000));

    assert_eq!(moves(&occurrences, cross, ahead), [1.7, 2.9, 4.1]);
}

#[test]
fn the_release_time_follows_a_turned_mu_before_the_next_may_enter() {
    for (release, expected) in [(0, [4.2, 7.4, 10.6]), (1, [4.2, 8.4, 12.6])] {
        let Cross { mut model, cross, spur, .. } = cross(secs(1), true);
        model.set_release_time(cross, secs(release));

        let (_, occurrences) = run_until(&model, secs(13));

        assert_eq!(moves(&occurrences, cross, spur), expected, "release time {release} s");
    }
}

#[test]
fn releasing_counts_as_switching_and_both_are_busy() {
    let Cross { mut model, cross, .. } = cross(secs(100), true);
    model.set_release_time(cross, secs(1));

    let (run, _) = run_until(&model, secs(110));

    let state = run.state();
    let statistics = object(&state, cross).statistics;
    // From 100 s: 0.6 s moving to the centre, 2 s switching, 0.6 s leaving, 1 s releasing.
    assert_eq!(object(&state, cross).cross.as_deref().unwrap().time_switching, secs(3));
    assert_eq!(statistics.time_busy, millis(4_200));
    assert_eq!(statistics.time_busy_not_full, SimTime::ZERO, "holding one MU at a time, it is full while busy");
    assert_eq!(object(&state, cross).process, ProcessState::Idle);
}

#[test]
fn a_destination_the_side_straight_across_lists_too_is_shared_by_its_strategy() {
    let Cross { mut model, cross, ahead, spur } = cross(secs(10), true);
    let to = model.add_destination(&[spur]);
    model.set_exit_destinations(cross, Side::Front, &[to]);

    let (_, occurrences) = run_until(&model, secs(35));

    // The front is its first exit, so round robin sends MU 1 straight on, without stopping, MU 2
    // left, turning, and MU 3 straight on again.
    assert_eq!(moves(&occurrences, cross, ahead), [11.2, 31.2]);
    assert_eq!(moves(&occurrences, cross, spur), [23.2]);
}

/// A cross conveyor with Stations at its left and right, both listed for the MU's destination,
/// a class of the two.
fn two_sides(process_secs: (u64, u64)) -> (ModelDescription, u32, u32, u32) {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let left = station(&mut model, process_secs.0);
    let right = station(&mut model, process_secs.1);
    let drain = model.add_drain();
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Left), left, None);
    model.connect_at(cross, Some(Side::Right), right, None);
    model.connect(left, drain);
    model.connect(right, drain);
    let mills = model.add_destination(&[left, right]);
    model.set_exit_destinations(cross, Side::Left, &[mills]);
    model.set_exit_destinations(cross, Side::Right, &[mills]);
    let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: mills, process_time: None }]);
    model.set_mu_kind(source, kind);
    (model, cross, left, right)
}

#[test]
fn among_several_sides_its_strategy_takes_turns() {
    let (model, cross, left, right) = two_sides((1, 1));

    let (_, occurrences) = run_until(&model, secs(45));

    assert_eq!(moves(&occurrences, cross, left), [13.2, 33.2]);
    assert_eq!(moves(&occurrences, cross, right), [23.2, 43.2]);
}

#[test]
fn as_it_is_let_in_it_picks_a_side_whose_successor_has_room() {
    // The left Station works 25 s on each MU, so it is busy when the second MU enters at 20 s,
    // whose turn it would be on the right anyway, and when the third does at 30 s, whose turn
    // would be the left's: it goes right too.
    let (model, cross, left, right) = two_sides((25, 1));

    let (_, occurrences) = run_until(&model, secs(35));

    assert_eq!(moves(&occurrences, cross, left), [13.2]);
    assert_eq!(moves(&occurrences, cross, right), [23.2, 33.2]);
}

#[test]
fn the_side_is_chosen_as_the_mu_is_let_in_not_at_the_centre() {
    // The left Station starts with an MU and works 10.3 s on it, so it is busy when MU 1 enters at
    // 10 s but free by the time MU 1 reaches the centre at 10.6 s. Its turn would be the left's,
    // but the left can't take it as it comes in, so it goes right.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let left = model.add_station_holding(millis(10_300), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), 1_000);
    let right = station(&mut model, 1);
    let drain = model.add_drain();
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Left), left, None);
    model.connect_at(cross, Some(Side::Right), right, None);
    model.connect(left, drain);
    model.connect(right, drain);
    let both = model.add_destination(&[left, right]);
    model.set_exit_destinations(cross, Side::Left, &[both]);
    model.set_exit_destinations(cross, Side::Right, &[both]);
    let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: both, process_time: None }]);
    model.set_mu_kind(source, kind);

    let (_, occurrences) = run_until(&model, secs(15));

    assert_eq!(moves(&occurrences, cross, right), [13.2]);
    assert!(moves(&occurrences, cross, left).is_empty());
}

#[test]
fn a_conveyors_mu_has_its_side_chosen_when_room_is_reserved_for_it() {
    // A feed conveyor's MU touches the cross conveyor at 119.4 s, which reserves room for it and
    // chooses its side then: the left Station, busy until 119.5 s, can't take it, so it goes right,
    // though the left is free by the time its booking point is over the cross conveyor at 120 s.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let feed = model.add_conveyor(10.0, 0.6, 0.5);
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let left = model.add_station_holding(millis(119_500), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), 1_000);
    let right = station(&mut model, 1);
    let drain = model.add_drain();
    model.connect(source, feed);
    model.connect_at(feed, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Left), left, None);
    model.connect_at(cross, Some(Side::Right), right, None);
    model.connect(left, drain);
    model.connect(right, drain);
    let both = model.add_destination(&[left, right]);
    model.set_exit_destinations(cross, Side::Left, &[both]);
    model.set_exit_destinations(cross, Side::Right, &[both]);
    let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: both, process_time: None }]);
    model.set_mu_kind(source, kind);

    let (_, occurrences) = run_until(&model, secs(130));

    assert_eq!(moves(&occurrences, cross, right), [123.2]);
    assert!(moves(&occurrences, cross, left).is_empty());
}

#[test]
fn with_every_listed_side_taken_it_waits_at_the_first_in_turn() {
    let (mut model, cross, left, right) = two_sides((25, 25));
    model.set_distribution(cross, DistributionRule::Weighted(WeightedRule { by: WeighBy::ProcessTime, order: Order::Smallest, rank: Rank::Successor, drains: None }));

    let (_, occurrences) = run_until(&model, secs(60));

    // Both work 25 s, so they rank equal and take turns: MU 1 left at 13.2, MU 2 right at 23.2.
    // MU 3 enters at 30 s, with the left due to free at 38.2 and the right at 48.2: its turn is
    // the left's, which it waits for at the side from 32.8 s. MU 4 enters as MU 3 leaves, at 40 s,
    // with both busy again: its turn is the right's, free at 48.2 s.
    assert_eq!(moves(&occurrences, cross, left), [13.2, 38.6]);
    assert_eq!(moves(&occurrences, cross, right), [23.2, 48.6]);
}

#[test]
fn a_switch_time_and_a_release_time_freeze_while_it_is_interrupted() {
    let Cross { mut model, cross, spur, .. } = cross(secs(100), true);
    model.set_release_time(cross, secs(2));
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    // Switching from 100.6 to 102.6 s, paused from 101 to 105 s with 1.6 s of it left.
    run.schedule_event(secs(101), EventType::PauseOn, cross).unwrap();
    run.schedule_event(secs(105), EventType::PauseOff, cross).unwrap();
    // Handed over at 107.2 s, releasing until 109.2 s, paused from 108 to 110 s with 1.2 s left.
    run.schedule_event(secs(108), EventType::PauseOn, cross).unwrap();
    run.schedule_event(secs(110), EventType::PauseOff, cross).unwrap();
    occurrences.extend(run.advance_to(secs(215)).unwrap());

    assert_eq!(moves(&occurrences, cross, spur), [107.2, 203.2], "the second MU enters at 200 s");
    let state = run.state();
    assert_eq!(object(&state, cross).cross.as_deref().unwrap().time_switching, millis(8_000), "2 + 2 switching and 2 + 2 releasing");
    assert_eq!(object(&state, cross).statistics.time_paused, secs(6));
}

#[test]
fn an_mu_too_long_for_its_way_across_is_refused_and_waits() {
    let mut model = ModelDescription::new();
    let long = model.add_mu_kind(MuKind { length: 0.8, width: 0.4, booking_point: 0.5 });
    let source = model.add_source(secs(10));
    model.set_mu_kind(source, long);
    let cross = model.add_cross_conveyor(0.6, 0.9, 0.5);
    let drain = model.add_drain();
    // Coming in at the right, its way runs across the 0.9 m width, which it fits.
    let other = model.add_source(secs(10));
    model.set_mu_kind(other, long);
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(other, None, cross, Some(Side::Right));
    model.connect_at(cross, Some(Side::Left), drain, None);

    let (run, occurrences) = run_until(&model, secs(15));

    assert!(moves(&occurrences, source, cross).is_empty(), "0.8 m long, along a way of 0.6 m");
    assert_eq!(moves(&occurrences, other, cross), [10.0]);
    assert_eq!(object(&run.state(), cross).blocked_predecessors, [source]);
}

/// Source -> a 10 m conveyor -> the cross conveyor's back -> its front or left -> a 10 m
/// conveyor -> a Drain, MUs going to the Drain at the end of the left one if `turn`.
fn belts(interval: SimTime, turn: bool) -> (ModelDescription, u32, u32, u32, u32) {
    belts_to(interval, turn, None)
}

/// The same, with a Station working `slow` seconds on each MU between the outfeed and the Drain.
fn belts_to(interval: SimTime, turn: bool, slow: Option<u64>) -> (ModelDescription, u32, u32, u32, u32) {
    let mut model = ModelDescription::new();
    let source = model.add_source(interval);
    let feed = model.add_conveyor(10.0, 0.6, 0.5);
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let out = model.add_conveyor(10.0, 0.6, 0.5);
    let drain = model.add_drain();
    model.connect(source, feed);
    model.connect_at(feed, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(if turn { Side::Left } else { Side::Front }), out, None);
    match slow {
        Some(secs) => {
            let station = station(&mut model, secs);
            model.connect(out, station);
            model.connect(station, drain);
        }
        None => model.connect(out, drain),
    }
    let to = model.add_destination(&[drain]);
    model.set_exit_destinations(cross, Side::Left, &[to]);
    if turn {
        let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: to, process_time: None }]);
        model.set_mu_kind(source, kind);
    }
    (model, feed, cross, out, drain)
}

#[test]
fn between_conveyors_an_mu_passes_straight_through_as_between_two_conveyors() {
    let (model, feed, cross, out, drain) = belts(secs(100), false);

    let (_, occurrences) = run_until(&model, secs(125));

    // Its booking point crosses the feed's end 20 s after entering, the cross conveyor's 0.6 m
    // later, and the outfeed's 10 m after that.
    assert_eq!(moves(&occurrences, feed, cross), [120.0]);
    assert_eq!(moves(&occurrences, cross, out), [121.2]);
    assert!(moves(&occurrences, out, drain).is_empty());
    let (_, occurrences) = run_until(&model, secs(142));
    assert_eq!(moves(&occurrences, out, drain), [141.2]);
}

#[test]
fn a_cross_conveyor_is_busy_from_an_mu_touching_it_until_its_rear_has_left() {
    let (model, _, cross, _, _) = belts(secs(100), false);

    let (run, _) = run_until(&model, secs(150));

    // The MU's front reaches the cross conveyor at 119.4 s, and its rear leaves it 0.6 + 0.6 m
    // later, at 121.8 s, though it belongs to it only from 120 to 121.2 s.
    let statistics = object(&run.state(), cross).statistics;
    assert_eq!(statistics.time_busy, millis(2_400));
    assert_eq!(statistics.time_idle, millis(150_000 - 2_400));
}

#[test]
fn a_turned_mu_lies_crosswise_on_the_next_conveyor() {
    let (model, _, cross, out, drain) = belts(secs(100), true);

    let (run, occurrences) = run_until(&model, secs(130));

    // At the centre at 120.6 s, sideways from 122.6 s, handed over 0.6 s later.
    assert_eq!(moves(&occurrences, cross, out), [123.2]);
    let position = object(&run.state(), out).positions[0];
    assert!(position.crosswise);
    // Its booking point, at its middle, entered with its front 0.2 m past the start.
    assert!((position.front - (0.2 + 6.8 * 0.5)).abs() < 1e-9, "{position:?}");
    let (_, occurrences) = run_until(&model, secs(145));
    assert_eq!(moves(&occurrences, out, drain), [143.2], "10 m by its booking point");
}

#[test]
fn a_conveyor_too_narrow_for_an_mu_lying_crosswise_refuses_it() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    // 0.5 m wide: room for the MU's 0.4 m width, but not its 0.6 m length across.
    let out = model.add_conveyor(10.0, 0.5, 0.5);
    let drain = model.add_drain();
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Left), out, None);
    model.connect(out, drain);
    let to = model.add_destination(&[drain]);
    model.set_exit_destinations(cross, Side::Left, &[to]);
    let kind = model.add_mu_kind_with_plan(MuKind::default(), vec![PlanStep { at: to, process_time: None }]);
    model.set_mu_kind(source, kind);

    let (run, occurrences) = run_until(&model, secs(200));

    assert!(moves(&occurrences, cross, out).is_empty());
    assert_eq!(object(&run.state(), cross).process, ProcessState::Blocked);
}

/// Nothing overlaps: at most one MU on the cross conveyor, the feed's first MU behind it, and no
/// MU on it before the last one handed to the outfeed has its rear off it.
fn assert_nothing_overlaps(state: &RunState, feed: u32, cross: u32, out: u32, at: SimTime) {
    const TOLERANCE: f64 = 1e-3;
    let on_cross = &object(state, cross).positions;
    assert!(on_cross.len() <= 1, "at {at:?}: {on_cross:?}");
    let first_on_feed = object(state, feed).positions.first();
    let last_on_out = object(state, out).positions.last();
    if let Some(mu) = on_cross.first() {
        let along = if mu.crosswise { 0.4 } else { 0.6 };
        if let Some(first) = first_on_feed.filter(|_| mu.from_side == Some(Side::Back)) {
            assert!(first.front <= 10.0 + (mu.front - along) + TOLERANCE, "feed into the cross conveyor at {at:?}: {first:?} and {mu:?}");
        }
        if let Some(last) = last_on_out {
            let rear = last.front - if last.crosswise { 0.4 } else { 0.6 };
            assert!(rear >= -TOLERANCE, "the outfeed's last MU is still on the cross conveyor at {at:?}: {last:?} and {mu:?}");
        }
    }
}

#[test]
fn mus_never_overlap_through_a_cross_conveyor_however_they_go() {
    for (interval, turn, slow_out) in [(millis(700), false, false), (millis(700), true, false), (millis(1_300), true, true), (millis(900), false, true)] {
        // A slow Station after the outfeed fills it, so the cross conveyor waits for room.
        let (mut model, feed, cross, out, _) = belts_to(interval, turn, slow_out.then_some(2));
        model.set_release_time(cross, millis(300));
        let mut run = model.build(3);
        run.init().unwrap();
        for tenth in 1..3_000 {
            let at = millis(tenth * 100);
            run.advance_to(at).unwrap();
            assert_nothing_overlaps(&run.state(), feed, cross, out, at);
        }
    }
}

#[test]
fn reset_forgets_what_a_cross_conveyor_was_doing_and_replays_the_same() {
    let (mut model, _, cross, _, _) = belts(millis(900), true);
    model.set_release_time(cross, millis(500));
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut first = run.init().unwrap();
    first.extend(run.advance_to(secs(300)).unwrap());
    let state = run.state();

    run.reset();
    let mut again = run.init().unwrap();
    again.extend(run.advance_to(secs(300)).unwrap());

    assert_eq!(first, again);
    assert_eq!(state, run.state());
    run.reset();
    run.init().unwrap();
    let fresh = run.state();
    assert!(object(&fresh, cross).positions.is_empty());
    assert_eq!(object(&fresh, cross).cross.as_deref().copied().unwrap(), crabsim::CrossStatistics::default());
}
