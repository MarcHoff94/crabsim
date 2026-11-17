//! A Station with capacity through the catalogue, as an agent builds, checks and runs one (spec
//! issue 110, PR 2).

mod common;

use common::Setup;
use serde_json::{json, Value};

/// In (one MU every 5 s) -> Oven, a Station working 15 s -> Out.
const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"5"}},
  {"name":"Oven","class":"Station","attributes":{"process_time":"15"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Oven","exit":1,"entrance":1},
  {"from":"Oven","to":"Out","exit":1,"entrance":1}]}"#;

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    setup
}

fn attribute(setup: &Setup, name: &str) -> Value {
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Oven" } }));
    got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == name).unwrap_or_else(|| panic!("no {name} in {got}")).clone()
}

fn set(name: &str, value: Value) -> Value {
    json!({ "change": "set_attribute", "target": { "instance": "Oven" }, "attribute": name, "value": value })
}

#[test]
fn a_station_has_a_capacity_of_1_and_individual_start_until_set() {
    let setup = setup("capacity-defaults");
    assert_eq!(attribute(&setup, "capacity"), json!({ "attribute": "capacity", "value": 1, "from": "built_in" }));
    assert_eq!(attribute(&setup, "start"), json!({ "attribute": "start", "value": "individual", "from": "built_in" }));
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [set("capacity", json!(3)), set("start", json!("batch"))] }));
    assert_eq!(attribute(&setup, "capacity")["value"], 3);
    assert_eq!(attribute(&setup, "start")["value"], "batch");
    assert_eq!(attribute(&setup, "start")["from"], "instance");
}

#[test]
fn edit_model_refuses_a_capacity_a_station_cant_have() {
    let setup = setup("capacity-refused");
    for (value, says) in [(json!(-1), "only a Buffer can be unlimited"), (json!(0), "at least 1"), (json!(20_000), "at most 10,000")] {
        let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [set("capacity", value.clone())] }));
        assert!(refused["problems"][0]["message"].as_str().unwrap().contains(says), "{value}: {refused}");
    }
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [set("start", json!("together"))] }));
    assert!(refused["problems"][0]["message"].as_str().unwrap().contains("is not a start"), "{refused}");
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [set("starting_mus", json!(2))] }));
    assert!(refused["problems"][0]["message"].as_str().unwrap().contains("holds only 1"), "{refused}");
}

#[test]
fn the_command_line_sets_a_stations_capacity_and_start() {
    let setup = setup("capacity-commands");
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "set Oven.capacity 4; set Oven.start batch; set Oven.starting 4" }));
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let oven = &model.instances[1].attributes;
    assert_eq!(oven.capacity, Some(crabsim_model::Capacity::Limited(4)));
    assert_eq!(oven.start, Some(crabsim_model::StartMode::Batch));
    assert_eq!(oven.starting_mus, Some(4));
}

#[test]
fn a_run_of_a_station_with_capacity_keeps_up_where_one_place_would_not() {
    let setup = setup("capacity-run");
    // With one place the Oven passes on an MU every 15 s while one comes every 5 s.
    let single = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 1 }));
    setup.finished(&single);
    assert!(setup.stored(&single, "Out")[&1]["mus_in"] < 250.0);
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [set("capacity", json!(3))] }));
    let three = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 1 }));
    setup.finished(&three);
    let out = setup.stored(&three, "Out");
    assert!(out[&1]["mus_in"] > 700.0, "three at once keep up with one every 5 s: {out:?}");
    let oven = setup.stored(&three, "Oven");
    assert!(oven[&1]["working_full_share"] > 0.99, "{oven:?}");
    let exported = setup.ok("export_model", json!({ "path": "line.json", "format": "cypher" }));
    let script = setup.folder.read(exported["files"][0]["path"].as_str().unwrap());
    assert!(script.contains("capacity: 3") && script.contains("start: 'individual'"), "{script}");
}
