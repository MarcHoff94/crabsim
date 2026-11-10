//! The Results tab: each Drain's throughput and lead time, every Station's shares of time, and
//! the likely bottleneck by the catalogue's own rule, from the run's full state at shown time. No
//! window.

mod common;

use common::Watched;
use crabsim::SimTime;

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
