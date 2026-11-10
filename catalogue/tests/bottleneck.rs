//! The likely bottleneck: the rule on its own, on shares of time, and `find_bottleneck` on a run in
//! the results store, called as JSON the way an agent calls it.

mod common;

use common::Setup;
use crabsim_catalogue::bottleneck::{likely_bottleneck, Shares};
use serde_json::json;

fn station(name: &str, working: f64, blocked: f64, idle: f64, failed: f64) -> Shares {
    Shares { name: name.into(), built_in: "Station".into(), working, blocked, idle, failed, paused: 0.0 }
}

fn connections(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|&(from, to)| (from.to_string(), to.to_string())).collect()
}

#[test]
fn the_likely_bottleneck_is_the_station_busy_or_stopped_most_with_its_neighbours_as_evidence() {
    let objects = vec![
        Shares { name: "In".into(), built_in: "Source".into(), working: 0.2, blocked: 0.8, idle: 0.0, failed: 0.0, paused: 0.0 },
        station("Saw", 0.30, 0.65, 0.05, 0.0),
        // Less working than Saw, but failed much of the time: what limits the line.
        station("Press", 0.60, 0.0, 0.05, 0.35),
        station("Pack", 0.25, 0.0, 0.75, 0.0),
        Shares { name: "Out".into(), built_in: "Drain".into(), working: 0.0, blocked: 0.0, idle: 1.0, failed: 0.0, paused: 0.0 },
    ];
    let edges = connections(&[("In", "Saw"), ("Saw", "Press"), ("Press", "Pack"), ("Pack", "Out")]);
    let found = likely_bottleneck(&objects, &edges).expect("Stations ran");
    assert_eq!(found.instance, "Press");
    assert!((found.not_idle_share - 0.95).abs() < 1e-9, "busy plus stopped: {}", found.not_idle_share);
    assert_eq!(found.upstream, vec![("Saw".to_string(), 0.65)], "upstream mostly blocked");
    assert_eq!(found.downstream, vec![("Pack".to_string(), 0.75)], "downstream mostly starved");
    let edges = connections(&[("In", "Saw"), ("Saw", "Press"), ("Press", "Pack"), ("Press", "Out"), ("Pack", "Out")]);
    let found = likely_bottleneck(&objects, &edges).unwrap();
    assert_eq!(found.downstream, vec![("Pack".to_string(), 0.75)], "a Drain is always idle, so it says nothing");
    let ranking: Vec<&str> = found.ranking.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(ranking, ["Press", "Saw", "Pack"], "Stations only, most busy or stopped first, the Source left out");
}

#[test]
fn without_stations_there_is_no_likely_bottleneck() {
    let objects = vec![Shares { name: "In".into(), built_in: "Source".into(), working: 1.0, blocked: 0.0, idle: 0.0, failed: 0.0, paused: 0.0 }];
    assert!(likely_bottleneck(&objects, &[]).is_none());
}

/// Fast, then Slow taking the longest, then After: Slow limits the line.
const LINE: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Fast"],"attributes":{"interval":"5"}},
  {"name":"Fast","class":"Station","successors":["Slow"],"attributes":{"process_time":"4"}},
  {"name":"Slow","class":"Station","successors":["After"],"attributes":{"process_time":"9"}},
  {"name":"After","class":"Station","successors":["Out"],"attributes":{"process_time":"2"}},
  {"name":"Out","class":"Drain"}]}"#;

#[test]
fn find_bottleneck_names_it_for_a_run_from_the_means_over_its_observations() {
    let setup = Setup::new("bottleneck-run");
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    let run = setup.start(json!({ "model": "line.json", "seed": 1, "length": "8:00:00", "observations": 3 }));
    setup.finished(&run);
    let found = setup.ok("find_bottleneck", json!({ "run": run }));
    common::assert_fits_its_schema(&setup.catalogue, "find_bottleneck", &found);
    assert_eq!(found["run"], run);
    assert_eq!(found["likely"]["instance"], "Slow", "{found:#}");
    assert!(found["likely"]["not_idle_share"].as_f64().unwrap() > 0.99, "{found:#}");
    assert_eq!(found["likely"]["upstream"][0]["instance"], "Fast");
    assert!(found["likely"]["upstream"][0]["blocked_share"].as_f64().unwrap() > 0.5, "Fast waits for Slow: {found:#}");
    assert_eq!(found["likely"]["downstream"][0]["instance"], "After");
    assert!(found["likely"]["downstream"][0]["idle_share"].as_f64().unwrap() > 0.5, "After waits for MUs: {found:#}");
    assert_eq!(found["ranking"][0]["instance"], "Slow");
    assert!(found["rule"].as_str().unwrap().contains("likely"));

    assert_eq!(setup.err("find_bottleneck", json!({ "run": "run-99" }))["kind"], "not_found");
}
