//! Distribution strategies (ADR 0016): which successor a leaving MU goes to, round robin or by
//! a ranking of weighted routes, tested through models built with `ModelDescription`.

use crabsim::distribution::{DistributionRule, Order, Rank, Ranking, WeighBy, WeightedRule};
use crabsim::random_numbers::Distribution;
use crabsim::{FailureProfile, ModelDescription, SimTime, SimulationRun};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

fn weighted(by: WeighBy, order: Order, rank: Rank, drains: Option<Vec<u32>>) -> DistributionRule {
    DistributionRule::Weighted(WeightedRule { by, order, rank, drains })
}

fn by_process_time(order: Order, rank: Rank) -> DistributionRule {
    weighted(WeighBy::ProcessTime, order, rank, None)
}

/// A Source every `interval` seconds into Split, which works no time and passes on to A (10 s)
/// then Out, or to B (2 s), then C (30 s), then Out. The lightest route onward is through A,
/// 10 s against B's 32 s, and the lightest successor on its own is B.
struct Fork {
    model: ModelDescription,
    split: u32,
    a: u32,
    b: u32,
}

fn fork(interval: u64) -> Fork {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(interval));
    let split = station(&mut model, 0);
    let a = station(&mut model, 10);
    let b = station(&mut model, 2);
    let c = station(&mut model, 30);
    let out = model.add_drain();
    model.connect(source, split);
    model.connect(split, a);
    model.connect(split, b);
    model.connect(a, out);
    model.connect(b, c);
    model.connect(c, out);
    Fork { model, split, a, b }
}

fn ran(model: &ModelDescription, hours: u64) -> SimulationRun {
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(hours * 3600)).unwrap();
    run
}

fn mus_in(run: &SimulationRun, id: u32) -> u64 {
    run.state().objects[id as usize].statistics.mus_in
}

fn ranking(groups: Vec<Vec<usize>>, totals: Vec<Option<u64>>) -> Ranking {
    Ranking { groups, totals }
}

#[test]
fn a_weighted_object_ranks_its_successors_by_route_or_by_their_own_weight() {
    let mut fork = fork(20);
    let ms = |s: u64| Some(s * 1000);
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    assert_eq!(fork.model.ranking(fork.split), Some(ranking(vec![vec![0], vec![1]], vec![ms(10), ms(32)])), "A's route is the lightest");
    fork.model.set_distribution(fork.split, by_process_time(Order::Largest, Rank::Route));
    assert_eq!(fork.model.ranking(fork.split), Some(ranking(vec![vec![1], vec![0]], vec![ms(10), ms(32)])), "B's the heaviest");
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Successor));
    assert_eq!(fork.model.ranking(fork.split), Some(ranking(vec![vec![1], vec![0]], vec![ms(10), ms(2)])), "B on its own is the lightest");
    fork.model.set_distribution(fork.split, DistributionRule::RoundRobin);
    assert_eq!(fork.model.ranking(fork.split), None);
}

#[test]
fn every_mu_goes_to_the_best_ranked_successor_while_it_can_take_one() {
    let mut fork = fork(20);
    // Round robin shares the MUs out.
    let run = ran(&fork.model, 2);
    assert!(mus_in(&run, fork.a) > 100 && mus_in(&run, fork.b) > 100, "round robin uses both");
    // A is free whenever an MU comes, so all of them go to it.
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    let run = ran(&fork.model, 2);
    assert_eq!(mus_in(&run, fork.b), 0);
    assert_eq!(mus_in(&run, fork.a), 360, "one every 20 s for 2 h");
    // B ranks first on its own weight, and takes every MU it can: C behind it takes 30 s, so it
    // is often blocked, and A takes the rest.
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Successor));
    let run = ran(&fork.model, 2);
    let (a, b) = (mus_in(&run, fork.a), mus_in(&run, fork.b));
    assert!(b > a && a > 0, "A {a}, B {b}");
}

#[test]
fn when_the_best_is_blocked_the_mu_goes_to_the_next_best_that_can_take_it() {
    // An MU every 6 s, and A takes 10 s, so A is often busy and B takes the rest.
    let mut fork = fork(6);
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    let run = ran(&fork.model, 2);
    let (a, b) = (mus_in(&run, fork.a), mus_in(&run, fork.b));
    assert!(a > 0 && b > 0, "A {a}, B {b}");
    assert!(a > b, "A, the best, takes most: A {a}, B {b}");
}

#[test]
fn successors_that_rank_equal_take_turns() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(20));
    let split = station(&mut model, 0);
    let first = station(&mut model, 10);
    let second = station(&mut model, 10);
    let out = model.add_drain();
    model.connect(source, split);
    model.connect(split, first);
    model.connect(split, second);
    model.connect(first, out);
    model.connect(second, out);
    model.set_distribution(split, by_process_time(Order::Smallest, Rank::Route));
    assert_eq!(model.ranking(split).unwrap().groups, vec![vec![0, 1]]);
    let run = ran(&model, 2);
    assert_eq!((mus_in(&run, first), mus_in(&run, second)), (180, 180), "parallel equal Stations share the work");
}

#[test]
fn only_the_drains_named_count_and_a_successor_with_no_route_to_one_ranks_last() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(20));
    let split = station(&mut model, 0);
    let slow = station(&mut model, 10);
    let fast = station(&mut model, 2);
    let good = model.add_drain();
    let scrap = model.add_drain();
    model.connect(source, split);
    model.connect(split, slow);
    model.connect(split, fast);
    model.connect(slow, good);
    model.connect(fast, scrap);
    model.set_distribution(split, weighted(WeighBy::ProcessTime, Order::Smallest, Rank::Route, Some(vec![good])));
    assert_eq!(model.ranking(split), Some(ranking(vec![vec![0], vec![1]], vec![Some(10_000), None])));
    let run = ran(&model, 1);
    assert_eq!(mus_in(&run, fast), 0, "the scrap route doesn't count");

    // Names that aren't Drains are ignored, and with none left every successor ranks the same.
    model.set_distribution(split, weighted(WeighBy::ProcessTime, Order::Smallest, Rank::Route, Some(vec![slow])));
    assert_eq!(model.ranking(split), Some(ranking(vec![vec![0, 1]], vec![None, None])));
    let run = ran(&model, 1);
    assert_eq!((mus_in(&run, slow), mus_in(&run, fast)), (90, 90), "they take turns");
}

#[test]
fn a_process_time_changed_during_a_run_ranks_the_successors_again_and_reset_restores_them() {
    let mut fork = fork(20);
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    let mut run = fork.model.build(0);
    run.init().unwrap();
    run.advance_to(secs(3600)).unwrap();
    assert_eq!((mus_in(&run, fork.a), mus_in(&run, fork.b)), (180, 0));
    // A slowed to 60 s makes the route through B, 32 s, the lightest.
    // B is often blocked behind C's 30 s, so A still takes what B can't.
    run.set_process_time(fork.a, secs(60)).unwrap();
    run.advance_to(secs(7200)).unwrap();
    let (a, b) = (mus_in(&run, fork.a) - 180, mus_in(&run, fork.b));
    assert!(b > a, "B takes most in the second hour: A {a}, B {b}");

    // Reset brings back A's 10 s and its ranking, so a second run is the first again.
    run.reset();
    run.init().unwrap();
    run.advance_to(secs(3600)).unwrap();
    assert_eq!((mus_in(&run, fork.a), mus_in(&run, fork.b)), (180, 0));
}

#[test]
fn the_heaviest_route_through_a_loop_passes_no_object_twice() {
    // Work then Check, which sends rework back to Work or passes on through Pack to Out. Split
    // chooses between Work and a Direct Station of 20 s.
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(20));
    let split = station(&mut model, 0);
    let work = station(&mut model, 5);
    let check = station(&mut model, 2);
    let pack = station(&mut model, 3);
    let direct = station(&mut model, 20);
    let out = model.add_drain();
    model.connect(source, split);
    model.connect(split, work);
    model.connect(split, direct);
    model.connect(work, check);
    model.connect(check, work);
    model.connect(check, pack);
    model.connect(pack, out);
    model.connect(direct, out);
    model.set_distribution(split, by_process_time(Order::Largest, Rank::Route));
    // Work, Check, Pack: 10 s, against Direct's 20 s.
    assert_eq!(model.ranking(split), Some(ranking(vec![vec![1], vec![0]], vec![Some(10_000), Some(20_000)])));
}

#[test]
fn the_same_seed_gives_the_same_weighted_run() {
    let mut fork = fork(6);
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    let first = ran(&fork.model, 4).state();
    let second = ran(&fork.model, 4).state();
    let counts = |state: &crabsim::RunState| state.objects.iter().map(|object| (object.statistics.mus_in, object.statistics.mus_out)).collect::<Vec<_>>();
    assert_eq!(counts(&first), counts(&second));
}

#[test]
fn a_checkpoint_replays_a_weighted_run_and_its_change_of_ranking_the_same_way() {
    // Going back restores a checkpoint, a clone of the run, and replays the logged changes.
    let mut fork = fork(6);
    fork.model.set_distribution(fork.split, by_process_time(Order::Smallest, Rank::Route));
    let mut run = fork.model.build(0);
    run.set_recording(crabsim::Recording::ALL);
    run.init().unwrap();
    run.advance_to(secs(1800)).unwrap();
    let mut checkpoint = run.clone();
    run.advance_to(secs(3600)).unwrap();
    run.set_process_time(fork.a, secs(60)).unwrap();
    let onward = run.advance_to(secs(7200)).unwrap();

    checkpoint.advance_to(secs(3600)).unwrap();
    checkpoint.set_process_time(fork.a, secs(60)).unwrap();
    let replayed = checkpoint.advance_to(secs(7200)).unwrap();
    assert_eq!(replayed, onward, "the same MUs to the same successors");
    assert_eq!(checkpoint.state(), run.state());
}
