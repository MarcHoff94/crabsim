//! Rules (ADR 0030) through `call(name, JSON)`, as an agent uses them: written with edit_model,
//! refused where they don't fit, renamed with what they name, explained by query_model and run.

mod common;

use common::Setup;
use serde_json::{json, Value};

/// A Source every 10 s into a Split, then Line A or Line B, then a Drain.
fn line() -> String {
    json!({
        "format_version": 2,
        "instances": [
            { "name": "In", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Split", "class": "Station", "attributes": { "process_time": "0" } },
            { "name": "Line A", "class": "Station", "attributes": { "process_time": "4" } },
            { "name": "Line B", "class": "Station", "attributes": { "process_time": "4" } },
            { "name": "Out", "class": "Drain" }
        ],
        "connections": [
            { "from": "In", "to": "Split", "exit": 1, "entrance": 1 },
            { "from": "Split", "to": "Line A", "exit": 1, "entrance": 1 },
            { "from": "Split", "to": "Line B", "exit": 2, "entrance": 1 },
            { "from": "Line A", "to": "Out", "exit": 1, "entrance": 1 },
            { "from": "Line B", "to": "Out", "exit": 1, "entrance": 2 }
        ]
    })
    .to_string()
}

fn set_rules(setup: &Setup, rules: Value) -> Result<Value, crabsim_catalogue::OperationError> {
    let change = json!({ "change": "set_attribute", "target": { "instance": "Split" }, "attribute": "distribution", "value": { "rules": rules } });
    setup.catalogue.call("edit_model", json!({ "path": "line.json", "changes": [change] }))
}

#[test]
fn rules_are_written_checked_renamed_explained_and_run() {
    let setup = Setup::new("rules-flow");
    setup.ok("write_model", json!({ "path": "line.json", "text": line() }));

    // A rule naming no successor is refused with where it is.
    let refused = set_rules(&setup, json!(["otherwise to Line C"])).unwrap_err();
    assert!(refused.problems.iter().any(|problem| problem.pointer == "/instances/1/attributes/distribution/rules/0" && problem.message.contains("`Line C` isn't one of the successors")), "{refused:?}");

    set_rules(&setup, json!(["when Line A.working_share last 10min >= 50% then to Line B", "otherwise to Line A"])).unwrap();
    // Renaming what the rules name renames it in them.
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [{ "change": "rename_instance", "name": "Line A", "to": "Line 1" }] }));
    let text = setup.folder.read("line.json");
    assert!(text.contains("when Line 1.working_share last 10min >= 50% then to Line B"), "{text}");
    assert!(text.contains("otherwise to Line 1"), "{text}");

    let explained = setup.ok("query_model", json!({ "path": "line.json", "query": { "kind": "distribution", "name": "Split" } }));
    assert_eq!(explained["strategy"], "rules");
    assert_eq!(explained["rules"][1], "otherwise to Line 1");

    // Line 1 takes every MU until it worked half the last 10 minutes, which at 4 s of every 10 s
    // it never does: Line B gets nothing.
    let run = setup.start(json!({ "model": "line.json", "length": "2:00:00", "deterministic": true }));
    setup.finished(&run);
    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT object, value FROM statistics WHERE run = '{run}' AND statistic = 'mus_in' ORDER BY object") }));
    let mus_in = |name: &str| rows["rows"].as_array().unwrap().iter().find(|row| row[0] == name).unwrap()[1].as_f64().unwrap();
    assert!(mus_in("Line 1") > 700.0, "{rows}");
    assert_eq!(mus_in("Line B"), 0.0, "{rows}");
}
