//! Work plans, detours and routing by destination through the catalogue (spec issue 148): the
//! command language, `edit_model`, `get_attributes`, `describe_model`, `query_model`,
//! `validate_model` and what a run stores.

mod common;

use common::Setup;
use crabsim_catalogue::commands::{complete, parse, Context, Parsed};
use serde_json::{json, Value};

/// A Source feeding two Mills of one class, which feed a Drain, all placed.
const LINE: &str = r#"{"format_version":2,"classes":[
  {"name":"Mill","parent":"Station","attributes":{"process_time":"0:45"}},
  {"name":"Housing","parent":"MU"}],"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"0:30","mu_class":"Housing"}},
  {"name":"Mill 1","class":"Mill","position":{"x":4,"y":2}},
  {"name":"Mill 2","class":"Mill","position":{"x":4,"y":-2}},
  {"name":"Out","class":"Drain","position":{"x":8,"y":0}}],"connections":[
  {"from":"In","to":"Mill 1","exit":1,"entrance":1},
  {"from":"In","to":"Mill 2","exit":2,"entrance":1},
  {"from":"Mill 1","to":"Out","exit":1,"entrance":1},
  {"from":"Mill 2","to":"Out","exit":1,"entrance":2}]}"#;

fn model() -> crabsim_model::Model {
    crabsim_model::load(LINE).unwrap()
}

/// The value the one change of `line` sets.
fn value_set(line: &str) -> Value {
    match parse(line, &model(), &Context::default()) {
        Ok(Parsed::Changes(planned)) => planned.into_iter().flat_map(|command| command.changes).next().expect("a change")["value"].clone(),
        other => panic!("{line}: {other:?}"),
    }
}

#[test]
fn a_work_plan_is_typed_as_steps_by_comma_each_with_its_time_last() {
    assert_eq!(
        value_set("set Housing.work_plan Mill 1 0:40, Mill, Out"),
        json!([{ "at": "Mill 1", "process_time": "0:40" }, { "at": "Mill" }, { "at": "Out" }]),
        "an instance with a time, a class, then the Drain"
    );
    assert_eq!(value_set("set Housing.work_plan Mill 2"), json!([{ "at": "Mill 2" }]), "a name ending in a number stays a name");
    assert_eq!(value_set("set Housing.work_plan mill 2 1:00"), json!([{ "at": "Mill 2", "process_time": "1:00" }]), "names matched as names are");
    assert_eq!(value_set(r#"set Housing.work_plan [{"at": "Out"}]"#), json!([{ "at": "Out" }]), "or as JSON");
    let refused = parse("set Housing.work_plan Mill 1, Saw 0:40", &model(), &Context::default()).expect_err("no Saw");
    assert!(refused.contains("step 2 (`Saw 0:40`)") && refused.contains("Saw"), "{refused}");
    let refused = parse("set Housing.work_plan Mill 1,, Out", &model(), &Context::default()).expect_err("an empty step");
    assert!(refused.contains("step 2 is empty"), "{refused}");
}

#[test]
fn a_detour_is_a_name_and_by_destination_a_word() {
    assert_eq!(value_set("set Mill 1.detour mill 2"), json!("Mill 2"));
    assert_eq!(value_set("set Mill 1.detour Mill"), json!("Mill"), "a class, named exactly, before instances holding its name");
    assert_eq!(value_set("set In.distribution by destination"), json!({ "by_destination": { "then": "round_robin" } }));
    assert_eq!(value_set("set In.distribution by_destination"), json!({ "by_destination": { "then": "round_robin" } }));
}

#[test]
fn completion_offers_names_in_a_work_plan_and_a_detour() {
    let model = model();
    assert!(complete("set Housing.work_plan Mill 1 0:40, Ou", &model, &[]).contains(&"set Housing.work_plan Mill 1 0:40, Out".to_string()));
    assert!(complete("set Mill 1.detour Mill", &model, &[]).contains(&"set Mill 1.detour Mill 2".to_string()));
    assert!(!complete("set Mill 1.detour Hou", &model, &[]).iter().any(|line| line.ends_with("Housing")), "an MU class is no place");
}

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": crabsim_model::load(LINE).unwrap().save() }));
    setup
}

fn run_line(setup: &Setup, line: &str) {
    setup.ok("run_commands", json!({ "path": "line.json", "commands": line }));
}

fn saved(setup: &Setup) -> crabsim_model::Model {
    crabsim_model::load(&setup.folder.read("line.json")).unwrap()
}

#[test]
fn renaming_an_instance_or_a_class_renames_it_in_work_plans_and_detours() {
    let setup = setup("rename_destinations");
    run_line(&setup, "set Housing.work_plan Mill 1 0:40, Mill, Out; set In.mu_values {\"work_plan\": [{\"at\": \"Mill 1\"}]}; set Mill 2.detour Mill 1");
    run_line(&setup, "rename Mill 1 to Saw; class rename Mill to Lathe");
    let model = saved(&setup);
    let plan: Vec<&str> = model.class("Housing").unwrap().attributes.work_plan.iter().flatten().map(|step| step.at.as_str()).collect();
    assert_eq!(plan, vec!["Saw", "Lathe", "Out"]);
    assert_eq!(model.instances[0].attributes.mu_values.as_ref().and_then(|values| values.work_plan.as_ref()).unwrap()[0].at, "Saw");
    assert_eq!(model.instances[2].attributes.detour.as_deref(), Some("Saw"));
    assert!(model.instances[3].attributes.is_empty(), "nothing added where nothing was named");
}

#[test]
fn removing_a_destination_is_refused_while_a_plan_names_it() {
    let setup = setup("remove_destination");
    run_line(&setup, "set Housing.work_plan Mill 1, Out");
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "remove Mill 1" }));
    assert!(refused.to_string().contains("nothing is called `Mill 1`"), "{refused}");
}

#[test]
fn get_attributes_gives_a_work_plan_and_where_it_comes_from() {
    let setup = setup("get_plan");
    run_line(&setup, "set Housing.work_plan Mill 1 0:40, Out; set Mill 1.detour Mill 2");
    let housing = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "class": "Housing" } }));
    let plan = housing["attributes"].as_array().unwrap().iter().find(|attribute| attribute["attribute"] == "work_plan").expect("work_plan listed").clone();
    assert_eq!(plan["value"], json!([{ "at": "Mill 1", "process_time": "00:00:40" }, { "at": "Out" }]));
    let mill = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill 1" } }));
    let detour = mill["attributes"].as_array().unwrap().iter().find(|attribute| attribute["attribute"] == "detour").expect("detour listed").clone();
    assert_eq!(detour["value"], json!("Mill 2"));
}

#[test]
fn describe_and_query_model_know_where_mus_go() {
    let setup = setup("describe_routing");
    let plain = setup.ok("describe_model", json!({ "path": "line.json" }));
    assert!(plain.get("routing").is_none(), "left out without work plans: {plain}");
    run_line(&setup, "set Housing.work_plan Mill 2, Out; set In.distribution by destination");
    let described = setup.ok("describe_model", json!({ "path": "line.json" }));
    assert_eq!(described["routing"]["destinations"], 2);
    assert_eq!(described["routing"]["work_plans"]["names"], json!(["Housing"]));
    assert_eq!(described["routing"]["by_destination"]["names"], json!(["In"]));
    let queried = setup.ok("query_model", json!({ "path": "line.json", "query": { "kind": "distribution", "name": "In" } }));
    assert_eq!(queried["strategy"], "by_destination");
    assert_eq!(queried["successors"][0]["reaches"], json!(["Out"]), "Mill 1 leads only to Out");
    assert_eq!(queried["successors"][1]["reaches"], json!(["Mill 2", "Out"]));
}

#[test]
fn a_run_stores_how_many_mus_each_station_worked_on_and_off_plan() {
    let setup = setup("stored_plan_counts");
    run_line(&setup, "set Housing.work_plan Mill 2, Out");
    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "observations": 1 }));
    setup.finished(&run);
    let mill_1 = setup.stored(&run, "Mill 1");
    let mill_2 = setup.stored(&run, "Mill 2");
    assert_eq!(mill_1[&1]["mus_on_plan"], 0.0);
    assert!(mill_1[&1]["mus_off_plan"] > 0.0, "round robin sends some the wrong way: {mill_1:?}");
    assert!(mill_2[&1]["mus_on_plan"] > 0.0);
    assert_eq!(mill_2[&1]["mus_off_plan"], 0.0);
    assert!(!setup.stored(&run, "Out")[&1].contains_key("mus_on_plan"), "only Stations");
}
