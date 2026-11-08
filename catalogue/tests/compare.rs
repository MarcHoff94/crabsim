//! Comparing two runs through `compare_runs`, by `call(name, JSON)`: paired when they used the
//! same seed, object by object, and unpaired otherwise, each figure checked against the store's own
//! values worked out here.

mod common;

use common::{write_models, Setup};
use serde_json::{json, Value};

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
}

/// Whether `shown` is `exact` to 4 significant digits, or within a hair of 0.
fn shown_as(shown: f64, exact: f64) -> bool {
    (shown - exact).abs() <= 5e-4 * exact.abs() + 1e-9
}

/// The demo with Station 2, its bottleneck, faster.
fn faster(setup: &Setup) {
    let mut model: Value = serde_json::from_str(&common::demo_text()).unwrap();
    let stations = model["instances"].as_array_mut().unwrap();
    let station = stations.iter_mut().find(|instance| instance["name"] == "Station 2").unwrap();
    station["attributes"]["process_time"] = json!("00:00:10");
    setup.ok("write_model", json!({ "path": "faster.json", "text": model.to_string() }));
}

/// Each observation's value of `statistic` for `object` in `run`, in order.
fn values(setup: &Setup, run: &str, object: &str, statistic: &str) -> Vec<f64> {
    let stored = setup.stored(run, object);
    stored.values().map(|figures| figures[statistic]).collect()
}

fn t(df: usize) -> f64 {
    [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228][df - 1]
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn variance(values: &[f64]) -> f64 {
    let m = mean(values);
    values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64
}

fn run(setup: &Setup, model: &str, seed: u64, observations: u32) -> String {
    let run = setup.start(json!({ "model": model, "seed": seed, "length": "4:00:00", "observations": observations }));
    setup.finished(&run);
    run
}

fn figure<'a>(compared: &'a Value, object: &str, statistic: &str) -> &'a Value {
    let object = compared["objects"].as_array().unwrap().iter().find(|o| o["name"] == object).unwrap_or_else(|| panic!("no {object} in {compared}"));
    &object["figures"][statistic]
}

#[test]
fn two_variants_run_with_the_same_seed_are_compared_pair_by_pair() {
    let setup = Setup::new("paired");
    write_models(&setup.folder);
    faster(&setup);
    let a = run(&setup, "demo.json", 7, 5);
    let b = run(&setup, "faster.json", 7, 5);
    let compared = setup.ok("compare_runs", json!({ "a": a, "b": b }));
    assert_eq!(compared["pairing"], "paired");
    assert_eq!(compared["dropped"], json!([]));
    assert_eq!(compared["objects"].as_array().unwrap().len(), 14, "a small model whole");

    let drain = figure(&compared, "Drain", "throughput_per_hour");
    let (va, vb) = (values(&setup, &a, "Drain", "throughput_per_hour"), values(&setup, &b, "Drain", "throughput_per_hour"));
    let differences: Vec<f64> = vb.iter().zip(&va).map(|(b, a)| b - a).collect();
    let half = t(4) * (variance(&differences) / 5.0).sqrt();
    assert_eq!(drain["method"], "paired");
    assert_eq!(drain["counted"], json!([5, 5]));
    assert!(shown_as(number(&drain["a"]), mean(&va)));
    assert!(shown_as(number(&drain["b"]), mean(&vb)));
    assert!(shown_as(number(&drain["difference"]), mean(&differences)), "{drain}");
    assert!(shown_as(number(&drain["ci95"][0]), mean(&differences) - half), "{drain}");
    assert!(shown_as(number(&drain["ci95"][1]), mean(&differences) + half), "{drain}");
    assert!(shown_as(number(&drain["relative"]), mean(&differences) / mean(&va)));
    assert_eq!(drain["clear"], true, "a faster bottleneck clearly raises throughput: {drain}");
}

#[test]
fn a_run_compared_with_itself_differs_by_exactly_nothing() {
    let setup = Setup::new("itself");
    write_models(&setup.folder);
    let a = run(&setup, "demo.json", 3, 3);
    let compared = setup.ok("compare_runs", json!({ "a": a, "b": a }));
    for object in compared["objects"].as_array().unwrap() {
        for (statistic, figure) in object["figures"].as_object().unwrap() {
            assert_eq!(number(&figure["difference"]), 0.0, "{statistic} of {}", object["name"]);
            assert_eq!(figure["clear"], false);
        }
    }
}

#[test]
fn runs_with_different_seeds_are_compared_with_welchs_interval() {
    let setup = Setup::new("welch");
    write_models(&setup.folder);
    let a = run(&setup, "demo.json", 1, 4);
    let b = run(&setup, "demo.json", 2, 6);
    let compared = setup.ok("compare_runs", json!({ "a": a, "b": b, "objects": ["Drain"] }));
    assert_eq!(compared["pairing"], "unpaired");
    assert!(compared["why_unpaired"].as_str().unwrap().contains("seeds"), "{compared}");
    let figure = figure(&compared, "Drain", "mus_in");
    assert_eq!(figure["method"], "welch");
    assert_eq!(figure["counted"], json!([4, 6]));
    let (va, vb) = (values(&setup, &a, "Drain", "mus_in"), values(&setup, &b, "Drain", "mus_in"));
    let (sa, sb) = (variance(&va) / 4.0, variance(&vb) / 6.0);
    let df = ((sa + sb).powi(2) / (sa.powi(2) / 3.0 + sb.powi(2) / 5.0)).floor() as usize;
    let half = t(df) * (sa + sb).sqrt();
    let difference = mean(&vb) - mean(&va);
    assert!(shown_as(number(&figure["difference"]), difference), "{figure}");
    assert!(shown_as(number(&figure["ci95"][0]), difference - half), "{figure}, df {df}");
    assert!(shown_as(number(&figure["ci95"][1]), difference + half), "{figure}, df {df}");
}

#[test]
fn an_object_is_paired_only_where_it_stands_in_the_same_place_in_both_models() {
    let setup = Setup::new("places");
    write_models(&setup.folder);
    let mut model: Value = serde_json::from_str(&common::demo_text()).unwrap();
    model["instances"].as_array_mut().unwrap().insert(0, json!({ "name": "Spare", "class": "Drain" }));
    setup.ok("write_model", json!({ "path": "inserted.json", "text": model.to_string() }));
    setup.ok("copy_model", json!({ "from": "demo.json", "to": "appended.json" })).to_string();
    setup.ok("adopt_model", json!({ "path": "appended.json" }));
    setup.ok("edit_model", json!({ "path": "appended.json", "changes": [{ "change": "add_instance", "name": "Spare", "class": "Drain" }] }));

    let a = run(&setup, "demo.json", 5, 3);
    let inserted = run(&setup, "inserted.json", 5, 3);
    let appended = run(&setup, "appended.json", 5, 3);

    let compared = setup.ok("compare_runs", json!({ "a": a, "b": inserted }));
    assert_eq!(compared["only_in_b"], json!(["Spare"]));
    let station = compared["objects"].as_array().unwrap().iter().find(|o| o["name"] == "Station 1").unwrap();
    assert_eq!(station["paired"], false);
    assert!(station["why_unpaired"].as_str().unwrap().contains("place"), "{station}");
    assert_eq!(station["figures"]["mus_in"]["method"], "welch");

    let compared = setup.ok("compare_runs", json!({ "a": a, "b": appended }));
    assert_eq!(compared["only_in_b"], json!(["Spare"]));
    assert!(compared["objects"].as_array().unwrap().iter().all(|o| o["paired"] == true), "appending keeps every object paired: {compared}");
}

#[test]
fn dropped_pairs_are_listed_with_why() {
    let setup = Setup::new("dropped");
    write_models(&setup.folder);
    // A seed whose observations of the loop model mix failing and passing ones.
    let mut found = None;
    for seed in 0..20 {
        // As in the jobs test: within 30 s an observation fails unless its Gate fails first.
        let run = setup.start(json!({ "model": "loop.json", "seed": seed, "length": "30", "observations": 6 }));
        setup.finished(&run);
        let failed = setup.ok("run_status", json!({ "run": run }))["failed"].as_u64().unwrap();
        if failed > 0 && failed < 6 {
            found = Some((run, failed));
            break;
        }
    }
    let (run, failed) = found.expect("a seed mixing failing and passing observations");
    let compared = setup.ok("compare_runs", json!({ "a": run, "b": run }));
    let dropped = compared["dropped"].as_array().unwrap();
    assert_eq!(dropped.len() as u64, failed);
    let results = setup.ok("run_results", json!({ "run": run, "values": true }));
    for entry in dropped {
        let k = entry["observation"].as_u64().unwrap();
        let observation = &results["observation_runs"][(k - 1) as usize];
        let reason = entry["reason"].as_str().unwrap();
        assert!(reason.contains("failed") && reason.contains(&run) && reason.contains(observation["error"].as_str().unwrap()), "{reason}");
    }
    let counted = &figure(&compared, "Gate", "mus_in")["counted"];
    assert_eq!(counted, &json!([6 - failed, 6 - failed]));
}

#[test]
fn chosen_objects_and_classes_are_compared_and_unknown_ones_refused() {
    let setup = Setup::new("chosen");
    write_models(&setup.folder);
    let a = run(&setup, "demo.json", 1, 2);
    let b = run(&setup, "demo.json", 1, 2);
    let names = |input: Value| -> Vec<String> {
        setup.ok("compare_runs", input)["objects"].as_array().unwrap().iter().map(|o| o["name"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(names(json!({ "a": a, "b": b, "objects": ["Station 1", "Drain"] })), ["Station 1", "Drain"]);
    assert_eq!(names(json!({ "a": a, "b": b, "classes": ["Drain"] })), ["Drain"]);
    assert_eq!(setup.err("compare_runs", json!({ "a": a, "b": b, "objects": ["Nobody"] }))["kind"], "not_found");
    assert_eq!(setup.err("compare_runs", json!({ "a": a, "b": "run-99" }))["kind"], "not_found");
}
