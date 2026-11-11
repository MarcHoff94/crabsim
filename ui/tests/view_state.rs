//! The view state, tested without a window: a real live session on a fake clock, its outputs
//! fed in as the window would feed them, and what would be drawn read back out.

use crabsim::{
    demo, Dimensions, FailureProfile, ModelDescription, SessionOutput, Position, RunError,
    SessionError, SessionStatus, SimTime, TimeMode,
};
use crabsim::random_numbers::Distribution;
use crabsim_ui::camera::Camera;
use crabsim_ui::view::{
    label_size, wall_time_roughly, Control, GoingOn, JumpProblem, Point, Rect, Shade, ViewState,
    DEFAULT_SPEED, MAX_LABEL_PIXELS, MAX_SPEED, MIN_LABEL_PIXELS, MIN_SPEED,
};
use std::time::Duration;

mod common;

use common::{Watched, STATION_1, STATION_2};

fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}

fn close(a: Point, b: Point) -> bool {
    (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
}

#[test]
fn every_object_of_the_demo_is_drawn_where_the_model_places_it_and_to_size() {
    let model = demo::model();
    let view = ViewState::new(&model);
    assert_eq!(view.objects().len(), 14);
    for object in view.objects() {
        let position = model.position(object.id).expect("the demo places every object");
        let size = model.dimensions(object.id);
        assert_eq!(object.centre, point(position.x, position.y));
        assert_eq!((object.length, object.width), (size.length, size.width));
        assert_eq!(object.kind, model.kind(object.id));
        assert_eq!(Some(object.name.as_str()), model.name(object.id));
    }
    assert_eq!(view.objects()[STATION_1 as usize].name, "Station 1");
}

#[test]
fn an_object_without_a_name_or_position_is_still_drawn_somewhere_of_its_own() {
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(5));
    let drain = model.add_drain();
    model.connect(source, drain);
    let view = ViewState::new(&model);
    assert_eq!(view.objects()[0].name, "Source 0");
    assert_eq!(view.objects()[1].name, "Drain 1");
    assert_ne!(view.objects()[0].centre, view.objects()[1].centre);
}

#[test]
fn a_moved_object_takes_the_ends_of_its_connections_with_it() {
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(5));
    let drain = model.add_drain();
    model.connect(source, drain);
    model.set_position(source, Position { x: 0.0, y: 0.0, z: 0.0 });
    model.set_position(drain, Position { x: 10.0, y: 0.0, z: 0.0 });
    let mut view = ViewState::new(&model);
    view.move_object(drain, point(0.0, 10.0));
    assert_eq!(view.objects()[drain as usize].centre, point(0.0, 10.0));
    assert_eq!(view.object_at(point(0.0, 10.0)), Some(drain), "it is found where it now is");
    let connection = &view.connections()[0];
    // Both are 1 m square, so the arrow now runs straight up between their edges.
    assert!(close(connection.start, point(0.0, 0.5)), "{:?}", connection.start);
    assert!(close(connection.end, point(0.0, 9.5)), "{:?}", connection.end);
}

#[test]
fn bounds_hold_every_object_whole() {
    let view = ViewState::new(&demo::model());
    let bounds = view.bounds();
    // The Source is 1 m square at x = 0, and Station 5c 2 x 1.5 m at y = -7.
    assert_eq!(bounds.min.x, -0.5);
    assert_eq!(bounds.min.y, -7.75);
    assert_eq!(bounds.max.x, 28.5);
    assert_eq!(bounds.max.y, 7.75);
}

#[test]
fn connections_run_from_edge_to_edge_in_the_order_the_model_lists_them() {
    let view = ViewState::new(&demo::model());
    let first = &view.connections()[0];
    assert_eq!((first.from, first.to), (0, STATION_1));
    // The Source is 1 m square at (0, 0), Station 1 is 2 m long at (4, 0).
    assert!(close(first.start, point(0.5, 0.0)), "{:?}", first.start);
    assert!(close(first.end, point(3.0, 0.0)), "{:?}", first.end);
    assert_eq!(view.connections().len(), 18);
}

#[test]
fn a_connection_leaves_a_rotated_object_at_its_rotated_edge() {
    let mut model = ModelDescription::new();
    let no_failures = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let station = model.add_station(SimTime::from_secs(5), no_failures);
    let drain = model.add_drain();
    model.connect(station, drain);
    model.set_position(station, Position { x: 0.0, y: 0.0, z: 0.0 });
    model.set_dimensions(station, Dimensions { length: 4.0, width: 1.0, height: 1.0, rotation_degrees: 90.0 });
    model.set_position(drain, Position { x: 0.0, y: 10.0, z: 0.0 });
    let view = ViewState::new(&model);
    let connection = &view.connections()[0];
    assert!(close(connection.start, point(0.0, 2.0)), "{:?}", connection.start);
    assert!(close(connection.end, point(0.0, 9.5)), "{:?}", connection.end);
}

#[test]
fn before_init_nothing_has_a_state_and_init_and_start_are_offered() {
    let mut watched = Watched::demo();
    watched.frame();
    let view = &watched.view;
    assert!(view.objects().iter().all(|object| view.shade(object.id) == Shade::Unknown));
    assert!(view.allows(Control::Init));
    assert!(view.allows(Control::Start));
    assert!(!view.allows(Control::Halt));
    assert!(!view.allows(Control::Reset));
    assert_eq!(view.shown_time(), "00:00:00");
}

#[test]
fn a_run_is_in_progress_only_from_init_until_reset() {
    let empty = ViewState::new(&ModelDescription::new());
    assert!(!empty.run_in_progress(), "with no model open there is no run to lose");

    let mut watched = Watched::demo();
    watched.frame();
    assert!(!watched.view.run_in_progress(), "before Init");
    watched.session.step_forward().unwrap();
    watched.frame();
    assert!(watched.view.run_in_progress(), "after Init");
    watched.session.reset().unwrap();
    watched.frame();
    assert!(!watched.view.run_in_progress(), "after Reset");
}

#[test]
fn init_shows_the_default_state_and_is_offered_no_more() {
    let mut watched = Watched::demo();
    watched.session.step_forward().unwrap();
    watched.frame();
    let view = &watched.view;
    // From Init on, the Source is working on its first MU, and everything else waits for one.
    assert_eq!(view.shade(0), Shade::Busy);
    assert!(view.objects()[1..].iter().all(|object| view.shade(object.id) == Shade::Idle));
    assert!(!view.allows(Control::Init));
    assert!(view.allows(Control::Start));
    assert!(view.allows(Control::Reset));
}

#[test]
fn while_advancing_shown_time_and_state_colours_follow_the_run() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    let view = &watched.view;
    assert_eq!(view.status(), SessionStatus::Advancing);
    assert_eq!(view.shown_time(), "00:00:12");
    // The first MU leaves the Source at 5 s and takes 10 s at Station 1.
    assert_eq!(view.shade(STATION_1), Shade::Busy);
    assert_eq!(view.shade(STATION_2), Shade::Idle);
    assert!(view.allows(Control::Halt));
    assert!(!view.allows(Control::Start));
}

#[test]
fn a_stop_shows_its_most_serious_interruption() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    watched.session.halt().unwrap();
    watched.session.pause(STATION_1).unwrap();
    watched.frame();
    assert_eq!(watched.view.shade(STATION_1), Shade::Paused);
    watched.session.force_failure(STATION_1).unwrap();
    watched.frame();
    assert_eq!(watched.view.shade(STATION_1), Shade::ForcedFailure);
    watched.session.resume(STATION_1).unwrap();
    watched.frame();
    assert_eq!(watched.view.shade(STATION_1), Shade::ForcedFailure);
    watched.session.end_forced_failure(STATION_1).unwrap();
    watched.frame();
    assert_eq!(watched.view.shade(STATION_1), Shade::Busy);
}

#[test]
fn reset_returns_to_time_zero_and_offers_init_again() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    watched.session.reset().unwrap();
    watched.frame();
    let view = &watched.view;
    assert_eq!(view.status(), SessionStatus::Halted);
    assert_eq!(view.shown_time(), "00:00:00");
    assert!(view.allows(Control::Init));
    assert!(!view.allows(Control::Reset));
    assert_eq!(view.shade(STATION_1), Shade::Idle);
}

#[test]
fn a_run_error_shows_until_the_run_is_reset() {
    let model = demo::model();
    let mut view = ViewState::new(&model);
    let wall = Duration::ZERO;
    view.observe(&SessionOutput::Error(crabsim::SessionError::ResetRequired), wall);
    assert!(view.error().is_some());
    let mut run = model.build(0);
    run.init().unwrap();
    view.observe(&SessionOutput::State(run.state()), wall);
    assert_eq!(view.error(), None);
}

#[test]
fn frames_are_drawn_while_advancing_and_stop_soon_after_halting() {
    let mut view = ViewState::new(&demo::model());
    let secs = Duration::from_secs_f64;
    assert!(view.wants_frames(secs(0.0)), "the first frames draw the model");
    assert!(view.wants_frames(secs(2.5)), "startup needs frames for a while");
    assert!(!view.wants_frames(secs(5.0)));
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), secs(10.0));
    assert!(view.wants_frames(secs(100.0)));
    view.observe(&SessionOutput::Status(SessionStatus::Halted), secs(100.0));
    assert!(view.wants_frames(secs(100.5)), "what a Halt brings is drawn");
    assert!(!view.wants_frames(secs(102.0)), "a halted window draws nothing");
    view.commanded(secs(200.0));
    assert!(view.wants_frames(secs(200.5)), "an answer to a command is drawn");
    assert!(!view.wants_frames(secs(202.0)));
}

#[test]
fn the_camera_starts_with_the_whole_model_in_view() {
    let bounds = Rect { min: point(-0.5, -7.75), max: point(28.5, 7.75) };
    let canvas = point(1000.0, 500.0);
    let camera = Camera::fitting(bounds, canvas);
    for corner in [bounds.min, bounds.max, point(bounds.min.x, bounds.max.y), point(bounds.max.x, bounds.min.y)] {
        let at = camera.to_screen(corner, canvas);
        assert!(at.x > 0.0 && at.x < canvas.x && at.y > 0.0 && at.y < canvas.y, "{corner:?} is at {at:?}");
    }
    assert!(close(camera.to_screen(point(14.0, 0.0), canvas), point(500.0, 250.0)));
}

#[test]
fn the_screen_is_upside_down_from_the_floor() {
    let camera = Camera { centre: point(0.0, 0.0), pixels_per_metre: 10.0 };
    let canvas = point(200.0, 100.0);
    assert!(close(camera.to_screen(point(1.0, 1.0), canvas), point(110.0, 40.0)));
    assert!(close(camera.to_world(point(110.0, 40.0), canvas), point(1.0, 1.0)));
}

#[test]
fn zooming_keeps_the_point_under_the_pointer_in_place() {
    let canvas = point(800.0, 600.0);
    let mut camera = Camera { centre: point(10.0, 5.0), pixels_per_metre: 20.0 };
    let pointer = point(650.0, 120.0);
    let under = camera.to_world(pointer, canvas);
    camera.zoom_at(2.0, pointer, canvas);
    assert_eq!(camera.pixels_per_metre, 40.0);
    assert!(close(camera.to_world(pointer, canvas), under));
}

#[test]
fn zoom_stays_within_its_limits() {
    let canvas = point(800.0, 600.0);
    let mut camera = Camera { centre: point(0.0, 0.0), pixels_per_metre: 20.0 };
    camera.zoom_at(1e9, point(0.0, 0.0), canvas);
    assert_eq!(camera.pixels_per_metre, Camera::MAX_PIXELS_PER_METRE);
    camera.zoom_at(1e-12, point(0.0, 0.0), canvas);
    assert_eq!(camera.pixels_per_metre, Camera::MIN_PIXELS_PER_METRE);
}

#[test]
fn dragging_moves_the_floor_with_the_pointer() {
    let canvas = point(800.0, 600.0);
    let mut camera = Camera { centre: point(0.0, 0.0), pixels_per_metre: 10.0 };
    let grabbed = point(300.0, 200.0);
    let under = camera.to_world(grabbed, canvas);
    camera.pan(point(50.0, -30.0));
    assert!(close(camera.to_world(point(350.0, 170.0), canvas), under));
}

#[test]
fn names_shrink_with_the_model_and_go_only_when_too_small_to_read() {
    let at = |pixels_per_metre: f64| label_size(&Camera { centre: point(0.0, 0.0), pixels_per_metre });
    let fitted = at(30.0).expect("readable with the demo fitted into view");
    let further = at(15.0).expect("still readable further out");
    assert!((further - fitted / 2.0).abs() < 1e-9, "half the zoom, half the size");
    assert!(at(15.0 / 1.5).is_some_and(|size| size >= MIN_LABEL_PIXELS));
    assert_eq!(at(5.0), None, "too small to read");
    assert_eq!(at(500.0), Some(MAX_LABEL_PIXELS), "no bigger than the panels' text when close up");
}

// ---------- Time controls ----------

fn shown_at(view: &mut ViewState, shown_secs: u64, wall_secs: f64) {
    view.observe(&SessionOutput::ShownTime(SimTime::from_secs(shown_secs)), Duration::from_secs_f64(wall_secs));
}

#[test]
fn speed_starts_at_60_times_real_time_and_stays_within_its_range() {
    let mut view = ViewState::new(&demo::model());
    assert_eq!(DEFAULT_SPEED, 60.0);
    assert_eq!(view.speed(), DEFAULT_SPEED);
    assert_eq!(view.set_speed(1e12), MAX_SPEED);
    assert_eq!(view.set_speed(0.0), MIN_SPEED);
    assert_eq!(view.set_speed(f64::NAN), MIN_SPEED);
    assert_eq!(view.set_speed(3_600.0), 3_600.0);
    assert_eq!(view.speed(), 3_600.0);
}

#[test]
fn time_mode_starts_continuous_and_can_skip_gaps() {
    let mut view = ViewState::new(&demo::model());
    assert_eq!(view.time_mode(), TimeMode::Continuous);
    view.set_time_mode(TimeMode::SKIPPING);
    assert_eq!(view.time_mode(), TimeMode::SKIPPING);
}

#[test]
fn without_animation_no_speed_is_missed_and_the_setting_carries_over_to_a_new_model() {
    let mut view = ViewState::new(&demo::model());
    assert!(view.animate(), "on to begin with");
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::from_secs(10));
    for second in 1..=6 {
        shown_at(&mut view, 10 * second, 10.0 + second as f64);
    }
    assert!(view.achieved_speed().is_some(), "animated, far behind 60 times");
    view.set_animate(false);
    assert_eq!(view.achieved_speed(), None, "without animation there is no speed to keep up with");
    assert!(!ViewState::with_settings_of(&demo::model(), &view).animate());
}

#[test]
fn the_achieved_speed_shows_only_while_playback_falls_behind() {
    let mut view = ViewState::new(&demo::model());
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::from_secs(10));
    shown_at(&mut view, 30, 10.5);
    assert_eq!(view.achieved_speed(), None, "too soon to tell");
    for second in 1..=3 {
        shown_at(&mut view, 60 * second, 10.0 + second as f64);
    }
    assert_eq!(view.achieved_speed(), None, "keeping up at 60 times");
    for second in 4..=6 {
        shown_at(&mut view, 180 + 30 * (second - 3), 10.0 + second as f64);
    }
    assert_eq!(view.achieved_speed(), Some(30.0), "half the speed asked for over the last 2 s");
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::from_secs(17));
    assert_eq!(view.achieved_speed(), None, "nothing to achieve while halted");
}

#[test]
fn a_new_speed_starts_measuring_afresh() {
    let mut view = ViewState::new(&demo::model());
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    for second in 1..=3 {
        shown_at(&mut view, 30 * second, second as f64);
    }
    assert_eq!(view.achieved_speed(), Some(30.0));
    view.set_speed(30.0);
    assert_eq!(view.achieved_speed(), None);
    shown_at(&mut view, 100, 3.5);
    assert_eq!(view.achieved_speed(), None, "too soon to tell at the new speed");
}

#[test]
fn skipping_ahead_of_the_speed_is_not_falling_behind() {
    let mut view = ViewState::new(&demo::model());
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    for second in 1..=3 {
        shown_at(&mut view, 600 * second, second as f64);
    }
    assert_eq!(view.achieved_speed(), None);
}

#[test]
fn a_session_on_time_keeps_up() {
    let mut watched = Watched::demo();
    watched.session.set_speed(DEFAULT_SPEED).unwrap();
    watched.session.start().unwrap();
    for second in 1..=4 {
        watched.at(second);
    }
    assert_eq!(watched.view.shown_time(), "00:04:00");
    assert_eq!(watched.view.achieved_speed(), None);
}

#[test]
fn stepping_is_offered_once_initialised_and_forward_only_while_halted() {
    let mut watched = Watched::demo();
    watched.frame();
    assert!(!watched.view.allows(Control::StepForward), "Init comes first");
    assert!(!watched.view.allows(Control::StepBack));
    watched.session.step_forward().unwrap();
    watched.frame();
    assert!(watched.view.allows(Control::StepForward));
    assert!(watched.view.allows(Control::StepBack));
    // The first instant after Init is the Source's first MU, at 5 s.
    watched.session.step_forward().unwrap();
    watched.frame();
    assert_eq!(watched.view.shown_time(), "00:00:05");
    watched.session.step_back().unwrap();
    watched.frame();
    assert_eq!(watched.view.shown_time(), "00:00:00");
    watched.session.start().unwrap();
    watched.frame();
    assert!(!watched.view.allows(Control::StepForward));
    assert!(watched.view.allows(Control::StepBack), "stepping back halts the run there");
}

#[test]
fn going_to_a_time_goes_back_or_on() {
    let mut watched = Watched::demo();
    assert_eq!(watched.view.jump_target("0"), Err(JumpProblem::NotInitialised));
    assert_eq!(watched.view.jump_target("1:00"), Ok(SimTime::from_secs(60)), "on from before Init, which Init does");
    watched.session.start().unwrap();
    watched.at(12);
    watched.session.halt().unwrap();
    watched.frame();
    let view = &watched.view;
    assert_eq!(view.jump_target("00:00:05"), Ok(SimTime::from_secs(5)));
    assert_eq!(view.jump_target("00:00:12"), Ok(SimTime::from_secs(12)), "the instant on screen");
    assert_eq!(view.jump_target("0:20"), Ok(SimTime::from_secs(20)), "a later time goes on");
    assert_eq!(view.jump_target("soon"), Err(JumpProblem::NotATime));
    watched.session.go_back(SimTime::from_secs(5)).unwrap();
    watched.frame();
    assert_eq!(watched.view.shown_time(), "00:00:05");
    assert_eq!(watched.view.status(), SessionStatus::Halted);
}

#[test]
fn going_on_animated_asks_first_only_when_it_would_take_more_than_a_minute() {
    let mut view = ViewState::new(&demo::model());
    let day = SimTime::from_secs(86_400);
    // A day at 60 times real time is 24 minutes of wall time.
    assert_eq!(view.going_on(day), GoingOn::Ask { wall: Duration::from_secs(1_440) });
    assert_eq!(view.going_on(SimTime::from_secs(3_600)), GoingOn::Now, "an hour is a minute");
    assert_eq!(view.going_on(SimTime::from_secs(3_660)), GoingOn::Ask { wall: Duration::from_secs(61) });
    view.set_speed(3_600.0);
    assert_eq!(view.going_on(day), GoingOn::Now, "24 seconds at 3,600 times");
    view.set_speed(DEFAULT_SPEED);
    view.set_end_time(Some(SimTime::from_secs(1_800)));
    assert_eq!(view.going_on(day), GoingOn::Now, "the run halts at its end time first");
    view.set_end_time(None);
    view.set_animate(false);
    assert_eq!(view.going_on(day), GoingOn::Now, "racing anyway");
    view.set_animate(true);
    shown_at(&mut view, 86_000, 1.0);
    assert_eq!(view.going_on(day), GoingOn::Now, "it counts from shown time");
    assert_eq!(view.going_on(SimTime::from_secs(60)), GoingOn::Now, "going back never asks");
}

#[test]
fn the_wait_is_said_roughly() {
    assert_eq!(wall_time_roughly(Duration::from_secs(90)), "about 2 minutes");
    assert_eq!(wall_time_roughly(Duration::from_secs(1_440)), "about 24 minutes");
    assert_eq!(wall_time_roughly(Duration::from_secs(43_200)), "about 12 hours");
    assert_eq!(wall_time_roughly(Duration::from_secs(3_600)), "about 1 hour");
    assert_eq!(wall_time_roughly(Duration::from_secs(5 * 86_400)), "about 5 days");
}

#[test]
fn racing_there_turns_animation_off_for_the_trip_only() {
    let mut watched = Watched::demo();
    let to = SimTime::from_secs(2 * 86_400);
    assert!(matches!(watched.view.going_on(to), GoingOn::Ask { .. }));
    // Race there: the window turns animation off and goes to the time.
    watched.view.race_to();
    assert!(!watched.view.animate(), "off for the trip");
    watched.session.set_animation(false).unwrap();
    watched.session.go_to(to).unwrap();
    watched.frame();
    assert_eq!(watched.view.shown_time(), "2d 00:00:00", "raced there without the clock moving");
    assert_eq!(watched.view.status(), SessionStatus::Halted);
    assert!(watched.view.animate(), "on again once there");
    assert!(watched.view.trip_ended(), "so the window turns it on in the session too");
    assert!(!watched.view.trip_ended(), "once");
    watched.session.set_animation(true).unwrap();
    watched.session.set_speed(watched.view.speed()).unwrap();
    watched.session.start().unwrap();
    watched.at(1);
    assert_eq!(watched.view.shown_time(), "2d 00:01:00", "animated again, at the speed");
}

#[test]
fn a_raced_trip_ends_when_halted_on_the_way_and_animating_anyway_changes_nothing() {
    let mut view = ViewState::new(&demo::model());
    view.race_to();
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    assert!(!view.trip_ended());
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::ZERO);
    assert!(view.animate());
    assert!(view.trip_ended(), "Halt on the way ends the trip too");

    view.race_to();
    view.set_animate(false);
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::ZERO);
    assert!(!view.animate(), "the user chose animation off on the way, which stays");
    assert!(!view.trip_ended());

    let mut view = ViewState::new(&demo::model());
    view.observe(&SessionOutput::Status(SessionStatus::Advancing), Duration::ZERO);
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::ZERO);
    assert!(view.animate() && !view.trip_ended(), "animating anyway is a plain go to");
}

#[test]
fn a_new_model_during_a_raced_trip_animates() {
    let mut view = ViewState::new(&demo::model());
    view.race_to();
    assert!(ViewState::with_settings_of(&demo::model(), &view).animate());
}

#[test]
fn a_refused_command_shows_until_the_next_command() {
    let mut view = ViewState::new(&demo::model());
    let refused = SessionError::NotInThePast { to: SimTime::from_secs(20), shown: SimTime::from_secs(12) };
    view.observe(&SessionOutput::Error(refused), Duration::ZERO);
    assert!(view.error().is_some());
    assert!(!view.run_failed());
    view.commanded(Duration::from_secs(1));
    assert_eq!(view.error(), None);
}

#[test]
fn a_run_error_stays_and_going_back_is_still_offered() {
    let model = demo::model();
    let mut view = ViewState::new(&model);
    let mut run = model.build(0);
    run.init().unwrap();
    view.observe(&SessionOutput::State(run.state()), Duration::ZERO);
    view.observe(&SessionOutput::Status(SessionStatus::Failed), Duration::ZERO);
    let error = RunError::ZeroTimeLoop { time: SimTime::from_secs(7) };
    view.observe(&SessionOutput::Error(SessionError::Run(error)), Duration::ZERO);
    view.commanded(Duration::from_secs(1));
    assert!(view.run_failed());
    assert!(view.error().is_some(), "a run error stays until Reset or going back");
    assert!(!view.allows(Control::Start));
    assert!(!view.allows(Control::StepForward));
    assert!(view.allows(Control::StepBack));
    assert!(view.allows(Control::Reset));
    view.observe(&SessionOutput::State(run.state()), Duration::from_secs(2));
    assert!(view.error().is_some(), "a state asked for while failed changes nothing");
    // Going back or Reset reports the state, then that the session is halted again.
    view.observe(&SessionOutput::Status(SessionStatus::Halted), Duration::from_secs(2));
    assert_eq!(view.error(), None);
}
