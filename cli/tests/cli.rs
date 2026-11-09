//! The headless CLI, run as its binary: a model file with a seed and a simulated length, then
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
fn it_runs_a_model_file_headless_and_prints_each_objects_statistics() {
    let output = crabsim(&["--model", &demo_file(), "--seed", "3", "--length", "2:00:00"]);

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
    let first = crabsim(&["--model", &demo_file(), "--seed", "5", "--length", "4:00:00"]);
    let again = crabsim(&["--model", &demo_file(), "--seed", "5", "--length", "4:00:00"]);
    let other = crabsim(&["--model", &demo_file(), "--seed", "6", "--length", "4:00:00"]);

    assert_eq!(results(&first), results(&again));
    assert_ne!(results(&first), results(&other));
}

#[test]
fn the_length_is_written_as_the_ui_shows_time() {
    let output = crabsim(&["--model", &demo_file(), "--length", "1d 00:00:30"]);

    assert!(output.status.success(), "{output:?}");
    let printed = stdout(&output);
    assert!(printed.contains("1d 00:00:30"), "{printed}");
    assert_eq!(parse_time("1d 00:00:30"), Some(SimTime::from_secs(86_430)));
}

#[test]
fn a_length_that_isnt_a_time_is_refused() {
    let output = crabsim(&["--model", &demo_file(), "--length", "soon"]);

    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("soon"), "{error}");
}

fn demo_file() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../models/demo.json").to_string()
}

#[test]
fn a_single_run_needs_a_model_file() {
    let output = crabsim(&["--seed", "4", "--length", "4:00:00"]);
    assert!(!output.status.success(), "there is no built-in model to run any more");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--model"), "{}", String::from_utf8_lossy(&output.stderr));

    let from_file = crabsim(&["--model", &demo_file(), "--seed", "4", "--length", "4:00:00"]);
    assert!(from_file.status.success(), "{from_file:?}");
    assert!(stdout(&from_file).starts_with("crabsim: "), "the header names the file");
    assert!(stdout(&from_file).contains("demo.json"));
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

// ---------- Changing models, as the user ----------

fn crabsim_with_stdin(args: &[&str], input: &str) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_crabsim"))
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("the binary runs");
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// An agent on the folder: a catalogue opened as the MCP server is, alive while it's kept.
fn agent(models: &Models) -> crabsim_catalogue::Catalogue {
    let agent = crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&models.0, crabsim_catalogue::Source::Mcp)).unwrap();
    agent.set_client_name("test-agent");
    agent
}

const SLOWER: &str = r#"[{ "change": "set_attribute", "target": { "instance": "Station 1" }, "attribute": "process_time", "value": "0:11" }]"#;

#[test]
fn edit_needs_a_file_adopted_then_applies_changes_from_a_file_or_stdin() {
    let models = Models::new("edit");
    let refused = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-"], SLOWER);
    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("adopt"), "the copied demo was never saved by crabsim: {}", stderr(&refused));

    let adopted = crabsim(&["adopt", "--models", models.arg(), "demo.json"]);
    assert!(adopted.status.success(), "{adopted:?}");
    assert!(stdout(&adopted).contains("demo.json adopted"), "{}", stdout(&adopted));

    let changes = models.0.join("slower.changes");
    std::fs::write(&changes, SLOWER).unwrap();
    let edited = crabsim(&["edit", "--models", models.arg(), "demo.json", changes.to_str().unwrap()]);
    assert!(edited.status.success(), "{edited:?}");
    assert!(stdout(&edited).contains("1 change applied"), "{}", stdout(&edited));
    assert!(std::fs::read_to_string(models.0.join("demo.json")).unwrap().contains("00:00:11"));

    let from_stdin = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-"], r#"[{ "change": "move_instance", "name": "Drain", "position": { "x": 40, "y": 0 } }]"#);
    assert!(from_stdin.status.success(), "{from_stdin:?}");

    let stale = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-", "--expected-hash", "0000"], SLOWER);
    assert!(!stale.status.success());
    assert!(stderr(&stale).contains("0000"), "{}", stderr(&stale));

    let bad = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-"], r#"[{ "change": "remove_instance", "name": "Nobody" }]"#);
    assert!(!bad.status.success());
    assert!(stderr(&bad).contains("changes/0"), "{}", stderr(&bad));
}

#[test]
fn attributes_prints_each_value_and_where_it_comes_from() {
    let models = Models::new("attributes");
    let printed = stdout(&crabsim(&["attributes", "--models", models.arg(), "demo.json", "Station 1"]));
    assert!(printed.contains("Station 1 (Demo station, Station)"), "{printed}");
    let line = |name: &str| printed.lines().find(|line| line.starts_with(name)).unwrap_or_else(|| panic!("no {name} in {printed}")).to_string();
    assert!(line("process_time").contains("instance"), "{printed}");
    assert!(line("failures").contains("class Demo station"), "{printed}");
    assert!(line("starting_mus").contains("built in"), "{printed}");
    assert!(line("description").contains("class Demo station"), "{printed}");
    let source = stdout(&crabsim(&["attributes", "--models", models.arg(), "demo.json", "Source", "--class"]));
    assert!(source.lines().any(|line| line.starts_with("interval") && line.contains("not set")), "a built-in Source has no interval: {source}");
    assert!(printed.contains("changed outside crabsim"), "the copied demo isn't adopted: {printed}");

    let class = stdout(&crabsim(&["attributes", "--models", models.arg(), "demo.json", "Demo station", "--class"]));
    assert!(class.lines().any(|line| line.starts_with("failures") && line.contains("class Demo station")), "{class}");
    let missing = crabsim(&["attributes", "--models", models.arg(), "demo.json", "Nobody"]);
    assert!(!missing.status.success());
}

#[test]
fn the_user_sees_and_manages_the_agents_claims() {
    let models = Models::new("editors");
    assert!(crabsim(&["adopt", "--models", models.arg(), "demo.json"]).status.success());
    let first = agent(&models);
    let second = agent(&models);
    first.call("claim_model", serde_json::json!({ "path": "demo.json", "goal": "a faster line", "doing": "making Station 4 faster" })).unwrap();
    let in_use = second.call("claim_model", serde_json::json!({ "path": "demo.json", "goal": "x", "doing": "checking the drain" })).unwrap_err().to_json();
    assert_eq!(in_use["kind"], "model_in_use");
    let id = |agent: &crabsim_catalogue::Catalogue| agent.call("get_editors", serde_json::json!({})).unwrap()["you"].as_str().unwrap().to_string();
    let (first_id, second_id) = (id(&first), id(&second));
    let editor = || first.call("get_editors", serde_json::json!({ "path": "demo.json" })).unwrap()["files"][0]["editor"].clone();

    let shown = stdout(&crabsim(&["editors", "show", "--models", models.arg(), "demo.json"]));
    for part in [first_id.as_str(), "test-agent", "a faster line", "making Station 4 faster"] {
        assert!(shown.contains(part), "{part} in {shown}");
    }

    let refused = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-"], SLOWER);
    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("--take-over"), "{}", stderr(&refused));
    let lent = crabsim_with_stdin(&["edit", "--models", models.arg(), "demo.json", "-", "--take-over"], SLOWER);
    assert!(lent.status.success(), "{lent:?}");
    assert_eq!(editor(), first_id.as_str(), "a take-over is for one command: the agent keeps its claim");
    let change = serde_json::json!({ "path": "demo.json", "changes": [{ "change": "set_model_description", "description": "agent" }] });
    assert_eq!(first.call("edit_model", change.clone()).unwrap_err().to_json()["kind"], "model_changed", "the agent reads the model again first");
    first.call("edit_model", change).unwrap();

    assert!(crabsim(&["editors", "revoke", "--models", models.arg(), "demo.json"]).status.success());
    assert_eq!(first.call("get_editors", serde_json::json!({ "path": "demo.json" })).unwrap()["files"], serde_json::json!([]));
    assert!(!crabsim(&["editors", "revoke", "--models", models.arg(), "demo.json"]).status.success(), "nobody holds it now");
    second.call("claim_model", serde_json::json!({ "path": "demo.json", "goal": "a checked drain", "doing": "checking the drain" })).unwrap();

    assert!(crabsim(&["editors", "block", "--models", models.arg(), &second_id]).status.success());
    let blocked = second.call("claim_model", serde_json::json!({ "path": "demo.json", "goal": "x", "doing": "y" })).unwrap_err().to_json();
    assert_eq!(blocked["kind"], "blocked");
    let everything = stdout(&crabsim(&["editors", "show", "--models", models.arg()]));
    assert!(everything.contains("Blocked") && everything.contains(&second_id), "every file, and the blocked agents: {everything}");
    assert!(crabsim(&["editors", "unblock", "--models", models.arg(), &second_id]).status.success());
    assert!(!crabsim(&["queue", "show", "--models", models.arg()]).status.success(), "`queue` went with the edit queue");
}

#[test]
fn compare_prints_each_figures_difference_and_whether_it_is_clear() {
    let models = Models::new("compare");
    for (label, seed) in [("one", "1"), ("two", "2"), ("again", "1")] {
        let output = crabsim(&["run", "--models", models.arg(), "--model", "demo.json", "--length", "2:00:00", "--observations", "3", "--seed", seed, "--label", label]);
        assert!(output.status.success(), "{output:?}");
    }
    let output = crabsim(&["compare", "--models", models.arg(), "run-1", "run-2", "--object", "Drain"]);
    assert!(output.status.success(), "{output:?}");
    let printed = stdout(&output);
    assert!(printed.starts_with("run-2 \"two\" minus run-1 \"one\": not paired, because the runs used different seeds"), "{printed}");
    let mus_in = printed.lines().find(|line| line.trim_start().starts_with("mus_in")).unwrap_or_else(|| panic!("{printed}"));
    assert!(mus_in.contains("welch") && (mus_in.contains("yes") || mus_in.contains("no")), "{mus_in}");

    let same = stdout(&crabsim(&["compare", "--models", models.arg(), "run-1", "run-3", "--object", "Drain"]));
    assert!(same.contains(": paired") && same.contains("Unchanged in every observation: Drain"), "the same seed twice: {same}");
    let missing = crabsim(&["compare", "--models", models.arg(), "run-1", "run-9"]);
    assert!(!missing.status.success());
}

// ---------- describe, graph and export ----------

/// Runs the CLI on a models folder, which must succeed, giving what it printed.
fn ok(args: &[&str]) -> String {
    let output = crabsim(args);
    assert!(output.status.success(), "{args:?} failed: {}", stderr(&output));
    stdout(&output)
}

#[test]
fn describe_prints_a_models_summary_from_a_file_or_a_run() {
    let models = Models::new("describe");
    let printed = ok(&["describe", "--models", models.arg(), "--model", "demo.json"]);
    assert!(printed.contains("14 instances"), "{printed}");
    assert!(printed.contains("The demo model: a Source"), "the model's description: {printed}");
    assert!(printed.lines().any(|line| line.contains("Demo station") && line.contains("12")), "instances per class: {printed}");
    assert!(printed.contains("Sources (1): Source"), "{printed}");
    assert!(printed.contains("Drains (1): Drain"), "{printed}");
    assert!(printed.contains("Station 4 (3 successors)"), "the widest branch point: {printed}");
    assert!(printed.contains("Source reaches Drain past 4 to 6 objects"), "{printed}");
    assert!(printed.contains("No loops"), "{printed}");
    assert!(printed.contains("No problems"), "{printed}");

    ok(&["run", "--models", models.arg(), "--model", "demo.json", "--length", "1:00", "--observations", "1"]);
    let from_run = ok(&["describe", "--models", models.arg(), "--run", "run-1"]);
    assert!(from_run.contains("14 instances"), "{from_run}");

    let neither = crabsim(&["describe", "--models", models.arg()]);
    assert!(!neither.status.success());
    assert!(stderr(&neither).contains("either"), "{}", stderr(&neither));
}

#[test]
fn describe_prints_the_problems_of_a_model_with_problems() {
    let models = Models::new("describe-broken");
    std::fs::write(models.0.join("broken.json"), r#"{ "format_version": 1, "instances": [ { "name": "In", "class": "Source", "successors": ["Nowhere"] } ] }"#).unwrap();
    let printed = ok(&["describe", "--models", models.arg(), "--model", "broken.json"]);
    assert!(printed.contains("2 problems"), "{printed}");
    assert!(printed.contains("Nowhere"), "{printed}");
}

#[test]
fn graph_answers_each_kind_of_question() {
    let models = Models::new("graph");
    let graph = |args: &[&str]| {
        let mut all = vec!["graph"];
        all.extend_from_slice(&args[..1]);
        all.extend(["--models", models.arg(), "--model", "demo.json"]);
        all.extend_from_slice(&args[1..]);
        ok(&all)
    };

    let neighbours = graph(&["neighbours", "Station 4"]);
    assert!(neighbours.contains("Predecessors") && neighbours.contains("Station 3"), "{neighbours}");
    let successors = neighbours.split("Successors").nth(1).expect("successors listed");
    assert!(["Station 4a", "Station 4b", "Station 4c"].iter().all(|name| successors.contains(name)), "{neighbours}");

    let upstream = graph(&["upstream", "Drain", "--steps", "1"]);
    assert!(upstream.lines().any(|line| line.trim_start().starts_with('1') && line.contains("Station 7")), "{upstream}");
    assert!(upstream.contains("4 instances"), "{upstream}");
    let downstream = graph(&["downstream", "Station 5"]);
    assert!(downstream.contains("Station 5c") && downstream.contains("Drain") && !downstream.contains("Station 4"), "{downstream}");

    let paths = graph(&["paths", "Source", "Drain"]);
    assert!(paths.contains("Shortest: Source -> Station 1 -> Station 2 -> Station 5 -> Station 5a -> Drain"), "{paths}");
    assert!(paths.contains("6 paths"), "{paths}");

    let around = graph(&["subgraph-around", "Station 2", "--steps", "1"]);
    assert!(around.contains("Station 1 -> Station 2") && around.contains("Station 2 -> Station 5"), "{around}");

    let found = graph(&["find-instances", "--class", "Demo station", "--where", "process_time > 0:20"]);
    assert!(found.contains("Station 4") && found.contains("Station 7") && found.contains("2 instances"), "{found}");
    assert!(found.contains("process_time = 00:00:25"), "the values it matched: {found}");
    let at_least = graph(&["find-instances", "--where", "process_time >= 0:25"]);
    assert!(at_least.contains("Station 4") && at_least.contains("2 instances"), "{at_least}");

    let tree = graph(&["class-tree", "--instances"]);
    let station = tree.lines().position(|line| line.trim_start().starts_with("Station ")).expect("Station listed");
    let demo = tree.lines().position(|line| line.trim_start().starts_with("Demo station")).expect("Demo station listed");
    assert!(demo > station && tree.lines().nth(demo).unwrap().starts_with("  "), "Demo station indented below Station: {tree}");
    assert!(tree.contains("Station 4c"), "instance names with --instances: {tree}");

    let unknown = crabsim(&["graph", "neighbours", "--models", models.arg(), "--model", "demo.json", "Nobody"]);
    assert!(!unknown.status.success() && stderr(&unknown).contains("Nobody"));
    let bad_condition = crabsim(&["graph", "find-instances", "--models", models.arg(), "--model", "demo.json", "--where", "process_time soon"]);
    assert!(!bad_condition.status.success());
}

#[test]
fn export_writes_a_model_into_exports_and_says_where() {
    let models = Models::new("export");
    let printed = ok(&["export", "--models", models.arg(), "--model", "demo.json", "--format", "graphml"]);
    assert!(printed.contains("exports/demo.graphml"), "{printed}");
    assert!(models.0.join("exports").join("demo.graphml").is_file());
    let csv = ok(&["export", "--models", models.arg(), "--model", "demo.json", "--format", "neo4j-csv"]);
    assert_eq!(csv.lines().filter(|line| line.contains("exports/demo/")).count(), 5, "{csv}");
    let refused = crabsim(&["export", "--models", models.arg(), "--model", "demo.json", "--format", "dot"]);
    assert!(!refused.status.success());
}

#[test]
fn while_a_window_has_the_folder_the_cli_reads_and_runs_but_changes_nothing() {
    let models = Models::new("ui-holds");
    std::fs::write(models.0.join("changes.json"), r#"[{ "change": "move_instance", "name": "Drain", "position": { "x": 30, "y": 0 } }]"#).unwrap();
    ok(&["adopt", "--models", models.arg(), "demo.json"]);
    let window = crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&models.0, crabsim_catalogue::Source::Ui)).unwrap();

    let changes = models.0.join("changes.json");
    let edit = crabsim(&["edit", "--models", models.arg(), "demo.json", changes.to_str().unwrap()]);
    assert!(!edit.status.success());
    assert!(stderr(&edit).contains("window"), "{}", stderr(&edit));
    let revoke = crabsim(&["editors", "revoke", "--models", models.arg(), "demo.json"]);
    assert!(!revoke.status.success() && stderr(&revoke).contains("window"), "{}", stderr(&revoke));
    ok(&["editors", "show", "--models", models.arg()]);
    ok(&["attributes", "--models", models.arg(), "demo.json", "Station 1"]);
    ok(&["describe", "--models", models.arg(), "--model", "demo.json"]);

    drop(window);
    ok(&["edit", "--models", models.arg(), "demo.json", changes.to_str().unwrap()]);
}
