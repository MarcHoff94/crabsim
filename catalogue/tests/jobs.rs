//! Runs as jobs, through `call(name, JSON)`: starting, polling, cancelling, seeds, threads taking
//! turns, run errors and warm-up, each checked against an independent headless run.

mod common;

use common::{headless_figures, splitmix64, write_models, Setup, BROKEN, LOOP};
use crabsim::time::parse_time;
use crabsim::SimTime;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// The figures of observation `k` of a headless run of `model_text` with `seed`, for every
/// object, after an optional warm-up.
fn headless(model_text: &str, seed: u64, length: &str, warm_up: Option<&str>) -> BTreeMap<String, BTreeMap<String, f64>> {
    let model = crabsim_model::load(model_text).unwrap();
    let mut run = model.to_description().build(seed);
    run.init().unwrap();
    let warm_up = warm_up.map_or(SimTime::ZERO, |typed| parse_time(typed).unwrap());
    if warm_up > SimTime::ZERO {
        run.advance_to(warm_up).unwrap();
        run.clear_statistics();
    }
    run.advance_to(warm_up + parse_time(length).unwrap()).unwrap();
    let state = run.state();
    model.instances.iter().zip(&state.objects).map(|(instance, object)| (instance.name.clone(), headless_figures(&object.statistics, state.time))).collect()
}

/// Asserts the store holds the same figures as `expected`, for every statistic `expected` has.
fn assert_same(stored: &BTreeMap<String, f64>, expected: &BTreeMap<String, f64>, what: &str) {
    for (statistic, value) in expected {
        let got = stored.get(statistic).unwrap_or_else(|| panic!("{what}: no {statistic} in {stored:?}"));
        assert!((got - value).abs() <= 1e-9 * value.abs().max(1.0), "{what} {statistic}: stored {got}, headless {value}");
    }
}

fn seeds_of(setup: &Setup, run: &str) -> Vec<u64> {
    let rows = setup.ok("query_results", json!({ "sql": format!("SELECT seed FROM observations WHERE run = '{run}' ORDER BY number") }));
    rows["rows"].as_array().unwrap().iter().map(|row| row[0].as_str().unwrap().parse().unwrap()).collect()
}

/// Every figure of every observation of a run, without wall times, for comparing runs.
fn all_figures(setup: &Setup, run: &str) -> Value {
    let sql = format!("SELECT observation, object, statistic, value FROM statistics WHERE run = '{run}' ORDER BY observation, object, statistic");
    setup.ok("query_results", json!({ "sql": sql }))["rows"].clone()
}

#[test]
fn a_run_starts_at_once_and_is_polled_until_done() {
    let setup = Setup::new("start");
    write_models(&setup.folder);
    let started = Instant::now();
    let output = setup.ok("start_run", json!({ "model": "small.json", "seed": 1, "length": "8:00:00", "observations": 3, "label": "first" }));
    assert_eq!(output["run"], "run-1");
    assert_eq!(output["state"], "queued");
    assert!(started.elapsed() < Duration::from_secs(5), "start_run doesn't wait for the run");

    let status = setup.finished("run-1");
    assert_eq!(status, json!({ "run": "run-1", "state": "done", "observations": 3, "ended": 3, "failed": 0, "running": [], "here": true }));
    let stored = setup.ok("query_results", json!({ "sql": "SELECT id, model_path, seed, length_s, warm_up_s, observations, label, source, crabsim_version, state, finished IS NOT NULL FROM runs" }));
    assert_eq!(stored["rows"], json!([["run-1", "small.json", "1", 28800.0, 0.0, 3, "first", "mcp", env!("CARGO_PKG_VERSION"), "done", 1]]));
    assert_eq!(setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00", "observations": 1 })), "run-2");
}

#[test]
fn an_invalid_model_is_refused_at_start_with_every_problem() {
    let setup = Setup::new("invalid");
    setup.folder.write("broken.json", BROKEN);
    let error = setup.err("start_run", json!({ "model": "broken.json", "seed": 0, "length": "1:00", "observations": 1 }));
    assert_eq!(error["kind"], "invalid_model");
    assert_eq!(error["problems"].as_array().unwrap().len(), crabsim_model::load(BROKEN).unwrap_err().len());
    assert_eq!(error["problems"][0]["pointer"], "/instances/0");
    let runs = setup.ok("query_results", json!({ "sql": "SELECT count(*) FROM runs" }));
    assert_eq!(runs["rows"], json!([[0]]), "nothing was stored or run");
}

#[test]
fn a_start_that_doesnt_fit_says_why() {
    let setup = Setup::new("bad-start");
    write_models(&setup.folder);
    let start = |input: Value| setup.err("start_run", input);
    assert_eq!(start(json!({ "model": "small.json", "seed": 0, "length": "soon", "observations": 1 }))["kind"], "invalid_input");
    assert_eq!(start(json!({ "model": "small.json", "seed": 0, "length": "0", "observations": 1 }))["kind"], "invalid_input");
    assert_eq!(start(json!({ "model": "small.json", "seed": 0, "length": "1:00", "warm_up": "x", "observations": 1 }))["kind"], "invalid_input");
    assert_eq!(start(json!({ "model": "small.json", "seed": 0, "length": "1:00", "observations": 0 }))["kind"], "invalid_input");
    assert_eq!(start(json!({ "model": "gone.json", "seed": 0, "length": "1:00", "observations": 1 }))["kind"], "not_found");
    assert_eq!(start(json!({ "model": "../small.json", "seed": 0, "length": "1:00", "observations": 1 }))["kind"], "outside_models_folder");
    assert_eq!(setup.err("run_status", json!({ "run": "run-9" }))["kind"], "not_found");
    assert_eq!(setup.err("run_status", json!({ "run": "nine" }))["kind"], "not_found");
}

#[test]
fn each_observation_gets_the_documented_seed_and_matches_a_headless_run_with_it() {
    let setup = Setup::new("seeds");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 42, "length": "4:00:00", "observations": 4 }));
    setup.finished(&run);

    let seeds = seeds_of(&setup, &run);
    assert_eq!(seeds, (1..=4).map(|k| splitmix64(42 + k)).collect::<Vec<_>>());
    let demo = common::demo_text();
    for (k, seed) in seeds.iter().enumerate() {
        let expected = headless(&demo, *seed, "4:00:00", None);
        for (object, figures) in &expected {
            let stored = setup.stored(&run, object);
            assert_same(&stored[&(k as u64 + 1)], figures, &format!("observation {} {object}", k + 1));
        }
    }
    assert_ne!(setup.stored(&run, "Drain")[&1], setup.stored(&run, "Drain")[&2], "observations differ");
}

#[test]
fn an_experiments_seed_reproduces_it_whatever_the_number_of_threads() {
    let one = Setup::with_threads("one-thread", 1);
    let three = Setup::with_threads("three-threads", 3);
    write_models(&one.folder);
    write_models(&three.folder);
    let input = json!({ "model": "demo.json", "seed": 7, "length": "2:00:00", "observations": 5 });
    let first = one.start(input.clone());
    let again = one.start(input.clone());
    let other = three.start(input.clone());
    for (setup, run) in [(&one, &first), (&one, &again), (&three, &other)] {
        assert_eq!(setup.finished(run)["state"], "done");
    }
    assert_eq!(all_figures(&one, &first), all_figures(&one, &again));
    assert_eq!(all_figures(&one, &first), all_figures(&three, &other));
    let different = one.start(json!({ "model": "demo.json", "seed": 8, "length": "2:00:00", "observations": 5 }));
    one.finished(&different);
    assert_ne!(all_figures(&one, &first), all_figures(&one, &different));
}

#[test]
fn statistics_cover_only_the_time_after_the_warm_up() {
    let setup = Setup::new("warm-up");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 3, "length": "3:00:00", "warm_up": "5:00:00", "observations": 2 }));
    setup.finished(&run);
    let demo = common::demo_text();
    for (k, seed) in seeds_of(&setup, &run).iter().enumerate() {
        let with_warm_up = headless(&demo, *seed, "3:00:00", Some("5:00:00"));
        let without = headless(&demo, *seed, "8:00:00", None);
        assert_ne!(with_warm_up["Drain"]["mus_in"], without["Drain"]["mus_in"], "the warm-up must make a difference");
        for (object, figures) in &with_warm_up {
            assert_same(&setup.stored(&run, object)[&(k as u64 + 1)], figures, &format!("observation {} {object}", k + 1));
        }
    }
    let results = setup.ok("run_results", json!({ "run": run }));
    assert_eq!(results["warm_up"], "05:00:00");
    assert_eq!(results["length"], "03:00:00");
}

#[test]
fn a_run_error_fails_only_its_observation_with_its_error_and_time() {
    let setup = Setup::new("run-error");
    write_models(&setup.folder);
    // Which observations fail, from independent headless runs.
    let fails = |seed: u64| {
        let mut run = crabsim_model::load(LOOP).unwrap().to_description().build(seed);
        run.init().unwrap();
        run.advance_to(SimTime::from_secs(30)).err().map(|failure| (failure.error.to_string(), run.time()))
    };
    // An experiment seed whose 4 observations mix failing and passing ones. Each failing run
    // costs a million events, so each is run once.
    let (seed, expected) = (0..100u64)
        .map(|seed| (seed, (1..=4).map(|k| fails(splitmix64(seed + k))).collect::<Vec<_>>()))
        .find(|(_, outcomes)| outcomes.iter().any(Option::is_some) && outcomes.iter().any(Option::is_none))
        .expect("some experiment seed mixes failing and passing observations");

    let run = setup.start(json!({ "model": "loop.json", "seed": seed, "length": "30", "observations": 4 }));
    let status = setup.finished(&run);
    let results = setup.ok("run_results", json!({ "run": run, "objects": ["Gate"] }));
    let mut failed = 0;
    for (k, observation) in results["observation_runs"].as_array().unwrap().iter().enumerate() {
        assert_eq!(observation["seed"], splitmix64(seed + k as u64 + 1).to_string());
        match &expected[k] {
            Some((error, time)) => {
                failed += 1;
                assert_eq!(observation["error"], error.as_str(), "observation {}", k + 1);
                assert_eq!(observation["error_time"], crabsim::time::format_time_exact(*time));
                assert_eq!(results["objects"][0]["figures"]["mus_in"]["values"][k], Value::Null, "a failed observation gives no figures");
            }
            None => {
                assert_eq!(observation["error"], Value::Null, "observation {}", k + 1);
                assert!(results["objects"][0]["figures"]["mus_in"]["values"][k].is_number());
            }
        }
    }
    assert_eq!(status["state"], "done", "a run with any observation that ran to its end is done");
    assert_eq!(status["failed"], failed);
    assert_eq!(status["ended"], 4);

    // An experiment whose only observation is one that failed above.
    let failing = expected.iter().position(Option::is_some).unwrap() as u64;
    let all_fail = seed + failing;
    let run = setup.start(json!({ "model": "loop.json", "seed": all_fail, "length": "30", "observations": 1 }));
    assert_eq!(setup.finished(&run)["state"], "failed");
}

/// Waits until `run` has an observation that has reached some simulated time.
fn wait_until_running(setup: &Setup, run: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = setup.ok("run_status", json!({ "run": run }));
        if status["running"].as_array().unwrap().iter().any(|running| running["reached"] != "00:00:00") {
            return status;
        }
        assert!(Instant::now() < deadline, "{run} never got going: {status}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_cancelled_run_stops_within_moments_and_leaves_no_thread_running() {
    let setup = Setup::with_threads("cancel", 2);
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 1, "length": "3650d", "observations": 6 }));
    let running = wait_until_running(&setup, &run);
    assert_eq!(running["state"], "running");
    assert_eq!(running["running"].as_array().unwrap().len(), 2, "one observation per thread");

    let asked = Instant::now();
    let cancelled = setup.ok("cancel_run", json!({ "run": run }));
    assert!(asked.elapsed() < Duration::from_secs(10), "cancelling took {:?}", asked.elapsed());
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["running"], json!([]));
    assert_eq!(cancelled["ended"], 0);
    let stored = setup.ok("query_results", json!({ "sql": format!("SELECT state, finished IS NOT NULL FROM runs WHERE id = '{run}'") }));
    assert_eq!(stored["rows"], json!([["cancelled", 1]]));

    // Both threads are free again: a quick run finishes, and the cancelled one never resumes.
    let quick = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 2 }));
    assert_eq!(setup.finished(&quick)["state"], "done");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(setup.ok("run_status", json!({ "run": run }))["running"], json!([]));
    assert_eq!(setup.ok("cancel_run", json!({ "run": quick }))["state"], "done", "cancelling a finished run changes nothing");
}

#[test]
fn a_short_run_started_behind_a_long_one_finishes_first() {
    let setup = Setup::with_threads("turns", 1);
    write_models(&setup.folder);
    let long = setup.start(json!({ "model": "demo.json", "seed": 1, "length": "10d", "observations": 6 }));
    wait_until_running(&setup, &long);
    let short = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00:00", "observations": 1 }));
    assert_eq!(setup.ok("run_status", json!({ "run": short }))["state"], "queued");

    assert_eq!(setup.finished(&short)["state"], "done");
    let long_status = setup.ok("run_status", json!({ "run": long }));
    assert_eq!(long_status["state"], "running", "{long_status}");
    assert!(long_status["ended"].as_u64().unwrap() < 6);
    setup.ok("cancel_run", json!({ "run": long }));
}

#[test]
fn a_run_uses_the_model_as_it_was_when_it_started() {
    let setup = Setup::with_threads("snapshot", 1);
    write_models(&setup.folder);
    // Queued behind a long run, so the file changes before the run gets a thread.
    let long = setup.start(json!({ "model": "demo.json", "seed": 1, "length": "365d", "observations": 1 }));
    let run = setup.start(json!({ "model": "small.json", "seed": 5, "length": "2:00:00", "observations": 2 }));
    let changed = common::SMALL.replace(r#""process_time":"8""#, r#""process_time":"9""#);
    setup.ok("write_model", json!({ "path": "small.json", "text": changed }));
    setup.ok("cancel_run", json!({ "run": long }));
    setup.finished(&run);

    for (k, seed) in seeds_of(&setup, &run).iter().enumerate() {
        let expected = headless(common::SMALL, *seed, "2:00:00", None);
        assert_same(&setup.stored(&run, "Work")[&(k as u64 + 1)], &expected["Work"], "the model as it was");
    }
    // The stored model rebuilds the same run, and is the canonical text.
    let stored = setup.ok("query_results", json!({ "sql": format!("SELECT model_text, model_hash FROM runs WHERE id = '{run}'") }));
    let text = stored["rows"][0][0].as_str().unwrap();
    assert_eq!(text, crabsim_model::load(common::SMALL).unwrap().save());
    let rebuilt = headless(text, seeds_of(&setup, &run)[0], "2:00:00", None);
    assert_same(&setup.stored(&run, "Work")[&1], &rebuilt["Work"], "rebuilt from the store");
    assert_eq!(setup.ok("run_results", json!({ "run": run }))["model"], "small.json");
}

#[test]
fn equal_hashes_mean_the_same_model() {
    let setup = Setup::new("hashes");
    write_models(&setup.folder);
    // The same model written differently is the same canonical model.
    setup.folder.write("small-again.json", &crabsim_model::load(common::SMALL).unwrap().save());
    let input = |model: &str| json!({ "model": model, "seed": 1, "length": "1:00", "observations": 1 });
    let runs = [setup.start(input("small.json")), setup.start(input("small-again.json")), setup.start(input("demo.json"))];
    let hash = |run: &str| setup.ok("run_results", json!({ "run": run }))["model_hash"].as_str().unwrap().to_string();
    assert_eq!(hash(&runs[0]), hash(&runs[1]));
    assert_ne!(hash(&runs[0]), hash(&runs[2]));
    assert_eq!(hash(&runs[0]).len(), 64, "SHA-256 in hex");
    for run in &runs {
        setup.finished(run);
    }
}
