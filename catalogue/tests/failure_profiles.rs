//! Several named failure profiles through the catalogue (spec issue 122, PR 3).

mod common;

use common::Setup;
use serde_json::json;

const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Lathe","class":"Station","attributes":{"process_time":"5","failures":[
    {"name":"Tool wear","stream":21,"availability":0.9,"repair_time":{"constant":"1:00"}},
    {"name":"Jam","stream":22,"availability":0.8,"repair_time":{"constant":"0:20"}}]}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Lathe","exit":1,"entrance":1},
  {"from":"Lathe","to":"Out","exit":1,"entrance":1}]}"#;

#[test]
fn a_run_stores_each_profiles_failures_and_failed_time_beside_the_objects_total() {
    let setup = Setup::new("failure-profiles-stored");
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    let run = setup.start(json!({ "model": "line.json", "length": "1d", "observations": 2 }));
    assert_eq!(setup.finished(&run)["state"], "done");
    let rows = setup.ok("query_results", json!({ "sql": format!(
        "SELECT observation, profile, failures, failed_s FROM failure_profiles WHERE run = '{run}' AND object = 'Lathe' ORDER BY observation, profile") }))["rows"].clone();
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 4, "two profiles in each of two observations: {rows:?}");
    assert_eq!(rows.iter().map(|row| row[1].as_str().unwrap()).collect::<Vec<_>>(), ["Jam", "Tool wear", "Jam", "Tool wear"]);
    let stored = setup.stored(&run, "Lathe");
    for (observation, figures) in stored {
        let of: Vec<_> = rows.iter().filter(|row| row[0] == observation).collect();
        let failures: f64 = of.iter().map(|row| row[2].as_f64().unwrap()).sum();
        assert_eq!(figures["failures"], failures, "the object's failures are its profiles' together");
    }
}

#[test]
fn forgetting_a_run_forgets_its_profiles_rows() {
    let setup = Setup::new("failure-profiles-forgotten");
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "observations": 1 }));
    setup.finished(&run);
    setup.ok("forget_run", json!({ "run": run }));
    let left = setup.ok("query_results", json!({ "sql": "SELECT COUNT(*) FROM failure_profiles" }));
    assert_eq!(left["rows"][0][0], 0);
}

#[test]
fn an_agent_sets_the_list_through_edit_model_and_reads_it_back_with_get_attributes() {
    let setup = Setup::new("failure-profiles-edited");
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "In" }, "attribute": "failures",
          "value": [{ "name": "Empty hopper", "stream": 30, "availability": 0.95, "repair_time": { "constant": "2:00" } }] } ] }));
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "In" } }));
    let failures = got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "failures").unwrap().clone();
    assert_eq!(failures["value"][0]["name"], "Empty hopper");
    let refused = setup.err("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "instance": "Lathe" }, "attribute": "failures",
          "value": [{ "name": "Jam", "availability": 0.9, "repair_time": { "constant": "1:00" } }, { "name": "Jam", "availability": 0.9, "repair_time": { "constant": "1:00" } }] } ] }));
    assert!(refused["problems"][0]["message"].as_str().unwrap().contains("already a failure profile named \"Jam\""), "{refused}");
}
