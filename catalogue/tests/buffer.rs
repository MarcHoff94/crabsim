//! A Buffer through the catalogue, as an agent builds, checks and runs one (spec issue 110).

mod common;

use common::Setup;
use serde_json::{json, Value};

/// In (one MU every 5 s) -> Store, a Buffer -> Slow (10 s) -> Out.
const BUFFERED: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"5"}},
  {"name":"Store","class":"Buffer"},
  {"name":"Slow","class":"Station","attributes":{"process_time":"10"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Store","exit":1,"entrance":1},
  {"from":"Store","to":"Slow","exit":1,"entrance":1},
  {"from":"Slow","to":"Out","exit":1,"entrance":1}]}"#;

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": BUFFERED }));
    setup
}

fn attribute(setup: &Setup, name: &str) -> Value {
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Store" } }));
    got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == name).unwrap_or_else(|| panic!("no {name} in {got}")).clone()
}

fn set(setup: &Setup, name: &str, value: Value) {
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Store" }, "attribute": name, "value": value } ] }));
}

#[test]
fn a_buffer_has_its_attributes_with_defaults_and_takes_minus_1_for_no_limit() {
    let setup = setup("buffer-attributes");
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Store" } }));
    let names: Vec<&str> = got["attributes"].as_array().unwrap().iter().map(|a| a["attribute"].as_str().unwrap()).collect();
    assert_eq!(names, ["failures", "starting_mus", "capacity", "dwelling_time", "exit_strategy", "histogram_bar_width", "stop_freezes_dwelling", "dimensions", "description", "state_notes", "distribution", "on_entry", "on_exit"], "every attribute a Buffer has");
    assert_eq!(attribute(&setup, "capacity"), json!({ "attribute": "capacity", "value": "unlimited", "from": "built_in" }));
    assert_eq!(attribute(&setup, "dwelling_time"), json!({ "attribute": "dwelling_time", "value": "00:00:00", "from": "built_in" }));
    assert_eq!(attribute(&setup, "exit_strategy"), json!({ "attribute": "exit_strategy", "value": "first_in_first_out", "from": "built_in" }));

    set(&setup, "capacity", json!(4));
    assert_eq!(attribute(&setup, "capacity"), json!({ "attribute": "capacity", "value": 4, "from": "instance" }));
    set(&setup, "capacity", json!(-1));
    assert_eq!(attribute(&setup, "capacity"), json!({ "attribute": "capacity", "value": "unlimited", "from": "instance" }));
    assert!(setup.folder.read("line.json").contains(r#""capacity": "unlimited""#), "saved as \"unlimited\"");
    set(&setup, "dwelling_time", json!("0:30"));
    set(&setup, "exit_strategy", json!("last_in_first_out"));
    set(&setup, "starting_mus", json!(2));
    assert_eq!(attribute(&setup, "dwelling_time")["value"], "00:00:30");
    assert_eq!(attribute(&setup, "exit_strategy")["value"], "last_in_first_out");

    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Store" }, "attribute": "capacity", "value": 1 } ] }));
    assert_eq!(refused["problems"][0]["message"], "it starts with 2 MUs but holds only 1");
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Store" }, "attribute": "exit_strategy", "value": "random" } ] }));
    assert!(refused["problems"][0]["message"].as_str().unwrap().contains("is not an exit strategy"), "{refused}");
}

#[test]
fn the_command_line_sets_a_buffers_values() {
    let setup = setup("buffer-commands");
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "set Store.capacity -1; set Store.dwell 0:20; set Store.exit last_in_first_out" }));
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    let store = &model.instances[1].attributes;
    assert_eq!(store.capacity, Some(crabsim_model::Capacity::Unlimited));
    assert_eq!(store.dwelling_time, Some(crabsim::SimTime::from_secs(20)));
    assert_eq!(store.exit_strategy, Some(crabsim_model::ExitStrategy::LastInFirstOut));
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "set Store.capacity unlimited; set Store.capacity 7" }));
    assert_eq!(attribute(&setup, "capacity")["value"], 7);
}

#[test]
fn validate_model_warns_about_a_buffer_with_no_limit_that_fills_faster_than_it_empties() {
    let setup = setup("buffer-warning");
    let checked = setup.ok("validate_model", json!({ "path": "line.json" }));
    assert_eq!(checked["valid"], true);
    let warnings = checked["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{checked}");
    assert_eq!(warnings[0]["name"], "Store");
    assert!(warnings[0]["message"].as_str().unwrap().contains("can get 720 MUs an hour but pass on only 360"), "{checked}");
    set(&setup, "capacity", json!(10));
    assert_eq!(setup.ok("validate_model", json!({ "path": "line.json" }))["warnings"], json!([]));
}

#[test]
fn a_model_with_a_buffer_runs_and_its_figures_are_stored() {
    let setup = setup("buffer-run");
    set(&setup, "capacity", json!(3));
    let run = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 2 }));
    assert_eq!(setup.finished(&run)["state"], "done");
    let stored = setup.stored(&run, "Store");
    let figures = &stored[&1];
    // One MU every 10 s leaves through Slow: the first at 5 s, then from 15 s on.
    assert_eq!(figures["mus_out"], 360.0);
    assert_eq!(figures["mus_in"], 363.0, "three more wait in it");
    assert!(figures["blocked_share"] > 0.99, "an MU waits for Slow nearly all the time: {figures:?}");
    assert!(setup.stored(&run, "In")[&1]["blocked_share"] > 0.4, "the full Buffer holds the Source up");
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Store"] }));
    assert!(results.to_string().contains("Store"), "{results}");
}

#[test]
fn find_instances_and_exports_know_a_buffers_values() {
    let setup = setup("buffer-graph");
    let find = |conditions: Value| -> Vec<String> {
        let found = setup.ok("query_model", json!({ "path": "line.json", "query": { "kind": "find_instances", "conditions": conditions } }));
        found["instances"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(find(json!([{ "attribute": "capacity", "op": "=", "value": "unlimited" }])), ["Store"]);
    assert_eq!(find(json!([{ "attribute": "capacity", "op": ">", "value": 1000 }])), ["Store"], "no limit is more than any number");
    set(&setup, "capacity", json!(5));
    set(&setup, "dwelling_time", json!("1:00"));
    assert_eq!(find(json!([{ "attribute": "capacity", "op": "<=", "value": 5 }])), ["Store", "Slow"], "a Station has a capacity too, 1 by default");
    assert_eq!(find(json!([{ "attribute": "capacity", "op": "=", "value": "unlimited" }])), Vec::<String>::new());
    assert_eq!(find(json!([{ "attribute": "dwelling_time", "op": ">=", "value": "0:30" }])), ["Store"]);
    let exported = setup.ok("export_model", json!({ "path": "line.json", "format": "cypher" }));
    let script = setup.folder.read(exported["files"][0]["path"].as_str().unwrap());
    assert!(script.contains("capacity: 5") && script.contains("dwelling_time_s: 60") && script.contains("exit_strategy: 'first_in_first_out'"), "{script}");
}

#[test]
fn a_run_keeps_a_buffers_fill_and_run_results_gives_its_histogram_when_named() {
    let setup = setup("buffer-fill");
    set(&setup, "capacity", json!(5));
    let run = setup.start(json!({ "model": "line.json", "seed": 1, "length": "1:00:00", "observations": 2 }));
    setup.finished(&run);
    let stored = setup.stored(&run, "Store");
    let figures = &stored[&1];
    assert_eq!((figures["max_mus"], figures["median_mus"]), (5.0, 5.0), "full nearly all the time: {figures:?}");
    assert!(figures["full_share"] > 0.9 && figures["mean_mus"] > 4.5, "{figures:?}");
    assert_eq!(figures["min_mus"], 0.0, "empty until the first MU");
    assert!(setup.stored(&run, "Slow")[&1].get("mean_mus").is_none(), "only a Buffer");

    let named = setup.ok("run_results", json!({ "run": run, "objects": ["Store"] }));
    let histogram = named["objects"][0]["histogram"].as_array().unwrap().clone();
    let last = histogram.last().unwrap();
    assert_eq!((last["from_mus"].as_u64(), last["to_mus"].as_u64()), (Some(5), Some(5)));
    assert!(last["share"].as_f64().unwrap() > 0.9, "{histogram:?}");
    let total: f64 = histogram.iter().map(|bar| bar["share"].as_f64().unwrap()).sum();
    assert!((total - 1.0).abs() < 1e-3, "{total}");
    let summary = setup.ok("run_results", json!({ "run": run }));
    assert!(summary["objects"].as_array().unwrap().iter().all(|object| object.get("histogram").is_none()), "only when named");

    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT count(*) AS bars FROM histograms WHERE run = '{run}' AND object = 'Store'") }));
    assert!(rows.to_string().contains("bars"), "{rows}");
    setup.ok("forget_run", json!({ "run": run }));
    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT count(*) AS bars FROM histograms WHERE run = '{run}'") }));
    assert!(rows.to_string().contains('0'), "forgotten with the run: {rows}");
}

#[test]
fn a_buffers_histogram_bar_width_is_an_attribute() {
    let setup = setup("buffer-bar-width");
    assert_eq!(attribute(&setup, "histogram_bar_width"), json!({ "attribute": "histogram_bar_width", "from": "not_set" }), "automatic");
    set(&setup, "histogram_bar_width", json!(4));
    assert_eq!(attribute(&setup, "histogram_bar_width")["value"], 4);
}
