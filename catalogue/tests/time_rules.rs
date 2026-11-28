//! A Station's process time by MU and which MUs an object takes (ADR 0030, spec issue 162's
//! PR 5) through `call(name, JSON)`, as an agent uses them: written with edit_model, refused where
//! they don't fit, renamed with what they name, run, and judged, the first MU that broke a limit
//! named in run_results and the results store with its class and step; and an older store
//! upgraded.

mod common;

use common::Setup;
use serde_json::{json, Value};

/// Frames, whose plan is Mill then Out, and totes without a plan, each every 100 s, into Mill
/// (20 s), then Out.
fn line() -> String {
    json!({
        "format_version": 2,
        "classes": [
            { "name": "Frame", "parent": "MU", "attributes": { "length": 1.2, "work_plan": [{ "at": "Mill" }, { "at": "Out" }] } },
            { "name": "Tote", "parent": "MU" }
        ],
        "instances": [
            { "name": "Frames", "class": "Source", "attributes": { "interval": "100", "mu_class": "Frame" } },
            { "name": "Totes", "class": "Source", "attributes": { "interval": "100", "mu_class": "Tote" } },
            { "name": "Mill", "class": "Station", "attributes": { "process_time": "20" } },
            { "name": "Out", "class": "Drain" }
        ],
        "connections": [
            { "from": "Frames", "to": "Mill", "exit": 1, "entrance": 1 },
            { "from": "Totes", "to": "Mill", "exit": 1, "entrance": 2 },
            { "from": "Mill", "to": "Out", "exit": 1, "entrance": 1 }
        ]
    })
    .to_string()
}

fn set_process_time(setup: &Setup, value: Value) -> Result<Value, crabsim_catalogue::OperationError> {
    let change = json!({ "change": "set_attribute", "target": { "instance": "Mill" }, "attribute": "process_time", "value": value });
    setup.catalogue.call("edit_model", json!({ "path": "line.json", "changes": [change] }))
}

#[test]
fn a_process_time_by_mu_is_written_checked_renamed_and_run() {
    let setup = Setup::new("time-rules-flow");
    setup.ok("write_model", json!({ "path": "line.json", "text": line() }));

    let refused = set_process_time(&setup, json!({ "rules": ["when class = Crate then 5s"], "default": "20" })).unwrap_err();
    assert!(refused.problems.iter().any(|problem| problem.pointer == "/instances/2/attributes/process_time/rules/0" && problem.message.contains("`Crate` isn't an MU class")), "{refused:?}");

    set_process_time(&setup, json!({ "rules": ["when class = Tote then 5s", "when step = Mill then 40s"], "default": "20" })).unwrap();
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "rename_class", "name": "Tote", "to": "Crate" },
        { "change": "rename_instance", "name": "Mill", "to": "Saw" }
    ] }));
    let text = setup.folder.read("line.json");
    assert!(text.contains("when class = Crate then 5s") && text.contains("when step = Saw then 40s"), "{text}");
    let attributes = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Saw" } }));
    let value = serde_json::to_string(&attributes).unwrap();
    assert!(value.contains("when step = Saw then 40s"), "{value}");

    // Frames are on their plan's step at the Saw, which has no time of its own: 40 s; crates 5 s.
    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "deterministic": true }));
    setup.finished(&run);
    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT value FROM statistics WHERE run = '{run}' AND object = 'Saw' AND statistic = 'working_share'") }));
    let busy = rows["rows"][0][0].as_f64().unwrap();
    // 45 s of every 100 s, but for the first 100 s.
    assert!((0.43..0.45).contains(&busy), "{busy}");
}

#[test]
fn a_limit_on_which_mus_an_object_takes_is_judged_with_the_first_mu_that_broke_it() {
    let setup = Setup::new("entry-limits");
    setup.ok("write_model", json!({ "path": "line.json", "text": line() }));
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "add_expectation", "id": "frames-only", "text": "Mill.class always in Frame" },
        { "change": "add_expectation", "id": "on-step", "text": "Mill.step always in Mill" },
        { "change": "add_expectation", "id": "no-crates", "text": "Mill.class always not in Frame, Tote" }
    ] }));
    // An unknown name is refused where it is.
    let refused = setup.error("edit_model", json!({ "path": "line.json", "changes": [{ "change": "add_expectation", "id": "bad", "text": "Mill.step always in Nowhere" }] }));
    assert!(refused.problems.iter().any(|problem| problem.pointer == "/expectations/3/text" && problem.message.contains("`Nowhere` isn't")), "{refused:?}");

    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "deterministic": true }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run }));
    let expectation = |id: &str| results["expectations"].as_array().unwrap().iter().find(|found| found["id"] == id).unwrap().clone();
    let frames_only = expectation("frames-only");
    assert_eq!(frames_only["verdict"], "not_met", "{frames_only:#}");
    assert_eq!(frames_only["comparison"], "in");
    let violation = &frames_only["violation"];
    assert_eq!((violation["time"].as_str(), violation["mu"].as_u64(), violation["class"].as_str(), violation["step"].as_str()), (Some("00:02:00"), Some(2), Some("Tote"), Some("none")), "{violation:#}");
    assert!(violation.get("value").is_none(), "the MU says it: {violation:#}");
    assert_eq!(expectation("on-step")["verdict"], "not_met", "a tote has no step");
    assert_eq!(expectation("no-crates")["verdict"], "not_met");
    // The store keeps the class and step.
    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT expectation, mu, class, step FROM violations WHERE run = '{run}' ORDER BY expectation") }));
    assert_eq!(rows["rows"], json!([["frames-only", 2, "Tote", "none"], ["no-crates", 1, "Frame", "Mill"], ["on-step", 2, "Tote", "none"]]), "{rows:#}");
    let values = setup.ok("query_results", json!({ "sql": format!("SELECT value FROM expectation_values WHERE run = '{run}' AND expectation = 'frames-only'") }));
    assert!(values["rows"][0][0].as_f64().unwrap() >= 30.0, "the totes that broke it: {values:#}");

    // Without the totes, each limit of frames is met.
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [{ "change": "remove_instance", "name": "Totes" }, { "change": "remove_expectation", "id": "no-crates" }] }));
    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "observations": 3 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run }));
    for expectation in results["expectations"].as_array().unwrap() {
        assert_eq!(expectation["verdict"], "met", "{expectation:#}");
    }
}

#[test]
fn a_store_of_the_ninth_version_gains_the_violations_class_and_step() {
    let setup = Setup::new("violations-upgrade");
    setup.ok("write_model", json!({ "path": "line.json", "text": line() }));
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [{ "change": "add_expectation", "id": "frames-only", "text": "Mill.class always in Frame" }] }));
    let run = setup.start(json!({ "model": "line.json", "length": "1:00:00", "deterministic": true }));
    setup.finished(&run);
    let folder = setup.folder.join("");
    drop(setup.catalogue);
    {
        let store = rusqlite::Connection::open(folder.join(".crabsim/results.sqlite")).unwrap();
        store.execute_batch("ALTER TABLE violations DROP COLUMN class; ALTER TABLE violations DROP COLUMN step; PRAGMA user_version = 9;").unwrap();
    }
    let catalogue = crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&folder, crabsim_catalogue::Source::Cli)).unwrap();
    let version = catalogue.call("query_results", json!({ "sql": "SELECT user_version FROM pragma_user_version" })).unwrap();
    assert_eq!(version["rows"][0][0], 10);
    // The older run's violation has no class or step, and is still judged.
    let results = catalogue.call("run_results", json!({ "run": run })).unwrap();
    assert_eq!(results["expectations"][0]["verdict"], "not_met");
    assert!(results["expectations"][0]["violation"].get("class").is_none());
}
