//! Hooks in model files (ADR 0029, spec issue 156, PR 2): read, written, checked against their
//! code and run.

use crabsim::{Recording, SimTime, TriggerPoint};
use crabsim_model::crabsim_code::{testing, Library, Options};
use crabsim_model::{load, DistributionStrategy};
use serde_json::{json, Value};
use std::path::PathBuf;

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// The example model in `code-examples/routing`.
fn every_nth() -> Value {
    serde_json::from_str(&std::fs::read_to_string(testing::repository().join("code-examples/routing/every-nth.json")).unwrap()).unwrap()
}

/// A models folder of its own holding the example module in `code/`.
fn folder(name: &str) -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("code-{name}"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("code")).unwrap();
    std::fs::copy(testing::routing(), folder.join("code/routing.wasm")).unwrap();
    folder
}

fn messages(problems: &[crabsim_model::Problem]) -> Vec<String> {
    problems.iter().map(ToString::to_string).collect()
}

#[test]
fn hooks_are_read_and_written_back_the_same() {
    let mut file = every_nth();
    file["instances"][2]["attributes"]["on_entry"] = json!({ "code": { "function": "routing.report", "with": { "drain": "Drain", "every": "0:30" } }, "at": ["rear", "front"] });
    file["instances"][3]["attributes"]["on_exit"] = json!({ "code": { "function": "routing.shortest_queue" } });
    let model = load(&file.to_string()).unwrap();
    let fork = &model.instances[1].attributes;
    let Some(DistributionStrategy::Code(code)) = &fork.distribution else { panic!("a code distribution") };
    assert_eq!(code.code.function, "routing.every_nth");
    assert_eq!(code.default, DistributionStrategy::RoundRobin);
    assert_eq!(model.instances[2].attributes.on_entry.as_ref().unwrap().at, [TriggerPoint::Front, TriggerPoint::Rear], "front first");
    assert!(model.hooks.after_init.is_some());
    let saved = model.save();
    assert_eq!(load(&saved).unwrap(), model);
    assert_eq!(load(&saved).unwrap().save(), saved);
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["instances"][2]["attributes"]["on_entry"]["at"], json!(["front", "rear"]));
    assert_eq!(saved["instances"][3]["attributes"]["on_exit"], json!({ "code": { "function": "routing.shortest_queue" } }), "the booking point alone and no parameters are left out");
    assert_eq!(saved["hooks"]["after_init"]["code"]["function"], "routing.report");
}

#[test]
fn a_hooks_shape_is_checked_without_its_code() {
    let mut file = every_nth();
    file["instances"][1]["attributes"]["distribution"] = json!({ "code": { "function": "every_nth", "with": 3 }, "asked_on_entry": "middle" });
    file["instances"][2]["attributes"]["on_entry"] = json!({ "code": { "function": "routing.report" }, "at": ["back"] });
    file["instances"][4]["attributes"]["on_exit"] = json!({ "code": { "function": "routing.report" } });
    file["hooks"]["after_reset"] = json!({ "function": "routing.report" });
    let problems = messages(&load(&file.to_string()).unwrap_err());
    for expected in [
        "/instances/1/attributes/distribution/code/function \"Fork\": \"every_nth\" isn't a function's name: write \"<module>.<function>\", such as \"routing.every_nth\"",
        "/instances/1/attributes/distribution/code/with \"Fork\": `with` holds the function's parameters by name, such as {\"n\": 3}",
        "/instances/1/attributes/distribution/asked_on_entry \"Fork\": `asked_on_entry` is a trigger point: \"front\", \"booking_point\" or \"rear\"",
        "/instances/2/attributes/on_entry/at \"Left\": a trigger point is \"front\", \"booking_point\" or \"rear\"",
        "/hooks/after_reset/function: unknown field `function` in a hook",
    ] {
        assert!(problems.iter().any(|problem| problem == expected), "missing {expected:?} in {problems:#?}");
    }
    assert!(problems.iter().any(|problem| problem.starts_with("/instances/4/attributes/on_exit \"Drain\"")), "a Drain has no hook on exit: {problems:#?}");
}

#[test]
fn a_cross_conveyors_distribution_moment_cant_move() {
    let file = json!({
        "format_version": 2,
        "instances": [
            { "name": "Cross", "class": "CrossConveyor", "attributes": { "distribution": { "code": { "function": "routing.shortest_queue" }, "asked_on_entry": "front" } } }
        ]
    });
    let problems = messages(&load(&file.to_string()).unwrap_err());
    assert_eq!(problems, ["/instances/0/attributes/distribution/asked_on_entry \"Cross\": a cross conveyor's distribution always runs as it lets the MU in, so its moment can't move"]);
}

#[test]
fn hooks_are_checked_against_their_code_where_they_are_written() {
    let mut file = every_nth();
    file["classes"] = json!([{ "name": "Fast", "parent": "Station", "attributes": { "on_entry": { "code": { "function": "routing.every_second" } } } }]);
    file["instances"][2]["class"] = json!("Fast");
    file["instances"][1]["attributes"]["distribution"]["code"]["with"] = json!({ "n": 1.5, "left": "Lfet", "right": "Right" });
    file["instances"][3]["attributes"]["on_exit"] = json!({ "code": { "function": "sorting.by_class" } });
    let model = load(&file.to_string()).unwrap();
    assert_eq!(model.code_modules(), ["routing", "sorting"]);
    let (library, problems) = model.load_code(&folder("checked"));
    assert_eq!(messages(&problems), ["/instances/3/attributes/on_exit/code/function \"Right\": there is no code module `sorting`: no file code/sorting.wasm"]);
    assert_eq!(
        messages(&model.code_problems(&library)),
        [
            "/classes/0/attributes/on_entry/code/function \"Fast\": code module `routing` has no function `every_second`; it has cycle, every_nth, pass, random_exit, report, shortest_queue",
            "/instances/1/attributes/distribution/code/with/n \"Fork\": parameter `n` of `routing.every_nth`: 1.5 isn't a whole number",
            "/instances/1/attributes/distribution/code/with/left \"Fork\": parameter `left` of `routing.every_nth`: there is no instance `Lfet`",
            "/instances/3/attributes/on_exit/code/function \"Right\": there is no code module `sorting`",
        ]
    );
}

#[test]
fn a_model_file_runs_with_its_code() {
    let folder = folder("runs");
    let model = load(&every_nth().to_string()).unwrap();
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = lines.clone();
    let options = Options { log: Some(std::sync::Arc::new(move |line: &crabsim_model::crabsim_code::LogLine| sink.lock().unwrap().push(line.to_string()))), ..Options::default() };
    let description = model.to_description_in(&folder, options).unwrap();
    let mut run = description.build(0);
    run.set_recording(Recording::NOTHING);
    run.init().unwrap();
    run.advance_to(secs(2 * 3600)).unwrap();
    let state = run.state();
    let (left, right) = (state.objects[2].statistics.mus_in, state.objects[3].statistics.mus_in);
    assert!(left > 200 && right == 2 * left || right == 2 * left + 1 || right == 2 * left + 2, "every third MU left: {left} left, {right} right");
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 3, "at Init and every hour: {lines:?}");
    assert!(lines[1].starts_with("01:00:00  -  routing.report: Drain has taken "), "{lines:?}");
}

#[test]
fn a_model_without_code_builds_as_ever() {
    let model = load(&std::fs::read_to_string(testing::repository().join("models/demo.json")).unwrap()).unwrap();
    assert!(!model.has_code());
    let with = model.to_description_with_code(std::sync::Arc::new(Library::new()), Options::default()).unwrap();
    let without = model.to_description();
    let run = |description: crabsim::ModelDescription| {
        let mut run = description.build(3);
        run.set_recording(Recording::ALL);
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(3600)).unwrap());
        (occurrences, run.state())
    };
    assert_eq!(run(with), run(without));
}
