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
    assert_eq!(names, ["failures", "dimensions", "description", "state_notes", "distribution", "width", "path", "speed", "gap", "accumulating", "direction", "min_radius", "max_incline"], "every attribute a Conveyor has");
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
    assert!(refused.to_string().contains("`radius` is missing"), "{refused}");
    assert_eq!(attribute(&setup, "direction"), json!({ "attribute": "direction", "value": "forward", "from": "built_in" }));
    assert_eq!(attribute(&setup, "max_incline"), json!({ "attribute": "max_incline", "value": 30.0, "from": "built_in" }));
    assert_eq!(attribute(&setup, "min_radius"), json!({ "attribute": "min_radius", "from": "not_set" }));
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

// ---------- Paths (spec issue 141, PR 1) ----------

#[test]
fn the_command_line_sets_a_path_in_its_short_form_and_says_what_is_wrong_with_one() {
    let setup = setup("conveyor-short-form");
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "set Belt.path 4, arc 1.5 90, transition 1 to straight, helix 1.2 2 rise 3; set Belt.direction backward" }));
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let belt = model.resolve(1).unwrap().attributes;
    assert_eq!(crabsim_model::path::short_form(belt.path.as_deref().unwrap()), "4, arc 1.5 90, transition 1 to straight, helix 1.2 2 rise 3");
    assert_eq!(belt.direction, Some(crabsim_model::Direction::Backward));
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "set Belt.path 4, arc 1.5" }));
    assert!(refused.to_string().contains("segment 2 (`arc 1.5`)"), "{refused}");
    // A path the short form reads but the model file's rules don't is refused by them.
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "set Belt.path arc 0.2 90" }));
    assert!(refused.to_string().contains("its inner edge would fold"), "{refused}");
}

#[test]
fn completion_offers_a_paths_words_and_the_two_directions() {
    let setup = setup("conveyor-completion");
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let complete = |line: &str| crabsim_catalogue::commands::complete(line, &model, &[]);
    assert_eq!(complete("set Belt.path 4, a"), vec!["set Belt.path 4, arc ".to_string()]);
    assert_eq!(complete("set Belt.path 4, arc 1.5 90, transition 1 t"), vec!["set Belt.path 4, arc 1.5 90, transition 1 to ".to_string()]);
    assert_eq!(complete("set Belt.path 4, helix 1 2 r"), vec!["set Belt.path 4, helix 1 2 rise ".to_string()]);
    assert_eq!(complete("set Belt.direction "), vec!["set Belt.direction forward ".to_string(), "set Belt.direction backward ".to_string()]);
    assert_eq!(complete("set Belt.dir"), vec!["set Belt.direction ".to_string()]);
}

#[test]
fn get_attributes_gives_a_conveyors_worked_out_path() {
    let setup = setup("conveyor-geometry");
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "move_instance", "name": "Belt", "position": { "x": 1, "y": 2, "z": 0.5 } },
        { "change": "set_attribute", "target": { "instance": "Belt" }, "attribute": "path",
          "value": [{ "straight": 4 }, { "arc": { "radius": 2, "angle": 90, "rise": 1 } }] } ] }));
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Belt" } }));
    let geometry = &got["geometry"];
    // 4 m on to (5, 2), then a quarter circle to the left of radius 2 to (7, 4), heading up.
    assert_eq!(geometry["start"], json!({ "x": 1.0, "y": 2.0, "z": 0.5, "heading_degrees": 0.0 }), "{got}");
    assert_eq!(geometry["end"], json!({ "x": 7.0, "y": 4.0, "z": 1.5, "heading_degrees": 90.0 }), "{got}");
    let arc = std::f64::consts::PI;
    assert_eq!(geometry["plan_length"], json!(((4.0 + arc) * 10_000.0_f64).round() / 10_000.0));
    assert_eq!(geometry["length"], json!(((4.0 + arc.hypot(1.0)) * 10_000.0_f64).round() / 10_000.0));
    assert_eq!(geometry["bounds"], json!({ "min_x": 1.0, "min_y": 1.7, "max_x": 7.3, "max_y": 4.0 }), "{got}");
    assert_eq!(geometry["tightest_radius"], json!(2.0));
    assert_eq!(geometry["placed"], json!(true));
    // Backward, its MUs enter at the arc's end, heading back down.
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "set Belt.direction backward" }));
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Belt" } }));
    assert_eq!(got["geometry"]["start"], json!({ "x": 7.0, "y": 4.0, "z": 1.5, "heading_degrees": -90.0 }), "{got}");
    // A class has no place, so its path is laid from (0, 0).
    let class = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "class": "Conveyor" } }));
    assert_eq!(class["geometry"]["end"], json!({ "x": 5.0, "y": 0.0, "z": 0.0, "heading_degrees": 0.0 }), "{class}");
    assert_eq!(class["geometry"]["placed"], json!(false));
    let station = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill" } }));
    assert!(station.get("geometry").is_none(), "{station}");
}

// ---------- Attaching (spec issue 141, PR 3) ----------

/// In -> Feed, a 4 m conveyor with a quarter turn of radius 2 to the left -> Lift, a 3 m conveyor
/// -> Mill, 2 m long -> Out, all placed apart.
const APART: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","position":{"x":-3,"y":1}},
  {"name":"Feed","class":"Conveyor","position":{"x":0,"y":0},"attributes":{"path":[{"straight":4},{"arc":{"radius":2,"angle":90}}]}},
  {"name":"Lift","class":"Conveyor","position":{"x":20,"y":20},"attributes":{"path":[{"straight":3}]}},
  {"name":"Mill","class":"Station","position":{"x":30,"y":30},"attributes":{"process_time":"5","dimensions":{"length":2,"width":1,"height":1}}},
  {"name":"Out","class":"Drain","position":{"x":40,"y":40}}],"connections":[
  {"from":"In","to":"Feed","exit":1,"entrance":1},
  {"from":"Feed","to":"Lift","exit":1,"entrance":1},
  {"from":"Lift","to":"Mill","exit":1,"entrance":1},
  {"from":"Mill","to":"Out","exit":1,"entrance":1}]}"#;

fn place_of(setup: &Setup, name: &str) -> (Value, Option<Value>) {
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": name } }));
    let dimensions = got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "dimensions").and_then(|a| a["value"].get("rotation_degrees").cloned());
    (got["position"].clone(), dimensions)
}

#[test]
fn attach_moves_and_turns_one_object_so_its_end_meets_its_neighbours() {
    let setup = Setup::new("attach-one");
    setup.ok("write_model", json!({ "path": "line.json", "text": APART }));
    // Lift after Feed, which ends at (6, 2) heading up the y axis.
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "attach Lift to Feed" }));
    assert_eq!(place_of(&setup, "Lift"), (json!({ "x": 6.0, "y": 2.0 }), Some(json!(90.0))));
    // Mill, with one neighbour on each side, needs saying which; Out has one.
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "attach Mill" }));
    assert!(refused.to_string().contains("`Mill` has 2 neighbours: say which"), "{refused}");
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "attach Out" }));
    assert!(refused.to_string().contains("neither `Out` nor `Mill` is a conveyor"), "{refused}");
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "attach In to Lift" }));
    assert!(refused.to_string().contains("aren't connected"), "{refused}");
    // One undo takes the attach back.
    setup.ok("undo_model", json!({ "path": "line.json" }));
    assert_eq!(place_of(&setup, "Lift").0, json!({ "x": 20.0, "y": 20.0 }));
}

#[test]
fn attach_lines_up_several_in_flow_order_from_the_first() {
    let setup = Setup::new("attach-several");
    setup.ok("write_model", json!({ "path": "line.json", "text": APART }));
    // Listed in any order: In stays, Feed's start goes half In's length past In's centre, and so on.
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "attach Mill, Lift, Feed, In" }));
    assert_eq!(place_of(&setup, "In").0, json!({ "x": -3.0, "y": 1.0 }));
    assert_eq!(place_of(&setup, "Feed").0, json!({ "x": -2.5, "y": 1.0 }));
    // Feed ends 6 m on and 2 m up from its start, heading up, so Lift starts at (3.5, 3).
    assert_eq!(place_of(&setup, "Lift"), (json!({ "x": 3.5, "y": 3.0 }), Some(json!(90.0))));
    // Lift ends 3 m up at (3.5, 6), so Mill's centre is 1 m on, turned up too.
    assert_eq!(place_of(&setup, "Mill"), (json!({ "x": 3.5, "y": 7.0 }), Some(json!(90.0))));
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "attach In, Lift" }));
    assert!(refused.to_string().contains("`Lift` isn't connected after `In` within the list"), "{refused}");
    // The command line completes it like any other.
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    assert_eq!(crabsim_catalogue::commands::complete("atta", &model, &[]), vec!["attach ".to_string()]);
}
