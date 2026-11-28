//! Bounds and reference figures (ADR 0030), through `call(name, JSON)`: an agent learns from
//! get_status and validate_model, before anything runs, that an expectation can't be met, and
//! what to do about it; a reference figure is judged by its tolerance.

mod common;

use common::TempFolder;
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

/// Source every 10 s, Press A 8 s, then a Drain: at most 360 MUs an hour.
fn line(expectations: Value) -> Value {
    json!({
        "format_version": 2,
        "instances": [
            { "name": "In", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Press A", "class": "Station", "attributes": { "process_time": "8" } },
            { "name": "Out", "class": "Drain" }
        ],
        "connections": [
            { "from": "In", "to": "Press A", "exit": 1, "entrance": 1 },
            { "from": "Press A", "to": "Out", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
}

fn open(name: &str, model: &Value) -> (TempFolder, Catalogue) {
    let folder = TempFolder::new(name);
    let user = Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap();
    user.call("write_model", json!({ "path": "line.json", "text": model.to_string() })).unwrap();
    (folder, user)
}

fn status(catalogue: &Catalogue) -> Value {
    let status = catalogue.call("get_status", json!({ "path": "line.json" })).unwrap();
    common::assert_fits_its_schema(catalogue, "get_status", &status);
    status
}

fn expectation<'s>(status: &'s Value, id: &str) -> &'s Value {
    status["expectations"].as_array().unwrap().iter().find(|expectation| expectation["id"] == id).unwrap_or_else(|| panic!("no {id} in {status:#}"))
}

#[test]
fn an_agent_learns_before_anything_runs_that_an_expectation_cant_be_met_and_what_to_do() {
    let (folder, user) = open("bounds-agent", &line(json!([])));
    let agent = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).unwrap();
    agent.set_client_name("test-harness");
    agent.call("claim_model", json!({ "path": "line.json", "goal": "a faster line", "doing": "agreeing" })).unwrap();
    agent
        .call(
            "edit_model",
            json!({ "path": "line.json", "changes": [
                { "change": "add_expectation", "id": "fast", "text": "Out.throughput_per_hour since start >= 400/h" },
                { "change": "add_expectation", "id": "busy", "text": "Press A.working_share since start >= 70%" }
            ] }),
        )
        .unwrap();
    let seen = status(&agent);
    let fast = expectation(&seen, "fast");
    let why = fast["cannot_be_met"].as_str().expect("a bound");
    assert!(why.contains("In (an MU every 10s)") && why.contains("360/h"), "{why}");
    assert!(expectation(&seen, "busy").get("cannot_be_met").is_none(), "{seen:#}");
    assert!(seen["next"].as_str().unwrap().starts_with("crabsim works out that fast can't be met as the model is") && seen["next"].as_str().unwrap().ends_with("change the model or it before asking the user to accept it"), "{seen:#}");
    // validate_model says so in its warnings, at the line.
    let validated = agent.call("validate_model", json!({ "path": "line.json" })).unwrap();
    let warning = validated["warnings"].as_array().unwrap().iter().find(|warning| warning["pointer"] == "/expectations/0/text").expect("a warning at the line");
    assert_eq!(warning["name"], "fast");
    // Nothing ran to find it.
    let runs = user.call("query_results", json!({ "sql": "SELECT count(*) FROM runs" })).unwrap();
    assert_eq!(runs["rows"][0][0], 0);

    // Accepted anyway, the agent is to question it.
    user.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "accept_expectation", "id": "fast" }, { "change": "accept_expectation", "id": "busy" }] })).unwrap();
    let next = status(&agent)["next"].as_str().unwrap().to_string();
    assert!(next.contains("change the model, or question it with edit_model question_expectation"), "{next}");
    assert!(status(&user)["next"].as_str().unwrap().ends_with("change the model, or change or remove it"), "{:#}", status(&user));
    // The user's acceptance is a change, so the agent's next change is refused once.
    let question = json!({ "path": "line.json", "changes": [{ "change": "question_expectation", "id": "fast", "question": why }] });
    assert!(format!("{:?}", agent.call("edit_model", question.clone()).unwrap_err()).contains("ModelChanged"));
    agent.call("edit_model", question).unwrap();
    let next = status(&agent)["next"].as_str().unwrap().to_string();
    assert!(next.starts_with("the user answers the question on fast"), "{next}");
}

#[test]
fn a_reference_figure_is_met_once_its_interval_lies_within_its_tolerance() {
    let mut model = line(json!([
        { "id": "measured", "text": "Out.mus_in since start = 8200 ± 3%", "stage": "validation", "source": "MES, October", "state": "accepted" },
        { "id": "too-tight", "text": "Out.mus_in since start = 8600 ± 1%", "stage": "validation", "source": "MES, October", "state": "accepted" }
    ]));
    // Press A fails now and then, so the runs differ, and it falls behind the Source: about 8200
    // MUs a day rather than 8640.
    model["instances"][1]["attributes"]["failures"] = json!([{ "name": "Wear", "availability": 0.95, "repair_time": { "erlang": { "mean": "00:01:00", "k": 2 } } }]);
    let (_folder, user) = open("bounds-reference", &model);
    let checked = user.call("check_stage", json!({ "path": "line.json", "stage": "validation", "wait": 60 })).unwrap();
    assert_eq!(expectation(&checked["status"], "measured")["verdict"], "met", "{checked:#}");
    assert_eq!(expectation(&checked["status"], "measured")["source"], "MES, October");
    assert_eq!(expectation(&checked["status"], "too-tight")["verdict"], "not_met", "{checked:#}");
    let run = checked["runs"][0].as_str().unwrap();
    let results = user.call("run_results", json!({ "run": run })).unwrap();
    let measured = results["expectations"].as_array().unwrap().iter().find(|result| result["id"] == "measured").unwrap();
    assert_eq!((measured["value"].clone(), measured["tolerance"].clone(), measured["band"].clone()), (json!("8200"), json!("3%"), json!(["7954", "8446"])), "{measured:#}");
    let [low, high] = [&measured["ci95"][0], &measured["ci95"][1]].map(|value| value.as_str().unwrap().parse::<f64>().unwrap());
    assert!(7954.0 <= low && high <= 8446.0, "{measured:#}");
}
