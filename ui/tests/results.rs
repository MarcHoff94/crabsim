//! The Results tab: each Drain's throughput and lead time, every Station's shares of time, and
//! the likely bottleneck by the catalogue's own rule, from the run's full state at shown time. No
//! window.

mod common;

use common::Watched;
use crabsim::random_numbers::Distribution;
use crabsim::{demo, FailureProfile, ModelDescription, SessionOutput, SessionStatus, SimTime};
use crabsim_ui::view::{Results, ViewState, MIN_PERIODS};
use std::time::Duration;

#[test]
fn results_come_only_once_a_full_state_is_there_and_are_asked_for_while_the_tab_is_open() {
    let mut watched = Watched::demo();
    assert!(watched.view.results().is_none(), "nothing before Init");
    watched.session.start().unwrap();
    watched.at(600);
    watched.session.halt().unwrap();
    watched.frame();
    assert!(!watched.view.wants_state(watched.wall()), "nothing selected and the tab closed");
    watched.view.set_results_open(true);
    assert!(watched.view.wants_state(watched.wall()), "the tab open wants the state at shown time");
    watched.details();
    let results = watched.view.results().expect("a full state at shown time");
    assert_eq!(results.as_of, SimTime::from_secs(600));
}

#[test]
fn results_give_every_drain_and_station_and_the_likely_bottleneck_by_the_catalogues_rule() {
    let mut watched = Watched::demo();
    watched.view.set_results_open(true);
    watched.session.start().unwrap();
    watched.at(4 * 3_600);
    watched.session.halt().unwrap();
    watched.frame();
    watched.details();
    let results = watched.view.results().unwrap();

    assert_eq!(results.drains.len(), 1);
    let drain = &results.drains[0];
    assert_eq!(drain.name, "Drain");
    assert!(drain.mus_in > 0 && drain.throughput_per_hour.is_some() && drain.mean_lead_time.is_some());

    assert_eq!(results.stations.len(), 12, "every Station of the demo, in model order");
    assert_eq!(results.stations[0].name, "Station 1");
    for station in &results.stations {
        let total = station.busy + station.blocked + station.idle + station.failed + station.paused;
        assert!((total - 1.0).abs() < 1e-9, "{} adds up to {total}", station.name);
    }

    let likely = results.bottleneck.as_ref().expect("the demo has Stations");
    let most = results.stations.iter().map(|station| station.busy + station.failed + station.paused).fold(0.0, f64::max);
    assert!((likely.not_idle_share - most).abs() < 1e-12, "the Station busy or stopped most");
    assert!(results.stations.iter().any(|station| station.name == likely.instance));
    assert!(!likely.upstream.is_empty(), "with its neighbours upstream as evidence");
}

#[test]
fn results_say_whole_seconds_while_running_and_milliseconds_when_halted() {
    let model = demo::model();
    let mut view = ViewState::new(&model);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_millis(1_104_151)).unwrap();
    view.observe(&SessionOutput::State(run.state()), Duration::ZERO);
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    let results = view.results().unwrap();
    assert_eq!(view.results_time(&results), "00:18:24");
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::ZERO);
    assert_eq!(view.results_time(&results), "00:18:24.151");
    view.observe(&SessionOutput::Status(SessionStatus::ReachedEnd), Duration::ZERO);
    assert_eq!(view.results_time(&results), "00:18:24.151");
}

const HOUR: SimTime = SimTime::from_secs(3_600);

/// What the Results tab shows for `model` run headless to `at`, measuring in periods of `period`.
fn results_of(model: &ModelDescription, period: Option<SimTime>, at: SimTime) -> Results {
    let mut view = ViewState::new(model);
    let mut run = model.build(5);
    run.set_measuring_period(period);
    run.init().unwrap();
    run.advance_to(at).unwrap();
    view.observe(&SessionOutput::State(run.state()), Duration::ZERO);
    view.results().unwrap()
}

#[test]
fn intervals_come_once_five_measuring_periods_are_complete() {
    assert_eq!(MIN_PERIODS, 5);
    let model = demo::model();
    let early = results_of(&model, Some(HOUR), HOUR.times(4) + SimTime::from_secs(1_800));
    let periods = early.periods.expect("measuring in periods");
    assert_eq!((periods.complete, periods.length), (4, HOUR), "run longer: 4 of 5");
    assert!(early.drains.iter().all(|drain| drain.throughput_interval.is_none()));
    assert!(early.stations.iter().all(|station| station.intervals.is_none()));

    let fifth = results_of(&model, Some(HOUR), HOUR.times(5));
    assert_eq!(fifth.periods.unwrap().complete, MIN_PERIODS);
    assert!(fifth.drains[0].throughput_interval.is_some(), "from the fifth period on");
    assert!(fifth.stations.iter().all(|station| station.intervals.is_some()));

    let later = results_of(&model, Some(HOUR), HOUR.times(12));
    assert_eq!(later.periods.unwrap().complete, 12);
    let mut run = model.build(5);
    run.set_measuring_period(Some(HOUR));
    run.init().unwrap();
    run.advance_to(HOUR.times(12)).unwrap();
    let state = run.state();
    let figures = &state.periods.as_ref().unwrap().objects;
    let drain = &later.drains[0];
    let interval = drain.throughput_interval.expect("an interval from 12 periods");
    let spread = figures[drain.id as usize].mus_in_per_hour;
    assert_eq!((interval.mean, interval.half_width), (spread.mean().unwrap(), spread.half_width().unwrap()));
    for station in &later.stations {
        let intervals = station.intervals.expect("every Station has intervals");
        let figures = figures[station.id as usize];
        assert_eq!(intervals.busy.mean, figures.busy.mean().unwrap());
        assert_eq!(intervals.busy.half_width, figures.busy.half_width().unwrap());
        assert_eq!(intervals.stopped.half_width, figures.failed.half_width().unwrap(), "the demo has no pauses");
    }
    assert!(later.stations.iter().any(|station| station.intervals.unwrap().busy.half_width > 0.0));
}

#[test]
fn without_periods_the_tab_gives_no_intervals_and_asks_for_none() {
    let results = results_of(&demo::model(), None, HOUR.times(12));
    assert_eq!(results.periods, None);
    assert!(results.stations.iter().all(|station| station.intervals.is_none()));
    assert_eq!(results.too_close, None);
}

/// A Source every 10 s into two Stations in a row, taking `first` and `second` seconds, into a
/// Drain, without failures.
fn two_stations(first: u64, second: u64) -> ModelDescription {
    let no_failures = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(10));
    let a = model.add_station(SimTime::from_secs(first), no_failures.clone());
    let b = model.add_station(SimTime::from_secs(second), no_failures);
    let drain = model.add_drain();
    model.connect(source, a);
    model.connect(a, b);
    model.connect(b, drain);
    model.set_name(a, "A");
    model.set_name(b, "B");
    model
}

#[test]
fn two_stations_alike_are_too_close_to_call() {
    let results = results_of(&two_stations(9, 9), Some(HOUR), HOUR.times(6));
    assert_eq!(results.bottleneck.as_ref().unwrap().instance, "A", "a tie goes to the one listed first");
    assert_eq!(results.too_close.as_deref(), Some("B"), "the runner-up can't be told from it");
}

#[test]
fn a_clear_bottleneck_is_not_too_close_to_call_and_nothing_is_before_five_periods() {
    let results = results_of(&two_stations(9, 3), Some(HOUR), HOUR.times(6));
    assert_eq!(results.bottleneck.as_ref().unwrap().instance, "A");
    assert_eq!(results.too_close, None);
    let early = results_of(&two_stations(9, 9), Some(HOUR), HOUR.times(4));
    assert_eq!(early.too_close, None, "no intervals yet to tell");
}

#[test]
fn periods_short_against_the_longest_time_between_failures_are_noted() {
    // The demo's Stations fail with a mean repair time of 60 s at 90% availability, so a mean
    // time between failures of 540 s.
    let model = demo::model();
    assert_eq!(model.longest_failure_time(), Some(SimTime::from_secs(540)));
    let short = results_of(&model, Some(SimTime::from_secs(1_800)), HOUR);
    assert_eq!(short.short_against, Some(SimTime::from_secs(540)), "half an hour is less than 5 times 540 s");
    let long = results_of(&model, Some(HOUR), HOUR);
    assert_eq!(long.short_against, None);
    assert_eq!(two_stations(9, 9).longest_failure_time(), None);
    assert_eq!(results_of(&two_stations(9, 9), Some(SimTime::from_secs(60)), HOUR).short_against, None);
}
