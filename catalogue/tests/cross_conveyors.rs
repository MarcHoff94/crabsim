//! Cross conveyors through the catalogue (spec issue 148, PR 2): the command language,
//! `edit_model`, `get_attributes`, `validate_model`, what a run stores and the likely bottleneck.

mod common;

use common::Setup;
use crabsim_catalogue::commands::{complete, parse, Context, Parsed};
use serde_json::{json, Value};

/// A Source of Parts going to the Spur, into a cross conveyor's back, with the Drains Ahead at
/// its front and Spur at its left, which lists the Spur, all placed.
const FORK: &str = r#"{"format_version":2,"classes":[
  {"name":"Part","parent":"MU","attributes":{"work_plan":[{"at":"Spur"}]}}],"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"0:10","mu_class":"Part"}},
  {"name":"Cross","class":"CrossConveyor","position":{"x":2,"y":0},"attributes":{"exits":{"left":["Spur"]}}},
  {"name":"Ahead","class":"Drain","position":{"x":4,"y":0}},
  {"name":"Spur","class":"Drain","position":{"x":2,"y":2}}],"connections":[
  {"from":"In","to":"Cross","exit":1,"entrance":1,"to_side":"back"},
  {"from":"Cross","to":"Ahead","exit":1,"entrance":1,"from_side":"front"},
  {"from":"Cross","to":"Spur","exit":2,"entrance":1,"from_side":"left"}]}"#;

fn model() -> crabsim_model::Model {
    crabsim_model::load(FORK).unwrap()
}

/// The changes `line` makes, as `edit_model` takes them.
fn changes(line: &str) -> Vec<Value> {
    match parse(line, &model(), &Context::default()) {
        Ok(Parsed::Changes(planned)) => planned.into_iter().flat_map(|command| command.changes).collect(),
        other => panic!("{line}: {other:?}"),
    }
}

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "fork.json", "text": model().save() }));
    setup
}

fn run_line(setup: &Setup, line: &str) {
    setup.ok("run_commands", json!({ "path": "fork.json", "commands": line }));
}

fn saved(setup: &Setup) -> crabsim_model::Model {
    crabsim_model::load(&setup.folder.read("fork.json")).unwrap()
}

#[test]
fn connecting_names_a_cross_conveyors_side_after_a_dot() {
    assert_eq!(changes("connect Cross.right > Ahead")[0], json!({ "change": "connect", "from": "Cross", "from_side": "right", "to": "Ahead" }));
    assert_eq!(changes("connect In>cross.Left")[0], json!({ "change": "connect", "from": "In", "to": "Cross", "to_side": "left" }), "names and sides matched as names are");
    assert_eq!(changes("disconnect Cross.front > Ahead")[0], json!({ "change": "disconnect", "from": "Cross", "to": "Ahead" }), "a side is no part of which connection it is");
}

#[test]
fn a_sides_exits_are_set_as_a_list_of_names_keeping_the_other_sides() {
    assert_eq!(
        changes("set Cross.exits.right Ahead, spur")[0],
        json!({ "change": "set_attribute", "target": { "instance": "Cross" }, "attribute": "exits", "value": { "left": ["Spur"], "right": ["Ahead", "Spur"] } })
    );
    assert_eq!(
        changes("restore Cross.exits.left")[0],
        json!({ "change": "restore_inheritance", "target": { "instance": "Cross" }, "attribute": "exits" }),
        "its last side taken away, nothing is set"
    );
    assert_eq!(changes("set Cross.switch_time 1.5")[0]["value"], json!("1.5"), "a time as typed");
    let refused = parse("set Cross.exits.up Ahead", &model(), &Context::default()).expect_err("no such side");
    assert!(refused.contains("`up` is no side"), "{refused}");
    let refused = parse("set Cross.exits.left Nowhere", &model(), &Context::default()).expect_err("no such place");
    assert!(refused.contains("Nowhere"), "{refused}");
}

#[test]
fn completion_offers_a_cross_conveyors_sides_and_the_places_an_exit_can_list() {
    let model = model();
    assert_eq!(complete("connect In > Cross.l", &model, &[]), vec!["connect In > Cross.left".to_string()]);
    assert_eq!(complete("connect Cross.", &model, &[]).len(), 4);
    assert_eq!(complete("set Cross.exits.f", &model, &[]), vec!["set Cross.exits.front ".to_string()]);
    assert!(complete("set Cross.exits.left Spur, Ah", &model, &[]).contains(&"set Cross.exits.left Spur, Ahead".to_string()));
    assert!(complete("set Cross.sw", &model, &[]).contains(&"set Cross.switch_time ".to_string()));
    assert!(complete("connect In > Ahead.", &model, &[]).is_empty(), "a Drain has no sides");
}

#[test]
fn edit_model_connects_at_sides_and_moves_a_connection_to_another_side() {
    let setup = setup("connect_sides");
    run_line(&setup, "disconnect Cross > Ahead; connect Cross.right > Ahead");
    let model = saved(&setup);
    let connection = model.connections.iter().find(|c| c.to == "Ahead").unwrap();
    assert_eq!(connection.from_side, Some(crabsim_model::Side::Right));
    setup.ok("edit_model", json!({ "path": "fork.json", "changes": [{ "change": "set_connection", "from": "Cross", "to": "Ahead", "from_side": "front" }] }));
    assert_eq!(saved(&setup).connections.iter().find(|c| c.to == "Ahead").unwrap().from_side, Some(crabsim_model::Side::Front));
    let refused = setup.err("run_commands", json!({ "path": "fork.json", "commands": "disconnect Cross > Ahead; connect Cross > Ahead" }));
    assert!(refused.to_string().contains("say which side the connection leaves by"), "{refused}");
    let history = setup.ok("get_history", json!({ "path": "fork.json" }));
    assert!(history.to_string().contains("connect Cross.right > Ahead"), "worded as typed: {history}");
}

#[test]
fn renaming_a_place_renames_it_in_exits() {
    let setup = setup("rename_in_exits");
    run_line(&setup, "rename Spur to Siding");
    let model = saved(&setup);
    assert_eq!(model.instances[1].attributes.exits.as_ref().unwrap()[&crabsim_model::Side::Left], vec!["Siding".to_string()]);
}

#[test]
fn get_attributes_lists_a_cross_conveyors_sides() {
    let setup = setup("cross_sides");
    let cross = setup.ok("get_attributes", json!({ "path": "fork.json", "target": { "instance": "Cross" } }));
    assert_eq!(
        cross["sides"],
        json!([
            { "side": "front", "way": "out", "neighbour": "Ahead" },
            { "side": "back", "way": "in", "neighbour": "In" },
            { "side": "left", "way": "out", "neighbour": "Spur", "exits": ["Spur"] },
            { "side": "right", "way": "none" }
        ])
    );
    let names: Vec<&str> = cross["attributes"].as_array().unwrap().iter().map(|attribute| attribute["attribute"].as_str().unwrap()).collect();
    for expected in ["length", "width", "speed", "switch_time", "release_time", "exits", "distribution"] {
        assert!(names.contains(&expected), "{expected} in {names:?}");
    }
    let switch = cross["attributes"].as_array().unwrap().iter().find(|attribute| attribute["attribute"] == "switch_time").unwrap().clone();
    assert_eq!((switch["value"].clone(), switch["from"].clone()), (json!("00:00:02"), json!("built_in")));
    let source = setup.ok("get_attributes", json!({ "path": "fork.json", "target": { "instance": "In" } }));
    assert!(source.get("sides").is_none());
}

#[test]
fn validate_model_warns_where_mus_would_wait_at_a_cross_conveyor_for_good() {
    let setup = setup("cross_warnings");
    run_line(&setup, "set Part.work_plan Ahead; disconnect Cross > Ahead; connect Cross.right > Ahead");
    let validated = setup.ok("validate_model", json!({ "path": "fork.json" }));
    assert!(validated.to_string().contains("no connection leaves by its front side straight across"), "{validated}");
}

#[test]
fn a_run_stores_a_cross_conveyors_switching_and_the_mus_out_of_each_side() {
    let setup = setup("stored_cross");
    let run = setup.start(json!({ "model": "fork.json", "length": "1:00:00", "observations": 1 }));
    setup.finished(&run);
    let cross = setup.stored(&run, "Cross");
    // An MU every 10 s turning left, each switching 2 s.
    assert_eq!(cross[&1]["mus_out_left"], 359.0);
    assert!(!cross[&1].contains_key("mus_out_front"), "a side no MU left by stores nothing");
    assert!((cross[&1]["switching_share"] - 0.2).abs() < 0.01, "{cross:?}");
    assert!(!setup.stored(&run, "In")[&1].contains_key("switching_share"));
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Cross"] }));
    assert!(results.to_string().contains("switching_share"), "{results}");
    let found = setup.ok("find_bottleneck", json!({ "run": run }));
    assert_eq!(found["likely"]["instance"], "Cross", "the only Station or conveyor: {found}");
}

#[test]
fn a_blocked_cross_conveyor_counts_as_full_for_the_bottleneck() {
    use crabsim_catalogue::bottleneck::{likely_bottleneck, Shares};
    let shares = |name: &str, built_in: &str, working: f64, blocked: f64| Shares {
        name: name.into(),
        built_in: built_in.into(),
        working,
        working_full: working,
        blocked,
        idle: 1.0 - working - blocked,
        failed: 0.0,
        paused: 0.0,
    };
    let objects = [shares("Mill", "Station", 0.6, 0.0), shares("Cross", "CrossConveyor", 0.4, 0.3)];
    let found = likely_bottleneck(&objects, &[]).unwrap();
    assert_eq!(found.instance, "Cross");
    assert!((found.full_or_interrupted_share - 0.7).abs() < 1e-9);
    let station = [shares("Mill", "Station", 0.6, 0.0), shares("Saw", "Station", 0.4, 0.3)];
    assert_eq!(likely_bottleneck(&station, &[]).unwrap().instance, "Mill", "a Station's blocked time isn't full");
}

#[test]
fn describe_and_query_model_know_a_cross_conveyors_sides_and_exits() {
    let setup = setup("describe_cross");
    let described = setup.ok("describe_model", json!({ "path": "fork.json" }));
    assert_eq!(described["routing"]["cross_conveyors"]["names"], json!(["Cross"]));
    let queried = setup.ok("query_model", json!({ "path": "fork.json", "query": { "kind": "distribution", "name": "Cross" } }));
    assert_eq!(queried["successors"][0]["side"], "front");
    assert_eq!(queried["successors"][0]["exits"], json!([]));
    assert_eq!((queried["successors"][1]["side"].clone(), queried["successors"][1]["exits"].clone()), (json!("left"), json!(["Spur"])));
    let source = setup.ok("query_model", json!({ "path": "fork.json", "query": { "kind": "distribution", "name": "In" } }));
    assert!(source["successors"][0].get("side").is_none());
}
