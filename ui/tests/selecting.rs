//! Selecting a connection on the canvas, and what Delete does with what is selected: a connection
//! goes at once, an instance with connections asks first, naming them, and nothing is removed
//! while a run is in progress. The view state only, with no window.

mod common;

use common::{Watched, SOURCE, STATION_1, STATION_2};
use crabsim::{FailureProfile, ModelDescription, Position, SimTime};
use crabsim::random_numbers::Distribution;
use crabsim_ui::view::{Deletion, Point, ViewState};

/// In → A → Out, and In → B, with A and B side by side, B not passing anything on.
fn fork() -> ModelDescription {
    let no_failures = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(10));
    let a = model.add_station(SimTime::from_secs(5), no_failures.clone());
    let b = model.add_station(SimTime::from_secs(5), no_failures);
    let out = model.add_drain();
    model.connect(source, a);
    model.connect(source, b);
    model.connect(a, out);
    for (id, name, x, y) in [(source, "In", 0.0, 0.0), (a, "A", 10.0, 0.0), (b, "B", 10.0, 6.0), (out, "Out", 20.0, 0.0)] {
        model.set_name(id, name);
        model.set_position(id, Position { x, y, z: 0.0 });
    }
    model
}

fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}

#[test]
fn a_click_near_a_connections_line_finds_it_and_far_from_any_finds_none() {
    let view = ViewState::new(&fork());
    // Halfway along In → A, which runs along y = 0.
    assert_eq!(view.connection_at(point(5.0, 0.2), 0.5), Some((0, 1)));
    assert_eq!(view.connection_at(point(5.0, 0.8), 0.5), None, "further than the tolerance");
    assert_eq!(view.connection_at(point(15.0, -0.3), 0.5), Some((1, 3)), "A → Out");
    assert_eq!(view.connection_at(point(5.0, 3.0), 0.5), Some((0, 2)), "In → B, on its slant");
    assert_eq!(view.connection_at(point(30.0, 0.0), 0.5), None, "past the end of every line");
}

#[test]
fn selecting_a_connection_unselects_the_object_and_the_other_way_round() {
    let mut view = ViewState::new(&fork());
    view.select(Some(1));
    view.select_connection(Some((0, 1)));
    assert_eq!((view.selected(), view.selected_connection()), (None, Some((0, 1))));
    view.select(Some(2));
    assert_eq!((view.selected(), view.selected_connection()), (Some(2), None));
    view.select_connection(Some((2, 3)));
    assert_eq!(view.selected_connection(), None, "B isn't connected to Out");
}

#[test]
fn delete_removes_a_connection_at_once_and_asks_before_an_instance_with_connections() {
    let mut view = ViewState::new(&fork());
    assert_eq!(view.deletion(), None, "nothing selected");
    view.select_connection(Some((0, 2)));
    let deletion = view.deletion().unwrap();
    assert_eq!(deletion, Deletion::Connection { from: "In".into(), to: "B".into() });
    assert!(!deletion.asks_first());

    view.select(Some(1));
    let deletion = view.deletion().unwrap();
    assert_eq!(
        deletion,
        Deletion::Instance { name: "A".into(), connections: vec![("In".into(), "A".into()), ("A".into(), "Out".into())] }
    );
    assert!(deletion.asks_first(), "its connections go with it, so it names them first");
}

#[test]
fn an_instance_without_connections_goes_at_once() {
    let mut model = fork();
    let lone = model.add_drain();
    model.set_name(lone, "Spare");
    let mut view = ViewState::new(&model);
    view.select(Some(lone));
    let deletion = view.deletion().unwrap();
    assert_eq!(deletion, Deletion::Instance { name: "Spare".into(), connections: vec![] });
    assert!(!deletion.asks_first());
}

#[test]
fn building_is_offered_only_with_no_run_in_progress() {
    let mut watched = Watched::demo();
    assert!(watched.view.allows_building());
    watched.view.select(Some(STATION_1));
    assert!(watched.view.deletion().is_some());
    watched.session.start().unwrap();
    watched.at(10);
    assert!(!watched.view.allows_building(), "Reset first to change the model");
    assert_eq!(watched.view.deletion(), None);
    watched.view.select_connection(Some((SOURCE, STATION_1)));
    assert_eq!(watched.view.selected_connection(), Some((SOURCE, STATION_1)), "a connection can be looked at during a run");
    assert_eq!(watched.view.deletion(), None);
    watched.session.reset().unwrap();
    watched.frame();
    assert!(watched.view.allows_building());
    watched.view.select(Some(STATION_2));
    assert!(watched.view.deletion().is_some());
}

#[test]
fn an_objects_connect_handle_sits_just_off_its_outgoing_side_and_a_drain_has_none() {
    let model = fork();
    let view = ViewState::new(&model);
    let a = &view.objects()[1];
    let handle = view.handle_of(1).unwrap();
    assert!((handle.y - a.centre.y).abs() < 1e-9, "level with its centre");
    assert!(handle.x > a.centre.x + a.length / 2.0, "just past its right-hand side, where MUs go out");
    assert!(handle.x < a.centre.x + a.length / 2.0 + 1.0);
    assert_eq!(view.handle_of(3), None, "a Drain passes no MU on");
    assert_eq!(view.handle_at(handle, 0.3), Some(1));
    assert_eq!(view.handle_at(point(handle.x + 1.0, handle.y), 0.3), None);
}

#[test]
fn a_turned_object_has_its_handle_turned_with_it() {
    let mut model = fork();
    model.set_dimensions(1, crabsim::Dimensions { length: 2.0, width: 1.0, height: 1.0, rotation_degrees: 90.0 });
    let view = ViewState::new(&model);
    let a = &view.objects()[1];
    let handle = view.handle_of(1).unwrap();
    assert!((handle.x - a.centre.x).abs() < 1e-9 && handle.y > a.centre.y + 1.0, "above it, its outgoing side turned up");
}

#[test]
fn no_handle_is_offered_during_a_run() {
    let mut watched = Watched::demo();
    assert!(watched.view.handle_of(STATION_1).is_some());
    watched.session.start().unwrap();
    watched.at(10);
    assert_eq!(watched.view.handle_of(STATION_1), None, "Reset first to change the model");
}

#[test]
fn tab_goes_through_the_objects_in_model_order_and_round_again() {
    let view = ViewState::new(&fork());
    assert_eq!(view.next_object(None, false), Some(0), "the first, with nothing selected");
    assert_eq!(view.next_object(Some(1), false), Some(2));
    assert_eq!(view.next_object(Some(3), false), Some(0), "after the last comes the first");
    assert_eq!(view.next_object(None, true), Some(3), "Shift+Tab from nothing goes to the last");
    assert_eq!(view.next_object(Some(0), true), Some(3));
    assert_eq!(ViewState::new(&ModelDescription::new()).next_object(None, false), None);
}

#[test]
fn ctrl_click_adds_and_takes_away_and_one_alone_is_the_selected_object() {
    let mut view = ViewState::new(&fork());
    view.select(Some(1));
    view.toggle(2);
    assert_eq!(view.selection(), &[1, 2]);
    assert_eq!(view.selected(), None, "two aren't one object to inspect");
    view.toggle(1);
    assert_eq!(view.selection(), &[2]);
    assert_eq!(view.selected(), Some(2));
    view.select_connection(Some((0, 1)));
    assert_eq!(view.selection(), &[] as &[u32]);
    view.toggle(3);
    assert_eq!((view.selection(), view.selected_connection()), (&[3][..], None));
}

#[test]
fn a_rectangle_selects_every_object_whose_centre_is_inside() {
    let mut view = ViewState::new(&fork());
    // A at (10, 0) and B at (10, 6): a box round both, but not round In at 0 or Out at 20.
    view.select_within(point(5.0, -2.0), point(15.0, 8.0));
    assert_eq!(view.selection(), &[1, 2]);
    view.select_within(point(15.0, 8.0), point(5.0, -2.0));
    assert_eq!(view.selection(), &[1, 2], "dragged either way");
    view.select_within(point(30.0, 30.0), point(40.0, 40.0));
    assert_eq!(view.selection(), &[] as &[u32]);
}

#[test]
fn delete_on_several_names_every_connection_going_with_them() {
    let mut view = ViewState::new(&fork());
    view.select(Some(1));
    view.toggle(3);
    let deletion = view.deletion().unwrap();
    assert_eq!(
        deletion,
        Deletion::Instances { names: vec!["A".into(), "Out".into()], connections: vec![("In".into(), "A".into()), ("A".into(), "Out".into())] }
    );
    assert!(deletion.asks_first());
}
