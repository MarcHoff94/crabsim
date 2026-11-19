//! A Station with capacity in the view: the inspector's three parts of busy and the statistics window's
//! Full column, with a real session on a fake clock and no window (spec issue 110, PR 2).

mod common;

use common::{FakeClock, Watched};
use crabsim::random_numbers::Distribution;
use crabsim::{Capacity, FailureProfile, LiveSession, ModelDescription, Position, SimTime, StartMode};
use crabsim_ui::view::ViewState;

const OVEN: u32 = 1;

/// Stock (MUs 1 to 4) -> Oven (capacity 2, 10 s each, individually) -> Pack (15 s) -> Out, halted at
/// 30 s. The Oven is full 0-10, works while MU 2 can't leave 10-20, is blocked 20-25 with nothing
/// in work, and works while MU 3 can't leave from 25.
fn halted_at_30() -> Watched {
    let never = || FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut model = ModelDescription::new();
    let stock = model.add_buffer(Capacity::Unlimited);
    model.set_starting_mus(stock, &[1, 2, 3, 4]);
    let oven = model.add_station(SimTime::from_secs(10), never());
    model.set_station_capacity(oven, 2, StartMode::Individual);
    let pack = model.add_station(SimTime::from_secs(15), never());
    let out = model.add_drain();
    for (place, (id, name)) in [(stock, "Stock"), (oven, "Oven"), (pack, "Pack"), (out, "Out")].into_iter().enumerate() {
        model.set_name(id, name);
        model.set_position(id, Position { x: 4.0 * place as f64, y: 0.0, z: 0.0 });
    }
    for (from, to) in [(stock, oven), (oven, pack), (pack, out)] {
        model.connect(from, to);
    }
    let view = ViewState::new(&model);
    let clock = FakeClock::default();
    let session = LiveSession::new(move || model.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.session.start().unwrap();
    watched.at(30);
    watched.session.halt().unwrap();
    watched.frame();
    watched.view.select(Some(OVEN));
    watched.details();
    watched
}

#[test]
fn the_inspector_splits_a_station_with_capacitys_busy_time_into_three() {
    let watched = halted_at_30();
    let inspection = watched.view.inspection().unwrap();
    assert_eq!(inspection.state, "Busy", "MU 4 is in work while MU 3 waits: work outranks blocking");
    let rows = inspection.statistics_rows();
    let names: Vec<&str> = rows.iter().map(|(label, _)| *label).collect();
    assert_eq!(
        names,
        ["MUs in", "MUs out", "Busy", "  every place in work", "  with free places", "  partly blocked", "Blocked", "Idle", "Failed", "Paused", "Failures", "Pauses", "Availability"]
    );
    let value = |label: &str| rows.iter().find(|(name, _)| *name == label).unwrap().1.clone();
    assert_eq!(value("Busy"), "83.3 %");
    assert_eq!(value("  every place in work"), "33.3 %");
    assert_eq!(value("  with free places"), "0.0 %");
    assert_eq!(value("  partly blocked"), "50.0 %");
    assert_eq!(value("Blocked"), "16.7 %");
}

#[test]
fn results_show_a_full_column_and_rank_the_bottleneck_by_it() {
    let mut watched = halted_at_30();
    watched.view.set_statistics_open(true);
    watched.details();
    let results = watched.view.results().unwrap();
    assert!(results.with_capacity);
    let oven = results.stations.iter().find(|station| station.name == "Oven").unwrap();
    assert!((oven.busy_full - 1.0 / 3.0).abs() < 1e-9, "{}", oven.busy_full);
    let pack = results.stations.iter().find(|station| station.name == "Pack").unwrap();
    assert_eq!(pack.busy_full, pack.busy);
    // Pack works from 10, two thirds of the time, against the Oven's third full.
    let likely = results.bottleneck.unwrap();
    assert_eq!(likely.instance, "Pack");
    assert_eq!(watched.view.live_figure("working_partly_blocked_share").unwrap()[OVEN as usize], Some(0.5));
}
