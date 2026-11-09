//! What the second harness trial asked for, through `call(name, JSON)`: hashes named for what they
//! hash, copies that say they need adopting, runs that record their build and agent, every problem
//! of a batch at once, waiting for short runs, and agents keeping to their own runs.

mod common;

use common::{write_models, Setup, SMALL};
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::json;

fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn a_file_hash_and_a_model_hash_are_named_apart() {
    let setup = Setup::new("hashes");
    // Written by hand: not canonical, so its bytes and its model hash differently.
    setup.folder.write("line.json", SMALL);
    let canonical = crabsim_model::load(SMALL).unwrap().save();
    let read = setup.ok("read_model", json!({ "path": "line.json" }));
    assert!(read.get("hash").is_none());
    assert_eq!(read["file_hash"], hash(SMALL));
    assert_eq!(read["model_hash"], hash(&canonical), "the hash a run records");
    let run = setup.start(json!({ "model": "line.json", "length": "1:00", "observations": 1 }));
    let stored = setup.ok("query_results", json!({ "sql": format!("SELECT model_hash FROM runs WHERE id = '{run}'") }));
    assert_eq!(stored["rows"][0][0], read["model_hash"]);

    let adopted = setup.ok("adopt_model", json!({ "path": "line.json" }));
    assert_eq!(adopted["file_hash"], hash(SMALL));
    let edited = setup.ok("edit_model", json!({ "path": "line.json", "expected_hash": hash(SMALL), "changes": [{ "change": "set_model_description", "description": "x" }] }));
    assert_eq!(edited["file_hash"], edited["model_hash"], "a file crabsim saved is canonical");
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Work" } }));
    assert_eq!(got["file_hash"], edited["file_hash"]);
    assert_eq!(got["model_hash"], edited["model_hash"]);
}

#[test]
fn a_copy_says_whether_it_needs_adopting() {
    let setup = Setup::new("copies");
    setup.folder.write("by-hand.json", SMALL);
    setup.ok("write_model", json!({ "path": "saved.json", "text": SMALL }));
    let copied = setup.ok("copy_model", json!({ "from": "by-hand.json", "to": "a.json" }));
    assert_eq!(copied["changed_outside"], true);
    assert!(copied.to_string().contains("adopt_model") || setup.catalogue.operations().find(|o| o.name == "copy_model").unwrap().description.contains("adopt_model"));
    assert_eq!(setup.ok("copy_model", json!({ "from": "saved.json", "to": "b.json" }))["changed_outside"], false);
    let adopt = setup.catalogue.operations().find(|o| o.name == "adopt_model").unwrap().description;
    assert!(adopt.contains("hash"), "comparing a hash you trust is enough: {adopt}");
}

#[test]
fn a_run_records_the_build_and_the_agent_or_user_that_started_it() {
    let setup = Setup::agent("who");
    write_models(&setup.folder);
    setup.catalogue.set_client_name("test-harness");
    let agents = setup.start(json!({ "model": "small.json", "length": "1:00", "observations": 1 }));
    let user = Catalogue::open(Settings::new(&setup.folder.path, Source::Cli)).unwrap();
    let users = user.call("start_run", json!({ "model": "small.json", "length": "1:00", "observations": 1, "wait": 30 })).unwrap()["run"].as_str().unwrap().to_string();
    setup.finished(&agents);

    let results = setup.ok("run_results", json!({ "run": agents }));
    let build = results["crabsim_build"].as_str().unwrap();
    assert!(!build.is_empty() && build != "0.1.0", "a git commit, or unknown outside git: {build}");
    assert!(results["agent"].as_str().unwrap().starts_with("agent-") && results["agent"].as_str().unwrap().contains("test-harness"), "{results}");
    assert_eq!(setup.ok("run_results", json!({ "run": users }))["agent"], "the user");
    let rows = setup.ok("query_results", json!({ "sql": "SELECT crabsim_build, agent FROM runs ORDER BY number" }))["rows"].clone();
    assert_eq!(rows[0][0], build);
}

#[test]
fn a_store_of_the_second_version_gains_build_and_agent_and_keeps_its_runs() {
    let folder = common::TempFolder::new("version-2");
    write_models(&folder);
    let run = {
        let catalogue = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).unwrap();
        catalogue.call("start_run", json!({ "model": "small.json", "length": "1:00", "observations": 1, "wait": 30 })).unwrap()["run"].as_str().unwrap().to_string()
    };
    {
        let store = rusqlite::Connection::open(folder.join(".crabsim/results.sqlite")).unwrap();
        store.execute_batch("ALTER TABLE runs DROP COLUMN crabsim_build; ALTER TABLE runs DROP COLUMN agent; PRAGMA user_version = 2;").unwrap();
    }
    let catalogue = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).unwrap();
    let results = catalogue.call("run_results", json!({ "run": run })).unwrap();
    assert_eq!(results["crabsim_build"], "unknown", "a run from before builds were recorded");
    assert_eq!(results["agent"], "unknown");
    let version = catalogue.call("query_results", json!({ "sql": "SELECT user_version FROM pragma_user_version" })).unwrap();
    assert_eq!(version["rows"][0][0], 3);
}

#[test]
fn a_batch_reports_every_change_that_cant_apply_and_every_problem_at_once() {
    let setup = Setup::new("every-problem");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL }));
    let before = setup.folder.read("line.json");
    let error = setup.err(
        "edit_model",
        json!({ "path": "line.json", "changes": [
            { "change": "set_attribute", "target": { "instance": "Work" }, "attribute": "process_time", "value": "-5" },
            { "change": "remove_instance", "name": "Ghost" },
            { "change": "connect", "from": "Nobody", "to": "Out" },
        ] }),
    );
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("changes/1 (remove_instance)") && message.contains("changes/2 (connect)"), "{message}");
    let pointers: Vec<&str> = error["problems"].as_array().unwrap().iter().map(|problem| problem["pointer"].as_str().unwrap()).collect();
    assert_eq!(pointers, ["/instances/1/attributes/process_time"], "and the problems of the changes that could apply");
    assert_eq!(setup.folder.read("line.json"), before);
}

#[test]
fn start_run_can_wait_for_a_short_run() {
    let setup = Setup::new("wait");
    write_models(&setup.folder);
    let started = setup.ok("start_run", json!({ "model": "small.json", "length": "1:00:00", "observations": 2, "wait": 30 }));
    assert_eq!(started["state"], "done");
    assert_eq!(started["status"]["ended"], 2);
    let long = setup.ok("start_run", json!({ "model": "demo.json", "length": "3650d", "observations": 1, "wait": 1 }));
    assert!(long["state"] == "running" || long["state"] == "queued", "it stops waiting: {long}");
    setup.ok("cancel_run", json!({ "run": long["run"] }));
    assert_eq!(setup.err("start_run", json!({ "model": "small.json", "length": "1:00", "wait": 61 }))["kind"], "invalid_input");
}

#[test]
fn an_agent_forgets_only_the_runs_its_own_session_started() {
    let setup = Setup::agent("forget-own");
    write_models(&setup.folder);
    let other = Catalogue::open(Settings::new(&setup.folder.path, Source::Mcp)).unwrap();
    let theirs = other.call("start_run", json!({ "model": "small.json", "length": "1:00", "observations": 1, "wait": 30 })).unwrap()["run"].as_str().unwrap().to_string();
    let mine = setup.ok("start_run", json!({ "model": "small.json", "length": "1:00", "observations": 1, "wait": 30 }))["run"].as_str().unwrap().to_string();

    let refused = setup.err("forget_run", json!({ "run": theirs }));
    assert_eq!(refused["kind"], "invalid_input");
    assert!(refused["message"].as_str().unwrap().contains("another agent"), "{refused}");
    setup.ok("label_run", json!({ "run": theirs, "label": "labels help everyone" }));
    setup.ok("forget_run", json!({ "run": mine }));
    let user = Catalogue::open(Settings::new(&setup.folder.path, Source::Cli)).unwrap();
    user.call("forget_run", json!({ "run": theirs })).unwrap();
}
