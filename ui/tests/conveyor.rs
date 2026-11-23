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
    // About one arrow every 4 m along its 10 m.
    let arrows = watched.view.belt_directions();
    assert_eq!(arrows.len(), 3);
    assert!(arrows.iter().all(|(start, end)| start.x < end.x), "it goes from its start at x = 5 towards its end at x = 15: {arrows:?}");
    assert!(arrows.windows(2).all(|pair| pair[0].0.x < pair[1].0.x), "spread along it: {arrows:?}");
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
    assert_eq!(path.path.as_ref().unwrap().text, "4, arc 1.5 90, 2.5 rise 0.5");
    assert_eq!(summary(&json!([{ "straight": 4.0 }, { "helix": { "radius": 1.2, "turns": 2, "rise": 3 } }])), "4, helix 1.2 2 rise 3");
    let rows = path.path.as_mut().unwrap();
    rows.text = "12, transition 1 to -2".into();
    rows.read_text();
    assert_eq!(path.value(), Ok(json!([{ "straight": 12.0 }, { "transition": { "length": 1.0, "to_radius": -2.0 } }])));

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

// ---------- Paths (spec issue 141, PR 2) ----------

/// In at (-2, 0) -> Belt, starting at (0, 0): 2 m straight along x, then a quarter circle of
/// radius 2 to the left, rising 0.5 m, ending at (4, 2) heading up the y axis, at 0.5 m/s -> Out,
/// 1 m long, placed `out` (its centre and height). An MU, 0.6 m long, every 10 s.
fn curved_text(out: (f64, f64, f64)) -> String {
    serde_json::json!({ "format_version": 2,
        "instances": [
            { "name": "In", "class": "Source", "position": { "x": -2, "y": 0 }, "attributes": { "interval": "10" } },
            { "name": "Belt", "class": "Conveyor", "position": { "x": 0, "y": 0 },
              "attributes": { "path": [{ "straight": 2 }, { "arc": { "radius": 2, "angle": 90, "rise": 0.5 } }] } },
            { "name": "Out", "class": "Drain", "position": { "x": out.0, "y": out.1, "z": out.2 }, "attributes": { "dimensions": { "length": 1, "width": 1, "height": 1, "rotation_degrees": 90 } } } ],
        "connections": [
            { "from": "In", "to": "Belt", "exit": 1, "entrance": 1 },
            { "from": "Belt", "to": "Out", "exit": 1, "entrance": 1 } ] })
    .to_string()
}

/// The curved line, halted at `millis`, its conveyor drawn along its path.
fn curved_at(millis: u64, out: (f64, f64, f64)) -> Watched {
    let model = crabsim_model::load(&curved_text(out)).unwrap();
    let description = model.to_description();
    let mut view = ViewState::new(&description);
    view.set_shapes(crabsim_ui::workspace::shapes_of(&model));
    let clock = FakeClock::default();
    let session = LiveSession::new(move || description.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.session.start().unwrap();
    watched.session.sync().unwrap();
    watched.clock.set(std::time::Duration::from_millis(millis));
    watched.frame();
    watched.session.halt().unwrap();
    watched.frame();
    watched
}

fn near(a: Point, b: Point, within: f64) -> bool {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() <= within
}

#[test]
fn an_mu_on_a_curve_is_drawn_where_the_path_puts_it_turned_with_it() {
    // It enters at 10 s with its centre on the start, so its centre is halfway round the arc,
    // whose length in 3D is pi rising 0.5, 2 s a metre later, heading 45°.
    let arc = std::f64::consts::PI.hypot(0.5);
    let millis = 10_000 + (2_000.0 * (2.0 + arc / 2.0)).round() as u64;
    let watched = curved_at(millis, (4.0, 2.5, 0.5));
    let markers: Vec<_> = watched.view.mu_markers(&close()).into_iter().filter(|marker| marker.holder == BELT).collect();
    let [marker] = markers[..] else { panic!("one MU on the conveyor: {markers:?}") };
    let half = std::f64::consts::FRAC_1_SQRT_2 * 2.0;
    assert!(near(marker.at, Point { x: 2.0 + half, y: 2.0 - half }, 0.02), "{marker:?}");
    assert!((marker.rotation_degrees - 45.0).abs() < 1.0, "{marker:?}");
}

#[test]
fn a_curved_conveyor_is_found_on_its_band_connects_at_its_ends_and_points_its_way() {
    let watched = curved_at(1_000, (4.0, 2.5, 0.5));
    let view = &watched.view;
    // On the arc, halfway round, and inside the corner, off it.
    let half = std::f64::consts::FRAC_1_SQRT_2 * 2.0;
    assert_eq!(view.object_at(Point { x: 2.0 + half, y: 2.0 - half }), Some(BELT));
    assert_eq!(view.object_at(Point { x: 3.0, y: 1.2 }), None);
    // Halfway along its straight, between two points of its centreline 2 m apart.
    assert_eq!(view.object_at(Point { x: 1.0, y: 0.2 }), Some(BELT));
    assert_eq!(view.object_at(Point { x: 1.0, y: 0.4 }), None);
    let into = view.connections().iter().find(|connection| connection.to == BELT).unwrap();
    assert!(near(into.end, Point { x: 0.0, y: 0.0 }, 1e-9), "{into:?}");
    let out_of = view.connections().iter().find(|connection| connection.from == BELT).unwrap();
    assert!(near(out_of.start, Point { x: 4.0, y: 2.0 }, 1e-9), "{out_of:?}");
    // Before Init, its handle is just past its end, the way MUs leave.
    let model = crabsim_model::load(&curved_text((4.0, 2.5, 0.5))).unwrap();
    let mut building = ViewState::new(&model.to_description());
    building.set_shapes(crabsim_ui::workspace::shapes_of(&model));
    let handle = building.handle_of(BELT).unwrap();
    assert!(near(handle, Point { x: 4.0, y: 2.0 + crabsim_ui::view::HANDLE_GAP }, 1e-9), "{handle:?}");
    // One arrow along its 5.1 m, across its middle, pointing up and to the right.
    let [(from, to)] = view.belt_directions()[..] else { panic!("one arrow") };
    assert!(to.x > from.x && to.y > from.y, "{from:?} {to:?}");
    // The arc's rise is labelled at its middle, and its end's height.
    let labels: Vec<&str> = view.path_labels().iter().map(|(_, text)| *text).collect();
    assert_eq!(labels, ["+0.5 m", "0.5 m high"]);
    // Its outline is its band: 0.3 m out from its end, across the way it heads.
    let outline = view.objects()[BELT as usize].outline(0.0);
    assert!(outline.iter().any(|&corner| near(corner, Point { x: 3.7, y: 2.0 }, 1e-9)), "{outline:?}");
}

#[test]
fn a_conveyors_ends_say_how_they_sit_against_their_neighbours() {
    // Out meets the end with its centre half its 1 m length on, turned up the y axis like it, as
    // high as it.
    let fits = curved_at(1_000, (4.0, 2.5, 0.5)).view.end_fits(BELT);
    let lines: Vec<String> = fits.iter().map(|fit| fit.describe()).collect();
    assert!(fits[1].meets() && !fits[0].meets());
    assert_eq!(lines[1], "End: meets Out");
    assert!(lines[0].starts_with("Start: In is 1.50 m from where it would meet this end"), "{lines:?}");
    // Moved off, the end says by how much, as information only.
    let fits = curved_at(1_000, (5.0, 2.5, 0.0)).view.end_fits(BELT);
    assert!(fits[1].describe().starts_with("End: Out is 1.00 m from where it would meet this end"), "{:?}", fits[1].describe());
    assert!(fits[1].describe().contains("0.50 m lower"), "Out stands at 0, the end 0.5 m up: {:?}", fits[1].describe());
    let inspected = curved_at(1_000, (5.0, 2.5, 0.0)).view.inspection_of(BELT).unwrap();
    assert_eq!(inspected.ends.len(), 2);
}

#[test]
fn a_path_is_rows_of_segments_each_saying_what_is_wrong_with_it() {
    use crabsim_ui::attributes::{PathRows, SegmentKind};
    let mut form = Form::new("path", Some(&json!([{ "straight": 4.0 }, { "arc": { "radius": 1.5, "angle": 90.0 } }])));
    let rows = form.path.as_mut().unwrap();
    assert_eq!(rows.text, "4, arc 1.5 90");
    assert_eq!((rows.rows[1].kind, rows.rows[1].first.as_str(), rows.rows[1].second.as_str()), (SegmentKind::Arc, "1.5", "90"));
    rows.set_kind(1, SegmentKind::Helix);
    assert_eq!((rows.rows[1].second.as_str(), rows.rows[1].rise.as_str()), ("1", "1"), "a helix starts with one turn, rising 1 m");
    rows.add();
    rows.swap_with_next(1);
    assert_eq!(rows.text, "4, 1, helix 1.5 1 rise 1");
    assert_eq!(form.value(), Ok(json!([{ "straight": 4.0 }, { "straight": 1.0 }, { "helix": { "radius": 1.5, "turns": 1.0, "rise": 1.0 } }])));
    assert!(!form.matches(Some(&json!([{ "straight": 4.0 }]))));

    let rows = form.path.as_mut().unwrap();
    rows.text = "3, arc 0.2 -90".into();
    rows.read_text();
    assert_eq!(rows.rows.len(), 2);
    let problems = rows.problems(0.6, None, 30.0);
    assert!(problems[0].is_empty() && problems[1][0].contains("its inner edge would fold"), "{problems:?}");
    rows.text = "3, arc".into();
    rows.read_text();
    assert!(rows.text_problem.as_ref().unwrap().contains("segment 2"), "{:?}", rows.text_problem);
    assert_eq!(rows.rows.len(), 2, "rows stay as they were");

    // Its figures: 2 m on, then a quarter turn of radius 2 to the left.
    let rows = PathRows::from_value(Some(&json!([{ "straight": 2.0 }, { "arc": { "radius": 2.0, "angle": 90.0 } }])));
    let figures = rows.figures().unwrap();
    assert!((figures.ahead - 4.0).abs() < 1e-9 && (figures.left - 2.0).abs() < 1e-9 && (figures.turned - 90.0).abs() < 1e-9, "{figures:?}");
    assert_eq!(figures.describe(), "5.14 m long (5.14 m in plan). Ends 4 m ahead, 2 m to the left, turned 90°. Tightest bend 2 m.");
    // A typed value that isn't a number says which.
    let mut rows = rows;
    rows.rows[1].second = "ninety".into();
    assert!(rows.segments().unwrap_err().contains("segment 2: `ninety` isn't a number, the angle °"));
    assert_eq!(rows.figures(), None);
}
