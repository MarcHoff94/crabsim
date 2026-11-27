//! The vocabulary of figures, expectations in model files and their verdicts on runs and
//! experiments, through `call(name, JSON)` as an agent uses them (ADR 0030).

mod common;

use common::Setup;
use serde_json::{json, Value};

/// A line where the Buffer fills, since Press B is slower than the Source, with a failing Mill
/// first: Source every 10 s, Mill 5 s failing 10% of the time, Press A 8 s, Buffer, Press B 12 s,
/// Drain.
fn line(expectations: Value) -> String {
    json!({
        "format_version": 2,
        "classes": [{ "name": "Press", "parent": "Station" }],
        "instances": [
            { "name": "Source", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Mill", "class": "Station", "attributes": { "process_time": "5", "failures": [
                { "name": "Wear", "availability": 0.9, "repair_time": { "erlang": { "mean": "0:05:00", "k": 2 } } }
            ] } },
            { "name": "Press A", "class": "Press", "attributes": { "process_time": "8" } },
            { "name": "Buffer", "class": "Buffer" },
            { "name": "Press B", "class": "Press", "attributes": { "process_time": "12" } },
            { "name": "Drain", "class": "Drain" }
        ],
        "connections": [
            { "from": "Source", "to": "Mill", "exit": 1, "entrance": 1 },
            { "from": "Mill", "to": "Press A", "exit": 1, "entrance": 1 },
            { "from": "Press A", "to": "Buffer", "exit": 1, "entrance": 1 },
            { "from": "Buffer", "to": "Press B", "exit": 1, "entrance": 1 },
            { "from": "Press B", "to": "Drain", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
    .to_string()
}

fn run(setup: &Setup, input: Value) -> Value {
    let run = setup.start(input);
    setup.finished(&run);
    setup.ok("run_results", json!({ "run": run }))
}

fn expectation<'r>(results: &'r Value, id: &str) -> &'r Value {
    results["expectations"].as_array().unwrap().iter().find(|expectation| expectation["id"] == id).unwrap_or_else(|| panic!("no {id} in {results:#}"))
}

#[test]
fn list_figures_gives_the_vocabulary_with_units_and_scopes() {
    let setup = Setup::agent("figures-list");
    let all = setup.ok("list_figures", json!({}));
    let names: Vec<&str> = all["figures"].as_array().unwrap().iter().map(|figure| figure["name"].as_str().unwrap()).collect();
    // Every statistic the store keeps is in the vocabulary.
    for statistic in crabsim_catalogue::STATISTICS.iter() {
        assert!(names.contains(statistic), "{statistic}");
    }
    let drain = setup.ok("list_figures", json!({ "of": "Drain" }));
    let throughput = drain["figures"].as_array().unwrap().iter().find(|figure| figure["name"] == "throughput_per_hour").unwrap();
    assert_eq!(throughput["unit"], "per hour");
    assert_eq!(throughput["scopes"], json!(["since start", "last <time>"]));
    assert!(throughput["of"].as_array().unwrap().contains(&json!("model")));
    let model = setup.ok("list_figures", json!({ "of": "model" }));
    assert_eq!(model["figures"].as_array().unwrap().len(), 3);
    let mu = setup.ok("list_figures", json!({ "of": "MU" }));
    assert!(mu["figures"].as_array().unwrap().iter().any(|figure| figure["name"] == "next_destination"));
    let wrong = setup.err("list_figures", json!({ "of": "Robot" }));
    assert!(wrong["message"].as_str().unwrap().contains("Station"), "{wrong}");
}

#[test]
fn a_deterministic_run_fails_nothing_takes_one_observation_and_judges_its_expectations() {
    let setup = Setup::agent("figures-deterministic");
    setup.folder.write(
        "line.json",
        &line(json!([
            { "id": "presses", "text": "every Press.working_share since start >= 60%" },
            { "id": "mill", "text": "Mill.failed_share since start = 0%" },
            { "id": "rate", "text": "model.throughput_per_hour last 1h >= 299/h" },
            { "id": "full", "text": "Buffer.mus always <= 20" },
            { "id": "lead", "text": "Drain.mean_lead_time_s since start <= 30s", "stage": "validation" }
        ])),
    );
    let results = run(&setup, json!({ "model": "line.json", "length": "4:00:00", "deterministic": true }));
    assert_eq!(results["observations"], 1, "a deterministic run needs one");
    assert_eq!(results["deterministic"], true);
    assert_eq!(expectation(&results, "presses")["verdict"], "met");
    assert_eq!(expectation(&results, "presses")["met"], "2 of 2");
    assert_eq!(expectation(&results, "mill")["verdict"], "met", "no failure in a deterministic run");
    assert_eq!(expectation(&results, "rate")["verdict"], "met");
    assert_eq!(expectation(&results, "rate")["mean"], "300/h");
    assert_eq!(expectation(&results, "rate")["scope"], "last 1h");
    let full = expectation(&results, "full");
    assert_eq!(full["verdict"], "not_met");
    assert_eq!(full["violation"]["object"], "Buffer");
    assert_eq!(full["violation"]["value"], "21");
    assert!(full["violation"]["mu"].as_u64().is_some());
    let lead = expectation(&results, "lead");
    assert_eq!((lead["verdict"].as_str(), lead["stage"].as_str()), (Some("not_met"), Some("validation")));
    assert!(lead.get("advice").is_none(), "one deterministic observation decides: {lead}");
    let stored = setup.ok("query_results", json!({ "sql": format!("SELECT deterministic FROM runs WHERE id = '{}'", results["run"].as_str().unwrap()) }));
    assert_eq!(stored["rows"][0][0], 1);
    // The same model with randomness fails.
    let random = run(&setup, json!({ "model": "line.json", "length": "4:00:00", "observations": 2 }));
    assert_eq!(expectation(&random, "mill")["verdict"], "not_met");
}

#[test]
fn an_experiment_is_undecided_until_enough_observations_decide_it() {
    let setup = Setup::agent("figures-undecided");
    setup.folder.write("line.json", &line(json!([{ "id": "mill", "text": "Mill.failed_share since start <= 11%" }])));
    let few = run(&setup, json!({ "model": "line.json", "length": "8:00:00", "observations": 3, "seed": 4 }));
    let mill = expectation(&few, "mill");
    assert_eq!(mill["verdict"], "undecided", "{mill:#}");
    assert!(mill["advice"].as_str().unwrap().contains("more observations"), "{mill:#}");
    assert!(mill["ci95"][0].as_str().unwrap().ends_with('%'));
    let many = run(&setup, json!({ "model": "line.json", "length": "8:00:00", "observations": 200, "seed": 4 }));
    assert_eq!(expectation(&many, "mill")["verdict"], "met", "{:#}", expectation(&many, "mill"));
    // One observation of a run with randomness holds only so far.
    let one = run(&setup, json!({ "model": "line.json", "length": "8:00:00", "observations": 1, "seed": 4 }));
    assert!(expectation(&one, "mill")["advice"].as_str().unwrap().contains("so far"));
}

#[test]
fn a_stretch_longer_than_the_run_or_a_figure_without_a_value_is_an_error() {
    let setup = Setup::agent("figures-errors");
    setup.folder.write(
        "line.json",
        &line(json!([
            { "id": "stretch", "text": "Drain.mus_in last 2h >= 1" },
            { "id": "lead", "text": "Drain.mean_lead_time_s since start <= 1h" }
        ])),
    );
    // In 30 s nothing reaches the Drain.
    let results = run(&setup, json!({ "model": "line.json", "length": "0:30", "deterministic": true }));
    assert_eq!(expectation(&results, "stretch")["verdict"], "error");
    assert_eq!(expectation(&results, "lead")["verdict"], "error");
    assert!(expectation(&results, "lead")["why"].as_str().unwrap().contains("no value"));
}

#[test]
fn expectations_are_added_changed_renamed_with_their_subject_and_removed() {
    let setup = Setup::new("figures-edit");
    setup.ok("write_model", json!({ "path": "line.json", "text": line(json!([])) }));
    let edit = |changes: Value| setup.ok("edit_model", json!({ "path": "line.json", "changes": changes }));
    edit(json!([
        { "change": "add_expectation", "text": "Press A.working_share since start >= 60%" },
        { "change": "add_expectation", "id": "presses", "text": "every  Press.idle_share since start <= 50%", "assumptions": "one shift" }
    ]));
    let read = |setup: &Setup| crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let model = read(&setup);
    assert_eq!(model.expectations.iter().map(|expectation| expectation.id.as_str()).collect::<Vec<_>>(), ["e1", "presses"]);
    assert_eq!(model.expectations[1].text, "every Press.idle_share since start <= 50%");
    // Renaming an instance or a class renames it where an expectation names it.
    edit(json!([
        { "change": "rename_instance", "name": "Press A", "to": "Press 1" },
        { "change": "rename_class", "name": "Press", "to": "Former" }
    ]));
    let model = read(&setup);
    assert_eq!(model.expectations[0].text, "Press 1.working_share since start >= 60%");
    assert_eq!(model.expectations[1].text, "every Former.idle_share since start <= 50%");
    edit(json!([
        { "change": "set_expectation", "id": "presses", "stage": "validation", "assumptions": "" },
        { "change": "remove_expectation", "id": "e1" }
    ]));
    let model = read(&setup);
    assert_eq!(model.expectations.len(), 1);
    assert_eq!(model.expectations[0].stage, crabsim_model::CheckStage::Validation);
    assert_eq!(model.expectations[0].assumptions, None);
    // A line that doesn't read refuses the batch, saying where.
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [{ "change": "add_expectation", "text": "Press 1.working_share since start >= 60" }] }));
    assert_eq!(refused["kind"], "invalid_model");
    assert!(refused.to_string().contains("/expectations/1/text"), "{refused}");
    assert!(refused.to_string().contains("percentage"), "{refused}");
    // Removing an instance an expectation names leaves a problem, so it is refused until the
    // expectation goes too.
    let removed = setup.err("edit_model", json!({ "path": "line.json", "changes": [{ "change": "remove_instance", "name": "Mill" }, { "change": "add_expectation", "id": "m", "text": "Mill.idle_share since start <= 10%" }] }));
    assert_eq!(removed["kind"], "invalid_model");
    let unknown = setup.err("edit_model", json!({ "path": "line.json", "changes": [{ "change": "remove_expectation", "id": "nope" }] }));
    assert!(unknown.to_string().contains("there are presses"), "{unknown}");
}
