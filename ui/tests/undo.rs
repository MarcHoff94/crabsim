//! Undo and redo in the window (spec issue 107), through the workspace on a real catalogue, with
//! an agent played by a second catalogue: what a step restores and touches, what is asked first,
//! where a click on the history goes, and one step for everything one command did. No window, as
//! in the other UI tests.

use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::history::{summary, touched, Entry, History, Jump, Kind};
use crabsim_ui::workspace::{Refusal, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"10"}},
  {"name":"Mill","class":"Station","position":{"x":5,"y":0},"attributes":{"process_time":"0:45"}},
  {"name":"Drill","class":"Station","position":{"x":5,"y":4},"attributes":{"process_time":"0:30"}},
  {"name":"Out","class":"Drain","position":{"x":10,"y":0}}],"connections":[
  {"from":"In","to":"Mill","exit":1,"entrance":1},
  {"from":"Mill","to":"Out","exit":1,"entrance":1}]}"#;

/// A models folder holding `line.json`, saved by crabsim, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-undo-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        Folder(path)
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.0.join("line.json")).unwrap()
    }

    /// An agent holding `line.json`.
    fn agent(&self) -> Catalogue {
        let agent = Catalogue::open(Settings::new(&self.0, Source::Mcp)).unwrap();
        agent.set_client_name("test-harness");
        agent.call("claim_model", json!({ "path": "line.json", "goal": "a faster line", "doing": "tuning the mill" })).unwrap();
        agent
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

/// Takes the changed file in, as the window does on its next frame.
fn pick_up(workspace: &mut Workspace) {
    workspace.check();
    workspace.apply_pending();
}

fn whats(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.what.as_str()).collect()
}

fn entry(number: i64, group: i64, undone: bool, yours: bool) -> Entry {
    Entry { number, author: if yours { "the user".into() } else { "agent-1 (x)".into() }, at: "2026-11-13T10:00:00Z".into(), what: format!("change {number}"), kind: Kind::Change, group, undone, yours }
}

// ---------- The workspace ----------

#[test]
fn undo_takes_the_last_change_back_and_says_what_it_touched_and_redo_brings_it_back() {
    let folder = Folder::new("steps");
    let mut workspace = open(&folder);
    let before = folder.text();
    workspace.set_attribute("Mill", "process_time", json!("0:20"), false).unwrap();
    pick_up(&mut workspace);
    let changed = folder.text();

    let undone = workspace.undo(1, false).unwrap();
    assert_eq!(folder.text(), before);
    assert_eq!(whats(&undone.entries), ["set Mill.process_time 0:20"]);
    assert_eq!(undone.touched, ["Mill"], "what the change touched is selected");
    pick_up(&mut workspace);

    let redone = workspace.redo(1, false).unwrap();
    assert_eq!(folder.text(), changed);
    assert_eq!(redone.touched, ["Mill"]);
    assert_eq!(whats(&redone.entries), ["set Mill.process_time 0:20"]);
}

#[test]
fn a_step_touches_the_ends_of_connections_and_what_came_back() {
    let folder = Folder::new("touched");
    let mut workspace = open(&folder);
    workspace.connect("In", "Drill", false).unwrap();
    pick_up(&mut workspace);
    assert_eq!(workspace.undo(1, false).unwrap().touched, ["In", "Drill"], "both ends of the connection that went");
    pick_up(&mut workspace);

    workspace.remove_instance("Mill", false).unwrap();
    pick_up(&mut workspace);
    let undone = workspace.undo(1, false).unwrap();
    assert_eq!(undone.touched, ["In", "Mill", "Out"], "the instance that came back, and the ends of its connections");
    pick_up(&mut workspace);

    // Taking back a placed instance leaves nothing to select.
    workspace.run_commands("add Station as New at 20 0", None, (0.0, 0.0), false).unwrap();
    pick_up(&mut workspace);
    assert!(workspace.undo(1, false).unwrap().touched.is_empty());
}

#[test]
fn with_nothing_to_undo_or_redo_it_says_so_and_changes_nothing() {
    let folder = Folder::new("nothing");
    let mut workspace = open(&folder);
    let before = folder.text();
    assert!(matches!(workspace.undo(1, false), Err(Refusal::Nothing(_))));
    assert!(matches!(workspace.redo(1, false), Err(Refusal::Nothing(_))));
    assert_eq!(folder.text(), before);
    assert!(workspace.history().unwrap().entries.is_empty());
}

#[test]
fn the_history_lists_who_changed_what_newest_first() {
    let folder = Folder::new("history");
    let mut workspace = open(&folder);
    workspace.set_attribute("Mill", "process_time", json!("0:20"), false).unwrap();
    let agent = folder.agent();
    agent.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "connect", "from": "In", "to": "Drill" }] })).unwrap();

    let history = workspace.history().unwrap();
    assert_eq!(whats(&history.entries), ["connect In > Drill", "set Mill.process_time 0:20"]);
    assert_eq!(history.entries[0].author, "agent-1 (test-harness)");
    assert!(!history.entries[0].yours && history.entries[1].yours);
    assert!(history.entries[0].asks() && !history.entries[1].asks());
    assert!(!history.more);
    assert_eq!(history.entries[0].line(), format!("connect In > Drill (agent-1 (test-harness), {})", history.entries[0].at));
}

#[test]
fn undoing_past_an_agents_claim_is_refused_until_it_is_a_take_over() {
    let folder = Folder::new("take-over");
    let mut workspace = open(&folder);
    let before = folder.text();
    let agent = folder.agent();
    agent.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "connect", "from": "In", "to": "Drill" }] })).unwrap();
    pick_up(&mut workspace);

    let refused = workspace.undo(1, false).unwrap_err();
    assert!(matches!(&refused, Refusal::NotEditor(why) if why.contains("agent-1")), "{refused:?}");
    assert_ne!(folder.text(), before);

    let undone = workspace.undo(1, true).unwrap();
    assert_eq!(folder.text(), before);
    assert!(undone.entries[0].asks());
    // The take-over was for the one step: the agent still holds the file.
    assert!(matches!(workspace.redo(1, false), Err(Refusal::NotEditor(_))));
}

#[test]
fn a_file_changed_outside_is_refused_until_adopted_and_then_asked_about() {
    let folder = Folder::new("outside");
    let mut workspace = open(&folder);
    workspace.set_attribute("Drill", "process_time", json!("0:31"), false).unwrap();
    pick_up(&mut workspace);
    let saved = folder.text();
    assert!(saved.contains("00:00:45"));
    std::fs::write(folder.0.join("line.json"), saved.replace("00:00:45", "00:00:50")).unwrap();
    pick_up(&mut workspace);
    assert!(matches!(workspace.undo(1, false), Err(Refusal::ChangedOutside(_))));

    workspace.adopt().unwrap();
    let history = workspace.history().unwrap();
    assert_eq!(history.entries[0].kind, Kind::Adopted);
    assert!(history.entries[0].asks(), "a change crabsim can't describe is asked about, though the user adopted it");
    workspace.undo(1, false).unwrap();
    assert_eq!(folder.text(), saved);
}

#[test]
fn everything_one_command_did_is_one_step() {
    let folder = Folder::new("group");
    let mut workspace = open(&folder);
    let before = folder.text();
    // As the window does around a macro that holds window commands, whose lines go one a frame.
    workspace.begin_group();
    workspace.run_commands("add Station as A at 20 0", None, (0.0, 0.0), false).unwrap();
    pick_up(&mut workspace);
    workspace.run_commands("add Station as B at 24 0; connect A > B", None, (0.0, 0.0), false).unwrap();
    workspace.end_group();
    pick_up(&mut workspace);
    workspace.set_attribute("A", "process_time", json!("0:20"), false).unwrap();
    pick_up(&mut workspace);

    let history = workspace.history().unwrap();
    assert_eq!(history.undo_steps(1).len(), 1);
    assert_eq!(history.undo_steps(2).len(), 3, "the macro's two lines are one step");
    assert_eq!(workspace.undo(2, false).unwrap().entries.len(), 3);
    assert_eq!(folder.text(), before);
}

// ---------- What a number of steps takes, and where a click goes ----------

#[test]
fn steps_are_counted_by_group_and_only_through_what_is_in_effect() {
    // Newest first: 6 and 5 undone, 4 alone, 3 and 2 one command, 1 alone.
    let history = History { entries: vec![entry(6, 6, true, true), entry(5, 5, true, true), entry(4, 4, false, true), entry(3, 2, false, false), entry(2, 2, false, true), entry(1, 1, false, true)], more: false };
    let numbers = |entries: Vec<&Entry>| entries.iter().map(|entry| entry.number).collect::<Vec<_>>();
    assert_eq!(numbers(history.undo_steps(1)), [4]);
    assert_eq!(numbers(history.undo_steps(2)), [4, 3, 2]);
    assert_eq!(numbers(history.undo_steps(9)), [4, 3, 2, 1]);
    assert_eq!(numbers(history.redo_steps(1)), [5], "redo brings the oldest undone change back first");
    assert_eq!(numbers(history.redo_steps(9)), [5, 6]);
    // Only someone else's change is asked about.
    assert_eq!(history.undo_steps(1).iter().filter(|entry| entry.asks()).count(), 0);
    assert_eq!(numbers(history.undo_steps(2).into_iter().filter(|entry| entry.asks()).collect()), [3]);
}

#[test]
fn a_click_on_an_entry_goes_back_or_on_to_the_model_as_it_was_after_it() {
    let history = History { entries: vec![entry(6, 6, true, true), entry(5, 5, true, true), entry(4, 4, false, true), entry(3, 2, false, false), entry(2, 2, false, true), entry(1, 1, false, true)], more: false };
    assert_eq!(history.jump_to(4), Some(Jump::Here));
    assert_eq!(history.jump_to(3), Some(Jump::Undo(1)), "an entry inside a step counts as its whole step");
    assert_eq!(history.jump_to(2), Some(Jump::Undo(1)));
    assert_eq!(history.jump_to(1), Some(Jump::Undo(2)));
    assert_eq!(history.jump_to(5), Some(Jump::Redo(1)));
    assert_eq!(history.jump_to(6), Some(Jump::Redo(2)));
    assert_eq!(history.jump_to(99), None);
}

#[test]
fn the_foot_line_says_what_a_step_did() {
    let one = [entry(4, 4, false, true)];
    assert_eq!(summary("Undone", &one), "Undone: change 4");
    let three = [entry(4, 4, false, true), entry(3, 2, false, true), entry(2, 2, false, true)];
    assert_eq!(summary("Undone", &three), "Undone 3 changes, the last: change 2");
    assert_eq!(summary("Redone", &one), "Redone: change 4");
}

#[test]
fn touched_names_only_what_is_still_there() {
    let before = crabsim_model::load(LINE).unwrap();
    let mut after = before.clone();
    after.instances.retain(|instance| instance.name != "Drill");
    assert!(touched(&before, &after).is_empty());
    assert_eq!(touched(&after, &before), ["Drill"]);
    assert!(touched(&before, &before).is_empty());
    // An instance that went with its connections: the ends still there are what it touched.
    let mut without_mill = before.clone();
    without_mill.instances.retain(|instance| instance.name != "Mill");
    without_mill.connections.clear();
    assert_eq!(touched(&before, &without_mill), ["In", "Out"]);
}
