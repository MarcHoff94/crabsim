//! The headless CLI, run as its binary: the demo model with a seed and a simulated length, then
//! each object's final statistics and how long the run took, and with a models folder, experiments
//! kept in the results store.

use crabsim::time::parse_time;
use crabsim::{demo, SimTime};
use std::process::{Command, Output};

fn crabsim(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crabsim")).args(args).output().expect("the binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("UTF-8 output")
}

/// The statistics the CLI prints, without the line naming the seed, and the line saying how long
/// the run took, which changes every time.
fn results(output: &Output) -> String {
    let printed = stdout(output);
    let results: Vec<&str> = printed.lines().filter(|line| !line.starts_with("crabsim:") && !line.starts_with("Ran ")).collect();
    assert!(results.len() > 10, "{printed}");
    results.join("\n")
}

#[test]
fn it_runs_the_demo_headless_and_prints_each_objects_statistics() {
    let output = crabsim(&["--seed", "3", "--length", "2:00:00"]);

    assert!(output.status.success(), "{output:?}");
    let printed = stdout(&output);
    let model = demo::model();
    for id in 0..model.object_count() {
        let name = model.name(id).expect("every demo object has a name");
        assert!(printed.lines().any(|line| line.contains(name)), "no row for {name}:\n{printed}");
    }
    let mut run = model.build(3);
    run.init().unwrap();
    run.advance_to(parse_time("2:00:00").unwrap()).unwrap();
    let drain = run.state().objects.last().unwrap().statistics;
    assert!(drain.mus_in > 0);
    assert!(printed.contains(&format!("Drain: {} MUs", drain.mus_in)), "{printed}");
    assert!(printed.contains("seed 3"), "{printed}");
    assert!(printed.lines().any(|line| line.starts_with("Ran ") && line.contains("events")), "{printed}");
}

#[test]
fn the_same_seed_prints_the_same_results_and_another_seed_others() {
    let first = crabsim(&["--seed", "5", "--length", "4:00:00"]);
    let again = crabsim(&["--seed", "5", "--length", "4:00:00"]);
    let other = crabsim(&["--seed", "6", "--length", "4:00:00"]);

    assert_eq!(results(&first), results(&again));
    assert_ne!(results(&first), results(&other));
}

#[test]
fn the_length_is_written_as_the_ui_shows_time() {
    let output = crabsim(&["--length", "1d 00:00:30"]);

    assert!(output.status.success(), "{output:?}");
    let printed = stdout(&output);
    assert!(printed.contains("1d 00:00:30"), "{printed}");
    assert_eq!(parse_time("1d 00:00:30"), Some(SimTime::from_secs(86_430)));
}

#[test]
fn a_length_that_isnt_a_time_is_refused() {
    let output = crabsim(&["--length", "soon"]);

    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("soon"), "{error}");
}

fn demo_file() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../models/demo.json").to_string()
}

#[test]
fn the_demo_as_a_model_file_gives_the_same_results_as_the_built_in_demo() {
    let from_file = crabsim(&["--model", &demo_file(), "--seed", "4", "--length", "4:00:00"]);
    let built_in = crabsim(&["--seed", "4", "--length", "4:00:00"]);

    assert!(from_file.status.success(), "{from_file:?}");
    assert!(stdout(&from_file).starts_with("crabsim: "), "the header names the file");
    assert!(stdout(&from_file).contains("demo.json"));
    assert_eq!(results(&from_file), results(&built_in));
}

#[test]
fn an_invalid_model_file_prints_every_problem_and_runs_nothing() {
    let path = std::env::temp_dir().join(format!("crabsim-cli-invalid-{}.json", std::process::id()));
    let text = r#"{ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["Nowhere"] },
        { "name": "Out", "class": "Drain", "attributes": { "process_time": "10" } } ] }"#;
    std::fs::write(&path, text).unwrap();
    let output = crabsim(&["--model", path.to_str().unwrap()]);
    std::fs::remove_file(&path).unwrap();

    assert!(!output.status.success());
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(errors.contains("/instances/0 \"In\": a Source needs an `interval`"), "{errors}");
    assert!(errors.contains("/instances/0/successors/0 \"In\": no instance is called `Nowhere`"), "{errors}");
    assert!(errors.contains("/instances/1/attributes/process_time \"Out\": a Drain has no attribute `process_time`"), "{errors}");
    assert!(!stdout(&output).contains("Ran "), "nothing ran");
}

#[test]
fn a_model_file_that_cant_be_read_is_refused() {
    let output = crabsim(&["--model", "no/such/model.json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("can't be read"));
}

/// A models folder of its own holding the demo model, removed afterwards.
struct Models(std::path::PathBuf);

impl Models {
    fn new(name: &str) -> Models {
        let path = std::env::temp_dir().join(format!("crabsim-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::copy(demo_file(), path.join("demo.json")).unwrap();
        Models(path)
    }

    fn arg(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Models {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn run_runs_an_experiment_prints_its_results_and_keeps_them() {
    let models = Models::new("run");
    let output = crabsim(&["run", "--models", models.arg(), "--model", "demo.json", "--seed", "4", "--length", "2:00:00", "--observations", "3", "--label", "baseline"]);
    assert!(output.status.success(), "{output:?}");
    let printed = stdout(&output);
    assert!(printed.starts_with("run-1 \"baseline\": demo.json, seed 4, 3 observations of 02:00:00"), "{printed}");
    assert!(printed.contains("Drain (Drain, Drain): all"), "a model this small is shown whole: {printed}");
    let lead_time = printed.lines().find(|line| line.starts_with("mean_lead_time_s")).expect("lead time is shown");
    assert!(lead_time.contains("00:0"), "as a time, as the UI shows it: {lead_time}");
    assert!(printed.contains("throughput_per_hour"), "{printed}");
    assert!(printed.contains("3 observations ended, none failed:"), "{printed}");
    assert!(!printed.contains("Observation"), "no table of observations by default: {printed}");
    assert!(models.0.join(".crabsim/results.sqlite").is_file());

    let detailed = stdout(&crabsim(&["results", "--models", models.arg(), "run-1", "--values"]));
    let table: Vec<&str> = detailed.lines().skip_while(|line| !line.starts_with("Observation")).take(4).collect();
    assert_eq!(table.len(), 4, "a header and 3 observations: {detailed}");

    // Each observation's reported seed reproduces it as a single run, also from the CLI.
    let query = crabsim(&["query", "--models", models.arg(), "SELECT o.seed, s.value FROM observations o JOIN statistics s ON s.run = o.run AND s.observation = o.number WHERE s.object = 'Drain' AND s.statistic = 'mus_in' ORDER BY o.number"]);
    assert!(query.status.success(), "{query:?}");
    let rows: Vec<Vec<String>> = stdout(&query).lines().skip(1).filter(|line| !line.ends_with("rows")).map(|line| line.split_whitespace().map(String::from).collect()).collect();
    assert_eq!(rows.len(), 3, "{}", stdout(&query));
    for row in rows {
        let single = crabsim(&["--model", &demo_file(), "--seed", &row[0], "--length", "2:00:00"]);
        let mus: f64 = row[1].parse().unwrap();
        assert!(stdout(&single).contains(&format!("Drain: {mus} MUs")), "seed {}: {}", row[0], stdout(&single));
    }
}

#[test]
fn runs_lists_the_runs_newest_first_and_results_prints_one() {
    let models = Models::new("runs");
    for label in ["first", "second"] {
        let output = crabsim(&["run", "--models", models.arg(), "--model", "demo.json", "--length", "1:00:00", "--label", label]);
        assert!(output.status.success(), "{output:?}");
    }
    let listed = stdout(&crabsim(&["runs", "--models", models.arg()]));
    let lines: Vec<&str> = listed.lines().collect();
    assert!(lines[0].starts_with("Run"), "{listed}");
    assert!(lines[1].starts_with("run-2  second"), "{listed}");
    assert!(lines[2].starts_with("run-1  first"), "{listed}");
    assert!(lines[1].contains("cli") && lines[1].contains("done"), "{listed}");

    let results = crabsim(&["results", "--models", models.arg(), "run-1", "--object", "Station 1"]);
    assert!(results.status.success(), "{results:?}");
    let printed = stdout(&results);
    assert!(printed.starts_with("run-1 \"first\": demo.json, seed 0, 10 observations"), "the defaults: {printed}");
    assert!(printed.contains("Station 1 (Demo station, Station): chosen"), "{printed}");
    assert!(!printed.contains("Drain (Drain"), "only what was asked for: {printed}");

    let missing = crabsim(&["results", "--models", models.arg(), "run-9"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("run-9"));
}

#[test]
fn query_refuses_to_change_the_store() {
    let models = Models::new("query");
    let output = crabsim(&["query", "--models", models.arg(), "DELETE FROM runs"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SELECT"));
    let output = crabsim(&["query", "--models", models.arg(), "SELECT 1 AS one"]);
    assert_eq!(stdout(&output), "one\n  1\n1 row\n");
}

#[test]
fn a_folder_holding_source_code_is_refused_as_a_models_folder() {
    let repository = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let output = crabsim(&["runs", "--models", repository]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("source code"));
    assert!(!std::path::Path::new(repository).join(".crabsim").exists(), "nothing was written");
}

#[test]
fn an_invalid_model_is_refused_by_run_with_every_problem() {
    let models = Models::new("invalid");
    std::fs::write(models.0.join("broken.json"), r#"{ "format_version": 1, "instances": [ { "name": "In", "class": "Source", "successors": ["Nowhere"] } ] }"#).unwrap();
    let output = crabsim(&["run", "--models", models.arg(), "--model", "broken.json"]);
    assert!(!output.status.success());
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(errors.contains("/instances/0 \"In\": a Source needs an `interval`"), "{errors}");
    assert!(errors.contains("/instances/0/successors/0 \"In\": no instance is called `Nowhere`"), "{errors}");
}

#[test]
fn a_run_whose_process_was_killed_is_interrupted() {
    let models = Models::new("killed");
    // Killed when dropped, so a failing assertion never leaves a run of a hundred years going.
    struct Killed(std::process::Child);
    impl Drop for Killed {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Killed(
        Command::new(env!("CARGO_BIN_EXE_crabsim"))
        .args(["run", "--models", models.arg(), "--model", "demo.json", "--length", "36500d", "--observations", "2"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let catalogue = || crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&models.0, crabsim_catalogue::Source::Cli)).unwrap();
    let status = || catalogue().call("run_status", serde_json::json!({ "run": "run-1" }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !status().is_ok_and(|status| status["state"] == "running") {
        assert!(std::time::Instant::now() < deadline, "the run never started");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(status().unwrap()["state"], "running", "its process is alive");

    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let status = status().unwrap();
    assert_eq!(status["state"], "interrupted", "{status}");
    let runs = stdout(&crabsim(&["runs", "--models", models.arg()]));
    assert!(runs.lines().nth(1).unwrap().contains("interrupted"), "{runs}");
}
