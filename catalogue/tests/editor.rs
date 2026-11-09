//! Claims (ADR 0013), through `call(name, JSON)`: several agents and the user on one models
//! folder. Each catalogue holds a process lock of its own, so two catalogues in one test are two
//! agents, and one opened as the CLI or the UI is the user.

mod common;

use common::TempFolder;
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};
use std::time::Duration;

const LINE: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","successors":["Out"],"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}]}"#;

/// A models folder holding `line.json`, and whoever works on it.
struct Folder {
    folder: TempFolder,
    timeout: Duration,
}

/// An agent or the user, on the folder.
struct Who {
    catalogue: Catalogue,
}

impl Folder {
    fn new(name: &str) -> Folder {
        Folder::with_timeout(name, Duration::from_secs(600))
    }

    fn with_timeout(name: &str, timeout: Duration) -> Folder {
        let folder = TempFolder::new(name);
        // Saved by crabsim, as the user, so it isn't changed outside.
        let user = Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        Folder { folder, timeout }
    }

    fn open(&self, source: Source) -> Who {
        let mut settings = Settings::new(&self.folder.path, source);
        settings.edit_timeout = self.timeout;
        let catalogue = Catalogue::open(settings).unwrap();
        catalogue.set_client_name("test-harness");
        Who { catalogue }
    }

    fn agent(&self) -> Who {
        self.open(Source::Mcp)
    }

    fn user(&self) -> Who {
        self.open(Source::Cli)
    }
}

impl Who {
    fn ok(&self, name: &str, input: Value) -> Value {
        self.catalogue.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"))
    }

    fn err(&self, name: &str, input: Value) -> Value {
        match self.catalogue.call(name, input.clone()) {
            Ok(output) => panic!("{name} {input} should fail, but gave {output}"),
            Err(error) => error.to_json(),
        }
    }

    fn claim(&self, goal: &str, doing: &str) -> Value {
        self.ok("claim_model", json!({ "path": "line.json", "goal": goal, "doing": doing }))
    }

    fn claim_err(&self, goal: &str, doing: &str) -> Value {
        self.err("claim_model", json!({ "path": "line.json", "goal": goal, "doing": doing }))
    }

    /// A small change to `line.json`.
    fn change(&self) -> Result<Value, Value> {
        self.change_to("changed")
    }

    fn change_to(&self, description: &str) -> Result<Value, Value> {
        self.catalogue
            .call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_model_description", "description": description }] }))
            .map_err(|error| error.to_json())
    }

    /// `line.json` as `get_editors` shows it, or null when nobody holds it.
    fn file(&self) -> Value {
        let files = self.ok("get_editors", json!({ "path": "line.json" }))["files"].clone();
        files.as_array().unwrap().first().cloned().unwrap_or(Value::Null)
    }

    /// The id this agent has in the folder.
    fn id(&self) -> String {
        self.ok("get_editors", json!({}))["you"].as_str().expect("an agent has an id").to_string()
    }
}

fn message(error: &Value) -> &str {
    error["message"].as_str().unwrap()
}

// ---------- Claiming ----------

#[test]
fn a_claim_is_granted_at_once_and_a_second_is_refused_naming_who_holds_the_file() {
    let folder = Folder::new("granted");
    let (a, b) = (folder.agent(), folder.agent());
    let granted = a.claim("find the bottleneck", "adding Mill 2 after Station 3");
    assert_eq!(granted["file"]["editor"], a.id());
    assert_eq!(granted["file"]["claim"]["goal"], "find the bottleneck");
    assert_eq!(granted["file"]["claim"]["doing"], "adding Mill 2 after Station 3");
    assert_eq!(granted["file"]["claim"]["client"], "test-harness");
    assert_eq!(granted["file"]["claim"]["paused"], false);

    let refused = b.claim_err("tidy the names", "renaming");
    assert_eq!(refused["kind"], "model_in_use");
    for part in [a.id().as_str(), "test-harness", "find the bottleneck", "adding Mill 2 after Station 3"] {
        assert!(message(&refused).contains(part), "{part} in {refused}");
    }
    assert_eq!(b.change().unwrap_err()["kind"], "not_editor");
    a.change().unwrap();
}

#[test]
fn a_claim_needs_a_goal_and_what_the_agent_is_doing() {
    let folder = Folder::new("goal");
    let agent = folder.agent();
    for input in [
        json!({ "path": "line.json", "doing": "x" }),
        json!({ "path": "line.json", "goal": "x" }),
        json!({ "path": "line.json", "goal": " ", "doing": "x" }),
        json!({ "path": "line.json", "goal": "x", "doing": "  " }),
        json!({ "path": "line.json", "purpose": "the old field" }),
    ] {
        assert_eq!(agent.err("claim_model", input.clone())["kind"], "invalid_input", "{input}");
    }
}

#[test]
fn claiming_again_updates_what_the_agent_is_doing_and_keeps_the_last_ten() {
    let folder = Folder::new("trail");
    let agent = folder.agent();
    agent.claim("find the bottleneck", "step 0");
    agent.claim("find the bottleneck", "step 0");
    for step in 1..=11 {
        agent.claim("find the bottleneck, then fix it", &format!("step {step}"));
    }
    let claim = &agent.file()["claim"];
    assert_eq!(claim["goal"], "find the bottleneck, then fix it");
    assert_eq!(claim["doing"], "step 11");
    let trail: Vec<&str> = claim["trail"].as_array().unwrap().iter().map(|entry| entry["doing"].as_str().unwrap()).collect();
    let expected: Vec<String> = (2..=11).rev().map(|step| format!("step {step}")).collect();
    assert_eq!(trail, expected, "newest first, at most 10, and claiming again with the same doing adds nothing");
    assert!(claim["trail"][0]["since"].as_str().unwrap().ends_with('Z'));
}

// ---------- Every change needs a claim ----------

#[test]
fn every_change_an_agent_makes_needs_a_claim_and_the_users_never_do() {
    let folder = Folder::new("every-change");
    let agent = folder.agent();
    let text = crabsim_model::load(LINE).unwrap().save();
    for (name, input) in [
        ("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_model_description", "description": "x" }] })),
        ("write_model", json!({ "path": "line.json", "text": text, "overwrite": true })),
        ("write_model", json!({ "path": "new.json", "text": text })),
        ("delete_model", json!({ "path": "line.json" })),
        ("adopt_model", json!({ "path": "line.json" })),
    ] {
        let refused = agent.err(name, input.clone());
        assert_eq!(refused["kind"], "not_editor", "{name} {input}");
        assert!(message(&refused).contains("claim_model"), "{name}: {refused}");
    }
    assert_eq!(agent.file(), Value::Null, "nothing was claimed by trying");

    agent.ok("claim_model", json!({ "path": "new.json", "goal": "a variant", "doing": "writing it" }));
    agent.ok("write_model", json!({ "path": "new.json", "text": text }));
    agent.claim("a cleaner line", "describing it");
    agent.change().unwrap();
    agent.ok("delete_model", json!({ "path": "line.json" }));
    agent.ok("release_model", json!({ "path": "line.json" }));

    let user = folder.user();
    user.ok("write_model", json!({ "path": "line.json", "text": text }));
    user.change().unwrap();
    // Reading, copying and running need no claim.
    agent.ok("validate_model", json!({ "path": "line.json" }));
    agent.ok("copy_model", json!({ "from": "line.json", "to": "copy.json" }));
}

// ---------- How a claim ends ----------

#[test]
fn a_claim_ends_on_release_and_when_its_agents_process_ends() {
    let folder = Folder::new("ends");
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("one", "working");
    let released = a.ok("release_model", json!({ "path": "line.json" }));
    assert_eq!(released["released"], true);
    assert_eq!(a.change().unwrap_err()["kind"], "not_editor");
    b.claim("two", "working");
    drop(b);
    a.claim("one again", "working");
}

#[test]
fn an_unused_claim_ends_after_the_timeout_and_a_change_or_a_claim_renews_it() {
    let folder = Folder::with_timeout("timeout", Duration::from_millis(400));
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("one", "working");
    std::thread::sleep(Duration::from_millis(250));
    a.change().unwrap();
    std::thread::sleep(Duration::from_millis(250));
    a.claim("one", "still working");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(b.claim_err("two", "next")["kind"], "model_in_use", "a change and a claim renew it");
    std::thread::sleep(Duration::from_millis(250));
    b.claim("two", "next");
    assert_eq!(a.change().unwrap_err()["kind"], "not_editor");
}

#[test]
fn the_user_revokes_a_claim_at_once() {
    let folder = Folder::new("revoke");
    let (a, b, user) = (folder.agent(), folder.agent(), folder.user());
    a.claim("one", "working");
    let revoked = user.ok("revoke_claim", json!({ "path": "line.json" }));
    assert_eq!(revoked["revoked"], a.id());
    assert_eq!(a.change().unwrap_err()["kind"], "not_editor");
    b.claim("two", "working");
    assert_eq!(user.err("revoke_claim", json!({ "path": "nobody.json" }))["kind"], "not_found");
}

// ---------- The user takes the right ----------

#[test]
fn taking_the_right_pauses_the_agents_claim_and_releasing_gives_it_back() {
    let folder = Folder::new("pause");
    let (a, b, user) = (folder.agent(), folder.agent(), folder.user());
    a.claim("find the bottleneck", "adding Mill 2");
    assert_eq!(user.change().unwrap_err()["kind"], "not_editor", "the user takes the right first, rather than changing past an agent");
    let taken = user.ok("take_model", json!({ "path": "line.json" }));
    assert_eq!(taken["file"]["editor"], "user");
    assert_eq!(taken["file"]["claim"]["paused"], true);
    assert_eq!(taken["file"]["claim"]["agent"], a.id(), "the claim stays the agent's");

    let paused = a.change().unwrap_err();
    assert_eq!(paused["kind"], "not_editor");
    assert!(message(&paused).contains("paused"), "{paused}");
    assert_eq!(b.claim_err("two", "x")["kind"], "model_in_use", "nobody slips in while it's paused");

    user.ok("release_model", json!({ "path": "line.json" }));
    let back = a.file();
    assert_eq!(back["editor"], a.id());
    assert_eq!(back["claim"]["paused"], false);
    a.change().unwrap();
}

#[test]
fn a_paused_claim_does_not_run_out() {
    let folder = Folder::with_timeout("paused-timeout", Duration::from_millis(300));
    let (a, user) = (folder.agent(), folder.user());
    a.claim("one", "working");
    user.ok("take_model", json!({ "path": "line.json" }));
    std::thread::sleep(Duration::from_millis(450));
    assert_eq!(user.ok("get_editors", json!({ "path": "line.json" }))["files"][0]["claim"]["agent"], a.id(), "still there after the timeout");
    user.ok("release_model", json!({ "path": "line.json" }));
    a.change().unwrap();
}

#[test]
fn the_users_right_ends_when_the_window_or_command_that_took_it_ends() {
    let folder = Folder::new("user-ends");
    let a = folder.agent();
    a.claim("one", "working");
    let window = folder.open(Source::Ui);
    window.ok("take_model", json!({ "path": "line.json" }));
    assert_eq!(a.change().unwrap_err()["kind"], "not_editor");
    drop(window);
    a.change().unwrap();
    assert_eq!(a.file()["editor"], a.id());
}

#[test]
fn the_user_may_take_a_file_nobody_holds_and_release_it() {
    let folder = Folder::new("user-free");
    let (a, user) = (folder.agent(), folder.user());
    user.ok("take_model", json!({ "path": "line.json" }));
    let refused = a.claim_err("one", "working");
    assert_eq!(refused["kind"], "model_in_use");
    assert!(message(&refused).contains("the user"), "{refused}");
    user.ok("release_model", json!({ "path": "line.json" }));
    assert_eq!(user.file(), Value::Null);
    a.claim("one", "working");
}

// ---------- Reading again after someone else's change ----------

#[test]
fn after_a_take_over_the_agents_next_change_is_refused_once_to_read_again() {
    let folder = Folder::new("read-again");
    let (a, user) = (folder.agent(), folder.user());
    a.claim("one", "working");
    a.change_to("the agent's").unwrap();
    user.catalogue.set_take_over(true);
    user.change_to("the user's").unwrap();
    assert_eq!(a.file()["editor"], a.id(), "the agent keeps its claim");
    let stale = a.change_to("the agent's again").unwrap_err();
    assert_eq!(stale["kind"], "model_changed");
    assert!(message(&stale).contains("read it again") && message(&stale).contains("the user"), "{stale}");
    a.change_to("the agent's again").unwrap();
    a.change_to("and again").unwrap();
}

#[test]
fn after_a_pause_the_agent_reads_again_only_if_the_user_changed_the_model() {
    let folder = Folder::new("pause-read-again");
    let (a, user) = (folder.agent(), folder.user());
    a.claim("one", "working");
    user.ok("take_model", json!({ "path": "line.json" }));
    user.ok("release_model", json!({ "path": "line.json" }));
    a.change().unwrap();

    user.ok("take_model", json!({ "path": "line.json" }));
    user.change_to("the user's").unwrap();
    user.ok("release_model", json!({ "path": "line.json" }));
    assert_eq!(a.change().unwrap_err()["kind"], "model_changed");
    a.change().unwrap();
}

#[test]
fn an_agents_first_change_is_measured_from_when_it_claimed() {
    let folder = Folder::new("since-claim");
    let (a, user) = (folder.agent(), folder.user());
    user.change_to("before the claim").unwrap();
    a.claim("one", "working");
    a.change().unwrap();
}

// ---------- Blocking and who may do what ----------

#[test]
fn a_blocked_agent_loses_its_claims_and_is_told_until_it_is_unblocked() {
    let folder = Folder::new("block");
    let (a, b, user) = (folder.agent(), folder.agent(), folder.user());
    a.claim("one", "working");
    user.ok("block_agent", json!({ "agent": a.id() }));
    assert_eq!(user.file(), Value::Null, "its claim ended");
    for refused in [a.claim_err("again", "x"), a.change().unwrap_err()] {
        assert_eq!(refused["kind"], "blocked");
        assert!(message(&refused).contains("blocked"), "{refused}");
    }
    assert!(user.ok("get_editors", json!({}))["blocked"].as_array().unwrap().iter().any(|agent| agent["agent"] == a.id()));
    b.claim("two", "working");
    b.ok("release_model", json!({ "path": "line.json" }));
    user.ok("unblock_agent", json!({ "agent": a.id() }));
    a.claim("again", "working");
}

#[test]
fn only_the_user_takes_revokes_and_blocks() {
    let folder = Folder::new("user-only");
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("one", "working");
    for (name, input) in [
        ("take_model", json!({ "path": "line.json" })),
        ("revoke_claim", json!({ "path": "line.json" })),
        ("block_agent", json!({ "agent": a.id() })),
        ("unblock_agent", json!({ "agent": a.id() })),
    ] {
        let refused = b.err(name, input);
        assert_eq!(refused["kind"], "invalid_input", "{name}");
        assert!(message(&refused).contains("only for the user"), "{name}: {refused}");
        assert!(b.catalogue.operations().find(|o| o.name == name).unwrap().user_only, "{name} is marked for front-ends to hide");
    }
    for agent_s in ["claim_model", "release_model", "get_editors"] {
        assert!(!b.catalogue.operations().find(|o| o.name == agent_s).unwrap().user_only, "{agent_s}");
    }
    for gone in ["get_edit_queue", "give_model", "move_in_queue", "remove_from_queue"] {
        assert!(b.catalogue.operations().all(|o| o.name != gone), "{gone} is gone with the queue");
    }
    let user = folder.user();
    assert_eq!(user.err("claim_model", json!({ "path": "line.json", "goal": "x", "doing": "y" }))["kind"], "invalid_input", "the user takes, rather than claims");
}

#[test]
fn an_agent_keeps_its_id_across_files_and_every_claim_is_listed() {
    let folder = Folder::new("ids");
    Catalogue::open(Settings::new(&folder.folder.path, Source::Cli)).unwrap().call("write_model", json!({ "path": "other.json", "text": LINE })).unwrap();
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("one", "working");
    b.ok("claim_model", json!({ "path": "other.json", "goal": "two", "doing": "elsewhere" }));
    assert_eq!(a.err("claim_model", json!({ "path": "other.json", "goal": "x", "doing": "y" }))["kind"], "model_in_use");
    let state: Value = serde_json::from_str(&folder.folder.read(".crabsim/editing.json")).expect("the edit state is JSON");
    assert!(state.to_string().contains("working") && state.to_string().contains("elsewhere"), "{state}");
    let everything = folder.user().ok("get_editors", json!({}));
    let files: Vec<(&str, &str)> = everything["files"].as_array().unwrap().iter().map(|file| (file["path"].as_str().unwrap(), file["editor"].as_str().unwrap())).collect();
    assert_eq!(files, [("line.json", a.id().as_str()), ("other.json", b.id().as_str())]);
    assert_eq!(everything["you"], Value::Null, "the user has no agent id");
}

// ---------- The edit state from before claims ----------

#[test]
fn an_edit_state_from_before_claims_is_read_without_its_queues() {
    let folder = Folder::new("old-state");
    let a = folder.agent();
    a.claim("one", "working");
    let process = {
        let state: Value = serde_json::from_str(&folder.folder.read(".crabsim/editing.json")).unwrap();
        state["agents"].as_object().unwrap().keys().next().unwrap().clone()
    };
    let old = json!({
        "version": 1,
        "next_agent": 1,
        "agents": { process.clone(): { "id": "agent-1", "client": "test-harness", "started": 0, "blocked": false } },
        "files": { "line.json": {
            "editor": { "process": process, "purpose": "the old purpose", "since": 0, "last_active": 9_999_999_999_999u64 },
            "queue": [{ "process": "gone", "purpose": "waiting", "asked": 0, "last_asked": 0 }]
        } },
        "records": serde_json::from_str::<Value>(&folder.folder.read(".crabsim/editing.json")).unwrap()["records"].clone()
    });
    folder.folder.write(".crabsim/editing.json", &old.to_string());
    let file = a.file();
    assert_eq!(file["editor"], "agent-1");
    assert_eq!(file["claim"]["goal"], "the old purpose", "the old purpose is read as the goal");
    assert_eq!(file["claim"]["doing"], "");
    assert!(file.get("queue").is_none());
    a.change().unwrap();
}
