//! A Buffer in the view: its Holding state, its MUs as a count and what the inspector says, with a
//! real session on a fake clock and no window (spec issue 110).

mod common;

use common::{FakeClock, Watched};
use crabsim::random_numbers::Distribution;
use crabsim::{Capacity, FailureProfile, LiveSession, ModelDescription, Position, SimTime};
use crabsim_ui::camera::Camera;
use crabsim_ui::view::{Point, Shade, ViewState, MU_MIN_PIXELS_PER_METRE};

const SOURCE: u32 = 0;
const STORE: u32 = 1;
const SLOW: u32 = 2;

/// In (one MU every 2 s) -> Store (a Buffer, MUs dwell 5 s) -> Slow (10 s) -> Out, halted at
/// `secs`. With a capacity of 3: MUs come in at 2, 4, 6 and 8, the first goes on at 7, and from 9
/// the second may leave but Slow works until 17.
fn halted_at(secs: u64, capacity: Capacity) -> Watched {
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(2));
    let store = model.add_buffer(capacity);
    model.set_dwelling_time(store, SimTime::from_secs(5));
    let slow = model.add_station(SimTime::from_secs(10), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0));
    let out = model.add_drain();
    for (place, (id, name)) in [(source, "In"), (store, "Store"), (slow, "Slow"), (out, "Out")].into_iter().enumerate() {
        model.set_name(id, name);
        model.set_position(id, Position { x: 4.0 * place as f64, y: 0.0, z: 0.0 });
    }
    for (from, to) in [(source, store), (store, slow), (slow, out)] {
        model.connect(from, to);
    }
    let view = ViewState::new(&model);
    let clock = FakeClock::default();
    let session = LiveSession::new(move || model.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.session.start().unwrap();
    watched.at(secs);
    watched.session.halt().unwrap();
    watched.frame();
    watched.view.select(Some(STORE));
    watched
}

fn close() -> Camera {
    Camera { centre: Point { x: 0.0, y: 0.0 }, pixels_per_metre: MU_MIN_PIXELS_PER_METRE * 2.0 }
}

#[test]
fn a_buffer_is_idle_when_empty_holding_with_mus_and_blocked_when_one_cant_leave() {
    let empty = halted_at(1, Capacity::Limited(3));
    assert_eq!(empty.view.shade(STORE), Shade::Idle);
    assert_eq!(empty.view.inspection().unwrap().state, "Idle");

    let holding = halted_at(3, Capacity::Limited(3));
    assert_eq!(holding.view.shade(STORE), Shade::Holding);
    assert_eq!(holding.view.hover_text(STORE), "Store: Holding");
    assert_eq!(holding.view.inspection().unwrap().state, "Holding");
    assert_eq!(holding.view.shade(SOURCE), Shade::Busy, "only a Buffer is holding");

    let blocked = halted_at(12, Capacity::Limited(3));
    assert_eq!(blocked.view.shade(STORE), Shade::Blocked);
    let inspection = blocked.view.inspection().unwrap();
    assert_eq!(inspection.state, "Blocked");
    assert_eq!(inspection.blocked.as_deref(), Some("Holding 3 MUs: Slow is busy"));
}

#[test]
fn a_buffers_mus_are_a_count_even_where_other_mus_are_drawn() {
    let watched = halted_at(12, Capacity::Limited(3));
    let holders: Vec<u32> = watched.view.mu_markers(&close()).iter().map(|marker| marker.holder).collect();
    assert_eq!(holders, [SOURCE, SLOW], "the Source's and the Station's MU, and none of the Buffer's three");
    assert_eq!(watched.view.counts(&close()), [(STORE, 3)]);
    // Too far out to read a count, it shows none either.
    let far = Camera { centre: Point { x: 0.0, y: 0.0 }, pixels_per_metre: 1.0 };
    assert!(watched.view.counts(&far).is_empty());
}

#[test]
fn the_inspector_shows_how_full_a_buffer_was_instead_of_work_and_stops() {
    let labels = |capacity: Capacity| -> Vec<(&'static str, String)> {
        let mut watched = halted_at(12, capacity);
        watched.details();
        watched.view.inspection().unwrap().statistics_rows()
    };
    let limited = labels(Capacity::Limited(3));
    let names: Vec<&str> = limited.iter().map(|(label, _)| *label).collect();
    assert_eq!(names, ["MUs in", "MUs out", "Holding", "Blocked", "Empty", "Most MUs", "Fewest MUs", "Mean MUs", "Median MUs", "Full"]);
    let value = |label: &str| limited.iter().find(|(name, _)| *name == label).unwrap().1.clone();
    assert_eq!((value("MUs in"), value("MUs out"), value("Most MUs")), ("4".to_string(), "1".to_string(), "3".to_string()));
    // Empty until 2, blocked from 9, and full from 6 to 7 and from 8: of 12 s.
    assert_eq!((value("Empty"), value("Blocked"), value("Full")), ("16.7 %".to_string(), "25.0 %".to_string(), "41.7 %".to_string()));
    let unlimited: Vec<&str> = labels(Capacity::Unlimited).iter().map(|(label, _)| *label).collect();
    assert_eq!(unlimited, ["MUs in", "MUs out", "Holding", "Blocked", "Empty", "Most MUs", "Fewest MUs", "Mean MUs", "Median MUs"], "one with no limit is never full");
}

#[test]
fn the_inspector_and_results_say_how_full_a_buffer_was_with_its_histogram() {
    // Empty to 2, 1 MU to 4, 2 to 6, 3 to 7, 2 to 8 and 3 to 12: half of the 12 s is reached at 2.
    let mut watched = halted_at(12, Capacity::Limited(3));
    watched.details();
    let rows = watched.view.inspection().unwrap().statistics_rows();
    let value = |label: &str| rows.iter().find(|(name, _)| *name == label).unwrap().1.clone();
    assert_eq!((value("Fewest MUs"), value("Median MUs")), ("0".to_string(), "2".to_string()));
    let details = watched.view.inspection().unwrap().details.clone().unwrap();
    let bars = crabsim_ui::view::fill_bars(details.fill.as_ref().unwrap());
    let shown: Vec<(&str, String)> = bars.iter().map(|bar| (bar.mus.as_str(), format!("{:.3}", bar.share))).collect();
    assert_eq!(shown, [("0", "0.167".to_string()), ("1", "0.167".to_string()), ("2", "0.250".to_string()), ("3", "0.417".to_string())]);

    watched.view.set_statistics_open(true);
    watched.details();
    let results = watched.view.results().unwrap();
    assert_eq!(results.buffers.len(), 1);
    let store = &results.buffers[0];
    assert_eq!((store.name.as_str(), store.median.as_str(), store.most), ("Store", "2", 3));
    assert!((store.full.unwrap() - 5.0 / 12.0).abs() < 1e-9);
    assert!((store.mean.unwrap() - 23.0 / 12.0).abs() < 1e-9, "{:?}", store.mean);
}
