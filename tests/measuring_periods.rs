//! Measuring periods: a run cut into stretches of simulated time whose figures each count as one
//! sample, kept as running sums so memory doesn't grow with the number of periods. Tested at the
//! run's interface against a headless run read at each period's end.

use crabsim::{demo, ObjectStatistics, RunState, SimTime, SimulationRun};

const HOUR: SimTime = SimTime::from_secs(3_600);

fn hours(h: u64) -> SimTime {
    HOUR.times(h)
}

fn demo_run(period: Option<SimTime>) -> SimulationRun {
    let mut run = demo::model().build(7);
    run.set_measuring_period(period);
    run.init().unwrap();
    run
}

/// Every object's statistics in a run without periods, read at each of `ends`.
fn statistics_at(ends: &[SimTime]) -> Vec<Vec<ObjectStatistics>> {
    let mut run = demo_run(None);
    ends.iter()
        .map(|&end| {
            run.advance_to(end).unwrap();
            run.state().objects.iter().map(|object| object.statistics).collect()
        })
        .collect()
}

fn mean_and_variance(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    (mean, values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (n - 1.0))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn each_period_is_the_difference_between_a_headless_runs_statistics_at_its_ends() {
    let mut run = demo_run(Some(HOUR));
    run.advance_to(hours(10) + SimTime::from_secs(1_800)).unwrap();
    let periods = run.state().periods.expect("measuring in periods");
    assert_eq!(periods.length, HOUR);
    assert_eq!(periods.complete, 10, "the half hour in progress isn't a sample yet");

    let ends: Vec<SimTime> = (0..=10).map(hours).collect();
    let at_ends = statistics_at(&ends);
    const LENGTH: f64 = 3_600_000.0;
    let mut stations_checked = 0;
    for (id, figures) in periods.objects.iter().enumerate() {
        let per_period = |figure: fn(&ObjectStatistics) -> f64| -> Vec<f64> {
            at_ends.windows(2).map(|pair| figure(&pair[1][id]) - figure(&pair[0][id])).collect()
        };
        let busy = per_period(|s| s.time_busy.as_millis() as f64 / LENGTH);
        let blocked = per_period(|s| s.time_blocked.as_millis() as f64 / LENGTH);
        let failed = per_period(|s| s.time_failed.as_millis() as f64 / LENGTH);
        let not_idle = per_period(|s| (s.time_busy + s.time_failed + s.time_paused).as_millis() as f64 / LENGTH);
        let mus_in = per_period(|s| s.mus_in as f64);
        for (spread, values) in [
            (figures.busy, &busy),
            (figures.blocked, &blocked),
            (figures.failed, &failed),
            (figures.not_idle, &not_idle),
            (figures.mus_in_per_hour, &mus_in),
        ] {
            let (mean, variance) = mean_and_variance(values);
            assert_eq!(spread.count(), 10);
            assert!(close(spread.mean().unwrap(), mean), "object {id}: {:?} against {mean}", spread.mean());
            assert!(close(spread.variance().unwrap(), variance), "object {id}: {:?} against {variance}", spread.variance());
        }
        if busy.iter().any(|&share| share != busy[0]) {
            stations_checked += 1;
        }
    }
    assert!(stations_checked >= 10, "the demo's Stations vary from period to period, so the spread is tested");
}

#[test]
fn the_mean_over_complete_periods_is_the_runs_own_figure_over_them() {
    let mut run = demo_run(Some(HOUR));
    // MUs counted at Init itself come before the first period.
    let at_init = run.state();
    run.advance_to(hours(10)).unwrap();
    let state = run.state();
    let periods = state.periods.as_ref().unwrap();
    assert_eq!(periods.complete, 10, "a period ends after every event at its end time, so a run halted there counts it");
    for ((object, figures), initial) in state.objects.iter().zip(&periods.objects).zip(&at_init.objects) {
        let statistics = object.statistics;
        let share = |time: SimTime| time.as_millis() as f64 / hours(10).as_millis() as f64;
        assert!(close(figures.busy.mean().unwrap(), share(statistics.time_busy)));
        assert!(close(figures.idle.mean().unwrap(), share(statistics.time_idle)));
        assert!(close(figures.mus_in_per_hour.mean().unwrap(), (statistics.mus_in - initial.statistics.mus_in) as f64 / 10.0));
    }
}

/// The run's state without its periods, to compare with a run that measures none.
fn without_periods(mut state: RunState) -> RunState {
    state.periods = None;
    state
}

#[test]
fn measuring_changes_nothing_else_in_the_run() {
    let mut measured = demo_run(Some(SimTime::from_secs(600)));
    let mut plain = demo_run(None);
    measured.advance_to(hours(30)).unwrap();
    plain.advance_to(hours(30)).unwrap();
    assert_eq!(measured.events_processed(), plain.events_processed());
    assert_eq!(without_periods(measured.state()), plain.state());
    assert_eq!(plain.state().periods, None);
}

#[test]
fn however_a_run_is_advanced_its_periods_come_out_the_same() {
    let mut whole = demo_run(Some(HOUR));
    whole.advance_to(hours(6)).unwrap();
    let mut split = demo_run(Some(HOUR));
    while split.time() < hours(6) {
        split.advance_bounded(hours(6), 37).unwrap();
        split.step().unwrap();
        split.advance_events(5).unwrap();
    }
    // Stepping can pass 6 h, so both go on to 7 h before comparing.
    whole.advance_to(hours(7)).unwrap();
    split.advance_to(hours(7)).unwrap();
    assert_eq!(split.state(), whole.state());
}

#[test]
fn clearing_statistics_starts_periods_afresh_from_then() {
    let mut run = demo_run(Some(HOUR));
    run.advance_to(hours(2) + SimTime::from_secs(1_800)).unwrap();
    run.clear_statistics();
    assert_eq!(run.state().periods.unwrap().complete, 0);
    run.advance_to(hours(5)).unwrap();
    assert_eq!(run.state().periods.unwrap().complete, 2, "periods end at 3:30 and 4:30");
}

#[test]
fn reset_keeps_the_setting_and_drops_the_periods() {
    let mut run = demo_run(Some(HOUR));
    run.advance_to(hours(3)).unwrap();
    run.reset();
    assert_eq!(run.measuring_period(), Some(HOUR));
    assert_eq!(run.state().periods, None, "nothing is measured before Init");
    run.init().unwrap();
    assert_eq!(run.state().periods.unwrap().complete, 0);
    run.advance_to(hours(3)).unwrap();
    let again = run.state();
    let mut fresh = demo_run(Some(HOUR));
    fresh.advance_to(hours(3)).unwrap();
    assert_eq!(again, fresh.state());
}

#[test]
fn no_length_or_zero_measures_no_periods() {
    for period in [None, Some(SimTime::ZERO)] {
        let mut run = demo_run(period);
        run.advance_to(hours(2)).unwrap();
        assert_eq!(run.state().periods, None);
        assert_eq!(run.measuring_period(), None);
    }
}

#[test]
fn a_long_gap_with_no_events_completes_every_period_in_it() {
    let mut description = crabsim::ModelDescription::new();
    let source = description.add_source(SimTime::from_secs(10 * 3_600));
    let drain = description.add_drain();
    description.connect(source, drain);
    let mut run = description.build(0);
    run.set_measuring_period(Some(HOUR));
    run.init().unwrap();
    run.advance_to(hours(25)).unwrap();
    let periods = run.state().periods.unwrap();
    assert_eq!(periods.complete, 25);
    let drain_figures = periods.objects[drain as usize].mus_in_per_hour;
    assert!(close(drain_figures.mean().unwrap(), 2.0 / 25.0), "MUs at 10 h and 20 h, in two of 25 periods");
}
