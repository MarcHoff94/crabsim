//! The vocabulary of figures read from a run's state, and deterministic runs (ADR 0030).

use crabsim::figures::{state_name, Figure, ScopeKind};
use crabsim::{demo, ObjectKind, RunState, SimTime};

fn run_to(model: &crabsim::ModelDescription, seed: u64, deterministic: bool, until: SimTime) -> RunState {
    let mut run = if deterministic { model.build_deterministic(seed) } else { model.build(seed) };
    run.init().unwrap();
    run.advance_to(until).unwrap();
    run.state()
}

#[test]
fn a_deterministic_run_has_no_failures_and_is_the_same_for_every_seed() {
    let model = demo::model();
    let day = SimTime::from_secs(86_400);
    let random = run_to(&model, 1, false, day);
    assert!(random.objects.iter().any(|object| object.statistics.failures > 0), "the demo fails when random");
    let first = run_to(&model, 1, true, day);
    let second = run_to(&model, 99, true, day);
    assert!(first.objects.iter().all(|object| object.statistics.failures == 0 && object.statistics.time_failed == SimTime::ZERO));
    assert_eq!(first.objects.iter().map(|object| object.statistics).collect::<Vec<_>>(), second.objects.iter().map(|object| object.statistics).collect::<Vec<_>>());
    // Building deterministically leaves the description's own failures as they were.
    assert_eq!(run_to(&model, 1, false, day).objects.iter().map(|object| object.statistics).collect::<Vec<_>>(), random.objects.iter().map(|object| object.statistics).collect::<Vec<_>>());
}

#[test]
fn a_stretch_from_the_start_reads_the_same_as_since_start() {
    // Before Init every statistic is zero, so the stretch from it is the whole run. One from t=0
    // after Init would leave out what Init itself counts, such as a Source's first MU.
    for model in [demo::model(), demo::buffered_line(), demo::conveyor_line(), demo::cross_line()] {
        let start = model.build(3).state();
        let end = run_to(&model, 3, false, SimTime::from_secs(8 * 3600));
        for (earlier, later) in start.objects.iter().zip(&end.objects) {
            for figure in Figure::ALL.into_iter().filter(|figure| figure.has_scope(ScopeKind::Last) && figure.applies_to(later.kind)) {
                let since = figure.value(later, end.time);
                let over = figure.value_over(earlier, later);
                match (figure, since, over) {
                    // Since start, throughput counts from the first MU's exit, a stretch over its whole length.
                    (Figure::ThroughputPerHour, _, _) => assert!(over.is_some() || later.statistics.mus_in == 0),
                    (_, Some(since), Some(over)) => assert!((since - over).abs() < 1e-9, "{figure:?} of object {}: {since} since start, {over} over the stretch", later.id),
                    (_, None, None) => {}
                    _ => panic!("{figure:?} of object {}: {since:?} since start, {over:?} over the stretch", later.id),
                }
            }
        }
    }
}

#[test]
fn the_last_hour_counts_only_what_happened_in_it() {
    let model = demo::model();
    let earlier = run_to(&model, 5, true, SimTime::from_secs(7 * 3600));
    let later = run_to(&model, 5, true, SimTime::from_secs(8 * 3600));
    let drain = later.objects.iter().position(|object| object.kind == ObjectKind::Drain).unwrap();
    let in_the_hour = (later.objects[drain].statistics.mus_in - earlier.objects[drain].statistics.mus_in) as f64;
    assert_eq!(Figure::MusIn.value_over(&earlier.objects[drain], &later.objects[drain]), Some(in_the_hour));
    assert_eq!(Figure::ThroughputPerHour.value_over(&earlier.objects[drain], &later.objects[drain]), Some(in_the_hour));
    assert_eq!(Figure::ThroughputPerHour.model_value_over(&earlier.objects, &later.objects), Some(in_the_hour));
    // The most MUs held can't be read over a stretch.
    assert_eq!(Figure::MaxMus.value_over(&earlier.objects[drain], &later.objects[drain]), None);
}

#[test]
fn limits_read_the_extremes_the_statistics_keep() {
    let state = run_to(&demo::buffered_line(), 2, false, SimTime::from_secs(8 * 3600));
    for object in &state.objects {
        let most = Figure::Mus.extreme(object, true);
        assert_eq!(Figure::Mus.extreme(object, false), None, "the fewest held is min_mus since start");
        match object.kind {
            // A Source keeps no MU statistics, and a Drain holds no MU.
            ObjectKind::Source => assert_eq!(most, None),
            ObjectKind::Drain => {
                assert_eq!(most, None);
                assert_eq!(Figure::LeadTime.extreme(object, true), object.statistics.drain.unwrap().lead_time_max.map(SimTime::as_secs_f64));
            }
            _ => assert_eq!(most, Some(object.statistics.max_contents as f64)),
        }
        assert!(["busy", "holding", "blocked", "idle", "receiving", "failed", "stopped", "paused"].contains(&state_name(object)));
    }
}
