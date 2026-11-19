//! Object windows (spec issue 116): what each keeps about its own instance, how the open ones
//! follow their instances through renames and removals, the dock's key figures and the keys that
//! open and close windows. No window, as in the other UI tests.

mod common;

use common::{Watched, STATION_1, STATION_2};
use crabsim::{demo, ObjectKind};
use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::inspector::{AttributeChange, Inspector, Inspectors, Section};
use crabsim_ui::keys::{Command, Key, Keys};
use crabsim_ui::view::ViewState;
use crabsim_ui::workspace::{Refusal, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","stream":11,"position":{"x":0,"y":0},"attributes":{"interval":"10"}},
  {"name":"Mill","class":"Station","stream":12,"position":{"x":5,"y":0},"attributes":{"process_time":"0:45"}},
  {"name":"Press","class":"Station","stream":13,"position":{"x":10,"y":0},"attributes":{"process_time":"0:30"}},
  {"name":"Out","class":"Drain","stream":14,"position":{"x":15,"y":0}}],"connections":[
  {"from":"In","to":"Mill","exit":1,"entrance":1},
  {"from":"Mill","to":"Press","exit":1,"entrance":1},
  {"from":"Press","to":"Out","exit":1,"entrance":1}]}"#;

struct Folder(PathBuf);

impl Folder {
    fn new() -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-windows-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    workspace
}

fn id_of(view: &ViewState, name: &str) -> u32 {
    view.objects().iter().find(|object| object.name == name).unwrap().id
}

// ---------- Sections ----------

#[test]
fn sections_open_by_situation_and_stay_as_the_user_left_them() {
    let mut window = Inspector::new(1);
    // Before Init or after Reset: attributes and actions, not statistics.
    assert!(window.is_open(Section::Attributes, false) && window.is_open(Section::Actions, false));
    assert!(!window.is_open(Section::Statistics, false) && !window.is_open(Section::Interventions, false));
    // During a run: the other way round.
    assert!(window.is_open(Section::Statistics, true) && window.is_open(Section::Interventions, true));
    assert!(!window.is_open(Section::Attributes, true));
    assert!(window.is_open(Section::Now, false) && window.is_open(Section::Now, true), "what is happening now is always open");
    assert!(!window.is_open(Section::States, false), "help on states only when asked for");
    // What the user chose stays, in either situation.
    window.set_open(Section::Attributes, false);
    window.set_open(Section::Statistics, false);
    assert!(!window.is_open(Section::Attributes, false));
    assert!(!window.is_open(Section::Statistics, true));
    window.set_open(Section::States, true);
    assert!(window.is_open(Section::States, true) && window.is_open(Section::States, false));
}

#[test]
fn fields_belong_to_their_section_and_their_window() {
    assert_eq!(Section::of_field("name"), Section::Actions);
    assert_eq!(Section::of_field("pause"), Section::Interventions);
    assert_eq!(Section::of_field("process_time"), Section::Interventions);
    assert_eq!(Section::of_field("capacity"), Section::Attributes);
    assert_ne!(Section::focus_key(1, "name"), Section::focus_key(2, "name"), "the same field in two windows is two fields");
}

// ---------- Windows ----------

#[test]
fn one_window_per_instance_and_opening_again_brings_it_forward() {
    let mut windows = Inspectors::default();
    assert!(windows.open(12), "newly opened");
    assert!(windows.open(13));
    assert!(!windows.open(12), "already open");
    assert_eq!(windows.streams(), vec![12, 13]);
    assert_eq!(windows.take_raise(), Some(12), "the one opened again comes forward");
    assert_eq!(windows.take_raise(), None, "once");
    windows.close(12);
    assert_eq!(windows.streams(), vec![13]);
    windows.close_all();
    assert!(windows.is_empty());
}

#[test]
fn a_window_follows_its_instance_through_a_rename_and_the_removal_of_an_earlier_one() {
    let folder = Folder::new();
    let mut workspace = open(&folder);
    let view = ViewState::new(workspace.description());
    let mut windows = Inspectors::default();
    windows.open(view.objects()[id_of(&view, "Press") as usize].stream);
    windows.open(view.objects()[id_of(&view, "Out") as usize].stream);
    assert_eq!(windows.follow(&view), vec![id_of(&view, "Press"), id_of(&view, "Out")]);

    workspace.rename_instance("Press", "Brake", false).unwrap();
    workspace.remove_instance("Mill", false).unwrap();
    assert!(workspace.open_model("line.json"));
    let view = ViewState::new(workspace.description());
    assert_eq!(view.objects().len(), 3, "the Mill is gone, so every later id shifted");
    let followed = windows.follow(&view);
    assert_eq!(followed, vec![id_of(&view, "Brake"), id_of(&view, "Out")], "found by stream: the same instances under their new ids");
    assert_eq!(view.objects()[followed[0] as usize].name, "Brake");

    workspace.remove_instance("Brake", false).unwrap();
    assert!(workspace.open_model("line.json"));
    let view = ViewState::new(workspace.description());
    assert_eq!(windows.follow(&view), vec![id_of(&view, "Out")]);
    assert_eq!(windows.len(), 1, "the window of a removed instance closes");
}

#[test]
fn a_window_reads_its_own_instances_values_once_until_told_to_forget_them() {
    let folder = Folder::new();
    let workspace = open(&folder);
    let mut mill = Inspector::new(12);
    let mut press = Inspector::new(13);
    mill.read(&workspace, "Mill", "Station");
    press.read(&workspace, "Press", "Station");
    let process_time = |window: &Inspector| {
        let attributes = window.attributes.as_ref().unwrap().as_ref().unwrap();
        attributes.iter().find(|view| view.attribute == "process_time").unwrap().value.clone().unwrap()
    };
    assert_eq!(process_time(&mill), json!("00:00:45"));
    assert_eq!(process_time(&press), json!("00:00:30"), "each window has its own instance's value");
    assert!(mill.forms.contains_key("process_time") && press.forms.contains_key("process_time"));

    mill.forms.clear();
    mill.read(&workspace, "Mill", "Station");
    assert!(mill.forms.is_empty(), "already read, so not read again");
    mill.forget_values();
    assert!(mill.attributes.is_none());
    mill.read(&workspace, "Mill", "Station");
    assert!(mill.forms.contains_key("process_time"));
}

#[test]
fn a_refusal_stays_with_the_window_that_asked() {
    let mut mill = Inspector::new(12);
    let press = Inspector::new(13);
    let change = AttributeChange::Set("process_time".to_string(), json!("0:20"));
    mill.refuse(&change, Refusal::NotEditor("an agent holds it".to_string()));
    let (attribute, _, again) = mill.refused.as_ref().unwrap();
    assert_eq!(attribute, "process_time");
    assert_eq!(again.as_ref(), Some(&change), "an agent holds the file, so the change can be made anyway");
    assert!(press.refused.is_none(), "the other window knows nothing of it");

    let mut other = Inspector::new(14);
    other.refuse(&change, Refusal::Invalid("not a time".to_string()));
    assert!(other.refused.as_ref().unwrap().2.is_none(), "an invalid value can't be made anyway");
    other.forget_values();
    assert!(other.refused.is_none());
}

#[test]
fn the_name_field_keeps_what_was_typed_until_the_name_changes() {
    let mut window = Inspector::new(12);
    window.sync_name("Mill");
    window.name_text.1 = "Gri".to_string();
    window.sync_name("Mill");
    assert_eq!(window.name_text.1, "Gri", "typing carries on");
    window.sync_name("Grinder");
    assert_eq!(window.name_text.1, "Grinder", "renamed elsewhere: the field shows the new name");
}

// ---------- The dock's summary ----------

#[test]
fn key_figures_by_type_and_none_before_a_full_state() {
    let mut watched = Watched::demo();
    watched.view.watch(&[0, STATION_1, STATION_2, 3]);
    assert!(watched.view.inspection_of(STATION_1).unwrap().key_figures().is_empty(), "no full state yet");
    watched.session.start().unwrap();
    watched.at(600);
    watched.session.halt().unwrap();
    watched.frame();
    watched.details();
    let labels = |id: u32| -> Vec<&'static str> { watched.view.inspection_of(id).unwrap().key_figures().into_iter().map(|(label, _)| label).collect() };
    let kind = |id: u32| watched.view.inspection_of(id).unwrap().kind;
    assert_eq!(kind(0), ObjectKind::Source);
    assert_eq!(labels(0), vec!["MUs out", "Blocked"]);
    assert_eq!(labels(STATION_1), vec!["Busy", "Blocked", "Idle"], "capacity 1: Busy");
    let drain = (0..watched.view.objects().len() as u32).find(|&id| kind(id) == ObjectKind::Drain).unwrap();
    assert_eq!(labels(drain), vec!["Throughput", "Lead time"]);
    let figures = watched.view.inspection_of(STATION_1).unwrap().key_figures();
    assert!(figures.iter().all(|(_, value)| value.ends_with('%')), "shares of time: {figures:?}");
}

#[test]
fn a_station_with_capacity_gives_busy_full_and_a_buffer_its_fill() {
    let model = demo::buffered_line();
    let mut watched = {
        let view = ViewState::new(&model);
        let clock = common::FakeClock::default();
        let session = crabsim::LiveSession::new(move || model.build(0), clock.clone());
        Watched { session, clock, view }
    };
    let ids: Vec<u32> = (0..watched.view.objects().len() as u32).collect();
    watched.view.watch(&ids);
    watched.session.start().unwrap();
    watched.at(3_600);
    watched.session.halt().unwrap();
    watched.frame();
    watched.details();
    let mut seen_buffer = false;
    let mut seen_capacity_station = false;
    for &id in &ids {
        let inspection = watched.view.inspection_of(id).unwrap();
        let labels: Vec<&str> = inspection.key_figures().into_iter().map(|(label, _)| label).collect();
        match inspection.kind {
            ObjectKind::Buffer => {
                seen_buffer = true;
                assert_eq!(&labels[..2], ["Mean MUs", "Most MUs"]);
                assert!(matches!(labels.last(), Some(&"Full") | Some(&"Empty")), "{labels:?}");
                assert_eq!(labels.len(), 3);
            }
            ObjectKind::Station if labels.first() == Some(&"Busy, full") => {
                seen_capacity_station = true;
                assert_eq!(labels, vec!["Busy, full", "Blocked", "Idle"]);
            }
            _ => {}
        }
    }
    assert!(seen_buffer, "the buffered line has Buffers");
    assert!(seen_capacity_station, "and a Station with capacity");
}

// ---------- Keys ----------

#[test]
fn w_opens_the_selections_window_and_w_and_gw_close_windows() {
    let mut keys = Keys::default();
    assert_eq!(keys.press(Key::Char('w')), vec![Command::OpenWindow]);
    assert_eq!(keys.press(Key::Char('W')), vec![Command::CloseWindow]);
    assert_eq!(keys.press(Key::Char('g')), Vec::<Command>::new(), "g waits for its second key");
    assert_eq!(keys.press(Key::Char('W')), vec![Command::CloseAllWindows]);
    // gg still goes to the first object.
    keys.press(Key::Char('g'));
    assert_eq!(keys.press(Key::Char('g')), vec![Command::First]);
    // In a prompt they are letters.
    keys.press(Key::Char(':'));
    assert_eq!(keys.press(Key::Char('w')), vec![Command::Prompted(crabsim_ui::keys::PromptKind::Command, "w".to_string())]);
    keys.press(Key::Escape);
    // A count before w is no count of anything: it opens the window.
    keys.press(Key::Char('3'));
    assert_eq!(keys.press(Key::Char('w')), vec![Command::OpenWindow]);
}
