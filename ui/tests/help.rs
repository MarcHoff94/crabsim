//! What the window says about states: the help for an object's type with the model's own notes,
//! one colour per state, and the name and state shown on hovering an object. No window.

mod common;

use common::{Watched, STATION_1};
use crabsim::ObjectKind;
use crabsim_model::State;
use crabsim_ui::help::states;
use crabsim_ui::view::{rgb, Shade};
use serde_json::json;

#[test]
fn the_help_lists_the_states_an_objects_type_can_be_in_each_with_what_it_means() {
    let station: Vec<&str> = states(ObjectKind::Station, None).iter().map(|help| help.state.label()).collect();
    assert_eq!(station, ["Busy", "Blocked", "Idle", "Failed", "Forced failure", "Paused"]);
    let source: Vec<State> = states(ObjectKind::Source, None).iter().map(|help| help.state).collect();
    assert_eq!(source, [State::Busy, State::Blocked]);
    let drain: Vec<State> = states(ObjectKind::Drain, None).iter().map(|help| help.state).collect();
    assert_eq!(drain, [State::Idle]);
    for kind in [ObjectKind::Source, ObjectKind::Station, ObjectKind::Buffer, ObjectKind::Drain] {
        assert!(states(kind, None).iter().all(|help| !help.meaning.is_empty() && help.note.is_none()), "{kind:?}");
    }
    let busy = |kind| states(kind, None)[0].meaning;
    assert_ne!(busy(ObjectKind::Source), busy(ObjectKind::Station), "what Busy means depends on the type");
}

#[test]
fn the_models_own_notes_sit_beside_crabsims_meaning_never_instead_of_it() {
    let notes = json!({ "blocked": "waits for the conveyor to clear" });
    let help = states(ObjectKind::Station, Some(&notes));
    let blocked = help.iter().find(|help| help.state == State::Blocked).unwrap();
    assert_eq!(blocked.note.as_deref(), Some("waits for the conveyor to clear"));
    assert_eq!(blocked.meaning, states(ObjectKind::Station, None)[1].meaning);
    assert!(help.iter().filter(|help| help.state != State::Blocked).all(|help| help.note.is_none()));
}

#[test]
fn every_state_has_its_own_colour_and_a_forced_failure_is_a_darker_red_than_a_failure() {
    let shades = [Shade::Unknown, Shade::Idle, Shade::Busy, Shade::Blocked, Shade::Paused, Shade::Failed, Shade::ForcedFailure];
    for (i, a) in shades.iter().enumerate() {
        for b in &shades[i + 1..] {
            assert_ne!(rgb(*a), rgb(*b), "{a:?} and {b:?}");
        }
    }
    let [failed_r, failed_g, failed_b] = rgb(Shade::Failed);
    let [forced_r, forced_g, forced_b] = rgb(Shade::ForcedFailure);
    assert!(forced_r < failed_r && forced_r > 2.0 * forced_g && forced_r > 2.0 * forced_b, "dark red");
    assert!(failed_r > 2.0 * failed_g && failed_r > 2.0 * failed_b, "red");
    for state in State::ALL {
        assert_ne!(Shade::of(state), Shade::Unknown);
    }
}

#[test]
fn hovering_an_object_names_it_and_its_state() {
    let mut watched = Watched::demo();
    watched.frame();
    assert_eq!(watched.view.hover_text(STATION_1), "Station 1: no state yet, before Init");
    watched.session.start().unwrap();
    watched.at(12);
    assert_eq!(watched.view.hover_text(STATION_1), "Station 1: Busy");
    watched.session.force_failure(STATION_1).unwrap();
    watched.frame();
    assert_eq!(watched.view.hover_text(STATION_1), "Station 1: Forced failure");
}
