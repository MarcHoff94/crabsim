//! Work plans, detours and routing by destination (spec issue 148, ADR 0025 and ADR 0026),
//! tested at a run's seam: build a model, run it, read occurrences and state.

use crabsim::distribution::{DistributionRule, Order, Rank, WeighBy, WeightedRule};
use crabsim::flows::NO_DESTINATION;
use crabsim::random_numbers::Distribution;
use crabsim::routing::PlanStep;
use crabsim::{Capacity, FailureProfile, ModelDescription, MuKind, Occurrence, OccurrenceKind, Recording, SimTime, SimulationRun, StartMode};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

fn step(at: u16, process_secs: Option<u64>) -> PlanStep {
    PlanStep { at, process_time: process_secs.map(secs) }
}

/// Gives the Source's MUs a kind of the built-in MU's size with this plan.
fn follow(model: &mut ModelDescription, source: u32, plan: Vec<PlanStep>) {
    let kind = model.add_mu_kind_with_plan(MuKind::default(), plan);
    model.set_mu_kind(source, kind);
}

fn by_destination(then: DistributionRule) -> DistributionRule {
    DistributionRule::ByDestination(Box::new(then))
}

fn run(model: &ModelDescription, until: u64) -> (SimulationRun, Vec<Occurrence>) {
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    (run, occurrences)
}

/// How long each Station worked on each MU, as (MU, Station, seconds), in order.
fn work(occurrences: &[Occurrence]) -> Vec<(u32, u32, u64)> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { mu_id, at, duration } => Some((mu_id, at, duration.as_secs())),
            _ => None,
        })
        .collect()
}

/// Where each MU went from `from`, in order.
fn moves_from(occurrences: &[Occurrence], from: u32) -> Vec<u32> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuMoved { from: moved_from, to, .. } if moved_from == from => Some(to),
            _ => None,
        })
        .collect()
}

fn plan_counts(run: &SimulationRun, id: u32) -> (u64, u64) {
    let plan = run.state().objects[id as usize].plan.expect("a Station");
    (plan.on_plan, plan.off_plan)
}

/// A Source every 100 s into Saw (10 s), Mill (20 s) and the Drain, in a line.
fn line() -> (ModelDescription, [u32; 4]) {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let saw = station(&mut model, 10);
    let mill = station(&mut model, 20);
    let drain = model.add_drain();
    for (from, to) in [(source, saw), (saw, mill), (mill, drain)] {
        model.connect(from, to);
    }
    (model, [source, saw, mill, drain])
}

#[test]
fn mus_follow_their_plan_with_its_process_times() {
    let (mut model, [source, saw, mill, drain]) = line();
    let (at_saw, at_mill, at_drain) = (model.add_destination(&[saw]), model.add_destination(&[mill]), model.add_destination(&[drain]));
    follow(&mut model, source, vec![step(at_saw, Some(40)), step(at_mill, None), step(at_drain, None)]);
    let (run, occurrences) = run(&model, 300);
    assert_eq!(work(&occurrences)[..2], [(1, saw, 40), (1, mill, 20)], "the step's time at the Saw, the Mill's own without one");
    assert_eq!(plan_counts(&run, saw), (3, 0), "the third MU is at the Saw at 300 s");
    assert_eq!(plan_counts(&run, mill), (2, 0));
    let lead_times: Vec<u64> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuLeft { lead_time, .. } => Some(lead_time.as_secs()),
            _ => None,
        })
        .collect();
    assert_eq!(lead_times, vec![60, 60]);
}

#[test]
fn a_station_its_step_doesnt_name_works_its_own_time_and_counts_it_off_plan() {
    let (mut model, [source, saw, mill, _]) = line();
    let at_mill = model.add_destination(&[mill]);
    follow(&mut model, source, vec![step(at_mill, Some(30))]);
    let (run, occurrences) = run(&model, 250);
    assert_eq!(work(&occurrences)[..2], [(1, saw, 10), (1, mill, 30)], "off plan at the Saw, which keeps its step for the Mill");
    assert_eq!(plan_counts(&run, saw), (0, 2));
    assert_eq!(plan_counts(&run, mill), (2, 0));
}

#[test]
fn mus_without_a_plan_count_neither_on_nor_off_plan() {
    let (mut model, [_, saw, mill, _]) = line();
    model.add_destination(&[mill]);
    let (run, occurrences) = run(&model, 150);
    assert_eq!(work(&occurrences)[..2], [(1, saw, 10), (1, mill, 20)]);
    assert_eq!(plan_counts(&run, saw), (0, 0));
    assert_eq!(plan_counts(&run, mill), (0, 0));
}

#[test]
fn a_plan_step_counts_only_when_it_is_next() {
    // Mill first, then Saw: passing the Saw on the way to the Mill doesn't count for the Saw.
    let (mut model, [source, saw, mill, _]) = line();
    let (at_saw, at_mill) = (model.add_destination(&[saw]), model.add_destination(&[mill]));
    follow(&mut model, source, vec![step(at_mill, Some(5)), step(at_saw, Some(50))]);
    let (run, occurrences) = run(&model, 250);
    assert_eq!(work(&occurrences)[..2], [(1, saw, 10), (1, mill, 5)]);
    assert_eq!(plan_counts(&run, saw), (0, 2));
}

/// A Source every `interval` seconds into a Fork (0 s) with the successors given, each a Station
/// of the time given into one Drain.
fn fork(interval: u64, times: &[u64]) -> (ModelDescription, u32, u32, Vec<u32>, u32) {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(interval));
    let fork = station(&mut model, 0);
    let drain = model.add_drain();
    model.connect(source, fork);
    let ways: Vec<u32> = times.iter().map(|&time| station(&mut model, time)).collect();
    for &way in &ways {
        model.connect(fork, way);
        model.connect(way, drain);
    }
    (model, source, fork, ways, drain)
}

#[test]
fn a_class_step_is_reached_at_any_of_its_instances_taking_turns_among_the_ways_that_reach() {
    let (mut model, source, fork, ways, _) = fork(10, &[5, 5, 5]);
    let any_mill = model.add_destination(&[ways[0], ways[2]]);
    follow(&mut model, source, vec![step(any_mill, Some(7))]);
    model.set_distribution(fork, by_destination(DistributionRule::RoundRobin));
    let (run, occurrences) = run(&model, 60);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[0], ways[2], ways[0], ways[2], ways[0], ways[2]]);
    assert!(work(&occurrences).iter().filter(|&&(_, at, _)| at != fork).all(|&(_, _, time)| time == 7));
    assert_eq!(plan_counts(&run, ways[0]), (3, 0));
    assert_eq!(plan_counts(&run, ways[1]), (0, 0), "never reached");
}

#[test]
fn by_destination_then_weighted_ranks_among_the_ways_that_reach() {
    let (mut model, source, fork, ways, _) = fork(10, &[5, 1, 3]);
    let any_of_two = model.add_destination(&[ways[0], ways[2]]);
    follow(&mut model, source, vec![step(any_of_two, None)]);
    let weighted = DistributionRule::Weighted(WeightedRule { by: WeighBy::ProcessTime, order: Order::Smallest, rank: Rank::Successor, drains: None });
    model.set_distribution(fork, by_destination(weighted));
    let (_, occurrences) = run(&model, 40);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[2]; 4], "the quickest that reaches, not the quickest of all");
}

#[test]
fn a_blocked_way_that_reaches_is_waited_for() {
    let (mut model, source, fork, ways, _) = fork(1, &[100, 1]);
    let first = model.add_destination(&[ways[0]]);
    follow(&mut model, source, vec![step(first, None)]);
    model.set_distribution(fork, by_destination(DistributionRule::RoundRobin));
    let (run, occurrences) = run(&model, 250);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[0], ways[0], ways[0]], "MUs wait for the slow way rather than go wrong");
    assert_eq!(run.state().objects[ways[1] as usize].statistics.mus_in, 0);
}

#[test]
fn an_mu_with_no_destination_left_is_offered_to_every_successor() {
    let (mut model, source, fork, ways, _) = fork(10, &[5, 5]);
    // Its only step is the Fork itself, so it has none left once there.
    let at_fork = model.add_destination(&[fork]);
    follow(&mut model, source, vec![step(at_fork, None)]);
    model.set_distribution(fork, by_destination(DistributionRule::RoundRobin));
    let (_, occurrences) = run(&model, 40);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[0], ways[1], ways[0], ways[1]]);
}

#[test]
fn an_mu_whose_destination_no_way_reaches_is_offered_to_every_successor() {
    let (mut model, source, fork, ways, _) = fork(10, &[5, 5]);
    let elsewhere = station(&mut model, 1);
    let unreachable = model.add_destination(&[elsewhere]);
    follow(&mut model, source, vec![step(unreachable, None)]);
    model.set_distribution(fork, by_destination(DistributionRule::RoundRobin));
    let (_, occurrences) = run(&model, 40);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[0], ways[1], ways[0], ways[1]]);
}

#[test]
fn mus_without_a_plan_go_round_robin_at_an_object_routing_by_destination() {
    let (mut model, _, fork, ways, _) = fork(10, &[5, 5]);
    model.add_destination(&[ways[0]]);
    model.set_distribution(fork, by_destination(DistributionRule::RoundRobin));
    let (_, occurrences) = run(&model, 40);
    assert_eq!(moves_from(&occurrences, fork), vec![ways[0], ways[1], ways[0], ways[1]]);
}

/// Inspection with a detour to Rework, then Pack: Source, Inspection routing by destination to
/// Rework or Pack, Rework into Pack, Pack into the Drain.
fn inspection() -> (ModelDescription, [u32; 5]) {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(100));
    let inspection = station(&mut model, 2);
    let rework = station(&mut model, 30);
    let pack = station(&mut model, 4);
    let drain = model.add_drain();
    for (from, to) in [(source, inspection), (inspection, rework), (inspection, pack), (rework, pack), (pack, drain)] {
        model.connect(from, to);
    }
    model.set_distribution(inspection, by_destination(DistributionRule::RoundRobin));
    (model, [source, inspection, rework, pack, drain])
}

#[test]
fn a_detour_sends_mus_somewhere_first_and_they_carry_on_with_their_plan() {
    let (mut model, [source, inspection, rework, pack, _]) = inspection();
    let (at_inspection, at_rework, at_pack) = (model.add_destination(&[inspection]), model.add_destination(&[rework]), model.add_destination(&[pack]));
    follow(&mut model, source, vec![step(at_inspection, None), step(at_pack, Some(9))]);
    model.set_detour(inspection, at_rework);
    let (run, occurrences) = run(&model, 150);
    assert_eq!(moves_from(&occurrences, inspection), vec![rework], "to Rework first, though Pack is its next step");
    assert_eq!(work(&occurrences)[..3], [(1, inspection, 2), (1, rework, 30), (1, pack, 9)], "Rework's own time, then the plan's");
    assert_eq!(plan_counts(&run, rework), (1, 0), "a detour reached counts with the plan");
    assert_eq!(plan_counts(&run, pack), (1, 0));
}

#[test]
fn a_detour_without_a_plan_still_sends_mus_there() {
    let (mut model, [_, inspection, rework, pack, _]) = inspection();
    let at_rework = model.add_destination(&[rework]);
    model.set_detour(inspection, at_rework);
    let (run, occurrences) = run(&model, 350);
    assert_eq!(moves_from(&occurrences, inspection), vec![rework, rework, rework]);
    assert_eq!(plan_counts(&run, pack), (0, 0), "no plan, nowhere left to go");
}

#[test]
fn a_detour_survives_a_change_of_capacity() {
    let (mut model, [_, inspection, rework, _, _]) = inspection();
    let at_rework = model.add_destination(&[rework]);
    model.set_detour(inspection, at_rework);
    model.set_station_capacity(inspection, 2, StartMode::Individual);
    assert_eq!(model.detour(inspection), at_rework);
    model.set_station_capacity(inspection, 1, StartMode::Individual);
    assert_eq!(model.detour(inspection), at_rework);
    model.set_detour(inspection, NO_DESTINATION);
    assert_eq!(model.detour(inspection), NO_DESTINATION);
}

#[test]
fn a_station_with_capacity_works_each_mu_for_its_own_step() {
    // Two MU kinds from two Sources into one Station of capacity 2: one worked on for 10 s, the
    // other for the Station's own 30 s.
    let mut model = ModelDescription::new();
    let quick = model.add_source(secs(100));
    let slow = model.add_source(secs(100));
    let oven = station(&mut model, 30);
    model.set_station_capacity(oven, 2, StartMode::Individual);
    let drain = model.add_drain();
    for (from, to) in [(quick, oven), (slow, oven), (oven, drain)] {
        model.connect(from, to);
    }
    let at_oven = model.add_destination(&[oven]);
    follow(&mut model, quick, vec![step(at_oven, Some(10))]);
    let (_, occurrences) = run(&model, 150);
    let mut worked: Vec<u64> = work(&occurrences).iter().map(|&(_, _, time)| time).collect();
    worked.sort();
    assert_eq!(worked, vec![10, 30]);
}

#[test]
fn a_batch_of_mus_with_their_own_step_times_finishes_each_at_its_own_time() {
    let mut model = ModelDescription::new();
    let quick = model.add_source(secs(1_000));
    let slow = model.add_source(secs(1_000));
    let oven = station(&mut model, 30);
    model.set_station_capacity(oven, 2, StartMode::Batch);
    let drain = model.add_drain();
    for (from, to) in [(quick, oven), (slow, oven), (oven, drain)] {
        model.connect(from, to);
    }
    let at_oven = model.add_destination(&[oven]);
    follow(&mut model, quick, vec![step(at_oven, Some(10))]);
    let (_, occurrences) = run(&model, 1_100);
    let left: Vec<u64> = occurrences
        .iter()
        .filter_map(|o| matches!(o.kind, OccurrenceKind::MuLeft { .. }).then_some(o.time.as_secs()))
        .collect();
    assert_eq!(left, vec![1_010, 1_030], "the batch starts at once, each MU finishing on its own time");
}

#[test]
fn reset_starts_plans_and_counts_afresh_and_keeps_detours() {
    let (mut model, [source, inspection, rework, pack, _]) = inspection();
    let (at_inspection, at_rework, at_pack) = (model.add_destination(&[inspection]), model.add_destination(&[rework]), model.add_destination(&[pack]));
    follow(&mut model, source, vec![step(at_inspection, None), step(at_pack, Some(9))]);
    model.set_detour(inspection, at_rework);
    let mut run = model.build(0);
    run.set_recording(Recording::ALL);
    run.init().unwrap();
    let first: Vec<Occurrence> = run.advance_to(secs(450)).unwrap();
    assert_eq!(plan_counts(&run, pack), (4, 0));
    run.reset();
    assert_eq!(plan_counts(&run, pack), (0, 0));
    let mut again = run.init().unwrap();
    again.extend(run.advance_to(secs(450)).unwrap());
    assert_eq!(&again[again.len() - first.len()..], &first[..], "the same run again");
}

#[test]
fn clearing_statistics_clears_plan_counts() {
    let (mut model, [source, saw, mill, _]) = line();
    let at_mill = model.add_destination(&[mill]);
    follow(&mut model, source, vec![step(at_mill, None)]);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(250)).unwrap();
    assert_eq!(plan_counts(&run, saw), (0, 2));
    run.clear_statistics();
    assert_eq!(plan_counts(&run, saw), (0, 0));
    run.advance_to(secs(350)).unwrap();
    assert_eq!(plan_counts(&run, saw), (0, 1));
}

#[test]
fn a_buffer_routes_each_mu_by_its_own_destination() {
    // Two kinds into a Buffer that routes by destination to two Stations: each MU goes its way.
    let mut model = ModelDescription::new();
    let to_a = model.add_source(secs(10));
    let to_b = model.add_source(secs(10));
    let store = model.add_buffer(Capacity::Unlimited);
    let a = station(&mut model, 1);
    let b = station(&mut model, 1);
    let drain = model.add_drain();
    for (from, to) in [(to_a, store), (to_b, store), (store, a), (store, b), (a, drain), (b, drain)] {
        model.connect(from, to);
    }
    let (at_a, at_b) = (model.add_destination(&[a]), model.add_destination(&[b]));
    follow(&mut model, to_a, vec![step(at_a, None)]);
    let kind_b = model.add_mu_kind_with_plan(MuKind { length: 0.6, width: 0.4, booking_point: 0.5 }, vec![step(at_b, None)]);
    model.set_mu_kind(to_b, kind_b);
    model.set_distribution(store, by_destination(DistributionRule::RoundRobin));
    let (run, _) = run(&model, 105);
    assert_eq!(plan_counts(&run, a), (10, 0));
    assert_eq!(plan_counts(&run, b), (10, 0));
}

#[test]
fn two_kinds_alike_but_for_their_plans_are_two_kinds() {
    let mut model = ModelDescription::new();
    let saw = station(&mut model, 1);
    let at_saw = model.add_destination(&[saw]);
    let plain = model.add_mu_kind(MuKind::default());
    let planned = model.add_mu_kind_with_plan(MuKind::default(), vec![step(at_saw, None)]);
    assert_eq!(plain, 0, "the built-in MU");
    assert_ne!(planned, plain);
    assert_eq!(model.add_mu_kind_with_plan(MuKind::default(), vec![step(at_saw, None)]), planned);
    assert_eq!(model.add_destination(&[saw]), at_saw, "one destination for the same objects");
    assert_eq!(model.work_plan(planned), &[step(at_saw, None)]);
}
