//! Runs whose process ended before they did: found through the lock file each crabsim process
//! holds while it lives, and marked interrupted, keeping what they finished.

mod common;

use common::{write_models, Setup, TempFolder};
use crabsim_catalogue::{Catalogue, Settings, Source};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::fs::File;
use std::path::Path;

/// The results store as the first version of the tables wrote it, before runs knew their process.
const TABLES_VERSION_1: &str = "
CREATE TABLE runs (
    number INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT GENERATED ALWAYS AS ('run-' || number) VIRTUAL UNIQUE,
    model_path TEXT NOT NULL, model_text TEXT NOT NULL, model_hash TEXT NOT NULL, seed TEXT NOT NULL,
    length_s REAL NOT NULL, warm_up_s REAL NOT NULL, observations INTEGER NOT NULL, label TEXT,
    source TEXT NOT NULL, crabsim_version TEXT NOT NULL, started TEXT NOT NULL, finished TEXT, state TEXT NOT NULL);
CREATE TABLE observations (run TEXT NOT NULL, number INTEGER NOT NULL, seed TEXT NOT NULL, events INTEGER NOT NULL,
    wall_time_s REAL NOT NULL, error TEXT, error_time_s REAL, PRIMARY KEY (run, number));
CREATE TABLE statistics (run TEXT NOT NULL, observation INTEGER NOT NULL, object TEXT NOT NULL, class TEXT NOT NULL,
    built_in_class TEXT NOT NULL, statistic TEXT NOT NULL, value REAL NOT NULL);
PRAGMA user_version = 1;
";

/// Adds a run that is still running, with one of its two observations ended, as a process that
/// then died would have left it.
fn add_stuck_run(store: &Connection, process: Option<&str>) -> String {
    let text = crabsim_model::load(common::SMALL).unwrap().save();
    let columns = if process.is_some() { ", process" } else { "" };
    let values = if process.is_some() { ", ?2" } else { "" };
    let sql = format!(
        "INSERT INTO runs (model_path, model_text, model_hash, seed, length_s, warm_up_s, observations, source, crabsim_version, started, state{columns})
         VALUES ('small.json', ?1, 'hash', '1', 3600, 0, 2, 'mcp', '0.1.0', '2026-11-07T10:00:00Z', 'running'{values})"
    );
    match process {
        Some(process) => store.execute(&sql, params![text, process]).unwrap(),
        None => store.execute(&sql, params![text]).unwrap(),
    };
    let id = format!("run-{}", store.last_insert_rowid());
    store.execute("INSERT INTO observations (run, number, seed, events, wall_time_s) VALUES (?1, 1, '5', 100, 0.1)", [&id]).unwrap();
    store
        .execute("INSERT INTO statistics (run, observation, object, class, built_in_class, statistic, value) VALUES (?1, 1, 'Out', 'Drain', 'Drain', 'mus_in', 42)", [&id])
        .unwrap();
    id
}

fn open(folder: &Path) -> Setup {
    let catalogue = Catalogue::open(Settings::new(folder, Source::Cli)).unwrap();
    Setup { folder: TempFolder { path: folder.to_path_buf() }, catalogue }
}

fn status(setup: &Setup, run: &str) -> Value {
    setup.ok("run_status", json!({ "run": run }))
}

#[test]
fn a_run_left_by_a_store_of_the_first_version_is_interrupted_and_keeps_what_it_finished() {
    let folder = TempFolder::new("version-1");
    write_models(&folder);
    std::fs::create_dir_all(folder.join(".crabsim")).unwrap();
    let store = Connection::open(folder.join(".crabsim/results.sqlite")).unwrap();
    store.execute_batch(TABLES_VERSION_1).unwrap();
    let stuck = add_stuck_run(&store, None);
    drop(store);
    let path = folder.path.clone();
    std::mem::forget(folder);

    let setup = open(&path);
    assert_eq!(status(&setup, &stuck), json!({ "run": stuck, "state": "interrupted", "observations": 2, "ended": 1, "failed": 0, "running": [], "here": false }));
    let results = setup.ok("run_results", json!({ "run": stuck, "values": true }));
    assert_eq!(results["state"], "interrupted");
    assert!(results["finished"].is_string(), "{results}");
    assert_eq!(results["objects"][0]["figures"]["mus_in"]["values"], json!([42.0, null]));
    let columns = setup.ok("query_results", json!({ "sql": "SELECT name FROM pragma_table_info('runs') WHERE name = 'process'" }));
    assert_eq!(columns["rows"], json!([["process"]]), "the tables were upgraded");
    assert_eq!(setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00", "observations": 1 })), "run-2");
    assert_eq!(setup.ok("cancel_run", json!({ "run": stuck }))["state"], "interrupted", "cancelling it changes nothing");
    setup.ok("forget_run", json!({ "run": stuck }));
}

#[test]
fn a_run_whose_process_holds_its_lock_stays_running_and_one_whose_lock_is_free_is_interrupted() {
    let setup = Setup::new("locks");
    write_models(&setup.folder);
    // The store exists, with this catalogue's own lock.
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    let processes = setup.folder.join(".crabsim/processes");
    let alive = File::create(processes.join("alive-1.lock")).unwrap();
    alive.try_lock().expect("the lock is free");
    File::create(processes.join("dead-1.lock")).unwrap();
    let store = Connection::open(setup.folder.join(".crabsim/results.sqlite")).unwrap();
    let living = add_stuck_run(&store, Some("alive-1"));
    let dead = add_stuck_run(&store, Some("dead-1"));
    let gone = add_stuck_run(&store, Some("gone-1"));

    assert_eq!(status(&setup, &living)["state"], "running");
    assert_eq!(setup.err("cancel_run", json!({ "run": living }))["kind"], "invalid_input", "only its own process can cancel it");
    assert_eq!(setup.err("forget_run", json!({ "run": living }))["kind"], "invalid_input", "nor forget it while it runs");
    assert_eq!(status(&setup, &dead)["state"], "interrupted");
    assert_eq!(status(&setup, &gone)["state"], "interrupted", "a process with no lock file is gone");
    assert!(!processes.join("dead-1.lock").exists(), "a dead process's lock file is tidied away");
    assert!(processes.join("alive-1.lock").exists());

    drop(alive);
    assert_eq!(status(&setup, &living)["state"], "interrupted", "once its process ends");
}

#[test]
fn another_catalogues_running_run_isnt_interrupted() {
    let setup = Setup::with_threads("two-catalogues", 1);
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "demo.json", "seed": 1, "length": "3650d", "observations": 1 }));
    let other = Catalogue::open(Settings::new(&setup.folder.path, Source::Cli)).unwrap();
    let seen = other.call("run_status", json!({ "run": run })).unwrap();
    assert_eq!(seen["here"], false);
    assert!(["queued", "running"].contains(&seen["state"].as_str().unwrap()), "{seen}");
    drop(other);
    assert_eq!(setup.ok("cancel_run", json!({ "run": run }))["state"], "cancelled");
}

#[test]
fn every_new_run_records_the_process_running_it() {
    let setup = Setup::new("records");
    write_models(&setup.folder);
    let run = setup.start(json!({ "model": "small.json", "seed": 1, "length": "1:00", "observations": 1 }));
    setup.finished(&run);
    let process = setup.ok("query_results", json!({ "sql": "SELECT process FROM runs" }))["rows"][0][0].as_str().unwrap().to_string();
    let id = setup.folder.join(&format!(".crabsim/processes/{process}.lock"));
    assert!(id.is_file());
    let Setup { folder, catalogue } = setup;
    drop(catalogue);
    assert!(!id.exists(), "a catalogue removes its lock file when it ends");
    drop(folder);
}

#[test]
fn lock_files_of_ended_processes_are_tidied_away_when_the_store_opens() {
    let setup = Setup::new("tidy");
    let processes = setup.folder.join(".crabsim/processes");
    std::fs::create_dir_all(&processes).unwrap();
    File::create(processes.join("ended-1.lock")).unwrap();
    let alive = File::create(processes.join("alive-1.lock")).unwrap();
    alive.try_lock().unwrap();
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    assert!(!processes.join("ended-1.lock").exists());
    assert!(processes.join("alive-1.lock").exists());
}
