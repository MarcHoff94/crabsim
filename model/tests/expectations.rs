//! Expectations in a model file (ADR 0030): read, written back, checked against the model, read
//! from runs and judged.

use crabsim::{OccurrenceKind, Recording, SimTime};
use crabsim_model::expect::{self, Observed, Verdict};
use crabsim_model::{load, CheckStage, Model};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A line where the second Press is slower than the Source, so the Buffer between them fills:
/// Source every 10 s, Press A 8 s, Buffer, Press B 12 s, Drain.
fn line(expectations: Value) -> String {
    json!({
        "format_version": 2,
        "classes": [{ "name": "Press", "parent": "Station" }],
        "instances": [
            { "name": "Source", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Press A", "class": "Press", "attributes": { "process_time": "8" } },
            { "name": "Buffer", "class": "Buffer" },
            { "name": "Press B", "class": "Press", "attributes": { "process_time": "12" } },
            { "name": "Drain", "class": "Drain" }
        ],
        "connections": [
            { "from": "Source", "to": "Press A", "exit": 1, "entrance": 1 },
            { "from": "Press A", "to": "Buffer", "exit": 1, "entrance": 1 },
            { "from": "Buffer", "to": "Press B", "exit": 1, "entrance": 1 },
            { "from": "Press B", "to": "Drain", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
    .to_string()
}

fn problems(text: &str) -> Vec<String> {
    load(text).expect_err("the file has problems").iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

/// Runs `model` deterministically for `length` after `warm_up`, stopping where its expectations
/// need, and gives each expectation's values, the first violations found by a replay.
fn observe(model: &Model, seed: u64, deterministic: bool, warm_up: SimTime, length: SimTime) -> Vec<Observed> {
    let description = model.to_description();
    let bound = model.bound_expectations();
    let mut run = if deterministic { description.build_deterministic(seed) } else { description.build(seed) };
    run.init().unwrap();
    run.advance_to(warm_up).unwrap();
    if warm_up > SimTime::ZERO {
        run.clear_statistics();
    }
    let mut snapshots = BTreeMap::new();
    for time in expect::snapshot_times(&bound, warm_up, length) {
        run.advance_to(time).unwrap();
        snapshots.insert(time, run.state());
    }
    run.advance_to(warm_up + length).unwrap();
    let mut observed = expect::observe(&bound, &run.state(), &snapshots, warm_up, length, &model.entry_watches(&bound));
    let broken: Vec<usize> = expect::broken_limits(&bound, &observed);
    let limits: Vec<&expect::Bound> = broken.iter().map(|&at| &bound[at]).collect();
    for (at, violation) in broken.iter().zip(expect::first_violations(&description, seed, deterministic, warm_up, length, &limits)) {
        observed[*at].violation = violation;
    }
    observed
}

const HOUR: SimTime = SimTime::from_secs(3600);

fn hours(n: u64) -> SimTime {
    SimTime::from_secs(n * 3600)
}

#[test]
fn expectations_are_read_saved_in_their_canonical_form_and_read_back_the_same() {
    let text = line(json!([
        { "id": "presses-busy", "text": "every   Press.working_share since start >=60%", "assumptions": "one shift", "source": "the planner" },
        { "id": "throughput", "text": "model.throughput_per_hour last 1h >= 250/h", "stage": "validation" }
    ]));
    let model = load(&text).unwrap();
    assert_eq!(model.expectations[0].text, "every Press.working_share since start >= 60%");
    assert_eq!(model.expectations[0].assumptions.as_deref(), Some("one shift"));
    assert_eq!(model.expectations[0].stage, CheckStage::Verification);
    assert_eq!(model.expectations[1].stage, CheckStage::Validation);
    let saved = model.save();
    assert!(!saved.contains("\"stage\": \"verification\""), "the default stage isn't written");
    assert_eq!(load(&saved).unwrap(), model);
    assert_eq!(load(&saved).unwrap().save(), saved);
}

#[test]
fn a_line_that_doesnt_fit_the_model_is_a_problem_at_its_place() {
    let found = problems(&line(json!([
        { "id": "a", "text": "Press A.working_share since start >= 60" },
        { "id": "b", "text": "Press.working_share since start >= 60%" },
        { "id": "c", "text": "every Press A.working_share since start >= 60%" },
        { "id": "d", "text": "Drain.working_full_share since start <= 10%" },
        { "id": "e", "text": "Press C.idle_share since start <= 10%" },
        { "id": "e", "text": "Source.state now = idle" },
        { "id": "g", "text": "Press B.mus always <= 3" },
        { "text": "Drain.mus_in since start >= 1" }
    ])));
    let expected = [
        "/expectations/0/text: at character 38: a share is written as a percentage, such as `60%`, not `60`",
        "/expectations/1/text: there is no instance `Press`: `Press` is a class, which takes `every` or `mean` before it",
        "/expectations/2/text: there is no class `Press A`: it's an instance, which takes no `every` or `mean`",
        "/expectations/3/text: a Drain has no `working_full_share`",
        "/expectations/4/text: there is no instance `Press C`",
        "/expectations/5/id: expectation 4 already has this id",
        "/expectations/5/text: a Source is never idle",
        "/expectations/7/id: an expectation needs an id",
    ];
    for (found, expected) in found.iter().zip(expected) {
        assert!(found.starts_with(expected), "{found}\n  expected {expected}");
    }
    assert_eq!(found.len(), expected.len(), "{found:#?}");
}

#[test]
fn a_deterministic_run_judges_each_kind_of_check() {
    let model = load(&line(json!([
        { "id": "busy", "text": "every Press.working_share since start >= 60%" },
        { "id": "mean", "text": "mean Press.working_share since start >= 80%" },
        { "id": "rate", "text": "model.throughput_per_hour since start >= 299/h" },
        { "id": "last", "text": "Drain.mus_in last 1h = 300" },
        { "id": "lead", "text": "Drain.lead_time_s always <= 30s" },
        { "id": "full", "text": "Buffer.mus always <= 20" },
        { "id": "state", "text": "Press B.state now = busy" },
        { "id": "no-mus", "text": "Drain.mean_lead_time_s since start <= 1h" }
    ])))
    .unwrap();
    let length = hours(4);
    let observed = observe(&model, 1, true, SimTime::ZERO, length);
    let bound = model.bound_expectations();
    let verdicts: Vec<(String, Verdict)> = bound.iter().map(|bound| (model.expectations[bound.index].id.clone(), expect::judge(bound, &[observed[bound.index].clone()], true, length).verdict)).collect();
    let verdict = |id: &str| verdicts.iter().find(|(found, _)| found == id).unwrap().1;
    // Press A works 8 of every 10 s, Press B always: each at least 60%, their mean 90%.
    assert_eq!(verdict("busy"), Verdict::Met);
    assert_eq!(verdict("mean"), Verdict::Met);
    // Press B lets one MU out every 12 s: 300 an hour.
    assert_eq!(verdict("rate"), Verdict::Met);
    assert_eq!(verdict("last"), Verdict::Met);
    // Every MU waits ever longer in the Buffer, which fills by one a minute.
    assert_eq!(verdict("lead"), Verdict::NotMet);
    assert_eq!(verdict("full"), Verdict::NotMet);
    assert_eq!(verdict("state"), Verdict::Met);
    assert_eq!(verdict("no-mus"), Verdict::Met);
    let judged = expect::judge(&bound[1], &[observed[1].clone()], true, length);
    assert!((judged.subjects[0].mean.unwrap() - 0.9).abs() < 0.01, "{judged:?}");
}

#[test]
fn a_limits_first_violation_is_the_moment_and_mu_that_broke_it() {
    let model = load(&line(json!([{ "id": "full", "text": "Buffer.mus always <= 20" }]))).unwrap();
    let observed = observe(&model, 1, true, SimTime::ZERO, hours(4));
    let violation = observed[0].violation.clone().expect("the Buffer went past 20");
    assert_eq!(violation.value, 21.0);
    // Worked out again from every move of a recorded run: the first time the Buffer held 21.
    let mut run = model.to_description().build_deterministic(1);
    run.set_recording(Recording::ALL);
    run.init().unwrap();
    let occurrences = run.advance_to(hours(4)).unwrap();
    let mut held = 0i64;
    let first = occurrences
        .iter()
        .find_map(|occurrence| {
            match occurrence.kind {
                OccurrenceKind::MuMoved { to: 2, mu_id, .. } => {
                    held += 1;
                    return (held > 20).then_some((occurrence.time, mu_id));
                }
                OccurrenceKind::MuMoved { from: 2, .. } => held -= 1,
                _ => {}
            }
            None
        })
        .unwrap();
    assert_eq!((violation.time, violation.mu, violation.object), (first.0, first.1, 2));
}

#[test]
fn a_limit_is_judged_from_the_end_of_the_warm_up() {
    // The Buffer fills by one a minute, so it holds about 60 after an hour: a warm-up of an hour
    // starts the count there, and the limit is broken at once.
    let model = load(&line(json!([{ "id": "full", "text": "Buffer.mus always <= 80" }]))).unwrap();
    let after_warm_up = observe(&model, 1, true, HOUR, HOUR);
    let violation = after_warm_up[0].violation.clone().expect("broken in the second hour");
    assert!(violation.time > HOUR && violation.time < hours(2), "{violation:?}");
    assert_eq!(observe(&model, 1, true, SimTime::ZERO, HOUR)[0].violation, None, "not broken in the first hour");
}

/// `line` with requests and stages beside its expectations.
fn agreed(expectations: Value, requests: Value, stages: Value) -> String {
    let mut model: Value = serde_json::from_str(&line(expectations)).unwrap();
    model["requests"] = requests;
    model["stages"] = stages;
    model.to_string()
}

#[test]
fn requests_agreement_and_stage_settings_are_read_and_written_back() {
    let text = agreed(
        json!([
            { "id": "busy", "text": "every Press.working_share since start >= 60%", "state": "accepted" },
            { "id": "full", "text": "Buffer.mus always <= 20", "state": "questioned", "question": "Press B is slower than the Source, so the Buffer fills without end", "length": "8:00:00" },
            { "id": "rate", "text": "model.throughput_per_hour since start >= 280/h", "stage": "validation", "observations": 30 }
        ]),
        json!([{ "id": "r1", "words": "the presses must earn their keep", "restatement": "both Presses working at least 60% of the time, and the Buffer never over 20", "expectations": ["busy", "full"], "accepted": true, "state": "built" }]),
        json!({ "verification": { "length": "2d" }, "validation": { "length": "30d", "warm_up": "1d", "observations": 20 } }),
    );
    let model = load(&text).unwrap();
    let request = &model.requests[0];
    assert_eq!((request.id.as_str(), request.accepted, request.built), ("r1", true, true));
    assert_eq!(request.expectations, ["busy", "full"]);
    assert_eq!(model.expectations[0].state, crabsim_model::Agreement::Accepted);
    assert_eq!(model.expectations[1].state, crabsim_model::Agreement::Questioned);
    assert_eq!(model.expectations[2].state, crabsim_model::Agreement::Proposed, "the default");
    let day = SimTime::from_secs(86_400);
    // Each expectation's own settings over its stage's, over the defaults.
    assert_eq!(model.settings_of(0), crabsim_model::Settings { length: SimTime::from_secs(2 * 86_400), warm_up: SimTime::ZERO, observations: 1 });
    assert_eq!(model.settings_of(1), crabsim_model::Settings { length: SimTime::from_secs(8 * 3600), warm_up: SimTime::ZERO, observations: 1 });
    assert_eq!(model.settings_of(2), crabsim_model::Settings { length: SimTime::from_secs(30 * 86_400), warm_up: day, observations: 30 });
    assert_eq!(model.checks_of(CheckStage::Verification).len(), 2, "two settings, two runs");
    let saved = model.save();
    assert_eq!(load(&saved).unwrap(), model);
    assert_eq!(load(&saved).unwrap().save(), saved);
    // The defaults aren't written.
    assert!(!saved.contains("\"proposed\"") && !saved.contains("\"open\""), "{saved}");
}

#[test]
fn a_request_or_its_agreement_that_doesnt_fit_is_a_problem_where_it_is() {
    let text = agreed(
        json!([
            { "id": "busy", "text": "every Press.working_share since start >= 60%", "state": "questioned" },
            { "id": "full", "text": "Buffer.mus always <= 20", "question": "why?", "observations": 0 },
            { "id": "agreed", "text": "Buffer.mus always <= 20", "state": "agreed" }
        ]),
        json!([
            { "id": "r1", "words": "", "expectations": ["busy", "missing", "busy"], "state": "met" },
            { "id": "r1", "words": "again" }
        ]),
        json!({ "verification": { "length": "0" }, "checking": {} }),
    );
    let found = problems(&text);
    for expected in [
        "/stages/checking: unknown field",
        "/stages/verification/length: a check runs for longer than 0",
        "/requests/0/words: a request needs the user's own `words`",
        "/requests/0/expectations/1: there is no expectation `missing`",
        "/requests/0/expectations/2: `busy` is listed twice",
        "/requests/0/state: a request's `state` is `open` or `built`",
        "/requests/1/id: request 0 already has this id",
        "/expectations/0/question: a questioned expectation says why",
        "/expectations/1/question: only a questioned expectation has a `question`",
        "/expectations/1/observations: a check has from 1 to 10000 observations",
        "/expectations/2/state: `state` is `proposed`, `accepted` or `questioned`",
    ] {
        assert!(found.iter().any(|problem| problem.starts_with(expected)), "no {expected:?} in {found:#?}");
    }
}
