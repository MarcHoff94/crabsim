//! The headless CLI, run as its binary: the demo model with a seed and a simulated length, then
//! each object's final statistics and how long the run took.

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
