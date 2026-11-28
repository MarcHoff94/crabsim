//! Stages (ADR 0030), through `call(name, JSON)`: checks of the verification and validation
//! stages, a model's stage dropping back only when a check finds a change broke it, verification
//! at its milestones and not after every change, and get_status's requests and next step.

mod common;

use common::TempFolder;
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

/// Source every 10 s, Press A 8 s, so it works 80% of the time, then a Drain.
fn line(expectations: Value) -> String {
    json!({
        "format_version": 2,
        "classes": [{ "name": "Press", "parent": "Station" }],
        "instances": [
            { "name": "In", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Press A", "class": "Press", "attributes": { "process_time": "8" } },
            { "name": "Out", "class": "Drain" }
        ],
        "connections": [
            { "from": "In", "to": "Press A", "exit": 1, "entrance": 1 },
            { "from": "Press A", "to": "Out", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
    .to_string()
}

struct Folder {
    folder: TempFolder,
    user: Catalogue,
}

impl Folder {
    fn new(name: &str, expectations: Value) -> Folder {
        let folder = TempFolder::new(name);
        let user = Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": line(expectations) })).unwrap();
        Folder { folder, user }
    }

    fn agent(&self) -> Catalogue {
        let agent = Catalogue::open(Settings::new(&self.folder.path, Source::Mcp)).unwrap();
        agent.set_client_name("test-harness");
        agent
    }

    fn status(&self, catalogue: &Catalogue) -> Value {
        let status = catalogue.call("get_status", json!({ "path": "line.json" })).unwrap();
        common::assert_fits_its_schema(catalogue, "get_status", &status);
        status
    }

    /// Checks a stage and waits for it.
    fn check(&self, stage: &str) -> Value {
        let checked = self.user.call("check_stage", json!({ "path": "line.json", "stage": stage, "wait": 60 })).unwrap();
        common::assert_fits_its_schema(&self.user, "check_stage", &checked);
        checked
    }

    fn edit(&self, changes: Value) {
        self.user.call("edit_model", json!({ "path": "line.json", "changes": changes })).unwrap();
    }

    fn runs(&self) -> usize {
        let rows = self.user.call("query_results", json!({ "sql": "SELECT count(*) FROM runs" })).unwrap();
        rows["rows"][0][0].as_u64().unwrap() as usize
    }

    /// Waits until no run of `catalogue` is going.
    fn settle(&self, catalogue: &Catalogue) {
        catalogue.wait_for_runs();
    }
}

fn expectation<'s>(status: &'s Value, id: &str) -> &'s Value {
    status["expectations"].as_array().unwrap().iter().find(|expectation| expectation["id"] == id).unwrap_or_else(|| panic!("no {id} in {status:#}"))
}

#[test]
fn a_model_is_verified_and_drops_back_only_once_a_check_finds_a_change_broke_it() {
    let folder = Folder::new("stages-verify", json!([{ "id": "busy", "text": "Press A.working_share since start >= 70%", "state": "accepted" }]));
    assert_eq!(folder.status(&folder.user)["stage"], "draft");
    assert_eq!(folder.status(&folder.user)["verification"]["summary"], "never checked");
    let checked = folder.check("verification");
    assert_eq!(checked["started"], true);
    assert_eq!(checked["status"]["stage"], "verified", "{checked:#}");
    assert_eq!(checked["status"]["verification"]["current"], true);
    assert_eq!(expectation(&checked["status"], "busy")["verdict"], "met");
    assert_eq!(expectation(&checked["status"], "busy")["found"], "79.99%", "the share of the day after the first MU's arrival");
    // Checking again as it is gives the same check.
    let again = folder.check("verification");
    assert_eq!((again["started"].clone(), again["runs"].clone()), (json!(false), checked["runs"].clone()));

    // A change isn't checked by itself, so the stage stays, not checked since that change.
    let before = folder.runs();
    folder.edit(json!([{ "change": "set_attribute", "target": { "instance": "Press A" }, "attribute": "process_time", "value": "5" }]));
    assert_eq!(folder.runs(), before, "no check after a change");
    let status = folder.status(&folder.user);
    assert_eq!(status["stage"], "verified");
    assert_eq!(status["verification"]["current"], false);
    let change = status["verification"]["not_checked_since"].as_i64().expect("a change since");
    assert!(status["verification"]["summary"].as_str().unwrap().ends_with(&format!("not checked since change {change}")), "{status:#}");
    assert_eq!(expectation(&status, "busy")["current"], false);
    // At 5 s a Press fed every 10 s works at most half the time, which crabsim says before a
    // check finds it.
    assert!(status["next"].as_str().unwrap().starts_with("crabsim works out that busy can't be met as the model is"), "{status:#}");
    // Checked, it drops back.
    let checked = folder.check("verification");
    assert_eq!(checked["status"]["stage"], "draft");
    assert_eq!(expectation(&checked["status"], "busy")["verdict"], "not_met");
    assert!(checked["status"]["next"].as_str().unwrap().contains("busy"), "{checked:#}");
    // Undone, the file is as it was when verified, which counts at once.
    folder.user.call("undo_model", json!({ "path": "line.json" })).unwrap();
    let status = folder.status(&folder.user);
    assert_eq!((status["stage"].clone(), status["verification"]["current"].clone()), (json!("draft"), json!(false)), "the latest check is of the 5 s version");
    let checked = folder.check("verification");
    assert_eq!(checked["status"]["stage"], "verified");
}

#[test]
fn only_what_the_user_accepted_makes_a_stage_and_validation_comes_after_verification() {
    let folder = Folder::new(
        "stages-validate",
        json!([
            { "id": "busy", "text": "Press A.working_share since start >= 70%" },
            { "id": "rate", "text": "model.throughput_per_hour since start >= 300/h", "stage": "validation", "observations": 3 }
        ]),
    );
    let checked = folder.check("verification");
    assert_eq!(expectation(&checked["status"], "busy")["verdict"], "met", "a proposed one is judged");
    assert_eq!(checked["status"]["stage"], "draft", "but no stage rests on it");
    assert!(checked["status"]["next"].as_str().unwrap().starts_with("accept the expectations busy, rate"), "{checked:#}");
    folder.edit(json!([{ "change": "accept_expectation", "id": "busy" }, { "change": "accept_expectation", "id": "rate" }]));
    assert_eq!(folder.check("verification")["status"]["stage"], "verified");
    let validated = folder.check("validation");
    assert_eq!(validated["runs"].as_array().unwrap().len(), 1);
    assert_eq!(validated["status"]["stage"], "validated", "{validated:#}");
    assert_eq!(validated["status"]["validation"]["settings"]["observations"], 10, "the stage's own");
    assert!(validated["status"]["next"].as_str().unwrap().starts_with("validated"));
    // The validation run had randomness and its expectation's own observations.
    let run = validated["runs"][0].as_str().unwrap();
    let rows = folder.user.call("query_results", json!({ "sql": format!("SELECT deterministic, observations, stage FROM runs WHERE id = '{run}'") })).unwrap();
    assert_eq!(rows["rows"][0], json!([0, 3, "validation"]));
}

#[test]
fn expectations_checked_otherwise_than_their_stage_get_a_run_of_their_own() {
    let folder = Folder::new(
        "stages-groups",
        json!([
            { "id": "busy", "text": "Press A.working_share since start >= 70%", "state": "accepted" },
            { "id": "night", "text": "Out.mus_in since start >= 2000", "state": "accepted", "length": "7d" }
        ]),
    );
    let checked = folder.check("verification");
    let runs = checked["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(checked["status"]["stage"], "verified", "{checked:#}");
    for run in runs {
        let results = folder.user.call("run_results", json!({ "run": run })).unwrap();
        assert_eq!(results["expectations"].as_array().unwrap().len(), 1, "each run judges its own: {results:#}");
    }
}

#[test]
fn an_agent_releasing_its_claim_or_an_experiment_has_the_model_verified_if_it_isnt_as_it_is() {
    let folder = Folder::new("stages-milestones", json!([{ "id": "busy", "text": "Press A.working_share since start >= 70%", "state": "accepted" }]));
    let agent = folder.agent();
    agent.call("claim_model", json!({ "path": "line.json", "goal": "a faster press", "doing": "tuning" })).unwrap();
    agent.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_attribute", "target": { "instance": "Press A" }, "attribute": "process_time", "value": "9" }] })).unwrap();
    let released = agent.call("release_model", json!({ "path": "line.json" })).unwrap();
    let verifying = released["verification"].as_array().expect("verifying").len();
    assert_eq!(verifying, 1, "{released:#}");
    folder.settle(&agent);
    assert_eq!(folder.status(&agent)["stage"], "verified");
    // Claimed and released again with no change, nothing runs.
    agent.call("claim_model", json!({ "path": "line.json", "goal": "a faster press", "doing": "looking" })).unwrap();
    assert!(agent.call("release_model", json!({ "path": "line.json" })).unwrap().get("verification").is_none());

    // An experiment on a model changed since is verified first, a deterministic run isn't.
    folder.edit(json!([{ "change": "set_attribute", "target": { "instance": "Press A" }, "attribute": "process_time", "value": "8" }]));
    let deterministic = folder.user.call("start_run", json!({ "model": "line.json", "length": "1:00:00", "deterministic": true })).unwrap();
    assert!(deterministic.get("verification").is_none(), "{deterministic:#}");
    let experiment = folder.user.call("start_run", json!({ "model": "line.json", "length": "1:00:00", "observations": 2 })).unwrap();
    assert_eq!(experiment["verification"].as_array().unwrap().len(), 1, "{experiment:#}");
    folder.settle(&folder.user);
    let status = folder.status(&folder.user);
    assert_eq!((status["stage"].clone(), status["verification"]["current"].clone()), (json!("verified"), json!(true)));
    let experiment = folder.user.call("start_run", json!({ "model": "line.json", "length": "1:00:00", "observations": 2 })).unwrap();
    assert!(experiment.get("verification").is_none(), "verified as it is: {experiment:#}");
    folder.settle(&folder.user);
}

#[test]
fn a_request_goes_from_open_to_built_to_met_or_broken_and_the_next_step_follows() {
    let folder = Folder::new("stages-requests", json!([]));
    let agent = folder.agent();
    assert!(folder.status(&agent)["next"].as_str().unwrap().starts_with("agree what the model must achieve"));
    agent.call("claim_model", json!({ "path": "line.json", "goal": "keep the press busy", "doing": "agreeing" })).unwrap();
    agent.call("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "add_expectation", "id": "busy", "text": "Press A.working_share since start >= 70%" },
        { "change": "add_request", "words": "keep the press busy", "restatement": "Press A working at least 70% of the time", "expectations": ["busy"] }
    ] })).unwrap();
    let status = folder.status(&agent);
    assert!(status["next"].as_str().unwrap().starts_with("ask the user to accept the requests r1"), "{status:#}");
    folder.user.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "accept_request", "id": "r1" }] })).unwrap();
    let status = folder.status(&agent);
    assert_eq!(status["requests"][0]["state"], "open");
    assert!(status["next"].as_str().unwrap().starts_with("build r1"), "{status:#}");
    let mark = json!({ "path": "line.json", "changes": [{ "change": "set_request", "id": "r1", "built": true }] });
    if agent.call("edit_model", mark.clone()).is_err() {
        // Read again after the user's change, as an agent is told to.
        agent.call("edit_model", mark).unwrap();
    }
    assert_eq!(folder.status(&agent)["requests"][0]["state"], "built", "built, not checked yet");
    agent.call("release_model", json!({ "path": "line.json" })).unwrap();
    folder.settle(&agent);
    let status = folder.status(&agent);
    assert_eq!(status["requests"][0]["state"], "met", "{status:#}");
    assert!(status["next"].as_str().unwrap().starts_with("verified. Next: add randomness"), "{status:#}");
    folder.edit(json!([{ "change": "set_attribute", "target": { "instance": "Press A" }, "attribute": "process_time", "value": "2" }]));
    assert_eq!(folder.status(&agent)["requests"][0]["state"], "built", "not checked since the change");
    folder.check("verification");
    assert_eq!(folder.status(&agent)["requests"][0]["state"], "broken");
}

#[test]
fn a_store_of_the_eighth_version_gains_the_columns_of_stages_checks() {
    let folder = Folder::new("stages-upgrade", json!([{ "id": "busy", "text": "Press A.working_share since start >= 70%", "state": "accepted" }]));
    folder.check("verification");
    drop(folder.user);
    {
        let store = rusqlite::Connection::open(folder.folder.join(".crabsim/results.sqlite")).unwrap();
        store
            .execute_batch("ALTER TABLE runs DROP COLUMN stage; ALTER TABLE runs DROP COLUMN check_number; ALTER TABLE runs DROP COLUMN checked_change; ALTER TABLE runs DROP COLUMN expectations; PRAGMA user_version = 8;")
            .unwrap();
    }
    let user = Catalogue::open(Settings::new(&folder.folder.path, Source::Cli)).unwrap();
    let version = user.call("query_results", json!({ "sql": "SELECT user_version FROM pragma_user_version" })).unwrap();
    assert_eq!(version["rows"][0][0], 10);
    // The old run was an experiment, so the model is unchecked until checked again.
    let status = user.call("get_status", json!({ "path": "line.json" })).unwrap();
    assert_eq!(status["verification"]["summary"], "never checked");
    let checked = user.call("check_stage", json!({ "path": "line.json", "wait": 60 })).unwrap();
    assert_eq!(checked["status"]["stage"], "verified");
}
