//! Rules in model files (ADR 0030): a distribution of when/then lines, read, saved in canonical
//! form, checked against the model where each line is written, and what a run of it does.

use crabsim::SimTime;
use crabsim_model::{load, DistributionStrategy};
use serde_json::{json, Value};

/// Frames and totes from two Sources into a Split, then Line A or Line B, then Shipping. The
/// Split's distribution is `distribution`.
fn line(distribution: Value) -> String {
    json!({
        "format_version": 2,
        "classes": [
            { "name": "Frame", "parent": "MU", "attributes": { "length": 1.2 } },
            { "name": "Big frame", "parent": "Frame" },
            { "name": "Line", "parent": "Station", "attributes": { "process_time": "4" } }
        ],
        "instances": [
            { "name": "Frames", "class": "Source", "attributes": { "interval": "30", "mu_class": "Big frame" } },
            { "name": "Totes", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Split", "class": "Station", "attributes": { "process_time": "0", "distribution": distribution } },
            { "name": "Line A", "class": "Line" },
            { "name": "Line B", "class": "Line" },
            { "name": "Shipping", "class": "Drain" }
        ],
        "connections": [
            { "from": "Frames", "to": "Split", "exit": 1, "entrance": 1 },
            { "from": "Totes", "to": "Split", "exit": 1, "entrance": 2 },
            { "from": "Split", "to": "Line A", "exit": 1, "entrance": 1 },
            { "from": "Split", "to": "Line B", "exit": 2, "entrance": 1 },
            { "from": "Line A", "to": "Shipping", "exit": 1, "entrance": 1 },
            { "from": "Line B", "to": "Shipping", "exit": 1, "entrance": 2 }
        ]
    })
    .to_string()
}

fn problems(text: &str) -> Vec<String> {
    load(text).expect_err("the file has problems").iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

fn mus_in(text: &str, name: &str, hours: u64) -> u64 {
    let model = load(text).unwrap();
    let id = model.instances.iter().position(|instance| instance.name == name).unwrap();
    let mut run = model.to_description().build(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(hours * 3600)).unwrap();
    run.state().objects[id].statistics.mus_in
}

#[test]
fn rules_are_read_saved_canonical_and_run() {
    let text = line(json!({ "rules": ["when class = Frame then to Line A", "otherwise to Line B"] }));
    let model = load(&text).unwrap();
    let Some(DistributionStrategy::Rules(rules)) = &model.instances[2].attributes.distribution else { panic!("rules") };
    assert_eq!(rules.lines, ["when class = Frame then to Line A", "otherwise to Line B"]);
    assert_eq!(rules.default, DistributionStrategy::RoundRobin);
    let saved = model.save();
    assert_eq!(load(&saved).unwrap(), model);
    // A class stands for the MU classes derived from it: the Big frames go to Line A.
    let a = mus_in(&text, "Line A", 1);
    assert!((119..=121).contains(&a), "a frame every 30 s: {a}");
    let b = mus_in(&text, "Line B", 1);
    assert!((359..=361).contains(&b), "a tote every 10 s: {b}");

    // Written loosely, saved canonical.
    let loose = line(json!({ "rules": ["when time of day < 6:00 then smallest mus now of Line A, Line B"], "default": { "weighted": { "by": "process_time" } } }));
    let saved = load(&loose).unwrap().save();
    assert!(saved.contains("when time of day < 06:00 then smallest mus now of Line A, Line B among those free"), "{saved}");
    assert!(saved.contains("\"weighted\""), "{saved}");
}

#[test]
fn a_rule_that_doesnt_fit_the_model_is_a_problem_where_it_is_written() {
    let text = line(json!({ "rules": [
        "when class = Line then to Line A",
        "when destination = Frame then to Line A",
        "when Split.working_share since start >= 50% then to Line A",
        "when Shipping.mean_mus since start >= 2 then to Line A",
        "when Line A.state now = holding then to Line A",
        "otherwise to Shipping",
        "otherwise weighted among Line A",
        "otherwise smallest throughput_per_hour since start",
        "when class = Frame to Line A"
    ] }));
    let found = problems(&text);
    let at = |place: usize, part: &str| {
        let pointer = format!("/instances/2/attributes/distribution/rules/{place}: ");
        assert!(found.iter().any(|problem| problem.starts_with(&pointer) && problem.contains(part)), "no problem at {pointer} saying {part:?} in {found:#?}");
    };
    at(0, "`Line` isn't an MU class");
    at(1, "`Frame` isn't a destination");
    at(2, "the object deciding");
    at(3, "a Drain has no `mean_mus`");
    at(4, "a Station is never holding");
    at(5, "`Shipping` isn't one of the successors");
    at(6, "make its `default`");
    at(7, "which has no `throughput_per_hour`");
    at(8, "followed by ` then");

    // Set in a class, a problem points there and names the instance it is for.
    let mut model: Value = serde_json::from_str(&line(json!("round_robin"))).unwrap();
    model["classes"].as_array_mut().unwrap().push(json!({ "name": "Splitter", "parent": "Station", "attributes": { "distribution": { "rules": ["otherwise to Line C"] } } }));
    model["instances"][2]["class"] = json!("Splitter");
    model["instances"][2]["attributes"].as_object_mut().unwrap().remove("distribution");
    let found = problems(&model.to_string());
    assert!(found.iter().any(|problem| problem.starts_with("/classes/3/attributes/distribution/rules/0: for its instance `Split`: `Line C` isn't one of the successors")), "{found:#?}");
}

#[test]
fn a_figure_over_the_last_stretch_and_another_objects_state_decide_a_run() {
    // Line A works 4 s an MU and gets every MU while it was idle half the last 10 minutes or more.
    let text = line(json!({ "rules": ["when Line A.idle_share last 10min >= 50% then to Line A", "otherwise to Line B"] }));
    let a = mus_in(&text, "Line A", 2);
    let b = mus_in(&text, "Line B", 2);
    // An MU every 7.5 s on average keeps Line A just over half busy if it took all, so it takes
    // most of them and B the rest.
    assert!(a > b && b > 0, "{a} and {b}");
}
