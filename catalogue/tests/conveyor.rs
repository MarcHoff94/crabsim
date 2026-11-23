//! A conveyor through the catalogue, as an agent builds, checks and runs one (spec issue 133,
//! PR 2).

mod common;

use common::Setup;
use serde_json::{json, Value};

/// In (one MU every 10 s) -> Belt, a 10 m conveyor -> Mill (15 s) -> Out.
const BELTED: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Belt","class":"Conveyor","attributes":{"path":[{"straight":10}]}},
  {"name":"Mill","class":"Station","attributes":{"process_time":"15"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Belt","exit":1,"entrance":1},
  {"from":"Belt","to":"Mill","exit":1,"entrance":1},
  {"from":"Mill","to":"Out","exit":1,"entrance":1}]}"#;

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": BELTED }));
    setup
}

fn attribute(setup: &Setup, name: &str) -> Value {
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Belt" } }));
    got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == name).unwrap_or_else(|| panic!("no {name} in {got}")).clone()
}

#[test]
fn a_conveyor_has_its_attributes_with_defaults_and_they_change_through_edit_model() {
    let setup = setup("conveyor-attributes");
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Belt" } }));
    let names: Vec<&str> = got["attributes"].as_array().unwrap().iter().map(|a| a["attribute"].as_str().unwrap()).collect();
    assert_eq!(names, ["failures", "dimensions", "description", "state_notes", "distribution", "width", "path", "speed", "gap", "accumulating"], "every attribute a Conveyor has");
    assert_eq!(attribute(&setup, "accumulating"), json!({ "attribute": "accumulating", "value": true, "from": "built_in" }));
    assert_eq!(attribute(&setup, "speed"), json!({ "attribute": "speed", "value": 0.5, "from": "built_in" }));
    assert_eq!(attribute(&setup, "path"), json!({ "attribute": "path", "value": [{ "straight": 10.0 }], "from": "instance" }));

    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Belt" }, "attribute": "speed", "value": 1.0 },
        { "change": "set_attribute", "target": { "instance": "Belt" }, "attribute": "accumulating", "value": false },
        { "change": "set_attribute", "target": { "class": "Conveyor" }, "attribute": "gap", "value": 0.2 } ] }));
    assert_eq!(attribute(&setup, "accumulating")["value"], false);
    assert_eq!(attribute(&setup, "speed")["value"], 1.0);
    assert_eq!(attribute(&setup, "gap"), json!({ "attribute": "gap", "value": 0.2, "from": "class", "class": "Conveyor" }));
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Belt" }, "attribute": "path", "value": [{ "helix": { "turns": 2 } }] } ] }));
    assert!(refused.to_string().contains("helices come with curved paths"), "{refused}");
}

#[test]
fn the_command_line_places_a_conveyor_and_sets_its_values() {
    let setup = setup("conveyor-commands");
    let placed = setup.ok("run_commands", json!({ "path": "line.json", "commands": "add Conveyor as Feeder; set Feeder.speed 0.8; set Feeder.path [{\"straight\": 6}]" }));
    assert_eq!(placed["placed"], json!(["Feeder"]), "{placed}");
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let feeder = model.instances.iter().position(|instance| instance.name == "Feeder").unwrap();
    let values = model.resolve(feeder).unwrap().attributes;
    assert_eq!((values.speed, crabsim_model::path_length(&values.path.unwrap())), (Some(0.8), 6.0));
}

#[test]
fn validate_model_warns_about_mus_too_wide_for_a_conveyor() {
    let setup = setup("conveyor-warning");
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "In" }, "attribute": "mu_values", "value": { "width": 0.9 } } ] }));
    let validated = setup.ok("validate_model", json!({ "path": "line.json" }));
    let warnings = validated["warnings"].to_string();
    assert!(warnings.contains("0.9 m wide, and conveyor `Belt` only 0.6 m"), "{validated}");
}

#[test]
fn a_model_with_a_conveyor_runs_and_its_successors_receiving_is_stored() {
    let setup = setup("conveyor-run");
    let run = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 1 }));
    assert_eq!(setup.finished(&run)["state"], "done");
    let mill = setup.stored(&run, "Mill");
    let figures = &mill[&1];
    // The Mill takes 15 s, so MUs back up on the conveyor, and the next is taken the moment the
    // Mill is done: 15.6 s a cycle, 0.6 s of it receiving, about 4% of the time.
    assert!((figures["receiving_share"] - 0.04).abs() < 0.01, "{figures:?}");
    let belt = setup.stored(&run, "Belt");
    assert!(belt[&1]["blocked_share"] > 0.5, "nothing on it moves while the Mill works: {:?}", belt[&1]);
    assert!(setup.stored(&run, "In")[&1]["blocked_share"] > 0.3, "the full conveyor holds the Source up");
}

// ---------- Conveyor statistics (spec issue 133, PR 4) ----------

#[test]
fn a_conveyors_statistics_are_stored_and_read_back_with_its_fill_histogram() {
    let setup = setup("conveyor-statistics");
    let run = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 2 }));
    assert_eq!(setup.finished(&run)["state"], "done");

    let stored = setup.stored(&run, "Belt");
    for figures in stored.values() {
        let parts = figures["working_full_share"] + figures.get("working_not_full_share").copied().unwrap_or(0.0) + figures.get("working_partly_blocked_share").copied().unwrap_or(0.0);
        assert!((parts - figures["working_share"]).abs() < 1e-9, "the three parts of working add up to it: {figures:?}");
        let shares = ["working_share", "blocked_share", "idle_share", "receiving_share", "failed_share", "paused_share"].iter().filter_map(|name| figures.get(*name)).sum::<f64>();
        assert!((shares - 1.0).abs() < 1e-9, "{figures:?}");
        // 20 s across, and longer where the Mill holds the first MU up.
        assert!(figures["mean_crossing_time_s"] >= 20.0 && figures["max_crossing_time_s"] >= figures["mean_crossing_time_s"], "{figures:?}");
        assert!(figures["max_length_taken_share"] > 0.9, "the conveyor fills up behind the Mill: {figures:?}");
        assert!(figures["mean_mus"] > 10.0 && figures["max_mus"] >= figures["mean_mus"] && figures["min_mus"] >= 0.0, "{figures:?}");
        assert!(!figures.contains_key("full_share"), "a conveyor has no limit in MUs");
    }
    assert!(!setup.stored(&run, "Mill")[&1].contains_key("mean_crossing_time_s"), "only a conveyor has crossing times");

    let bars = setup.ok("query_results", json!({ "sql": format!("SELECT count(*) FROM histograms WHERE run = '{run}' AND object = 'Belt'") }));
    assert!(bars["rows"][0][0].as_i64().unwrap() > 3, "{bars}");

    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Belt"] }));
    common::assert_fits_its_schema(&setup.catalogue, "run_results", &results);
    let belt = &results["objects"][0];
    assert_eq!(belt["built_in_class"], "Conveyor");
    assert!(belt["figures"]["mean_crossing_time_s"]["mean"].as_f64().unwrap() >= 20.0, "{belt:#}");
    assert!(belt["histogram"].as_array().is_some_and(|bars| bars.len() > 3), "its fill histogram, as a Buffer's: {belt:#}");
}

#[test]
fn a_conveyor_that_is_full_all_the_time_is_the_likely_bottleneck_and_compare_runs_sees_a_faster_one() {
    // Totes booked at their front enter the moment their predecessor's rear is clear, so a slow
    // conveyor is full (its start taken) throughout, which holds the Source up.
    let line = |speed: f64| {
        format!(
            r#"{{"format_version":2,"classes":[{{"name":"Tote","parent":"MU","attributes":{{"booking_point":0.0}}}}],"instances":[
  {{"name":"In","class":"Source","attributes":{{"interval":"1","mu_class":"Tote"}}}},
  {{"name":"Belt","class":"Conveyor","attributes":{{"path":[{{"straight":10}}],"speed":{speed}}}}},
  {{"name":"Mill","class":"Station","attributes":{{"process_time":"1"}}}},
  {{"name":"Out","class":"Drain"}}],"connections":[
  {{"from":"In","to":"Belt","exit":1,"entrance":1}},
  {{"from":"Belt","to":"Mill","exit":1,"entrance":1}},
  {{"from":"Mill","to":"Out","exit":1,"entrance":1}}]}}"#
        )
    };
    let setup = Setup::new("conveyor-bottleneck");
    setup.ok("write_model", json!({ "path": "slow.json", "text": line(0.1) }));
    setup.ok("write_model", json!({ "path": "quick.json", "text": line(0.3) }));
    let slow = setup.start(json!({ "model": "slow.json", "seed": 1, "length": "1:00:00", "observations": 3 }));
    setup.finished(&slow);
    let quick = setup.start(json!({ "model": "quick.json", "seed": 1, "length": "1:00:00", "observations": 3 }));
    setup.finished(&quick);

    let found = setup.ok("find_bottleneck", json!({ "run": slow }));
    common::assert_fits_its_schema(&setup.catalogue, "find_bottleneck", &found);
    assert_eq!(found["likely"]["instance"], "Belt", "{found:#}");
    assert!(found["likely"]["full_or_interrupted_share"].as_f64().unwrap() > 0.9, "{found:#}");
    assert_eq!(found["likely"]["upstream"][0], json!({ "instance": "In", "blocked_share": found["likely"]["upstream"][0]["blocked_share"] }));
    assert!(found["likely"]["upstream"][0]["blocked_share"].as_f64().unwrap() > 0.5, "the Source is held up: {found:#}");
    let ranking: Vec<&str> = found["ranking"].as_array().unwrap().iter().map(|r| r["instance"].as_str().unwrap()).collect();
    assert_eq!(ranking, ["Belt", "Mill"], "Stations and Conveyors, not the Source");

    let compared = setup.ok("compare_runs", json!({ "a": slow, "b": quick, "objects": ["Belt"] }));
    let belt = compared["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Belt").unwrap_or_else(|| panic!("{compared:#}"));
    assert!(belt["figures"].get("mean_crossing_time_s").is_some(), "{belt:#}");
}
