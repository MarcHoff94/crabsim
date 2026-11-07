//! What the catalogue's tests share: a models folder of their own, removed afterwards, and calls
//! through the catalogue as JSON, the way an agent makes them.

#![allow(dead_code)]

use crabsim_catalogue::{Catalogue, OperationError, Settings, Source};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A folder of its own under the system's temporary folder, removed when dropped.
pub struct TempFolder {
    pub path: PathBuf,
}

impl TempFolder {
    pub fn new(name: &str) -> TempFolder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let unique = format!("crabsim-catalogue-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(unique);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempFolder { path }
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.join(relative)).unwrap()
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A models folder and a catalogue on it.
pub struct Setup {
    pub folder: TempFolder,
    pub catalogue: Catalogue,
}

impl Setup {
    pub fn new(name: &str) -> Setup {
        let folder = TempFolder::new(name);
        let catalogue = Catalogue::open(Settings::new(&folder.path, Source::Mcp)).expect("an empty folder opens");
        Setup { folder, catalogue }
    }

    /// Calls an operation that must succeed.
    pub fn ok(&self, name: &str, input: Value) -> Value {
        self.catalogue.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"))
    }

    /// Calls an operation that must fail, giving its error as JSON, as an agent sees it.
    pub fn err(&self, name: &str, input: Value) -> Value {
        match self.catalogue.call(name, input.clone()) {
            Ok(output) => panic!("{name} {input} should fail, but gave {output}"),
            Err(error) => error.to_json(),
        }
    }

    pub fn error(&self, name: &str, input: Value) -> OperationError {
        self.catalogue.call(name, input).expect_err("should fail")
    }
}

/// The demo model's file text.
pub fn demo_text() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../models/demo.json")).unwrap()
}

/// A small valid model, in a form other than canonical.
pub const SMALL: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","successors":["Out"],"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}]}"#;

/// A model with three problems.
pub const BROKEN: &str = r#"{ "format_version": 1, "instances": [
    { "name": "In", "class": "Source", "successors": ["Nowhere"] },
    { "name": "Out", "class": "Drain", "attributes": { "process_time": "10" } } ] }"#;

impl Setup {
    /// A catalogue whose pool has `threads` threads.
    pub fn with_threads(name: &str, threads: usize) -> Setup {
        let folder = TempFolder::new(name);
        let mut settings = Settings::new(&folder.path, Source::Mcp);
        settings.threads = Some(threads);
        let catalogue = Catalogue::open(settings).expect("an empty folder opens");
        Setup { folder, catalogue }
    }

    /// Starts a run, giving its id.
    pub fn start(&self, input: Value) -> String {
        self.ok("start_run", input)["run"].as_str().unwrap().to_string()
    }

    /// Polls a run until it has finished, failing the test rather than hanging if it never does.
    pub fn finished(&self, run: &str) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
        loop {
            let status = self.ok("run_status", serde_json::json!({ "run": run }));
            if ["done", "failed", "cancelled"].contains(&status["state"].as_str().unwrap()) {
                return status;
            }
            assert!(std::time::Instant::now() < deadline, "{run} never finished: {status}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Each observation's figures for one object, from the store: observation, then statistic
    /// and value.
    pub fn stored(&self, run: &str, object: &str) -> std::collections::BTreeMap<u64, std::collections::BTreeMap<String, f64>> {
        let sql = format!("SELECT observation, statistic, value FROM statistics WHERE run = '{run}' AND object = '{object}'");
        let rows = self.ok("query_results", serde_json::json!({ "sql": sql }));
        let mut stored = std::collections::BTreeMap::<u64, std::collections::BTreeMap<String, f64>>::new();
        for row in rows["rows"].as_array().unwrap() {
            stored.entry(row[0].as_u64().unwrap()).or_default().insert(row[1].as_str().unwrap().to_string(), row[2].as_f64().unwrap());
        }
        stored
    }
}

/// An object's figures as an independent headless run gives them, for the statistics a test
/// compares.
pub fn headless_figures(statistics: &crabsim::ObjectStatistics, now: crabsim::SimTime) -> std::collections::BTreeMap<String, f64> {
    let mut figures = std::collections::BTreeMap::new();
    figures.insert("mus_in".to_string(), statistics.mus_in as f64);
    figures.insert("mus_out".to_string(), statistics.mus_out as f64);
    figures.insert("failures".to_string(), statistics.failures as f64);
    let measured = statistics.measured_time().as_millis() as f64;
    figures.insert("working_share".to_string(), statistics.time_busy.as_millis() as f64 / measured);
    figures.insert("blocked_share".to_string(), statistics.time_blocked.as_millis() as f64 / measured);
    figures.insert("failed_share".to_string(), statistics.time_failed.as_millis() as f64 / measured);
    if let Some(rate) = statistics.mus_per_hour(now) {
        figures.insert("throughput_per_hour".to_string(), rate);
    }
    if let Some(lead) = statistics.lead_time_mean() {
        figures.insert("mean_lead_time_s".to_string(), lead.as_secs_f64());
    }
    figures
}

/// The documented seed of observation `k`, written out again here: splitmix64(seed + k).
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// The models the job tests run, written into a models folder.
pub fn write_models(folder: &TempFolder) {
    folder.write("demo.json", &demo_text());
    folder.write("small.json", SMALL);
    folder.write("loop.json", LOOP);
    folder.write("long-line.json", &long_line());
}

/// MUs pass a Gate that often fails, and an MU through it reaches two dummy Stations that pass
/// it back and forth for ever without time moving: a zero-time loop, which is a run error. So an
/// observation fails unless the Gate fails before its first MU is through, which depends on the
/// seed.
pub const LOOP: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Gate"],"attributes":{"interval":"10"}},
  {"name":"Gate","class":"Station","successors":["A"],"attributes":{"process_time":"10",
     "failures":{"availability":0.2,"repair_time":{"erlang":{"mean":"1:00","k":1}}}}},
  {"name":"A","class":"Station","successors":["B"]},
  {"name":"B","class":"Station","successors":["A"]}]}"#;

/// A line of 16 Stations before a slow one, with failures on 12 of them, so more than 10 are
/// blocked and more than 10 fail, and a class of the model's own.
pub fn long_line() -> String {
    let mut instances = vec![r#"{"name":"In","class":"Source","successors":["S1"],"attributes":{"interval":"2"}}"#.to_string()];
    for n in 1..=16 {
        let next = if n == 16 { "Slow".to_string() } else { format!("S{}", n + 1) };
        let class = if n <= 12 { "Fragile" } else { "Station" };
        instances.push(format!(r#"{{"name":"S{n}","class":"{class}","successors":["{next}"],"attributes":{{"process_time":"{}"}}}}"#, 1 + n % 3));
    }
    instances.push(r#"{"name":"Slow","class":"Station","successors":["Out"],"attributes":{"process_time":"9"}}"#.to_string());
    instances.push(r#"{"name":"Out","class":"Drain"}"#.to_string());
    format!(
        r#"{{"format_version":1,"classes":[{{"name":"Fragile","parent":"Station","attributes":{{"failures":{{"availability":0.8,"repair_time":{{"erlang":{{"mean":"1:00","k":2}}}}}}}}}}],"instances":[{}]}}"#,
        instances.join(",")
    )
}
