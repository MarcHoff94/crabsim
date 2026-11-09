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

/// Whether `shown` is `exact` as results show it: to 4 significant digits, but never fewer than
/// its whole number. Either way the rounding is at most 1 part in 2,000.
fn shown_as(shown: f64, exact: f64) -> bool {
    (shown - exact).abs() <= 5e-4 * exact.abs() + 1e-12
}

#[test]
fn each_figures_mean_interval_minimum_and_maximum_agree_with_the_observations() {
    let setup = Setup::new("figures");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 11, "length": "4:00:00", "observations": 5 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Drain", "Station 1"], "values": true }));

    for object in results["objects"].as_array().unwrap() {
        let stored = setup.stored(&run, object["name"].as_str().unwrap());
        for (statistic, figure) in object["figures"].as_object().unwrap() {
            let shown: Vec<f64> = figure["values"].as_array().unwrap().iter().map(number).collect();
            assert_eq!(shown.len(), 5);
            let values: Vec<f64> = (1..=5u64).map(|k| stored[&k][statistic]).collect();
            for (k, value) in shown.iter().enumerate() {
                assert!(shown_as(*value, values[k]), "{statistic} of observation {}: {value} for {}", k + 1, values[k]);
            }
            let mean = values.iter().sum::<f64>() / 5.0;
            let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 4.0).sqrt();
            // t for 4 degrees of freedom, 97.5%.
            let half = 2.776 * sd / 5f64.sqrt();
            assert!(shown_as(number(&figure["mean"]), mean), "{statistic} mean");
            assert!(shown_as(number(&figure["ci95"][0]), mean - half), "{statistic} interval: {figure}");
            assert!(shown_as(number(&figure["ci95"][1]), mean + half), "{statistic} interval: {figure}");
            assert!(shown_as(number(&figure["min"]), values.iter().copied().fold(f64::INFINITY, f64::min)), "{statistic} min");
            assert!(shown_as(number(&figure["max"]), values.iter().copied().fold(f64::NEG_INFINITY, f64::max)), "{statistic} max");
        }
    }
    let names: Vec<&str> = results["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Station 1", "Drain"], "in model order");
    let drain = &results["objects"][1];
    assert!(drain["figures"]["throughput_per_hour"]["mean"].as_f64().unwrap() > 0.0);
    assert!(drain["figures"]["mean_lead_time_s"]["mean"].as_f64().unwrap() > 0.0, "lead time in seconds, named as in the store");
    assert!(drain["figures"].get("mean_lead_time").is_none());
}

/// Every number in `value`, with where it is.
fn numbers<'a>(value: &'a Value, at: String, found: &mut Vec<(String, &'a Value)>) {
    match value {
        Value::Number(_) => found.push((at, value)),
        Value::Array(items) => items.iter().enumerate().for_each(|(k, item)| numbers(item, format!("{at}/{k}"), found)),
        Value::Object(fields) => fields.iter().for_each(|(key, item)| numbers(item, format!("{at}/{key}"), found)),
        _ => {}
    }
}

#[test]
fn the_summary_stays_the_same_size_whatever_the_number_of_observations() {
    let setup = Setup::new("summary-size");
    write_models(&setup.folder);
    let summary = |observations: u32| {
        let run = setup.start(json!({ "model": "demo.json", "seed": 4, "length": "2:00:00", "observations": observations }));
        setup.finished(&run);
        (run.clone(), setup.ok("run_results", json!({ "run": run })))
    };
    let (_, few) = summary(2);
    let (many_run, many) = summary(24);
    for results in [&few, &many] {
        assert!(results.get("observation_runs").is_none(), "no list of observations by default: {results}");
        assert_eq!(results["failed_observations"], json!([]));
        assert!(results["events"].as_u64().unwrap() > 0, "every observation's events, added up");
        assert!(results["wall_time_s"].as_f64().unwrap() > 0.0);
        for object in results["objects"].as_array().unwrap() {
            for (statistic, figure) in object["figures"].as_object().unwrap() {
                assert!(figure.get("values").is_none(), "{statistic} of {}: no values by default", object["name"]);
            }
        }
    }
    let (few_size, many_size) = (few["objects"].to_string().len() as f64, many["objects"].to_string().len() as f64);
    assert!((many_size / few_size - 1.0).abs() < 0.1, "{few_size} bytes for 2 observations, {many_size} for 24");

    let detailed = setup.ok("run_results", json!({ "run": many_run, "values": true }));
    assert_eq!(detailed["observation_runs"].as_array().unwrap().len(), 24);
    assert_eq!(detailed["objects"][0]["figures"]["mus_in"]["values"].as_array().unwrap().len(), 24);
}

#[test]
fn figures_that_are_0_in_every_observation_are_left_out() {
    let setup = Setup::new("zeros");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 5, "length": "4:00:00", "observations": 3 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Source", "Station 1", "Drain"] }));
    let mut left_out = 0;
    for object in results["objects"].as_array().unwrap() {
        let name = object["name"].as_str().unwrap();
        let stored = setup.stored(&run, name);
        let names: BTreeSet<&String> = stored.values().flat_map(|figures| figures.keys()).collect();
        for statistic in names {
            let all_zero = stored.values().all(|figures| figures.get(statistic).is_none_or(|value| *value == 0.0));
            assert_eq!(object["figures"].get(statistic).is_none(), all_zero, "{statistic} of {name}");
            left_out += usize::from(all_zero);
        }
    }
    assert!(left_out > 0, "the test needs a figure that is 0 throughout, such as paused_share");
}

#[test]
fn numbers_have_at_most_4_significant_digits_but_keep_their_whole_number() {
    let setup = Setup::new("rounding");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 6, "length": "4:00:00", "observations": 3 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "classes": ["Station", "Source", "Drain"], "values": true }));
    let mut found = Vec::new();
    numbers(&results["objects"], String::new(), &mut found);
    let mut fractions = 0;
    for (at, value) in found {
        let value = value.as_f64().unwrap();
        if value.fract() == 0.0 {
            continue;
        }
        fractions += 1;
        assert!(value.abs() < 1000.0, "{at}: {value} keeps a fraction beyond 4 digits");
        let digits: String = format!("{}", value.abs()).chars().filter(char::is_ascii_digit).collect();
        let significant = digits.trim_start_matches('0').len();
        assert!(significant <= 4, "{at}: {value} has {significant} significant digits");
    }
    assert!(fractions > 0);
    // The store itself keeps every digit.
    let stored = setup.stored(&run, "Station 1");
    assert!(stored.values().flat_map(|figures| figures.values()).any(|value| format!("{value}").len() > 8));
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
    // A model this small is shown whole.
    let summary = setup.ok("run_results", json!({ "run": run }));
    let names: Vec<&str> = summary["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["In", "Work", "Out"]);
    assert!(summary["objects"].as_array().unwrap().iter().all(|object| object["why"] == json!(["all"])));
}

/// The objects whose mean of `statistic` is highest and above `floor`, at most 10, by the store's
/// own arithmetic.
fn top(setup: &Setup, run: &str, statistic: &str, floor: f64) -> BTreeSet<String> {
    let sql = format!(
        "SELECT object, avg(value) AS mean FROM statistics WHERE run = '{run}' AND statistic = '{statistic}' GROUP BY object ORDER BY mean DESC"
    );
    let rows = setup.ok("query_results", json!({ "sql": sql }))["rows"].clone();
    let rows: Vec<(String, f64)> = rows.as_array().unwrap().iter().map(|row| (row[0].as_str().unwrap().to_string(), row[1].as_f64().unwrap())).collect();
    assert!(rows.iter().filter(|(_, mean)| *mean > floor).count() > 10, "the test needs more than 10 objects above {floor} {statistic}: {rows:?}");
    assert!(rows[9].1 > rows[10].1, "the test needs no tie at the 10th place for {statistic}: {rows:?}");
    rows.into_iter().take(10).map(|(name, _)| name).collect()
}

/// The objects whose failed share's 95% interval lies wholly above that of the median object,
/// worked out here from each observation's stored figure.
fn clearly_most_failed(setup: &Setup, run: &str, observations: usize) -> BTreeSet<String> {
    let sql = format!("SELECT object, value FROM statistics WHERE run = '{run}' AND statistic = 'failed_share' ORDER BY rowid");
    let rows = setup.ok("query_results", json!({ "sql": sql }))["rows"].clone();
    let mut values: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for row in rows.as_array().unwrap() {
        values.entry(row[0].as_str().unwrap().to_string()).or_default().push(row[1].as_f64().unwrap());
    }
    // t for 4 degrees of freedom, 97.5%.
    assert_eq!(observations, 5);
    let interval = |values: &[f64]| {
        let mean = values.iter().sum::<f64>() / 5.0;
        let sd = (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 4.0).sqrt();
        (mean, mean - 2.776 * sd / 5f64.sqrt(), mean + 2.776 * sd / 5f64.sqrt())
    };
    let mut intervals: Vec<(String, (f64, f64, f64))> = values.iter().map(|(name, values)| (name.clone(), interval(values))).collect();
    intervals.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0));
    let median_high = intervals[intervals.len() / 2].1 .2;
    let mut above: Vec<(String, f64)> = intervals.into_iter().filter(|(_, (_, low, _))| *low > median_high).map(|(name, (mean, _, _))| (name, mean)).collect();
    above.sort_by(|a, b| b.1.total_cmp(&a.1));
    above.into_iter().take(10).map(|(name, _)| name).collect()
}

#[test]
fn the_default_summary_holds_every_drain_the_busiest_the_most_blocked_and_the_clearly_most_failed() {
    let setup = Setup::new("summary");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "wide-line.json", "seed": 2, "length": "1d", "observations": 5 }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run }));

    let busiest = top(&setup, &run, "working_share", 0.0);
    let blocked = top(&setup, &run, "blocked_share", 0.01);
    let failed = clearly_most_failed(&setup, &run, 5);
    for weak in ["W5", "W12", "W20"] {
        assert!(failed.contains(weak), "{weak} fails far more often than the rest: {failed:?}");
    }
    let mut why: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for object in results["objects"].as_array().unwrap() {
        let reasons = object["why"].as_array().unwrap().iter().map(|reason| reason.as_str().unwrap().to_string()).collect();
        why.insert(object["name"].as_str().unwrap().to_string(), reasons);
    }
    let names: BTreeSet<String> = why.keys().cloned().collect();
    let expected: BTreeSet<String> = busiest.iter().chain(&blocked).chain(&failed).cloned().chain(["Out".to_string()]).collect();
    assert_eq!(names, expected);
    assert!(names.len() < 28, "not the whole model");
    assert!(why["Out"].contains("drain"));
    for (names, reason) in [(&busiest, "busiest"), (&blocked, "most_blocked"), (&failed, "most_failed")] {
        for name in names {
            assert!(why[name].contains(reason), "{name} is {reason}: {:?}", why[name]);
        }
    }
    // In model order.
    let order: Vec<&str> = results["objects"].as_array().unwrap().iter().map(|object| object["name"].as_str().unwrap()).collect();
    let model = crabsim_model::load(&common::wide_line()).unwrap();
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
    let setup = Setup::agent("settings");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "small.json", "seed": 9, "length": "1:30:00", "warm_up": "0:30:00", "observations": 2, "label": "check" }));
    setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "values": true }));
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
    let setup = Setup::agent("shared");
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
