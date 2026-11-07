//! Selecting objects, the inspector, interventions and their marks, tested without a window.

mod common;

use common::{Watched, SOURCE, STATION_1, STATION_2};
use crabsim::{demo, FailureProfile, ModelDescription, ObjectKind, Position, SimTime, Dimensions};
use crabsim::random_numbers::Distribution;
use crabsim_ui::view::{Intervention, Point, StepBackLoss, ViewState};
use std::time::Duration;

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn point(x: f64, y: f64) -> Point {
    Point { x, y }
}

/// The demo started and run to `secs`, then halted, with `id` selected.
fn halted_at(secs: u64, id: u32) -> Watched {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(secs);
    watched.session.halt().unwrap();
    watched.frame();
    watched.view.select(Some(id));
    watched
}

/// Sends an intervention as the window does: to the session, and noted in the view.
fn intervene(watched: &mut Watched, id: u32, intervention: Intervention) {
    match intervention {
        Intervention::Pause => watched.session.pause(id),
        Intervention::Resume => watched.session.resume(id),
        Intervention::ForceFailure => watched.session.force_failure(id),
        Intervention::EndForcedFailure => watched.session.end_forced_failure(id),
        Intervention::SetProcessTime(time) => watched.session.set_process_time(id, time),
    }
    .unwrap();
    let wall = watched.wall();
    watched.view.intervened(id, intervention, wall);
    watched.frame();
}

// ---------- Selecting ----------

#[test]
fn a_click_inside_an_object_finds_it_and_one_beside_it_finds_nothing() {
    let view = ViewState::new(&demo::model());
    // Station 1 is 2 x 1.5 m at (4, 0).
    assert_eq!(view.object_at(point(4.9, 0.7)), Some(STATION_1));
    assert_eq!(view.object_at(point(4.0, 0.0)), Some(STATION_1));
    assert_eq!(view.object_at(point(5.1, 0.0)), None);
    assert_eq!(view.object_at(point(2.0, 0.0)), None, "between the Source and Station 1");
}

#[test]
fn a_click_finds_a_rotated_object_by_its_rotated_outline() {
    let mut model = ModelDescription::new();
    let no_failures = FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let station = model.add_station(secs(5), no_failures);
    model.set_position(station, Position { x: 0.0, y: 0.0, z: 0.0 });
    model.set_dimensions(station, Dimensions { length: 4.0, width: 1.0, height: 1.0, rotation_degrees: 90.0 });
    let view = ViewState::new(&model);
    assert_eq!(view.object_at(point(0.0, 1.8)), Some(station), "along its length, now upright");
    assert_eq!(view.object_at(point(1.8, 0.0)), None, "across it");
}

#[test]
fn selecting_nothing_inspects_nothing() {
    let mut view = ViewState::new(&demo::model());
    assert_eq!(view.selected(), None);
    assert!(view.inspection().is_none());
    view.select(Some(STATION_1));
    assert_eq!(view.selected(), Some(STATION_1));
    view.select(None);
    assert!(view.inspection().is_none());
}

// ---------- What the inspector says ----------

#[test]
fn the_inspector_names_the_object_and_says_what_it_is_doing() {
    let watched = halted_at(12, STATION_1);
    let inspection = watched.view.inspection().unwrap();
    assert_eq!(inspection.name, "Station 1");
    assert_eq!(inspection.kind, ObjectKind::Station);
    assert_eq!(inspection.state, "Working");
    assert_eq!(inspection.contents.len(), 1);
    // Its first MU came at 5 s and takes 10 s.
    assert_eq!(inspection.remaining, Some(secs(3)));
    assert_eq!(inspection.blocked, None);
    assert!(inspection.stopped.is_empty());
}

#[test]
fn a_blocked_object_names_the_mu_it_holds_and_what_each_successor_is_doing() {
    // The Source makes an MU every 5 s, and Station 1 takes 10 s, so from 10 s the Source waits.
    let mut watched = halted_at(12, SOURCE);
    let held = watched.view.picture().object(SOURCE).unwrap().contents()[0];
    let inspection = watched.view.inspection().unwrap();
    assert_eq!(inspection.state, "Blocked");
    assert_eq!(inspection.blocked.as_deref(), Some(format!("Holding MU {held}: Station 1 is working").as_str()));

    intervene(&mut watched, STATION_1, Intervention::ForceFailure);
    let inspection = watched.view.inspection().unwrap();
    assert_eq!(
        inspection.blocked.as_deref(),
        Some(format!("Holding MU {held}: Station 1 has a forced failure").as_str())
    );
}

#[test]
fn a_stopped_object_lists_every_interruption_and_since_when() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    intervene(&mut watched, STATION_1, Intervention::Pause);
    watched.at(14);
    intervene(&mut watched, STATION_1, Intervention::ForceFailure);
    watched.at(15);
    watched.view.select(Some(STATION_1));
    let inspection = watched.view.inspection().unwrap();
    assert_eq!(inspection.state, "Stopped");
    assert_eq!(inspection.stopped, vec!["Forced failure since 00:00:14", "Pause since 00:00:12"]);
    // The work froze at 12 s, 3 s before its MU was done.
    assert_eq!(inspection.remaining, Some(secs(3)));
}

#[test]
fn details_come_from_a_full_state() {
    let mut watched = halted_at(12, STATION_1);
    let from_init = watched.view.inspection().unwrap().details.unwrap();
    assert_eq!(from_init.as_of, SimTime::ZERO, "the state from Init, until one is asked for");
    watched.details();
    let details = watched.view.inspection().unwrap().details.unwrap();
    assert_eq!(details.as_of, secs(12));
    assert_eq!(details.process_time, Some(secs(10)));
    assert_eq!(details.configured_process_time, Some(secs(10)));
    assert_eq!(details.waiting, vec!["Source"], "the Source waits for Station 1");
    assert_eq!(details.statistics.mus_in, 1);
    assert_eq!(details.statistics_since, SimTime::ZERO);
}

// ---------- Asking for full states ----------

#[test]
fn full_states_are_asked_for_only_while_something_is_selected() {
    let mut watched = halted_at(12, STATION_1);
    watched.view.select(None);
    assert!(!watched.view.wants_state(watched.wall()));
    watched.view.select(Some(STATION_1));
    assert!(watched.view.wants_state(watched.wall()));
}

#[test]
fn while_halted_a_state_is_asked_for_once_and_again_only_after_a_change() {
    let mut watched = halted_at(12, STATION_1);
    watched.details();
    let later = watched.wall() + Duration::from_secs(10);
    assert!(!watched.view.wants_state(later), "nothing has changed, so nothing is drawn");
    intervene(&mut watched, STATION_1, Intervention::SetProcessTime(secs(20)));
    assert!(watched.view.wants_state(watched.wall() + Duration::from_secs(1)));
    watched.details();
    let details = watched.view.inspection().unwrap().details.unwrap();
    assert_eq!(details.process_time, Some(secs(20)));
    assert_eq!(details.configured_process_time, Some(secs(10)));
}

#[test]
fn while_advancing_a_state_is_asked_for_twice_a_second() {
    let mut watched = Watched::demo();
    watched.view.select(Some(STATION_1));
    watched.session.start().unwrap();
    watched.at(1);
    watched.details();
    let asked = watched.wall();
    watched.at(2);
    assert!(watched.view.wants_state(asked + Duration::from_millis(500)));
    assert!(!watched.view.wants_state(asked + Duration::from_millis(300)));
}

#[test]
fn a_state_asked_for_doesnt_restart_measuring_the_speed_reached() {
    let mut watched = Watched::demo();
    watched.view.select(Some(STATION_1));
    watched.view.set_speed(120.0);
    watched.session.start().unwrap();
    watched.frame();
    for second in 1..=4 {
        watched.at(second);
        watched.details();
    }
    // The session runs at its own speed, 1, while the view asked for 120, so it reads as behind.
    assert_eq!(watched.view.achieved_speed(), Some(1.0));
}

// ---------- Interventions ----------

#[test]
fn interventions_are_offered_on_stations_once_initialised() {
    let mut watched = Watched::demo();
    watched.view.select(Some(STATION_1));
    assert!(watched.view.inspection().unwrap().interventions.is_none(), "Init first");
    watched.session.step_forward().unwrap();
    watched.frame();
    let offered = watched.view.inspection().unwrap().interventions.unwrap();
    assert!(!offered.paused && !offered.forced);
    watched.view.select(Some(SOURCE));
    assert!(watched.view.inspection().unwrap().interventions.is_none(), "only a Station so far");
}

#[test]
fn the_inspector_shows_which_interventions_are_on() {
    let mut watched = halted_at(12, STATION_2);
    intervene(&mut watched, STATION_2, Intervention::Pause);
    let offered = watched.view.inspection().unwrap().interventions.unwrap();
    assert!(offered.paused && !offered.forced);
}

#[test]
fn each_intervention_leaves_a_mark_at_the_time_it_took_effect() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    intervene(&mut watched, STATION_1, Intervention::Pause);
    watched.at(20);
    intervene(&mut watched, STATION_1, Intervention::Resume);
    watched.at(30);
    let marks: Vec<(SimTime, String)> =
        watched.view.marks().iter().map(|mark| (mark.at, watched.view.describe(mark))).collect();
    assert_eq!(
        marks,
        vec![(secs(12), "Pause on Station 1".to_string()), (secs(20), "Resume on Station 1".to_string())]
    );
}

#[test]
fn going_back_counts_the_marks_it_would_drop_and_drops_them() {
    let mut watched = Watched::demo();
    watched.session.start().unwrap();
    watched.at(12);
    intervene(&mut watched, STATION_1, Intervention::Pause);
    watched.at(20);
    intervene(&mut watched, STATION_1, Intervention::Resume);
    watched.at(30);
    assert_eq!(watched.view.lost_going_back(secs(30)), 0);
    assert_eq!(watched.view.lost_going_back(secs(20)), 0, "what happened at the time gone back to stays");
    assert_eq!(watched.view.lost_going_back(secs(15)), 1);
    assert_eq!(watched.view.lost_going_back(secs(10)), 2);
    watched.session.go_back(secs(15)).unwrap();
    watched.frame();
    assert_eq!(watched.view.marks().len(), 1);
    watched.session.reset().unwrap();
    watched.frame();
    assert!(watched.view.marks().is_empty(), "Reset drops every intervention");
}

#[test]
fn stepping_back_counts_the_marks_after_the_instant_before_the_one_on_screen() {
    // The Pause makes 12 s an instant, and the one before it is the Source's at 10 s.
    let mut watched = halted_at(12, STATION_1);
    intervene(&mut watched, STATION_1, Intervention::Pause);
    assert_eq!(watched.view.lost_stepping_back(), StepBackLoss::Exactly(1));
    watched.session.step_back().unwrap();
    watched.frame();
    assert_eq!(watched.view.shown_time(), "00:00:10");
    assert!(watched.view.marks().is_empty());
    // The view saw the instant now on screen, but not the one before it.
    intervene(&mut watched, STATION_1, Intervention::Pause);
    assert_eq!(watched.view.lost_stepping_back(), StepBackLoss::UpTo(1));
}

