//! The editor and the edit queue, through `call(name, JSON)`: several agents and the user on one
//! models folder. Each catalogue holds a process lock of its own, so two catalogues in one test
//! are two agents, and one opened as the CLI is the user.

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

    fn claim(&self, purpose: &str) -> Value {
        self.ok("claim_model", json!({ "path": "line.json", "purpose": purpose }))
    }

    /// A small change to `line.json`, which only its editor, or anyone when nobody holds or waits
    /// for it, may make.
    fn change(&self) -> Result<Value, Value> {
        self.catalogue
            .call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_model_description", "description": "changed" }] }))
            .map_err(|error| error.to_json())
    }

    fn queue(&self) -> Value {
        self.ok("get_edit_queue", json!({ "path": "line.json" }))
    }

    /// The id this agent has in the folder, as the queue shows it.
    fn id(&self) -> String {
        self.ok("get_edit_queue", json!({ "path": "line.json" }))["you"].as_str().expect("an agent has an id").to_string()
    }
}

/// The ids in the queue, in order.
fn waiting(queue: &Value) -> Vec<String> {
    queue["queue"].as_array().unwrap().iter().map(|entry| entry["agent"].as_str().unwrap().to_string()).collect()
}

#[test]
fn a_change_with_nobody_editing_or_waiting_goes_through_without_a_claim() {
    let folder = Folder::new("lone");
    let agent = folder.agent();
    agent.change().unwrap();
    assert_eq!(agent.queue()["editor"], Value::Null, "a one-off change holds the right for that call only");
    let user = folder.user();
    user.change().unwrap();
}

#[test]
fn the_first_claim_is_granted_and_the_others_wait_their_turn_in_order() {
    let folder = Folder::new("queue");
    let (a, b, c) = (folder.agent(), folder.agent(), folder.agent());
    assert_eq!(a.claim("adding a mill")["state"], "granted");
    let queued = b.claim("checking the drain");
    assert_eq!(queued["state"], "queued");
    assert_eq!(queued["place"], 1);
    assert_eq!(c.claim("tidying")["place"], 2);
    assert_eq!(b.claim("checking the drain")["place"], 1, "asking again keeps the place");

    let refused = b.change().unwrap_err();
    assert_eq!(refused["kind"], "not_editor");
    let message = refused["message"].as_str().unwrap();
    for part in [a.id().as_str(), "test-harness", "adding a mill", "2 waiting"] {
        assert!(message.contains(part), "{part} in {message}");
    }
    a.change().unwrap();

    let queue = a.queue();
    assert_eq!(queue["editor"]["agent"], a.id());
    assert_eq!(queue["editor"]["purpose"], "adding a mill");
    assert_eq!(waiting(&queue), [b.id(), c.id()]);

    a.ok("release_model", json!({ "path": "line.json" }));
    assert_eq!(b.claim("checking the drain")["state"], "granted");
    b.change().unwrap();
    assert_eq!(waiting(&b.queue()), [c.id()]);
}

#[test]
fn a_claim_needs_a_purpose() {
    let folder = Folder::new("purpose");
    let agent = folder.agent();
    assert_eq!(agent.err("claim_model", json!({ "path": "line.json" }))["kind"], "invalid_input");
    assert_eq!(agent.err("claim_model", json!({ "path": "line.json", "purpose": "  " }))["kind"], "invalid_input");
}

#[test]
fn the_right_passes_on_when_its_agents_process_ends() {
    let folder = Folder::new("ends");
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("working");
    b.claim("next");
    drop(a);
    assert_eq!(b.claim("next")["state"], "granted");
}

#[test]
fn an_unused_right_passes_on_after_the_timeout_and_a_queued_agent_that_stops_asking_drops_out() {
    let folder = Folder::with_timeout("timeout", Duration::from_millis(400));
    let (a, b, c) = (folder.agent(), folder.agent(), folder.agent());
    a.claim("working");
    c.claim("later");
    std::thread::sleep(Duration::from_millis(250));
    a.change().unwrap();
    b.claim("next");
    std::thread::sleep(Duration::from_millis(250));
    b.claim("next");
    assert_eq!(b.queue()["editor"]["agent"], a.id(), "a change renews the right");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(b.claim("next")["state"], "granted", "a's right passed on, and c stopped asking");
    assert_eq!(waiting(&b.queue()), Vec::<String>::new());
    let lost = a.change().unwrap_err();
    assert_eq!(lost["kind"], "not_editor");
}

#[test]
fn the_user_takes_the_right_at_once_and_releases_it_to_the_first_waiting() {
    let folder = Folder::new("take");
    let (a, b, user) = (folder.agent(), folder.agent(), folder.user());
    a.claim("working");
    b.claim("next");
    assert_eq!(user.change().unwrap_err()["kind"], "not_editor", "the user takes the right first, rather than changing past an agent");
    user.ok("take_model", json!({ "path": "line.json" }));
    let lost = a.change().unwrap_err();
    assert_eq!(lost["kind"], "not_editor");
    assert!(lost["message"].as_str().unwrap().contains("the user"), "{lost}");
    user.change().unwrap();
    assert_eq!(user.queue()["editor"]["user"], true);
    assert_eq!(waiting(&user.queue()), [b.id()], "a, which lost the right, isn't queued again");
    user.ok("release_model", json!({ "path": "line.json" }));
    assert_eq!(b.claim("next")["state"], "granted");
}

#[test]
fn the_user_gives_the_right_to_any_waiting_agent_and_orders_the_queue() {
    let folder = Folder::new("order");
    let (a, b, c, d, user) = (folder.agent(), folder.agent(), folder.agent(), folder.agent(), folder.user());
    a.claim("one");
    b.claim("two");
    c.claim("three");
    d.claim("four");
    user.ok("move_in_queue", json!({ "path": "line.json", "agent": d.id(), "place": 0 }));
    assert_eq!(waiting(&user.queue()), [d.id(), b.id(), c.id()]);
    user.ok("move_in_queue", json!({ "path": "line.json", "agent": d.id(), "place": 2 }));
    assert_eq!(waiting(&user.queue()), [b.id(), c.id(), d.id()]);
    assert_eq!(user.err("move_in_queue", json!({ "path": "line.json", "agent": d.id(), "place": 3 }))["kind"], "invalid_input");
    assert_eq!(user.err("move_in_queue", json!({ "path": "line.json", "agent": a.id(), "place": 0 }))["kind"], "not_found", "a is the editor, not waiting");

    user.ok("give_model", json!({ "path": "line.json", "agent": c.id() }));
    assert_eq!(user.queue()["editor"]["agent"], c.id());
    assert_eq!(waiting(&user.queue()), [b.id(), d.id()], "a isn't queued again");
    c.change().unwrap();
    assert_eq!(a.change().unwrap_err()["kind"], "not_editor");

    user.ok("remove_from_queue", json!({ "path": "line.json", "agent": d.id() }));
    assert_eq!(waiting(&user.queue()), [b.id()]);
    user.ok("remove_from_queue", json!({ "path": "line.json", "agent": c.id() }));
    assert_eq!(user.queue()["editor"]["agent"], b.id(), "removing the editor passes the right on");
}

#[test]
fn a_blocked_agent_loses_the_right_and_is_told_until_it_is_unblocked() {
    let folder = Folder::new("block");
    let (a, b, user) = (folder.agent(), folder.agent(), folder.user());
    a.claim("working");
    b.claim("next");
    user.ok("block_agent", json!({ "agent": a.id() }));
    assert_eq!(user.queue()["editor"]["agent"], b.id());
    for refused in [a.err("claim_model", json!({ "path": "line.json", "purpose": "again" })), a.change().unwrap_err()] {
        assert_eq!(refused["kind"], "blocked");
        assert!(refused["message"].as_str().unwrap().contains("blocked"), "{refused}");
    }
    assert!(user.queue()["blocked"].as_array().unwrap().iter().any(|agent| agent["agent"] == a.id()));
    user.ok("unblock_agent", json!({ "agent": a.id() }));
    assert_eq!(a.claim("again")["state"], "queued");
}

#[test]
fn only_the_user_controls_the_queue() {
    let folder = Folder::new("user-only");
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("working");
    b.claim("next");
    for (name, input) in [
        ("take_model", json!({ "path": "line.json" })),
        ("give_model", json!({ "path": "line.json", "agent": b.id() })),
        ("move_in_queue", json!({ "path": "line.json", "agent": b.id(), "place": 0 })),
        ("remove_from_queue", json!({ "path": "line.json", "agent": a.id() })),
        ("block_agent", json!({ "agent": a.id() })),
        ("unblock_agent", json!({ "agent": a.id() })),
    ] {
        let refused = b.err(name, input);
        assert_eq!(refused["kind"], "invalid_input", "{name}");
        assert!(refused["message"].as_str().unwrap().contains("only for the user"), "{name}: {refused}");
        assert!(b.catalogue.operations().find(|o| o.name == name).unwrap().user_only, "{name} is marked for front-ends to hide");
    }
    for agent_s in ["claim_model", "release_model", "get_edit_queue"] {
        assert!(!b.catalogue.operations().find(|o| o.name == agent_s).unwrap().user_only, "{agent_s}");
    }
    let user = folder.user();
    assert_eq!(user.err("claim_model", json!({ "path": "line.json", "purpose": "x" }))["kind"], "invalid_input", "the user takes, rather than claims");
}

#[test]
fn an_agent_keeps_its_id_across_files_and_the_state_is_readable_json() {
    let folder = Folder::new("ids");
    Catalogue::open(Settings::new(&folder.folder.path, Source::Cli)).unwrap().call("write_model", json!({ "path": "other.json", "text": LINE })).unwrap();
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("working");
    b.ok("claim_model", json!({ "path": "other.json", "purpose": "elsewhere" }));
    let other = b.ok("get_edit_queue", json!({ "path": "other.json" }));
    assert_eq!(other["editor"]["agent"], b.id());
    assert_ne!(a.id(), b.id());
    let state: Value = serde_json::from_str(&folder.folder.read(".crabsim/editing.json")).expect("the edit state is JSON");
    assert!(state.to_string().contains("working") && state.to_string().contains("elsewhere"), "{state}");
    let everything = a.ok("get_edit_queue", json!({}));
    assert_eq!(everything["files"].as_array().unwrap().len(), 2, "without a path, every file someone edits or waits for");
}

#[test]
fn write_model_and_delete_model_follow_the_same_rules() {
    let folder = Folder::new("write-delete");
    let (a, b) = (folder.agent(), folder.agent());
    a.claim("working");
    let text = crabsim_model::load(LINE).unwrap().save();
    assert_eq!(b.err("write_model", json!({ "path": "line.json", "text": text, "overwrite": true }))["kind"], "not_editor");
    assert_eq!(b.err("delete_model", json!({ "path": "line.json" }))["kind"], "not_editor");
    a.ok("write_model", json!({ "path": "line.json", "text": text, "overwrite": true }));
    b.ok("write_model", json!({ "path": "new.json", "text": text }));
    b.ok("validate_model", json!({ "path": "line.json" }));
    b.ok("copy_model", json!({ "from": "line.json", "to": "copy.json" }));
}
