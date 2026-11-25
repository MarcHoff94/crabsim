//! A cross conveyor in the view (spec issue 148, PR 3): its sides, arrows, ways across and exit
//! labels, connections at its sides' middles, the free side nearest a point, and its MUs crossing,
//! switching at its centre and leaving sideways, with a real session on a fake clock and no window.

mod common;

use common::{FakeClock, Watched};
use crabsim::{LiveSession, ModelDescription, MuKind, PlanStep, Position, Side, SimTime};
use crabsim_ui::camera::Camera;
use crabsim_ui::view::{Point, Shade, SideUse, ViewState, MU_MIN_PIXELS_PER_METRE};

const SOURCE: u32 = 0;
const CROSS: u32 = 1;
const TO_A: u32 = 2;
const TO_B: u32 = 3;

/// In (an MU every 20 s, 0.5 by 0.3 m, going to To A) -> Cross (0.6 by 0.6 m at 0.5 m/s, its
/// centre at x = 10, entered at its back) -> To A at its left, which lists To A, and To B at its
/// front. MU 1 enters at 20 s with its booking point, its middle, at the back's edge, reaches the
/// centre 0.6 s later, switches for 2 s, and leaves sideways from 22.6 s.
fn model() -> ModelDescription {
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(20));
    let cross = model.add_cross_conveyor(0.6, 0.6, 0.5);
    let to_a = model.add_drain();
    let to_b = model.add_drain();
    for (id, name, x, y) in [(source, "In", 8.0, 0.0), (cross, "Cross", 10.0, 0.0), (to_a, "To A", 10.0, 2.0), (to_b, "To B", 12.0, 0.0)] {
        model.set_name(id, name);
        model.set_position(id, Position { x, y, z: 0.0 });
    }
    let a = model.add_destination(&[to_a]);
    let tote = model.add_mu_kind_with_plan(MuKind { length: 0.5, width: 0.3, booking_point: 0.5 }, vec![PlanStep { at: a, process_time: None }]);
    model.set_mu_kind(source, tote);
    model.set_exit_destinations(cross, Side::Left, &[a]);
    model.connect_at(source, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Left), to_a, None);
    model.connect_at(cross, Some(Side::Front), to_b, None);
    model
}

fn halted_at(millis: u64) -> Watched {
    let model = model();
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

fn near(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
}

#[test]
fn a_cross_conveyor_knows_each_side_from_its_connections() {
    let view = ViewState::new(&model());
    let sides = view.objects()[CROSS as usize].sides.clone().expect("a cross conveyor has sides");
    assert_eq!(sides[Side::Back.index()], SideUse::Entry { from: SOURCE });
    assert_eq!(sides[Side::Left.index()], SideUse::Exit { to: TO_A, destinations: Vec::new() });
    assert_eq!(sides[Side::Front.index()], SideUse::Exit { to: TO_B, destinations: Vec::new() });
    assert_eq!(sides[Side::Right.index()], SideUse::Free);
    assert!(view.objects()[SOURCE as usize].sides.is_none(), "only a cross conveyor has sides");
}

#[test]
fn connections_to_and_from_a_cross_conveyor_meet_its_sides_in_their_middles() {
    let view = ViewState::new(&model());
    let connection = |from: u32, to: u32| *view.connections().iter().find(|connection| connection.from == from && connection.to == to).unwrap();
    let into = connection(SOURCE, CROSS);
    assert_eq!(into.to_side, Some(Side::Back));
    assert!(near(into.end, Point { x: 9.7, y: 0.0 }), "{into:?}");
    let left = connection(CROSS, TO_A);
    assert_eq!(left.from_side, Some(Side::Left));
    assert!(near(left.start, Point { x: 10.0, y: 0.3 }), "{left:?}");
    // The Drain's edge faces the side's middle, straight below it.
    assert!(near(left.end, Point { x: 10.0, y: 1.5 }), "{left:?}");
    let front = connection(CROSS, TO_B);
    assert!(near(front.start, Point { x: 10.3, y: 0.0 }), "{front:?}");
}

#[test]
fn a_turned_cross_conveyor_meets_its_connections_at_its_turned_sides() {
    let mut model = model();
    model.set_dimensions(CROSS, crabsim::Dimensions { length: 0.6, width: 0.6, height: 0.5, rotation_degrees: 90.0 });
    let view = ViewState::new(&model);
    let into = *view.connections().iter().find(|connection| connection.to == CROSS).unwrap();
    // Turned a quarter, its back faces down.
    assert!(near(into.end, Point { x: 10.0, y: -0.3 }), "{into:?}");
}

#[test]
fn each_entry_and_exit_has_an_arrow_and_a_way_straight_across_goes_from_an_entry_to_an_exit() {
    let view = ViewState::new(&model());
    let arrows = view.side_arrows();
    assert_eq!(arrows.len(), 3, "one arrow at each side with a connection: {arrows:?}");
    // In at the back, pointing in from its edge.
    assert!(arrows.iter().any(|&(start, end)| near(start, Point { x: 9.7, y: 0.0 }) && end.x > 9.7 && end.x < 10.0), "{arrows:?}");
    // Out at the left, pointing out to its edge.
    assert!(arrows.iter().any(|&(start, end)| near(end, Point { x: 10.0, y: 0.3 }) && start.y > 0.0 && start.y < 0.3), "{arrows:?}");
    let ways = view.straight_ways();
    assert_eq!(ways.len(), 1);
    assert!(near(ways[0].0, Point { x: 9.7, y: 0.0 }) && near(ways[0].1, Point { x: 10.3, y: 0.0 }), "{ways:?}");
}

#[test]
fn an_exit_side_is_labelled_with_the_destinations_it_lists() {
    let mut view = ViewState::new(&model());
    assert!(view.exit_labels().is_empty(), "nothing listed yet");
    view.set_exit_lists(vec![(CROSS, vec![(Side::Left, vec!["To A".into(), "Rework".into()]), (Side::Right, vec!["Ignored".into()])])]);
    let labels = view.exit_labels();
    // The right side has no exit, so it isn't labelled.
    assert_eq!(labels.len(), 1, "{labels:?}");
    assert_eq!(labels[0].1, "To A, Rework");
    assert!(labels[0].0.y > 0.3 && (labels[0].0.x - 10.0).abs() < 1e-9, "just outside the left side: {labels:?}");
}

#[test]
fn a_connection_dragged_to_a_cross_conveyor_takes_the_free_side_nearest_the_pointer() {
    let mut lone = ModelDescription::new();
    let cross = lone.add_cross_conveyor(0.6, 0.6, 0.5);
    let view = ViewState::new(&lone);
    for (pointer, side) in [((0.0, -5.0), Side::Right), ((5.0, 0.4), Side::Front), ((-5.0, 0.4), Side::Back), ((0.4, 5.0), Side::Left)] {
        assert_eq!(view.free_side_near(cross, Point { x: pointer.0, y: pointer.1 }), Some(side), "towards {pointer:?}");
    }
    // In the line, only its right side is free, so a point behind it takes that.
    let view = ViewState::new(&model());
    assert_eq!(view.free_side_near(CROSS, Point { x: 5.0, y: 0.1 }), Some(Side::Right));
    assert_eq!(view.free_side_near(SOURCE, Point { x: 0.0, y: 0.0 }), None, "only a cross conveyor has sides");
}

#[test]
fn an_mu_turning_stands_at_the_centre_while_switching_and_the_cross_conveyor_is_busy() {
    let watched = halted_at(21_500);
    let markers: Vec<_> = watched.view.mu_markers(&close()).into_iter().filter(|marker| marker.holder == CROSS).collect();
    assert_eq!(markers.len(), 1, "{markers:?}");
    let marker = markers[0];
    assert!(near(marker.at, Point { x: 10.0, y: 0.0 }), "{marker:?}");
    assert_eq!((marker.length, marker.width, marker.rotation_degrees), (0.5, 0.3, 0.0));
    assert_eq!(watched.view.shade(CROSS), Shade::Busy);
}

#[test]
fn an_mu_that_turned_leaves_sideways_lying_crosswise() {
    // 0.4 s into its second leg, from the right side towards the left: its front 0.65 m from the
    // right side's edge, its middle 0.15 m behind, so 0.2 m left of the centre.
    let watched = halted_at(23_000);
    let markers: Vec<_> = watched.view.mu_markers(&close()).into_iter().filter(|marker| marker.holder == CROSS).collect();
    assert_eq!(markers.len(), 1, "{markers:?}");
    let marker = markers[0];
    assert!(near(marker.at, Point { x: 10.0, y: 0.2 }), "{marker:?}");
    // Its width along its way, its length across: it lies as it did before it turned.
    assert_eq!((marker.length, marker.width, marker.rotation_degrees), (0.3, 0.5, 90.0));
}

#[test]
fn an_mu_entering_a_cross_conveyor_crosses_from_its_entry_side() {
    // 0.3 s in: its front 0.25 m + 0.15 m from the back's edge, its middle 0.15 m from it.
    let watched = halted_at(20_300);
    let marker = watched.view.mu_markers(&close()).into_iter().find(|marker| marker.holder == CROSS).expect("on the cross conveyor");
    assert!(near(marker.at, Point { x: 9.85, y: 0.0 }), "{marker:?}");
}

// ---------- Statistics ----------

/// The cross line (`demo::cross_line`) in a session, halted after an hour on the clock, with its
/// full state.
fn cross_line_run_a_while() -> Watched {
    let model = crabsim::demo::cross_line();
    let view = ViewState::new(&model);
    let clock = FakeClock::default();
    let session = LiveSession::new(move || model.build(0), clock.clone());
    let mut watched = Watched { session, clock, view };
    watched.view.set_statistics_open(true);
    watched.session.start().unwrap();
    watched.at(3_600);
    watched.session.halt().unwrap();
    watched.frame();
    watched.details();
    watched
}

#[test]
fn the_statistics_window_gives_each_cross_conveyor_s_shares_and_mus_out_by_side() {
    let watched = cross_line_run_a_while();
    let results = watched.view.results().unwrap();
    let names: Vec<&str> = results.crosses.iter().map(|cross| cross.name.as_str()).collect();
    assert_eq!(names, ["Fork", "Corner", "Join"], "in model order");
    let fork = &results.crosses[0];
    let total = fork.busy + fork.blocked + fork.idle + fork.interrupted;
    assert!((total - 1.0).abs() < 1e-9, "the Fork's shares add up to {total}");
    assert!(fork.switching > 0.0 && fork.switching < fork.busy, "switching is part of busy: {fork:?}");
    // Housings turn left to the Spur, Covers go on to the Lane.
    assert!(fork.mus_out[Side::Left.index()] > 0 && fork.mus_out[Side::Front.index()] > 0, "{fork:?}");
    assert_eq!(fork.mus_out[Side::Back.index()] + fork.mus_out[Side::Right.index()], 0);
}

#[test]
fn stations_count_the_mus_they_worked_on_on_and_off_plan() {
    let watched = cross_line_run_a_while();
    let results = watched.view.results().unwrap();
    assert!(results.with_plans, "the Stations work on MUs with work plans");
    let mill = results.stations.iter().find(|station| station.name == "Mill").unwrap();
    assert!(mill.on_plan > 0 && mill.off_plan == 0, "only Housings reach the Mill: {mill:?}");
    let without = {
        let mut watched = Watched::demo();
        watched.view.set_statistics_open(true);
        watched.session.start().unwrap();
        watched.at(600);
        watched.session.halt().unwrap();
        watched.frame();
        watched.details();
        watched.view.results().unwrap()
    };
    assert!(!without.with_plans, "the demo's MUs go nowhere in particular");
}

#[test]
fn an_object_window_gives_a_cross_conveyor_s_switching_and_sides_and_a_station_s_plan_counts() {
    let mut watched = cross_line_run_a_while();
    let id = |name: &str| watched.view.objects().iter().find(|object| object.name == name).unwrap().id;
    let (fork, mill) = (id("Fork"), id("Mill"));
    watched.view.select(Some(fork));
    watched.details();
    let rows = watched.view.inspection_of(fork).unwrap().statistics_rows();
    let labels: Vec<&str> = rows.iter().map(|(label, _)| *label).collect();
    assert!(labels.contains(&"  switching"), "{labels:?}");
    assert!(labels.contains(&"Out at the left") && labels.contains(&"Out at the front"), "{labels:?}");
    assert!(!labels.contains(&"Out at the back"), "no MU left by the back: {labels:?}");
    let rows = watched.view.inspection_of(mill).unwrap().statistics_rows();
    let on = rows.iter().find(|(label, _)| *label == "On plan").map(|(_, value)| value.clone());
    assert!(on.is_some_and(|on| on != "0"), "{rows:?}");
}

#[test]
fn a_click_on_a_cross_conveyor_between_conveyors_finds_it_and_not_the_conveyors_ending_there() {
    // The Fork's centre is half a conveyor's width from where the Feed ends and the Lane and the
    // Spur start.
    let view = ViewState::new(&crabsim::demo::cross_line());
    let id = |name: &str| view.objects().iter().find(|object| object.name == name).unwrap().id;
    let fork = view.objects()[id("Fork") as usize].centre;
    assert_eq!(view.object_at(fork), Some(id("Fork")));
    assert_eq!(view.object_at(Point { x: fork.x + 0.25, y: fork.y + 0.25 }), Some(id("Fork")), "near its corner too");
    assert_eq!(view.object_at(Point { x: fork.x + 0.4, y: fork.y }), Some(id("Lane")), "just past its front, the Lane");
}

#[test]
fn i_shows_names_but_conveyors_then_every_name_with_annotations_then_none() {
    use crabsim::ObjectKind;
    use crabsim_ui::view::Labels;
    let shown = Labels::default();
    assert_eq!(shown, Labels::Names);
    assert!(shown.shows_name(ObjectKind::Station) && shown.shows_name(ObjectKind::CrossConveyor));
    assert!(!shown.shows_name(ObjectKind::Conveyor), "a conveyor's name lies over its neighbours");
    assert!(!shown.shows_annotations());
    let all = shown.next();
    assert!(all.shows_name(ObjectKind::Conveyor) && all.shows_annotations());
    let none = all.next();
    assert!(!none.shows_name(ObjectKind::Station) && !none.shows_annotations());
    assert_eq!(none.next(), Labels::Names, "and round again");
}
