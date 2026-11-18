//! A Buffer, and how blocked predecessors wait once an object holds several MUs (spec issue 110,
//! ADR 0018), tested at the run's interface: Init, advance, Reset, then read occurrences and state.

use crabsim::random_numbers::Distribution;
use crabsim::{
    Capacity, ExitStrategy, FailureProfile, ModelDescription, ObjectStatistics, Occurrence, OccurrenceKind, ProcessState, Recording, RunState, SimTime, SimulationRun,
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

fn preloaded_station(model: &mut ModelDescription, process_secs: u64, mu_id: u32) -> u32 {
    model.add_station_holding(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), mu_id)
}

/// A Buffer that starts holding these MUs, in this order of entry.
fn buffer_holding(model: &mut ModelDescription, capacity: Capacity, mus: &[u32]) -> u32 {
    let buffer = model.add_buffer(capacity);
    model.set_starting_mus(buffer, mus);
    buffer
}

fn connect(model: &mut ModelDescription, connections: &[(u32, u32)]) {
    for &(from, to) in connections {
        model.connect(from, to);
    }
}

/// The model's run from Init to `until`, with every occurrence.
fn run_to(model: &ModelDescription, until: u64) -> (SimulationRun, Vec<Occurrence>) {
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    (run, occurrences)
}

/// Each MU that left the model: its id and when, in seconds.
fn exits(occurrences: &[Occurrence]) -> Vec<(u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuLeft { mu_id, .. } => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect()
}

/// Each MU that moved from `from` to `to`: its id and when, in seconds.
fn moves(occurrences: &[Occurrence], from: u32, to: u32) -> Vec<(u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { mu_id, from: f, to: t } if f == from && t == to => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect()
}

fn statistics_of(run: &SimulationRun, id: u32) -> ObjectStatistics {
    run.state().objects[id as usize].statistics
}

fn process_of(state: &RunState, id: u32) -> ProcessState {
    state.objects[id as usize].process.clone()
}

fn assert_times_add_up(run: &SimulationRun) {
    let state = run.state();
    for object in &state.objects {
        assert_eq!(object.statistics.measured_time(), state.time, "object {}", object.id);
    }
}

#[test]
fn a_lone_mu_passes_straight_through_an_empty_buffer() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let buffer = model.add_buffer(Capacity::Unlimited);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, drain)]);
    let (run, occurrences) = run_to(&model, 35);
    assert_eq!(exits(&occurrences), [(1, 10), (2, 20), (3, 30)]);
    let buffer = statistics_of(&run, buffer);
    assert_eq!((buffer.mus_in, buffer.mus_out), (3, 3), "no MU is lost or kept");
    assert_times_add_up(&run);
}

#[test]
fn mus_leave_a_buffer_first_in_first_out_by_default() {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2, 3]);
    let slow = station(&mut model, 10);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, slow), (slow, drain)]);
    let (_, occurrences) = run_to(&model, 100);
    assert_eq!(moves(&occurrences, buffer, slow), [(1, 0), (2, 10), (3, 20)]);
    assert_eq!(exits(&occurrences), [(1, 10), (2, 20), (3, 30)]);
}

#[test]
fn a_last_in_first_out_buffer_passes_on_its_newest_mu() {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2, 3]);
    model.set_exit_strategy(buffer, ExitStrategy::LastInFirstOut);
    let slow = station(&mut model, 10);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, slow), (slow, drain)]);
    let (_, occurrences) = run_to(&model, 100);
    assert_eq!(exits(&occurrences), [(3, 10), (2, 20), (1, 30)]);
}

#[test]
fn an_mu_stays_for_the_dwelling_time_counted_from_its_own_entry() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let buffer = model.add_buffer(Capacity::Unlimited);
    model.set_dwelling_time(buffer, secs(25));
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, drain)]);
    let (run, occurrences) = run_to(&model, 60);
    // Entered at 10, 20, 30, ... and each leaves 25 s later, so three are inside at once.
    assert_eq!(exits(&occurrences), [(1, 35), (2, 45), (3, 55)]);
    assert_eq!(statistics_of(&run, buffer).max_contents, 3);
    assert_times_add_up(&run);
}

#[test]
fn starting_mus_dwell_from_init() {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2]);
    model.set_dwelling_time(buffer, secs(7));
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, drain)]);
    let (run, occurrences) = run_to(&model, 20);
    assert_eq!(exits(&occurrences), [(1, 7), (2, 7)]);
    assert_eq!(statistics_of(&run, buffer).mus_in, 2, "a starting MU counts at Init");
}

#[test]
fn last_in_first_out_picks_among_the_mus_whose_dwelling_time_is_over() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let buffer = model.add_buffer(Capacity::Unlimited);
    model.set_dwelling_time(buffer, secs(15));
    model.set_exit_strategy(buffer, ExitStrategy::LastInFirstOut);
    // Busy until 50, so the Source's MUs 101 to 104 (in at 10, 20, 30, 40) wait in the Buffer.
    let slow = preloaded_station(&mut model, 50, 100);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let (_, occurrences) = run_to(&model, 99);
    // At 50, MU 104 came in last but has dwelt only 10 s: MU 103 is the newest that may leave.
    assert_eq!(moves(&occurrences, buffer, slow), [(103, 50)]);
}

#[test]
fn a_full_buffer_refuses_mus_and_takes_the_next_once_one_leaves() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(5));
    let buffer = model.add_buffer(Capacity::Limited(2));
    let slow = station(&mut model, 20);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let (run, occurrences) = run_to(&model, 100);
    // MU 1 passes through at 5, MUs 2 and 3 fill the Buffer at 10 and 15, and MU 4 waits in the
    // Source from 20 until the Station takes MU 2 at 25.
    assert_eq!(moves(&occurrences, source, buffer), [(1, 5), (2, 10), (3, 15), (4, 25), (5, 45), (6, 65), (7, 85)]);
    assert_eq!(moves(&occurrences, buffer, slow), [(1, 5), (2, 25), (3, 45), (4, 65), (5, 85)]);
    let state = run.state();
    assert_eq!(state.objects[buffer as usize].capacity, Capacity::Limited(2));
    assert_eq!(state.objects[buffer as usize].contents, [6, 7]);
    let buffer_statistics = statistics_of(&run, buffer);
    assert_eq!(buffer_statistics.max_contents, 2);
    assert_eq!(buffer_statistics.time_full, secs(10) + secs(75), "full from 15 to 25, and from 25 on but for instants");
    assert_eq!(statistics_of(&run, source).time_blocked, secs(60), "20 to 25, three times 15 s, and from 90");
    assert_times_add_up(&run);
}

#[test]
fn an_unlimited_buffer_takes_every_mu() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let buffer = model.add_buffer(Capacity::Unlimited);
    let slow = station(&mut model, 1_000);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let (run, _) = run_to(&model, 500);
    let state = run.state();
    assert_eq!(state.objects[buffer as usize].capacity, Capacity::Unlimited);
    assert_eq!(state.objects[buffer as usize].contents.len(), 499);
    assert_eq!(statistics_of(&run, source).time_blocked, SimTime::ZERO);
    assert_eq!(statistics_of(&run, buffer).time_full, SimTime::ZERO, "an unlimited Buffer is never full");
}

#[test]
fn a_buffer_is_empty_holding_or_blocked() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let buffer = model.add_buffer(Capacity::Unlimited);
    model.set_dwelling_time(buffer, secs(5));
    let slow = station(&mut model, 30);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    let mut advance = |run: &mut SimulationRun, to: u64| occurrences.extend(run.advance_to(secs(to)).unwrap());
    advance(&mut run, 5);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Idle, "empty");
    advance(&mut run, 12);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Processing, "holding MU 1, which dwells until 15");
    advance(&mut run, 16);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Idle, "MU 1 went on at 15");
    advance(&mut run, 22);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Processing, "MU 2 dwells until 25");
    advance(&mut run, 26);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Blocked, "MU 2 may leave and the Station works until 45");
    advance(&mut run, 46);
    let blocking: Vec<(u64, bool)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::Blocked { object_id } if object_id == buffer => Some((o.time.as_secs(), true)),
            OccurrenceKind::Unblocked { object_id } if object_id == buffer => Some((o.time.as_secs(), false)),
            _ => None,
        })
        .collect();
    // MU 3 has dwelt since 35, so the Buffer is blocked again as soon as MU 2 has left.
    assert_eq!(blocking, [(25, true), (45, false), (45, true)]);
    let statistics = statistics_of(&run, buffer);
    assert_eq!(statistics.time_blocked, secs(21));
    assert_eq!(statistics.time_busy, secs(5) + secs(5), "holding without being blocked: 10 to 15 and 20 to 25");
    assert_eq!(statistics.time_idle, secs(10) + secs(5));
    assert_times_add_up(&run);
}

/// A Buffer holding MUs 1 and 2 and a Station finishing MU 3 at `station_ready` both feed one
/// Station, which works on MU 100 until 20 and then 20 s on each MU. Returns the order the three
/// reach it.
fn order_into_the_shared_station(station_ready: u64, buffer_dwelling: u64) -> Vec<(u32, u64)> {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2]);
    model.set_dwelling_time(buffer, secs(buffer_dwelling));
    let feeder = preloaded_station(&mut model, station_ready, 3);
    let shared = preloaded_station(&mut model, 20, 100);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, shared), (feeder, shared), (shared, drain)]);
    let (_, occurrences) = run_to(&model, 200);
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { mu_id, to, .. } if to == shared => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect()
}

#[test]
fn the_mu_that_has_been_ready_longest_goes_first_whichever_object_holds_it() {
    // Both of the Buffer's MUs have been ready since 2, the Station's since 5: the Buffer's second
    // MU goes before the Station's, though the Buffer has just had its turn.
    assert_eq!(order_into_the_shared_station(5, 2), [(1, 20), (2, 40), (3, 60)]);
}

#[test]
fn a_predecessor_whose_mu_moved_waits_again_by_its_next_mu() {
    // Now the Station's MU has been ready since 1 and the Buffer's since 2, so the Station goes
    // first and then both of the Buffer's.
    assert_eq!(order_into_the_shared_station(1, 2), [(3, 20), (1, 40), (2, 60)]);
}

#[test]
fn mus_ready_at_the_same_instant_go_in_the_order_they_joined_the_line() {
    // All three have been ready since 5. The Buffer joined the line first, with MU 1, and joins
    // it again with MU 2 at 20, behind the Station.
    assert_eq!(order_into_the_shared_station(5, 5), [(1, 20), (3, 40), (2, 60)]);
}

#[test]
fn a_buffer_waiting_at_two_successors_keeps_its_place_at_the_other_when_one_takes_an_mu() {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2, 3]);
    let feeder = preloaded_station(&mut model, 5, 4);
    // `first` frees at 10 and 20, and `second` at 25. The Buffer waits at both from 0, the feeder
    // at `second` from 5.
    let first = preloaded_station(&mut model, 10, 100);
    let second = preloaded_station(&mut model, 25, 101);
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, first), (buffer, second), (feeder, second), (first, drain), (second, drain)]);
    let (_, occurrences) = run_to(&model, 100);
    assert_eq!(moves(&occurrences, buffer, first), [(1, 10), (2, 20)]);
    // MU 3 has been ready since 0, longer than the feeder's MU 4, so it gets `second` at 25,
    // although two of the Buffer's MUs went elsewhere in between.
    assert_eq!(moves(&occurrences, buffer, second), [(3, 25)]);
    assert_eq!(moves(&occurrences, feeder, second), [(4, 50)]);
}

#[test]
fn a_buffer_passes_on_every_mu_that_may_leave_at_one_instant() {
    let mut model = ModelDescription::new();
    let buffer = buffer_holding(&mut model, Capacity::Unlimited, &[1, 2, 3]);
    model.set_dwelling_time(buffer, secs(4));
    let drain = model.add_drain();
    connect(&mut model, &[(buffer, drain)]);
    let (run, occurrences) = run_to(&model, 10);
    assert_eq!(exits(&occurrences), [(1, 4), (2, 4), (3, 4)]);
    assert_eq!(process_of(&run.state(), buffer), ProcessState::Idle);
}

#[test]
fn buffers_in_a_row_hand_mus_on() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(2));
    let one = model.add_buffer(Capacity::Limited(1));
    let two = model.add_buffer(Capacity::Limited(2));
    let slow = station(&mut model, 10);
    let drain = model.add_drain();
    connect(&mut model, &[(source, one), (one, two), (two, slow), (slow, drain)]);
    let (run, occurrences) = run_to(&model, 1_000);
    let left: Vec<u32> = exits(&occurrences).iter().map(|&(mu, _)| mu).collect();
    assert_eq!(left, (1..=99).collect::<Vec<u32>>(), "in order, one every 10 s once the line is full");
    let state = run.state();
    let inside: usize = state.objects.iter().map(|object| object.contents.len()).sum();
    assert_eq!(statistics_of(&run, source).mus_in as usize, 99 + inside, "every MU created is in the model or has left");
    assert_times_add_up(&run);
}

#[test]
fn reset_returns_a_buffer_to_its_starting_mus() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(3));
    let buffer = buffer_holding(&mut model, Capacity::Limited(4), &[1, 2]);
    model.set_dwelling_time(buffer, secs(2));
    model.set_exit_strategy(buffer, ExitStrategy::LastInFirstOut);
    let slow = station(&mut model, 7);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let before = run.state();
    let mut first = run.init().unwrap();
    first.extend(run.advance_to(secs(203)).unwrap());
    assert_eq!(run.state().objects[buffer as usize].blocked_predecessors, [source], "the Source waits for the full Buffer");
    run.reset();
    assert_eq!(run.state(), before);
    assert_eq!(before.objects[buffer as usize].contents, [1, 2]);
    let mut second = run.init().unwrap();
    second.extend(run.advance_to(secs(203)).unwrap());
    assert_eq!(first, second);
}

#[test]
fn a_clone_of_a_run_with_buffers_carries_on_as_the_original() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(3));
    let buffer = model.add_buffer(Capacity::Limited(3));
    model.set_dwelling_time(buffer, secs(4));
    let slow = station(&mut model, 5);
    let drain = model.add_drain();
    connect(&mut model, &[(source, buffer), (buffer, slow), (slow, drain)]);
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    run.init().unwrap();
    run.advance_to(secs(101)).unwrap();
    let mut copy = run.clone();
    assert_eq!(run.advance_to(secs(400)).unwrap(), copy.advance_to(secs(400)).unwrap());
    assert_eq!(run.state(), copy.state());
}

#[test]
fn a_source_keeps_its_next_mu_for_the_whole_interval_when_two_successors_free_up_at_once() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(2));
    let a = preloaded_station(&mut model, 1, 1);
    let b = preloaded_station(&mut model, 1, 2);
    // Holds two MUs for 100 s each, and lets both go at 101, which frees both Stations at once.
    let gate = model.add_buffer(Capacity::Limited(2));
    model.set_dwelling_time(gate, secs(100));
    let drain = model.add_drain();
    connect(&mut model, &[(source, a), (source, b), (a, gate), (b, gate), (gate, drain)]);
    let (_, occurrences) = run_to(&model, 104);
    let left_the_source: Vec<(u32, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { mu_id, from, .. } if from == source => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect();
    // MU 5 waits from 6 and goes at 101. MU 6 is created then, and its interval ends at 103.
    assert_eq!(left_the_source, [(3, 2), (4, 4), (5, 101), (6, 103)]);
}

// ---------- Fill statistics (PR 3) ----------

/// A Buffer starting with MUs 1 to 4 before a Station working 10 s: it holds 3 from 0 to 10, 2 to
/// 20, 1 to 30 and none after.
fn emptying() -> (ModelDescription, u32) {
    let mut model = ModelDescription::new();
    let store = buffer_holding(&mut model, Capacity::Limited(4), &[1, 2, 3, 4]);
    let slow = station(&mut model, 10);
    let out = model.add_drain();
    connect(&mut model, &[(store, slow), (slow, out)]);
    (model, store)
}

#[test]
fn a_buffer_reports_its_fewest_mus_its_median_and_a_histogram_of_its_fill() {
    let (model, store) = emptying();
    let (run, _) = run_to(&model, 40);
    let state = run.state();
    let fill = state.objects[store as usize].fill.as_deref().expect("a Buffer has fill statistics");
    assert_eq!(fill.lowest, 0);
    assert_eq!(fill.histogram.width, 1);
    assert_eq!(&fill.histogram.bars[..5], &[secs(10), secs(10), secs(10), secs(10), SimTime::ZERO]);
    assert_eq!(fill.median(), Some((1, Some(1))), "half the time at 1 or fewer");
    let statistics = state.objects[store as usize].statistics;
    assert_eq!((statistics.max_contents, statistics.average_contents()), (4, Some(1.5)));
    assert!(state.objects.iter().filter(|object| object.id != store).all(|object| object.fill.is_none()), "only a Buffer has them");
}

#[test]
fn fill_statistics_start_afresh_when_cleared_and_at_reset() {
    let (model, store) = emptying();
    let (mut run, _) = run_to(&model, 25);
    run.clear_statistics();
    run.advance_to(secs(28)).unwrap();
    let fill = run.state().objects[store as usize].fill.clone().unwrap();
    assert_eq!((fill.lowest, fill.histogram.total(), fill.histogram.bars[1]), (1, secs(3), secs(3)));
    run.reset();
    assert_eq!(run.state(), model.build(0).state());
}

#[test]
fn a_set_bar_width_holds_and_its_last_bar_takes_the_rest() {
    let (mut model, store) = emptying();
    model.set_histogram_bar_width(store, Some(2));
    let (run, _) = run_to(&model, 40);
    let fill = run.state().objects[store as usize].fill.clone().unwrap();
    assert_eq!(fill.histogram.width, 2);
    assert_eq!(&fill.histogram.bars[..3], &[secs(20), secs(20), SimTime::ZERO], "0 and 1, then 2 and 3");
    assert_eq!(fill.median(), Some((0, Some(1))));
}
