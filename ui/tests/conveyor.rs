//! A conveyor in the view: its MUs drawn at their own size where their motion puts them at shown
//! time, the direction it goes, and an object receiving from it, with a real session on a fake
//! clock and no window (spec issue 133, PR 2).

mod common;

use common::{FakeClock, Watched};
use crabsim::{LiveSession, ModelDescription, MuKind, Position, SimTime};
use crabsim_ui::attributes::{summary, Form};
use crabsim_ui::camera::Camera;
use crabsim_ui::view::{Point, Shade, ViewState, MU_MIN_PIXELS_PER_METRE};
use serde_json::json;

const SOURCE: u32 = 0;
const BELT: u32 = 1;
const OUT: u32 = 2;

/// In (an MU every 10 s, 0.8 by 0.4 m) -> Belt (10 m at 0.5 m/s, its centre at x = 10, so its
/// start at x = 5) -> Out, halted at `millis`. MU 1 enters at 10 s with its front 0.4 m in.
fn halted_at(millis: u64) -> Watched {
    let mut model = ModelDescription::new();
    let tote = model.add_mu_kind(MuKind { length: 0.8, width: 0.4, booking_point: 0.5 });
    let source = model.add_source(SimTime::from_secs(10));
    model.set_mu_kind(source, tote);
    let belt = model.add_conveyor(10.0, 0.6, 0.5);
    let out = model.add_drain();
    for (id, name, x) in [(source, "In", 2.0), (belt, "Belt", 10.0), (out, "Out", 17.0)] {
        model.set_name(id, name);
        model.set_position(id, Position { x, y: 0.0, z: 0.0 });
    }
    model.connect(source, belt);
    model.connect(belt, out);
    let view = ViewState::new(&model);
    let clock = FakeClock::default();
    let session = LiveSession::new(move || model.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.session.start().unwrap();
    watched.session.sync().unwrap();
    watched.clock.set(std::time::Duration::from_millis(millis));
    watched.frame();
    watched.session.halt().unwrap();
    watched.frame();
    watched
}

fn close() -> Camera {
    Camera { centre: Point { x: 10.0, y: 0.0 }, pixels_per_metre: MU_MIN_PIXELS_PER_METRE * 2.0 }
}

#[test]
fn an_mu_on_a_conveyor_is_drawn_at_its_size_where_it_is_at_shown_time() {
    let watched = halted_at(15_000);

    let markers: Vec<_> = watched.view.mu_markers(&close()).into_iter().filter(|marker| marker.holder == BELT).collect();
    assert_eq!(markers.len(), 1);
    let marker = markers[0];
    assert_eq!((marker.mu, marker.holder, marker.length, marker.width), (1, BELT, 0.8, 0.4));
    // Its front 0.4 m in at 10 s, then 2.5 m on: 2.9 m from the start, its centre 0.4 m behind.
    assert!((marker.at.x - 7.5).abs() < 1e-3 && marker.at.y.abs() < 1e-9, "{marker:?}");
    assert_eq!(watched.view.shade(BELT), Shade::Busy);
    assert_eq!(watched.view.inspection_of(BELT).unwrap().state, "Busy");
}

#[test]
fn a_conveyor_is_empty_until_an_mu_enters_and_its_arrow_points_from_start_to_end() {
    let watched = halted_at(5_000);
    assert_eq!(watched.view.inspection_of(BELT).unwrap().state, "Empty");
    let [(start, end)] = watched.view.belt_directions()[..] else { panic!("one conveyor") };
    assert!(start.x < end.x, "it goes from its start at x = 5 towards its end at x = 15");
}

#[test]
fn what_receives_from_a_conveyor_has_its_own_colour() {
    // The front reaches the end at 29.4 s, its booking point at 30 s.
    let watched = halted_at(29_700);
    assert_eq!(watched.view.shade(OUT), Shade::Receiving);
    assert_eq!(watched.view.hover_text(OUT), "Out: Receiving");
    assert_ne!(crabsim_ui::view::rgb(Shade::Receiving), crabsim_ui::view::rgb(Shade::Idle));
    let after = halted_at(30_500);
    assert_eq!(after.view.shade(OUT), Shade::Idle);
    let _ = SOURCE;
}

#[test]
fn a_conveyors_path_speed_and_gap_are_fields() {
    let mut path = Form::new("path", Some(&json!([{ "straight": 4.0 }, { "arc": { "radius": 1.5, "angle": 90.0 } }, { "straight": { "length": 2.5, "rise": 0.5 } }])));
    assert_eq!(path.parts[0].text, "4, arc 1.5 90, 2.5 rise 0.5");
    assert_eq!(summary(&json!([{ "straight": 4.0 }, { "helix": { "radius": 1.2, "turns": 2, "rise": 3 } }])), "4, helix 1.2 2 rise 3");
    path.parts[0].text = "12, transition 1 to -2".into();
    assert_eq!(path.value(), Ok(json!([{ "straight": 12.0 }, { "transition": { "length": 1.0, "to_radius": -2.0 } }])));
    path.parts[0].text = "12, arc 2".into();
    assert!(path.value().unwrap_err().contains("segment 2 (`arc 2`)"), "{:?}", path.value());

    let mut speed = Form::new("speed", Some(&json!(0.5)));
    speed.parts[0].text = "0".into();
    assert!(speed.value().unwrap_err().contains("above 0"));
    let mut gap = Form::new("gap", Some(&json!(0.0)));
    gap.parts[0].text = "0.1".into();
    assert_eq!(gap.value(), Ok(json!(0.1)));
}

/// In (an MU every 10 s, 0.8 by 0.4 m) -> Belt (10 m at 0.5 m/s) -> Mill, a Station working 100 s
/// on each, halted at `millis`. MU 1 enters the Mill at 30 s.
fn into_a_station_at(millis: u64) -> Watched {
    let mut model = ModelDescription::new();
    let tote = model.add_mu_kind(MuKind { length: 0.8, width: 0.4, booking_point: 0.5 });
    let source = model.add_source(SimTime::from_secs(10));
    model.set_mu_kind(source, tote);
    let belt = model.add_conveyor(10.0, 0.6, 0.5);
    let mill = model.add_station(SimTime::from_secs(100), crabsim::FailureProfile::new(1.0, crabsim::random_numbers::Distribution::CONSTANT { value: 0.0 }, 0));
    for (id, name, x) in [(source, "In", 2.0), (belt, "Belt", 10.0), (mill, "Mill", 17.0)] {
        model.set_name(id, name);
        model.set_position(id, Position { x, y: 0.0, z: 0.0 });
    }
    model.connect(source, belt);
    model.connect(belt, mill);
    let view = ViewState::new(&model);
    let clock = FakeClock::default();
    let session = LiveSession::new(move || model.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.session.start().unwrap();
    watched.session.sync().unwrap();
    watched.clock.set(std::time::Duration::from_millis(millis));
    watched.frame();
    watched.session.halt().unwrap();
    watched.frame();
    watched
}

#[test]
fn an_mu_keeps_its_shape_from_a_conveyor_into_a_station_and_after_a_full_state() {
    let size = |watched: &Watched, holder: u32| -> (f64, f64) {
        let markers = watched.view.mu_markers(&close());
        let marker = markers.iter().find(|marker| marker.mu == 1).unwrap_or_else(|| panic!("MU 1 in {markers:?}"));
        assert_eq!(marker.holder, holder);
        (marker.length, marker.width)
    };
    let on_belt = into_a_station_at(20_000);
    assert_eq!(size(&on_belt, BELT), (0.8, 0.4));
    let mut in_mill = into_a_station_at(40_000);
    assert_eq!(size(&in_mill, 2), (0.8, 0.4), "the same rectangle in the Station");
    assert_eq!(size(&in_mill, 2), size(&on_belt, BELT));
    // A full state, as after going back, knows each MU's kind too.
    in_mill.details();
    assert_eq!(size(&in_mill, 2), (0.8, 0.4));
    // The Source's next MU is of its kind as well.
    let source_mu = in_mill.view.mu_markers(&close()).into_iter().find(|marker| marker.holder == SOURCE).unwrap();
    assert_eq!((source_mu.length, source_mu.width), (0.8, 0.4));
}

#[test]
fn connections_meet_a_conveyors_start_and_leave_from_its_end() {
    let watched = halted_at(0);

    let connections = watched.view.connections();
    let (into, out_of) = (&connections[0], &connections[1]);
    // The belt is 10 m long with its centre at x = 10: start at 5, end at 15.
    assert!((into.end.x - 5.0).abs() < 1e-9 && into.end.y.abs() < 1e-9, "{into:?}");
    assert!((out_of.start.x - 15.0).abs() < 1e-9 && out_of.start.y.abs() < 1e-9, "{out_of:?}");
}

#[test]
fn the_object_window_and_the_statistics_window_show_a_conveyors_figures() {
    // MUs enter at 10, 20, 30, 40 and 50 s, and each crosses in 20 s, booking point to booking point.
    let mut watched = halted_at(55_000);
    watched.view.select(Some(BELT));
    watched.details();

    let inspection = watched.view.inspection().unwrap();
    let rows = inspection.statistics_rows();
    let value = |label: &str| rows.iter().find(|(name, _)| *name == label).unwrap_or_else(|| panic!("no {label} in {rows:?}")).1.clone();
    assert_eq!(value("Mean crossing"), "00:00:20");
    assert_eq!(value("  partly blocked"), "0.0 %");
    let parts = ["  start taken", "  room at its start", "  partly blocked"].map(|label| value(label));
    assert!(parts.iter().all(|part| part.ends_with('%')), "{parts:?}");
    let details = inspection.details.clone().unwrap();
    assert!(!crabsim_ui::view::fill_bars(details.fill.as_ref().unwrap()).is_empty(), "its fill histogram, as a Buffer's");
    let figures: Vec<&str> = inspection.key_figures().iter().map(|(name, _)| *name).collect();
    assert_eq!(figures, ["Busy, full", "Blocked", "Crossing"]);

    watched.view.set_statistics_open(true);
    watched.details();
    let results = watched.view.results().unwrap();
    assert_eq!(results.conveyors.len(), 1);
    let belt = &results.conveyors[0];
    assert_eq!((belt.id, belt.name.as_str(), belt.mean_crossing), (BELT, "Belt", Some(SimTime::from_secs(20))));
    assert!(belt.busy > 0.8 && belt.busy_full < belt.busy, "{belt:?}");
    assert_eq!(results.bottleneck.as_ref().map(|likely| likely.instance.as_str()), Some("Belt"), "a Conveyor is in the likely-bottleneck rule");
}

#[test]
fn a_conveyors_direction_and_limits_are_fields() {
    let mut direction = Form::new("direction", Some(&json!("backward")));
    assert_eq!(direction.parts[0].text, "backward");
    assert_eq!(summary(&json!("backward")), "backward");
    direction.parts[0].text = "forward".into();
    assert_eq!(direction.value(), Ok(json!("forward")));
    let mut least = Form::new("min_radius", None);
    assert_eq!(least.value(), Ok(serde_json::Value::Null), "unset, only half its width counts");
    least.parts[0].text = "0".into();
    assert!(least.value().unwrap_err().contains("above 0"));
    let mut steepest = Form::new("max_incline", Some(&json!(30.0)));
    assert_eq!(steepest.parts[0].text, "30");
    steepest.parts[0].text = "90".into();
    assert!(steepest.value().unwrap_err().contains("below 90"));
}
