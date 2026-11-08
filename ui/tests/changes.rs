//! Picking up changes to the open model: an agent's edit becomes a new model for the window, a file
//! written outside crabsim is marked and can be adopted, and a file that now has problems shows them
//! while the model open keeps running. Whether the window rebuilds at once or on Reset is the view
//! state's to say. No window, as in the other UI tests.

mod common;

use common::Watched;
use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::workspace::{ModelChange, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LINE: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","position":{"x":5,"y":0},"successors":["Out"],"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain","position":{"x":10,"y":0}}]}"#;

const BROKEN: &str = r#"{ "format_version": 1, "instances": [ { "name": "In", "class": "Source", "successors": ["Nowhere"] } ] }"#;

/// A models folder holding `line.json` and `other.json`, both saved by crabsim, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-changes-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        for file in ["line.json", "other.json"] {
            user.call("write_model", json!({ "path": file, "text": LINE })).unwrap();
        }
        Folder(path)
    }

    /// Writes a file past crabsim, as a text editor would.
    fn write_by_hand(&self, relative: &str, text: &str) {
        std::fs::write(self.0.join(relative), text).unwrap();
    }

    fn agent(&self) -> Catalogue {
        Catalogue::open(Settings::new(&self.0, Source::Mcp)).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder, path: &str) -> Workspace {
    let mut workspace = Workspace::open(Some(&folder.0));
    assert!(workspace.open_model(path), "{path} opens");
    workspace
}

fn add_spare(agent: &Catalogue) {
    agent.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "add_instance", "name": "Spare", "class": "Drain" }] })).unwrap();
}

#[test]
fn nothing_changed_is_nothing_to_report() {
    let folder = Folder::new("quiet");
    let mut workspace = open(&folder, "line.json");
    assert_eq!(workspace.check(), None);
    assert_eq!(workspace.check(), None);
    assert!(!workspace.has_pending());
    assert!(workspace.watched_file().is_some_and(|file| file.ends_with("line.json")));
}

#[test]
fn an_agents_edit_becomes_a_new_model_waiting_to_be_used() {
    let folder = Folder::new("agent-edit");
    let mut workspace = open(&folder, "line.json");
    add_spare(&folder.agent());
    assert_eq!(workspace.check(), Some(ModelChange::Changed));
    assert!(workspace.has_pending());
    assert_eq!(workspace.description().object_count(), 3, "the model in use stays until the window uses the new one");
    assert_eq!(workspace.check(), None, "reported once");

    workspace.apply_pending();
    assert!(!workspace.has_pending());
    let model = workspace.description();
    assert_eq!(model.object_count(), 4);
    assert!(model.position(3).is_some(), "the new instance is laid out");
    assert!(!workspace.changed_outside(), "crabsim saved it");
}

#[test]
fn a_file_written_again_unchanged_is_no_change() {
    let folder = Folder::new("touched");
    let mut workspace = open(&folder, "line.json");
    let text = std::fs::read_to_string(folder.0.join("line.json")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    folder.write_by_hand("line.json", &text);
    assert_eq!(workspace.check(), None, "the same bytes, so the same hash");
    assert!(!workspace.has_pending());
}

#[test]
fn a_file_that_now_has_problems_shows_them_and_the_model_keeps_running() {
    let folder = Folder::new("broken");
    let mut workspace = open(&folder, "line.json");
    folder.write_by_hand("line.json", BROKEN);
    assert_eq!(workspace.check(), Some(ModelChange::NowHasProblems));
    assert!(!workspace.has_pending());
    assert_eq!(workspace.opened(), Some("line.json"));
    assert_eq!(workspace.description().object_count(), 3, "the model in use stays");
    let report = workspace.problems().expect("its problems are shown");
    assert_eq!(report.path, "line.json");
    assert!(report.problems.iter().any(|problem| problem.contains("Nowhere")), "{:?}", report.problems);
    assert_eq!(workspace.check(), None, "reported once");
    std::thread::sleep(std::time::Duration::from_millis(20));
    folder.write_by_hand("line.json", BROKEN);
    assert_eq!(workspace.check(), None, "written again with the same problems, so nothing new");

    folder.write_by_hand("line.json", LINE);
    assert_eq!(workspace.check(), Some(ModelChange::Changed), "fixed, it can be used again");
    assert_eq!(workspace.problems(), None);
}

#[test]
fn a_file_changed_outside_crabsim_is_marked_until_adopted() {
    let folder = Folder::new("outside");
    let mut workspace = open(&folder, "line.json");
    assert!(!workspace.changed_outside());
    folder.write_by_hand("line.json", &LINE.replace("\"8\"", "\"9\""));
    assert_eq!(workspace.check(), Some(ModelChange::Changed));
    assert!(workspace.changed_outside(), "written past crabsim");
    let refused = folder.agent().call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "add_instance", "name": "Spare", "class": "Drain" }] }));
    assert!(refused.is_err(), "agents can't change it until it's adopted");

    workspace.adopt().unwrap();
    assert!(!workspace.changed_outside());
    assert_eq!(workspace.check(), None, "adopting changes no file");
    add_spare(&folder.agent());
    assert_eq!(workspace.check(), Some(ModelChange::Changed));
}

#[test]
fn opening_another_model_drops_a_change_waiting_to_be_used() {
    let folder = Folder::new("switch");
    let mut workspace = open(&folder, "line.json");
    add_spare(&folder.agent());
    assert_eq!(workspace.check(), Some(ModelChange::Changed));
    assert!(workspace.open_model("other.json"));
    assert!(!workspace.has_pending());
    assert!(workspace.watched_file().is_some_and(|file| file.ends_with("other.json")));
}

#[test]
fn a_changed_model_is_used_at_once_only_when_no_run_is_in_progress() {
    let mut watched = Watched::demo();
    assert!(watched.view.reloads_now(), "before Init");
    watched.session.start().unwrap();
    watched.at(10);
    assert!(!watched.view.reloads_now(), "while advancing");
    watched.session.halt().unwrap();
    watched.frame();
    assert!(!watched.view.reloads_now(), "halted part way through a run");
    watched.view.set_model_changed(true);
    assert!(watched.view.model_changed(), "the window says Reset will use the new model");
    watched.session.reset().unwrap();
    watched.frame();
    assert!(watched.view.reloads_now(), "after Reset");
}
