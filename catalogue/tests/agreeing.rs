//! Agreeing (ADR 0030), through `call(name, JSON)`: an agent proposes requests and expectations,
//! the user accepts them, and the agent may then question an accepted expectation but never
//! change, remove or accept what the user agreed, whatever operation it tries.

mod common;

use common::TempFolder;
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

const LINE: &str = r#"{"format_version":2,"classes":[{"name":"Press","parent":"Station"}],"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Press A","class":"Press","attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Press A","exit":1,"entrance":1},
  {"from":"Press A","to":"Out","exit":1,"entrance":1}]}"#;

struct Both {
    folder: TempFolder,
    user: Catalogue,
    agent: Catalogue,
}

impl Both {
    fn new(name: &str) -> Both {
        let folder = TempFolder::new(name);
        let user = Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        let agent = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).unwrap();
        agent.set_client_name("test-harness");
        agent.call("claim_model", json!({ "path": "line.json", "goal": "agree what the line must do", "doing": "proposing" })).unwrap();
        Both { folder, user, agent }
    }

    fn edit(catalogue: &Catalogue, changes: Value) -> Result<Value, crabsim_catalogue::OperationError> {
        catalogue.call("edit_model", json!({ "path": "line.json", "changes": changes }))
    }

    /// The agent's change, which must go through, reading the model again first if the user
    /// changed it since.
    fn agent_edits(&self, changes: Value) -> Value {
        match Both::edit(&self.agent, changes.clone()) {
            Err(error) if error.kind == crabsim_catalogue::ErrorKind::ModelChanged => Both::edit(&self.agent, changes).unwrap(),
            other => other.unwrap_or_else(|error| panic!("{error}")),
        }
    }

    /// The agent's change, which must be refused as touching what was agreed.
    fn agent_refused(&self, operation: &str, input: Value) -> String {
        let mut outcome = self.agent.call(operation, input.clone());
        if matches!(&outcome, Err(error) if error.kind == crabsim_catalogue::ErrorKind::ModelChanged) {
            outcome = self.agent.call(operation, input);
        }
        let error = outcome.expect_err("refused");
        assert_eq!(error.kind, crabsim_catalogue::ErrorKind::Agreed, "{error}");
        error.message
    }

    fn model(&self) -> crabsim_model::Model {
        crabsim_model::load(&self.folder.read("line.json")).unwrap()
    }
}

#[test]
fn an_agent_proposes_the_user_accepts_in_one_step_and_the_agent_may_only_question_then() {
    let both = Both::new("agreeing-flow");
    both.agent_edits(json!([
        { "change": "add_expectation", "id": "busy", "text": "Press A.working_share since start >= 70%" },
        { "change": "add_expectation", "id": "out", "text": "model.throughput_per_hour since start >= 350/h" },
        { "change": "add_request", "words": "keep the press busy", "restatement": "Press A works at least 70% of the time, and the line ships 350 an hour", "expectations": ["busy", "out"] }
    ]));
    let model = both.model();
    assert!(model.expectations.iter().all(|expectation| expectation.state == crabsim_model::Agreement::Proposed));
    assert_eq!((model.requests[0].id.as_str(), model.requests[0].accepted), ("r1", false));
    // Its own proposals it may change, until accepted.
    both.agent_edits(json!([{ "change": "set_expectation", "id": "out", "text": "model.throughput_per_hour since start >= 340/h" }]));
    // Accepting is the user's alone.
    let refused = Both::edit(&both.agent, json!([{ "change": "accept_request", "id": "r1" }])).unwrap_err();
    assert!(refused.message.contains("only the user accepts"), "{refused}");
    // The user accepts without taking the file from the agent, which waits for it.
    Both::edit(&both.user, json!([{ "change": "accept_request", "id": "r1" }])).unwrap();
    let model = both.model();
    assert!(model.requests[0].accepted);
    assert!(model.expectations.iter().all(|expectation| expectation.state == crabsim_model::Agreement::Accepted));

    // Now the agent can't move the goalposts, by any operation.
    let changed = both.agent_refused("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_expectation", "id": "busy", "text": "Press A.working_share since start >= 10%" }] }));
    assert!(changed.contains("changes the expectation `busy`"), "{changed}");
    both.agent_refused("edit_model", json!({ "path": "line.json", "changes": [{ "change": "remove_expectation", "id": "out" }] }));
    both.agent_refused("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_request", "id": "r1", "words": "anything" }] }));
    let shorter = both.agent_refused("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_stage_settings", "stage": "verification", "length": "1:00" }] }));
    assert!(shorter.contains("how `busy` is checked"), "{shorter}");
    let mut weakened: Value = serde_json::from_str(&both.folder.read("line.json")).unwrap();
    weakened["expectations"][0]["text"] = json!("Press A.working_share since start >= 1%");
    both.agent_refused("write_model", json!({ "path": "line.json", "text": weakened.to_string(), "overwrite": true }));
    both.agent_refused("delete_model", json!({ "path": "line.json" }));
    let commands = both.agent.call("run_commands", json!({ "path": "line.json", "commands": "set Press A.process_time 1" }));
    assert!(commands.is_ok(), "the model itself is the agent's to change: {commands:?}");

    // Renaming what an accepted expectation names keeps it, renamed with it.
    both.agent_edits(json!([{ "change": "rename_instance", "name": "Press A", "to": "Press 1" }]));
    assert_eq!(both.model().expectations[0].text, "Press 1.working_share since start >= 70%");

    // It may question one, which stays in force, and mark the request built.
    both.agent_edits(json!([
        { "change": "question_expectation", "id": "out", "question": "the Source sends 360 an hour, so 340 leaves almost nothing for a failure" },
        { "change": "set_request", "id": "r1", "built": true }
    ]));
    let model = both.model();
    assert_eq!(model.expectations[1].state, crabsim_model::Agreement::Questioned);
    assert!(model.requests[0].built);
    // Answering it is the user's.
    let accepting = Both::edit(&both.agent, json!([{ "change": "accept_expectation", "id": "out" }])).unwrap_err();
    assert!(accepting.message.contains("only the user accepts"), "{accepting}");
    both.user.set_take_over(true);
    Both::edit(&both.user, json!([{ "change": "set_expectation", "id": "out", "text": "model.throughput_per_hour since start >= 300/h" }])).unwrap();
    let answered = &both.model().expectations[1];
    assert_eq!((answered.state, answered.question.as_deref()), (crabsim_model::Agreement::Accepted, None), "the user's change answers it");
}

#[test]
fn an_agent_cant_slip_an_acceptance_in_by_writing_or_adopting_the_file() {
    let both = Both::new("agreeing-sideways");
    let mut agreed: Value = serde_json::from_str(LINE).unwrap();
    agreed["expectations"] = json!([{ "id": "busy", "text": "Press A.working_share since start >= 1%", "state": "accepted" }]);
    let written = both.agent_refused("write_model", json!({ "path": "line.json", "text": agreed.to_string(), "overwrite": true }));
    assert!(written.contains("accepts the expectation `busy`"), "{written}");
    // Written outside crabsim, it still can't be adopted by the agent, only by the user.
    both.folder.write("line.json", &agreed.to_string());
    both.agent_refused("adopt_model", json!({ "path": "line.json" }));
    both.user.set_take_over(true);
    both.user.call("adopt_model", json!({ "path": "line.json" })).unwrap();
    // The same instance under the same name is the same subject, but another one isn't.
    let mut swapped: Value = serde_json::from_str(&both.folder.read("line.json")).unwrap();
    swapped["instances"][1]["stream"] = json!(99);
    both.agent_refused("write_model", json!({ "path": "line.json", "text": swapped.to_string(), "overwrite": true }));
}

#[test]
fn the_user_writes_accepted_expectations_and_an_agent_undoes_only_what_wasnt_agreed() {
    let both = Both::new("agreeing-undo");
    both.user.set_take_over(true);
    Both::edit(&both.user, json!([{ "change": "add_expectation", "text": "Press A.working_share since start >= 70%" }])).unwrap();
    assert_eq!(both.model().expectations[0].state, crabsim_model::Agreement::Accepted, "the user's own");
    both.agent_edits(json!([{ "change": "add_expectation", "id": "mine", "text": "Out.mus_in since start >= 1" }]));
    both.agent.call("undo_model", json!({ "path": "line.json" })).unwrap();
    assert_eq!(both.model().expectations.len(), 1);
    let refused = both.agent.call("undo_model", json!({ "path": "line.json" })).unwrap_err();
    assert_eq!(refused.kind, crabsim_catalogue::ErrorKind::NotYours, "{refused}");
}
