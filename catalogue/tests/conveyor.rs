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
