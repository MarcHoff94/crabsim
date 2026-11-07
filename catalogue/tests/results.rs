//! Reading results through `run_results`, labelling and forgetting runs, and the store across a
//! restart, through `call(name, JSON)`.

mod common;

use common::{write_models, Setup, TempFolder};
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

fn number(amount: &Value) -> f64 {
    amount.as_f64().unwrap_or_else(|| panic!("not a number: {amount}"))
}

#[test]
fn each_figures_mean_interval_minimum_and_maximum_agree_with_the_observations() {
    let setup = Setup::new("figures");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 11, "length": "4:00:00", "observations": 5 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Drain", "Station 1"] }));

    for object in results["objects"].as_array().unwrap() {
        let stored = setup.stored(&run, object["name"].as_str().unwrap());
        for (statistic, figure) in object["figures"].as_object().unwrap() {
            if statistic == "mean_lead_time" {
                continue;
            }
            let values: Vec<f64> = figure["values"].as_array().unwrap().iter().map(number).collect();
            assert_eq!(values.len(), 5);
            for (k, value) in values.iter().enumerate() {
                assert_eq!(*value, stored[&(k as u64 + 1)][statistic], "{statistic} of observation {}", k + 1);
            }
            let mean = values.iter().sum::<f64>() / 5.0;
            let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 4.0).sqrt();
            // t for 4 degrees of freedom, 97.5%.
            let half = 2.776 * sd / 5f64.sqrt();
            let close = |a: f64, b: f64| (a - b).abs() <= 1e-9 * b.abs().max(1.0);
            assert!(close(number(&figure["mean"]), mean), "{statistic} mean");
            assert!(close(number(&figure["ci95"][0]), mean - half), "{statistic} interval: {figure}");
            assert!(close(number(&figure["ci95"][1]), mean + half), "{statistic} interval: {figure}");
            assert_eq!(number(&figure["min"]), values.iter().copied().fold(f64::INFINITY, f64::min), "{statistic} min");
            assert_eq!(number(&figure["max"]), values.iter().copied().fold(f64::NEG_INFINITY, f64::max), "{statistic} max");
        }
    }
    let names: Vec<&str> = results["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Station 1", "Drain"], "in model order");
    let drain = &results["objects"][1];
    assert!(drain["figures"]["throughput_per_hour"]["mean"].as_f64().unwrap() > 0.0);
    let lead_time = drain["figures"]["mean_lead_time"]["mean"].as_str().expect("a time, as the UI shows it");
    assert!(crabsim::time::parse_time(lead_time).is_some(), "{lead_time}");
}

#[test]
fn a_single_observation_has_no_interval() {
    let setup = Setup::new("one");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 1 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Work"] }));
    let figure = &results["objects"][0]["figures"]["mus_in"];
    assert_eq!(figure["ci95"], Value::Null);
    assert_eq!(figure["mean"], figure["min"]);
    // Nothing is blocked or fails in this model, so its summary is its Drain alone.
    let summary = setup.ok("run_results", json!({ "run": run }));
    let names: Vec<&str> = summary["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Out"]);
}

/// The objects whose mean of `statistic` is highest and above 0, at most 10, by the store's own
/// arithmetic.
fn top(setup: &Setup, run: &str, statistic: &str) -> BTreeSet<String> {
    let sql = format!(
        "SELECT object, avg(value) AS mean FROM statistics WHERE run = '{run}' AND statistic = '{statistic}' GROUP BY object ORDER BY mean DESC"
    );
    let rows = setup.ok("query_results", json!({ "sql": sql }))["rows"].clone();
    let rows: Vec<(String, f64)> = rows.as_array().unwrap().iter().map(|row| (row[0].as_str().unwrap().to_string(), row[1].as_f64().unwrap())).collect();
    assert!(rows.iter().filter(|(_, mean)| *mean > 0.0).count() > 10, "the test needs more than 10 objects with some {statistic}: {rows:?}");
    assert!(rows[9].1 > rows[10].1, "the test needs no tie at the 10th place for {statistic}: {rows:?}");
    rows.into_iter().take(10).map(|(name, _)| name).collect()
}

#[test]
fn the_default_summary_holds_every_drain_and_the_ten_most_blocked_and_most_failed() {
    let setup = Setup::new("summary");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "long-line.json", "seed": 2, "length": "1d", "observations": 3 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run }));

    let blocked = top(&setup, &run, "blocked_share");
    let failed = top(&setup, &run, "failed_share");
    let mut why: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for object in results["objects"].as_array().unwrap() {
        let reasons = object["why"].as_array().unwrap().iter().map(|reason| reason.as_str().unwrap().to_string()).collect();
        why.insert(object["name"].as_str().unwrap().to_string(), reasons);
    }
    let names: BTreeSet<String> = why.keys().cloned().collect();
    let expected: BTreeSet<String> = blocked.iter().chain(&failed).cloned().chain(["Out".to_string()]).collect();
    assert_eq!(names, expected);
    assert!(why["Out"].contains("drain"));
    for name in &blocked {
        assert!(why[name].contains("most_blocked"), "{name}");
    }
    for name in &failed {
        assert!(why[name].contains("most_failed"), "{name}");
    }
    // In model order.
    let order: Vec<&str> = results["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    let model = crabsim_model::load(&common::long_line()).unwrap();
    let positions: Vec<usize> = order.iter().map(|name| model.instances.iter().position(|i| i.name == *name).unwrap()).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
}

#[test]
fn the_filters_give_exactly_the_objects_and_classes_they_name() {
    let setup = Setup::new("filters");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "long-line.json", "seed": 2, "length": "2:00:00", "observations": 2 }));
    setup.finished(&run);
    let names = |input: Value| -> Vec<String> {
        let results = setup.ok("run_results", input);
        results["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(names(json!({ "run": run, "objects": ["S3", "In"] })), ["In", "S3"]);
    assert_eq!(names(json!({ "run": run, "classes": ["Fragile"] })), (1..=12).map(|n| format!("S{n}")).collect::<Vec<_>>());
    let stations: Vec<String> = (1..=16).map(|n| format!("S{n}")).chain(["Slow".to_string()]).collect();
    assert_eq!(names(json!({ "run": run, "classes": ["Station"] })), stations, "a built-in class takes in its derived classes");
    assert_eq!(names(json!({ "run": run, "classes": ["Drain"], "objects": ["S1"] })), ["S1", "Out"]);

    let results = setup.ok("run_results", json!({ "run": run, "objects": ["S2"] }));
    assert_eq!(results["objects"][0]["class"], "Fragile");
    assert_eq!(results["objects"][0]["built_in_class"], "Station");
    assert_eq!(results["objects"][0]["why"], json!(["chosen"]));
    assert_eq!(setup.err("run_results", json!({ "run": run, "objects": ["Nobody"] }))["kind"], "not_found");
    assert_eq!(setup.err("run_results", json!({ "run": run, "classes": ["Nothing"] }))["kind"], "not_found");
    assert_eq!(setup.err("run_results", json!({ "run": "run-99" }))["kind"], "not_found");
}

#[test]
fn results_carry_what_reproduces_them() {
    let setup = Setup::new("settings");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "small.json", "seed": 9, "length": "1:30:00", "warm_up": "0:30:00", "observations": 2, "label": "check" }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run }));
    for (field, value) in [
        ("run", json!(run)),
        ("label", json!("check")),
        ("state", json!("done")),
        ("model", json!("small.json")),
        ("seed", json!("9")),
        ("length", json!("01:30:00")),
        ("warm_up", json!("00:30:00")),
        ("observations", json!(2)),
        ("source", json!("mcp")),
        ("crabsim_version", json!(env!("CARGO_PKG_VERSION"))),
    ] {
        assert_eq!(results[field], value, "{field}");
    }
    assert!(results["started"].as_str().unwrap().ends_with('Z'));
    assert!(results["finished"].is_string());
    let observations = results["observation_runs"].as_array().unwrap();
    assert_eq!(observations.len(), 2);
    for (k, observation) in observations.iter().enumerate() {
        assert_eq!(observation["observation"], k + 1);
        assert_eq!(observation["seed"], common::splitmix64(9 + k as u64 + 1).to_string());
        assert!(observation["events"].as_u64().unwrap() > 0);
        assert!(observation["wall_time_s"].as_f64().unwrap() >= 0.0);
        assert_eq!(observation["error"], Value::Null);
    }
}

#[test]
fn the_store_survives_a_restart() {
    let folder = TempFolder::new("restart");
    write_models(&folder);
    let open = || Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap();
    let catalogue = open();
    let first = catalogue.call("start_run", json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 2 })).unwrap();
    let run = first["run"].as_str().unwrap().to_string();
    let setup = common::Setup { folder, catalogue };
    setup.finished(&run);
    let before = setup.ok("run_results", json!({ "run": run }));
    let Setup { folder, catalogue } = setup;
    drop(catalogue);

    let setup = common::Setup { catalogue: Catalogue::open(Settings::new(&folder.path, Source::Cli)).unwrap(), folder };
    assert_eq!(setup.ok("run_results", json!({ "run": run })), before);
    let status = setup.ok("run_status", json!({ "run": run }));
    assert_eq!(status, json!({ "run": run, "state": "done", "observations": 2, "ended": 2, "failed": 0, "running": [], "here": false }));
    assert_eq!(setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00", "observations": 1 })), "run-2");
    assert_eq!(before["source"], "cli");
}

#[test]
fn a_label_can_be_set_changed_and_removed() {
    let setup = Setup::new("label");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00", "observations": 1 }));
    setup.finished(&run);
    assert_eq!(setup.ok("label_run", json!({ "run": run, "label": "baseline" })), json!({ "run": run, "label": "baseline" }));
    assert_eq!(setup.ok("run_results", json!({ "run": run }))["label"], "baseline");
    setup.ok("label_run", json!({ "run": run, "label": "two more mills" }));
    assert_eq!(setup.ok("run_results", json!({ "run": run }))["label"], "two more mills");
    setup.ok("label_run", json!({ "run": run }));
    assert_eq!(setup.ok("run_results", json!({ "run": run }))["label"], Value::Null);
    assert_eq!(setup.err("label_run", json!({ "run": "run-9", "label": "x" }))["kind"], "not_found");
}

#[test]
fn forgetting_a_run_deletes_all_its_rows_and_its_id_is_never_used_again() {
    let setup = Setup::new("forget");
    write_models(&setup.folder);
    let input = json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 2 });
    let first = setup.start(input.clone());
    let second = setup.start(input.clone());
    setup.finished(&first);
    setup.finished(&second);

    assert_eq!(setup.ok("forget_run", json!({ "run": second })), json!({ "forgotten": second }));
    let count = |table: &str, run: &str| {
        let column = if table == "runs" { "id" } else { "run" };
        setup.ok("query_results", json!({ "sql": format!("SELECT count(*) FROM {table} WHERE {column} = '{run}'") }))["rows"][0][0].as_u64().unwrap()
    };
    for table in ["runs", "observations", "statistics"] {
        assert_eq!(count(table, &second), 0, "{table}");
        assert!(count(table, &first) > 0, "{table} keeps the other run");
    }
    assert_eq!(setup.err("run_results", json!({ "run": second }))["kind"], "not_found");
    assert_eq!(setup.err("forget_run", json!({ "run": second }))["kind"], "not_found");
    assert_eq!(setup.start(input), "run-3");
}

#[test]
fn a_running_run_cant_be_forgotten() {
    let setup = Setup::with_threads("forget-running", 1);
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 1, "length": "3650d", "observations": 1 }));
    assert_eq!(setup.err("forget_run", json!({ "run": run }))["kind"], "invalid_input");
    setup.ok("cancel_run", json!({ "run": run }));
    setup.ok("forget_run", json!({ "run": run }));
}

#[test]
fn a_cli_and_another_catalogue_share_one_store() {
    let setup = Setup::new("shared");
    write_models(&setup.folder);
    let other = Catalogue::open(Settings::new(&setup.folder.path, Source::Cli)).unwrap();
    let mine = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 2 }));
    let theirs = other.call("start_run", json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 2 })).unwrap();
    let theirs = theirs["run"].as_str().unwrap();
    assert_ne!(mine, theirs);
    setup.finished(&mine);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while other.call("run_status", json!({ "run": theirs })).unwrap()["state"] != "done" {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let sources = setup.ok("query_results", json!({ "sql": "SELECT id, source FROM runs ORDER BY number" }));
    assert_eq!(sources["rows"], json!([[mine, "mcp"], [theirs, "cli"]]));
    assert_eq!(setup.ok("run_results", json!({ "run": theirs }))["source"], "cli");
}
