//! The change history, undo and redo (spec issue 107, ADR 0017), through `call(name, JSON)`: the
//! user and agents on one models folder, each a catalogue of its own.

mod common;

use common::TempFolder;
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Work","exit":1,"entrance":1},
  {"from":"Work","to":"Out","exit":1,"entrance":1}]}"#;

/// A models folder holding `line.json`, saved by crabsim as the user.
struct Folder {
    folder: TempFolder,
    changes: usize,
    bytes: u64,
}

struct Who {
    catalogue: Catalogue,
}

impl Folder {
    fn new(name: &str) -> Folder {
        Folder::with_limits(name, 200, 50 * 1024 * 1024)
    }

    fn with_limits(name: &str, changes: usize, bytes: u64) -> Folder {
        let folder = Folder { folder: TempFolder::new(name), changes, bytes };
        folder.user().ok("write_model", json!({ "path": "line.json", "text": LINE }));
        folder
    }

    fn open(&self, source: Source) -> Who {
        let mut settings = Settings::new(&self.folder.path, source);
        settings.history_changes = self.changes;
        settings.history_bytes = self.bytes;
        let catalogue = Catalogue::open(settings).unwrap();
        catalogue.set_client_name("test-harness");
        Who { catalogue }
    }

    fn user(&self) -> Who {
        self.open(Source::Cli)
    }

    fn agent(&self) -> Who {
        self.open(Source::Mcp)
    }

    fn text(&self) -> String {
        self.folder.read("line.json")
    }
}

impl Who {
    fn ok(&self, name: &str, input: Value) -> Value {
        let output = self.catalogue.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"));
        common::assert_fits_its_schema(&self.catalogue, name, &output);
        output
    }

    fn err(&self, name: &str, input: Value) -> Value {
        match self.catalogue.call(name, input.clone()) {
            Ok(output) => panic!("{name} {input} should fail, but gave {output}"),
            Err(error) => error.to_json(),
        }
    }

    fn edit(&self, changes: Value) -> Value {
        self.ok("edit_model", json!({ "path": "line.json", "changes": changes }))
    }

    fn describe(&self, description: &str) -> Value {
        self.edit(json!([{ "change": "set_model_description", "description": description }]))
    }

    fn undo(&self) -> Value {
        self.ok("undo_model", json!({ "path": "line.json" }))
    }

    fn redo(&self) -> Value {
        self.ok("redo_model", json!({ "path": "line.json" }))
    }

    fn claim(&self) {
        self.ok("claim_model", json!({ "path": "line.json", "goal": "trying undo", "doing": "changing the line" }));
    }

    /// The history's entries, newest first.
    fn history(&self) -> Vec<Value> {
        self.ok("get_history", json!({ "path": "line.json" }))["entries"].as_array().unwrap().clone()
    }

    fn hash(&self) -> String {
        self.ok("read_model", json!({ "path": "line.json" }))["file_hash"].as_str().unwrap().to_string()
    }
}

fn what(entry: &Value) -> &str {
    entry["what"].as_str().unwrap()
}

// ---------- Undo ----------

#[test]
fn an_undo_restores_the_files_exact_text_and_hash_after_removing_an_instance_with_its_connections() {
    let folder = Folder::new("undo-exact");
    let user = folder.user();
    let (before, hash) = (folder.text(), user.hash());
    let edited = user.edit(json!([{ "change": "remove_instance", "name": "Work" }, { "change": "connect", "from": "In", "to": "Out" }]));
    assert_ne!(folder.text(), before);
    let undone = user.undo();
    assert_eq!(folder.text(), before, "the text is what it was, byte for byte");
    assert_eq!(undone["file_hash"], hash.as_str());
    assert_ne!(undone["file_hash"], edited["file_hash"]);
    assert_eq!(user.hash(), hash);
    assert_eq!(undone["undone"].as_array().unwrap().len(), 1);
    assert_eq!(what(&undone["undone"][0]), "remove Work; connect In > Out");
    // crabsim saved it, so it isn't taken for changed outside, and changes go on.
    assert_eq!(user.ok("read_model", json!({ "path": "line.json" }))["changed_outside"], false);
    user.describe("after the undo");
}

#[test]
fn the_history_lists_every_change_newest_first_with_its_author_time_and_wording() {
    let folder = Folder::new("history-list");
    let user = folder.user();
    user.edit(json!([{ "change": "set_attribute", "target": { "instance": "Work" }, "attribute": "process_time", "value": "0:20" }]));
    user.edit(json!([{ "change": "rename_instance", "name": "Work", "to": "Mill" }]));
    let history = user.history();
    assert_eq!(history.len(), 2, "{history:?}");
    assert_eq!(what(&history[0]), "rename Work to Mill");
    assert_eq!(what(&history[1]), "set Work.process_time 0:20");
    for entry in &history {
        assert_eq!(entry["author"], "the user");
        assert_eq!(entry["kind"], "change");
        assert_eq!(entry["undone"], false);
        assert_eq!(entry["yours"], true);
        let at = entry["at"].as_str().unwrap();
        assert!(at.len() == 20 && at.ends_with('Z'), "{at}");
    }
    assert!(history[0]["number"].as_i64() > history[1]["number"].as_i64());
}

#[test]
fn a_change_that_changes_nothing_makes_no_entry() {
    let folder = Folder::new("history-nothing");
    let user = folder.user();
    user.describe("the same");
    user.describe("the same");
    assert_eq!(user.history().len(), 1);
}

#[test]
fn with_nothing_to_undo_or_redo_it_says_so() {
    let folder = Folder::new("undo-nothing");
    let user = folder.user();
    let refused = user.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "nothing_to_undo", "{refused}");
    let refused = user.err("redo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "nothing_to_undo", "{refused}");
    let refused = user.err("undo_model", json!({ "path": "missing.json" }));
    assert_eq!(refused["kind"], "not_found", "{refused}");
    let refused = user.err("undo_model", json!({ "path": "line.json", "steps": 0 }));
    assert_eq!(refused["kind"], "invalid_input", "{refused}");
}

// ---------- Redo ----------

#[test]
fn redo_brings_an_undone_change_back_and_a_new_change_drops_what_could_be_redone() {
    let folder = Folder::new("redo");
    let user = folder.user();
    user.describe("one");
    let one = folder.text();
    user.describe("two");
    let two = folder.text();
    user.undo();
    assert_eq!(folder.text(), one);
    assert_eq!(user.history()[0]["undone"], true);
    let redone = user.redo();
    assert_eq!(folder.text(), two);
    assert_eq!(what(&redone["redone"][0]), "describe the model as two");
    assert_eq!(user.history()[0]["undone"], false);
    // Several steps at once, and back.
    let undone = user.ok("undo_model", json!({ "path": "line.json", "steps": 2 }));
    assert_eq!(undone["undone"].as_array().unwrap().len(), 2);
    user.ok("redo_model", json!({ "path": "line.json", "steps": 2 }));
    assert_eq!(folder.text(), two);
    // A new change after an undo: the undone one is gone for good.
    user.undo();
    user.describe("three");
    let history = user.history();
    assert_eq!(history.iter().map(what).collect::<Vec<_>>(), ["describe the model as three", "describe the model as one"]);
    assert_eq!(user.err("redo_model", json!({ "path": "line.json" }))["kind"], "nothing_to_undo");
}

#[test]
fn more_steps_than_there_are_changes_undoes_them_all() {
    let folder = Folder::new("undo-all");
    let user = folder.user();
    let before = folder.text();
    user.describe("one");
    user.describe("two");
    let undone = user.ok("undo_model", json!({ "path": "line.json", "steps": 9 }));
    assert_eq!(undone["undone"].as_array().unwrap().len(), 2);
    assert_eq!(folder.text(), before);
}

// ---------- Groups ----------

#[test]
fn everything_one_command_did_is_undone_in_one_step() {
    let folder = Folder::new("undo-group");
    let user = folder.user();
    user.describe("before the macro");
    let before = folder.text();
    // As the window does around a macro run with a count.
    user.catalogue.begin_group();
    user.ok("run_commands", json!({ "path": "line.json", "commands": "add Station as A at 4 4" }));
    user.ok("run_commands", json!({ "path": "line.json", "commands": "add Station as B at 8 4; connect A > B" }));
    user.catalogue.end_group();
    user.describe("after the macro");
    let history = user.history();
    assert_eq!(history.len(), 4);
    // A line is worded as it was typed, not as the changes it became.
    user.ok("run_commands", json!({ "path": "line.json", "commands": "make class Slow from work" }));
    assert_eq!(what(&user.history()[0]), "make class Slow from work");
    user.undo();
    user.describe("after the macro");
    assert_eq!(history[1]["group"], history[2]["group"], "the two lines share a group");
    assert_ne!(history[0]["group"], history[1]["group"]);
    assert_ne!(history[3]["group"], history[2]["group"]);
    assert_eq!(what(&history[1]), "add Station as B at 8 4; connect A > B");
    user.undo();
    let undone = user.undo();
    assert_eq!(undone["undone"].as_array().unwrap().len(), 2, "one step takes the whole group");
    assert_eq!(folder.text(), before);
    let redone = user.redo();
    assert_eq!(redone["redone"].as_array().unwrap().len(), 2);
    assert!(folder.text().contains("\"B\""));
}

// ---------- write_model and adopt_model ----------

#[test]
fn writing_a_file_whole_is_an_entry_and_is_undone() {
    let folder = Folder::new("undo-written");
    let user = folder.user();
    let before = folder.text();
    let other = LINE.replace("\"8\"", "\"30\"");
    user.ok("write_model", json!({ "path": "line.json", "text": other, "overwrite": true }));
    let history = user.history();
    assert_eq!(history.len(), 1, "a new file makes no entry, a file written over does");
    assert_eq!(history[0]["kind"], "written");
    assert_eq!(what(&history[0]), "written whole");
    user.undo();
    assert_eq!(folder.text(), before);
}

#[test]
fn a_change_from_outside_is_refused_until_adopted_and_then_undone_in_one_step() {
    let folder = Folder::new("undo-adopted");
    let user = folder.user();
    user.describe("crabsim's own");
    let saved = folder.text();
    folder.folder.write("line.json", &saved.replace("crabsim's own", "edited by hand"));
    let refused = user.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "changed_outside", "{refused}");
    assert!(folder.text().contains("edited by hand"), "nothing was changed");
    user.ok("adopt_model", json!({ "path": "line.json" }));
    let history = user.history();
    assert_eq!(history[0]["kind"], "adopted");
    assert_eq!(what(&history[0]), "changed outside crabsim");
    user.undo();
    assert_eq!(folder.text(), saved, "the file is as crabsim last saved it");
    user.undo();
    assert!(!folder.text().contains("crabsim's own"));
}

#[test]
fn adopting_a_file_crabsim_never_saved_makes_no_entry() {
    let folder = Folder::new("undo-never-saved");
    let user = folder.user();
    folder.folder.write("other.json", LINE);
    user.ok("adopt_model", json!({ "path": "other.json" }));
    assert!(user.ok("get_history", json!({ "path": "other.json" }))["entries"].as_array().unwrap().is_empty());
}

// ---------- Agents ----------

#[test]
fn an_agent_undoes_its_own_latest_changes_and_stops_at_someone_elses() {
    let folder = Folder::new("undo-agent");
    let (user, agent) = (folder.user(), folder.agent());
    user.describe("the user's");
    let users = folder.text();
    agent.claim();
    agent.describe("the agent's first");
    agent.describe("the agent's second");
    let history = agent.history();
    assert_eq!(history[0]["author"], "agent-1 (test-harness)");
    assert_eq!(history[0]["yours"], true);
    assert_eq!(history[2]["yours"], false);
    assert_eq!(user.history()[0]["yours"], false);
    // It asks for three steps and gets its own two.
    let undone = agent.ok("undo_model", json!({ "path": "line.json", "steps": 3 }));
    assert_eq!(undone["undone"].as_array().unwrap().len(), 2);
    assert!(undone["stopped"].as_str().unwrap().contains("the user"), "{undone}");
    assert_eq!(folder.text(), users);
    // Nothing of its own is left to undo.
    let refused = agent.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "not_yours", "{refused}");
    assert_eq!(folder.text(), users);
    // It redoes its own.
    agent.redo();
    assert!(folder.text().contains("the agent's first"));
}

#[test]
fn an_agent_needs_its_claim_to_undo_and_cant_redo_what_the_user_undid_of_the_users() {
    let folder = Folder::new("undo-agent-claim");
    let (user, agent) = (folder.user(), folder.agent());
    user.describe("the user's");
    let refused = agent.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "not_editor", "{refused}");
    user.undo();
    agent.claim();
    let refused = agent.err("redo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "not_yours", "{refused}");
    assert!(!folder.text().contains("the user's"));
}

#[test]
fn the_user_undoes_an_agents_change_past_its_claim_and_the_agent_reads_again() {
    let folder = Folder::new("undo-agents-change");
    let (user, agent) = (folder.user(), folder.agent());
    let before = folder.text();
    agent.claim();
    agent.describe("the agent's");
    // The agent holds the file, so the user's undo is refused until it takes over.
    let refused = user.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "not_editor", "{refused}");
    user.catalogue.set_take_over(true);
    user.undo();
    assert_eq!(folder.text(), before);
    // The undo is the user's change, so the agent's next change is refused once.
    let refused = agent.err("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_model_description", "description": "again" }] }));
    assert_eq!(refused["kind"], "model_changed", "{refused}");
    agent.describe("again");
}

// ---------- The store ----------

#[test]
fn the_history_survives_a_new_catalogue_on_the_same_folder() {
    let folder = Folder::new("history-survives");
    let before = folder.text();
    {
        let user = folder.user();
        user.describe("from the first window");
    }
    let user = folder.user();
    assert_eq!(what(&user.history()[0]), "describe the model as from the first window");
    user.undo();
    assert_eq!(folder.text(), before);
    assert!(folder.folder.join(".crabsim/history.sqlite").is_file());
}

#[test]
fn the_history_keeps_only_its_newest_changes() {
    let folder = Folder::with_limits("history-cap-count", 3, 50 * 1024 * 1024);
    let user = folder.user();
    for n in 1..=5 {
        user.describe(&format!("change {n}"));
    }
    let history = user.history();
    assert_eq!(history.iter().map(what).collect::<Vec<_>>(), ["describe the model as change 5", "describe the model as change 4", "describe the model as change 3"]);
    let undone = user.ok("undo_model", json!({ "path": "line.json", "steps": 9 }));
    assert_eq!(undone["undone"].as_array().unwrap().len(), 3);
    assert!(folder.text().contains("change 2"), "it goes back as far as the history reaches");
}

#[test]
fn the_history_drops_its_oldest_changes_past_its_size() {
    let size = Folder::new("history-size").text().len() as u64;
    // Room for about three versions of the file: two changes at most.
    let folder = Folder::with_limits("history-cap-size", 200, size * 7 / 2);
    let user = folder.user();
    for n in 1..=5 {
        user.describe(&format!("{n}"));
    }
    let history = user.history();
    assert!(history.len() == 2, "{history:?}");
    assert_eq!(what(&history[0]), "describe the model as 5");
}

#[test]
fn deleting_a_file_drops_its_history_and_a_copy_starts_with_none() {
    let folder = Folder::new("history-delete");
    let user = folder.user();
    user.describe("changed");
    user.ok("copy_model", json!({ "from": "line.json", "to": "copy.json" }));
    assert!(user.ok("get_history", json!({ "path": "copy.json" }))["entries"].as_array().unwrap().is_empty());
    // Also at a path whose file went without crabsim: a copy or a new file there starts afresh.
    let copy_history = || user.ok("get_history", json!({ "path": "copy.json" }))["entries"].as_array().unwrap().len();
    user.ok("edit_model", json!({ "path": "copy.json", "changes": [{ "change": "set_model_description", "description": "the copy's own" }] }));
    assert_eq!(copy_history(), 1);
    std::fs::remove_file(folder.folder.join("copy.json")).unwrap();
    user.ok("copy_model", json!({ "from": "line.json", "to": "copy.json" }));
    assert_eq!(copy_history(), 0);
    user.ok("edit_model", json!({ "path": "copy.json", "changes": [{ "change": "set_model_description", "description": "the copy's own" }] }));
    std::fs::remove_file(folder.folder.join("copy.json")).unwrap();
    user.ok("write_model", json!({ "path": "copy.json", "text": LINE }));
    assert_eq!(copy_history(), 0);
    user.ok("delete_model", json!({ "path": "line.json" }));
    assert!(user.history().is_empty(), "the history went with the file");
    user.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    assert!(user.history().is_empty());
    assert_eq!(user.err("undo_model", json!({ "path": "line.json" }))["kind"], "nothing_to_undo");
}

#[test]
fn the_history_shows_its_newest_entries_and_says_when_there_are_more() {
    let folder = Folder::new("history-limit");
    let user = folder.user();
    for n in 1..=4 {
        user.describe(&format!("{n}"));
    }
    let shown = user.ok("get_history", json!({ "path": "line.json", "limit": 3 }));
    assert_eq!(shown["entries"].as_array().unwrap().len(), 3);
    assert_eq!(shown["more"], true);
    assert_eq!(what(&shown["entries"][0]), "describe the model as 4");
    let all = user.ok("get_history", json!({ "path": "line.json" }));
    assert_eq!(all["more"], false);
}

#[test]
fn a_history_that_no_longer_matches_the_file_is_dropped_rather_than_used() {
    let folder = Folder::new("history-stale");
    {
        let user = folder.user();
        user.describe("one");
        user.describe("two");
    }
    // As if the history were from another copy of the folder: crabsim's record says the file is
    // its own, but the history ends at another text. Closed first, so the file holds it all.
    std::fs::copy(folder.folder.join(".crabsim/history.sqlite"), folder.folder.join("kept.sqlite")).unwrap();
    {
        let user = folder.user();
        assert_eq!(user.history().len(), 2);
        user.describe("three");
    }
    for extra in ["history.sqlite-wal", "history.sqlite-shm"] {
        let _ = std::fs::remove_file(folder.folder.join(".crabsim").join(extra));
    }
    std::fs::copy(folder.folder.join("kept.sqlite"), folder.folder.join(".crabsim/history.sqlite")).unwrap();
    let user = folder.user();
    let text = folder.text();
    let refused = user.err("undo_model", json!({ "path": "line.json" }));
    assert_eq!(refused["kind"], "nothing_to_undo", "{refused}");
    assert_eq!(folder.text(), text);
    assert!(user.history().is_empty());
}
