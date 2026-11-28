//! A Station's process time by MU and which MUs an object takes (ADR 0030, spec issue 162's
//! PR 5), in model files: `process_time` as rule lines, read, saved canonical, inherited as a
//! whole, checked where each line is written and run; `step = …` in distribution rules; and
//! `class` and `step` limits with `in` and `not in`, checked against the model, watched in a run
//! and judged, the first MU that broke one named with its class and step.

use crabsim::{OccurrenceKind, Recording, SimTime};
use crabsim_model::expect::{self, Verdict};
use crabsim_model::load;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Frames, which follow a plan of Weld (any Welding Station) then Shipping, and totes, without a
/// plan, from two Sources into Mill, then Weld 1, then Shipping. Mill's process time is
/// `process_time`, and the model's expectations `expectations`.
fn line(process_time: Value, expectations: Value) -> String {
    json!({
        "format_version": 2,
        "classes": [
            { "name": "Frame", "parent": "MU", "attributes": { "length": 1.2, "work_plan": [{ "at": "Welding" }, { "at": "Shipping" }] } },
            { "name": "Big frame", "parent": "Frame" },
            { "name": "Tote", "parent": "MU" },
            { "name": "Welding", "parent": "Station", "attributes": { "process_time": "10" } },
            { "name": "Spot welding", "parent": "Welding" }
        ],
        "instances": [
            { "name": "Frames", "class": "Source", "attributes": { "interval": "100", "mu_class": "Big frame" } },
            { "name": "Totes", "class": "Source", "attributes": { "interval": "100", "mu_class": "Tote" } },
            { "name": "Mill", "class": "Station", "attributes": { "process_time": process_time } },
            { "name": "Weld 1", "class": "Spot welding" },
            { "name": "Shipping", "class": "Drain" }
        ],
        "connections": [
            { "from": "Frames", "to": "Mill", "exit": 1, "entrance": 1 },
            { "from": "Totes", "to": "Mill", "exit": 1, "entrance": 2 },
            { "from": "Mill", "to": "Weld 1", "exit": 1, "entrance": 1 },
            { "from": "Weld 1", "to": "Shipping", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
    .to_string()
}

fn problems(text: &str) -> Vec<String> {
    load(text).expect_err("the file has problems").iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

/// How long the instance called `name` worked on each MU, in seconds, in order.
fn work(text: &str, name: &str, until: u64) -> Vec<u64> {
    let model = load(text).unwrap();
    let id = model.instances.iter().position(|instance| instance.name == name).unwrap() as u32;
    let mut run = model.to_description().build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(SimTime::from_secs(until)).unwrap());
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { at, duration, .. } if at == id => Some(duration.as_secs()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_process_time_by_mu_is_read_saved_canonical_and_run() {
    let text = line(json!({ "rules": ["when class = Frame then 45s", "when step = Welding then 1:00", "otherwise default"], "default": "20" }), json!([]));
    let model = load(&text).unwrap();
    let mill = &model.instances[2].attributes;
    assert_eq!(mill.process_time, Some(SimTime::from_secs(20)));
    assert_eq!(mill.process_time_rules.as_deref(), Some(&["when class = Frame then 45s".to_string(), "when step = Welding then 00:01:00".into(), "otherwise default".into()][..]));
    assert_eq!(load(&model.save()).unwrap(), model);
    // A Big frame is a Frame: 45 s; a tote 20 s, the Station's own. Both Sources offer at the same
    // instants, frames first.
    let times = work(&text, "Mill", 1000);
    assert!(times.len() >= 18 && times.chunks_exact(2).all(|pair| pair == [45, 20]), "{times:?}");

    // Inherited as a whole: a class's lines, and an instance's own plain time cutting them off.
    let mut value: Value = serde_json::from_str(&line(json!("20"), json!([]))).unwrap();
    value["classes"][3]["attributes"]["process_time"] = json!({ "rules": ["when class = Tote then 3s"], "default": "8" });
    let text = value.to_string();
    let model = load(&text).unwrap();
    let weld = model.resolve(3).unwrap();
    assert_eq!(weld.attributes.process_time, Some(SimTime::from_secs(8)));
    assert_eq!(weld.attributes.process_time_rules.as_deref(), Some(&["when class = Tote then 3s".to_string()][..]));
    // Frames are on their plan at Weld 1, whose step has no time of its own, so the lines decide:
    // the Station's own 8 s for a frame, 3 s for a tote.
    let times = work(&text, "Weld 1", 1000);
    assert!(times.chunks_exact(2).all(|pair| pair == [8, 3]), "{times:?}");
    value["instances"][3]["attributes"] = json!({ "process_time": "6" });
    let model = load(&value.to_string()).unwrap();
    assert_eq!(model.resolve(3).unwrap().attributes.process_time_rules, None, "a plain time set on the instance cuts the lines off too");
}

#[test]
fn a_plan_steps_own_time_wins_over_the_lines() {
    let mut value: Value = serde_json::from_str(&line(json!("20"), json!([]))).unwrap();
    value["classes"][0]["attributes"]["work_plan"][0]["process_time"] = json!("7");
    value["classes"][3]["attributes"]["process_time"] = json!({ "rules": ["otherwise 30s"], "default": "8" });
    let times = work(&value.to_string(), "Weld 1", 1000);
    assert!(times.chunks_exact(2).all(|pair| pair == [7, 30]), "the frames on plan take their step's 7 s: {times:?}");
}

#[test]
fn a_process_times_line_that_doesnt_fit_is_a_problem_where_it_is_written() {
    let text = line(
        json!({ "rules": [
            "when destination = Welding then 5s",
            "when class = Welding then 5s",
            "when step = Frame then 5s",
            "when step = Nowhere then 5s",
            "when Mill.working_share since start >= 50% then 5s",
            "when Weld 1.working_share since start >= 50% and step = Weld 1, Spot welding then 5s"
        ], "default": "20" }),
        json!([]),
    );
    let found = problems(&text);
    let at = |place: usize| format!("/instances/2/attributes/process_time/rules/{place}");
    for (place, part) in [(0, "`step = …`, not its destination"), (1, "`Welding` isn't an MU class"), (2, "`Frame` isn't a class of objects or an instance"), (3, "`Nowhere` isn't"), (4, "the object deciding")] {
        assert!(found.iter().any(|problem| problem.starts_with(&at(place)) && problem.contains(part)), "line {place}, {part}: {found:#?}");
    }
    assert!(!found.iter().any(|problem| problem.starts_with(&at(5))), "{found:#?}");

    for (process_time, part) in [
        (json!({ "rules": ["when class = Frame then 45"] , "default": "20" }), "with its unit, such as `45s`"),
        (json!({ "rules": ["when class = Frame then soon"], "default": "20" }), "isn't a time"),
        (json!({ "rules": ["when class = Frame then 45s"] }), "`default` is the Station's own time"),
        (json!({ "rules": [], "default": "20" }), "at least one"),
    ] {
        let found = problems(&line(process_time, json!([])));
        assert!(found.iter().any(|problem| problem.contains(part)), "{part}: {found:#?}");
    }
}

#[test]
fn a_distribution_rule_reads_the_step() {
    let mut value: Value = serde_json::from_str(&line(json!("0"), json!([]))).unwrap();
    value["instances"][2]["attributes"]["distribution"] = json!({ "rules": ["when step = Welding then to Weld 1", "otherwise to Shipping"] });
    value["connections"].as_array_mut().unwrap().push(json!({ "from": "Mill", "to": "Shipping", "exit": 2, "entrance": 2 }));
    let text = value.to_string();
    let model = load(&text).unwrap();
    let mut run = model.to_description().build(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(1000)).unwrap();
    let state = run.state();
    // Frames, whose step is Weld 1's class's parent Welding, to Weld 1; totes straight on.
    assert_eq!(state.objects[3].statistics.mus_in, state.objects[0].statistics.mus_out);
    assert!(state.objects[3].statistics.mus_in >= 9);
}

#[test]
fn a_step_names_what_it_stands_for() {
    let model = load(&line(json!("20"), json!([]))).unwrap();
    // A class stands for itself, the classes derived from it and their instances; an instance
    // for itself alone.
    assert!(model.step_covers("Welding", "Welding"));
    assert!(model.step_covers("Welding", "Spot welding"));
    assert!(model.step_covers("Welding", "Weld 1"));
    assert!(model.step_covers("Station", "Weld 1"));
    assert!(!model.step_covers("Spot welding", "Welding"));
    assert!(!model.step_covers("Weld 1", "Welding"));
    assert!(model.step_covers("Weld 1", "Weld 1"));
}

#[test]
fn a_limit_on_which_mus_an_object_takes_reads_and_fits_the_model() {
    for (typed, canonical) in [
        ("Mill.step always in Welding,Shipping", "Mill.step always in Welding, Shipping"),
        ("every Welding.class always not in  Tote", "every Welding.class always not in Tote"),
    ] {
        let check = expect::Check::parse(typed).unwrap_or_else(|error| panic!("{typed}: {error}"));
        assert_eq!(check.text(), canonical);
        assert_eq!(expect::Check::parse(canonical).unwrap(), check);
    }
    for (typed, part) in [
        ("Mill.step since start in Welding", "with `always`"),
        ("Mill.class always = Frame", "`in` or `not in`"),
        ("Mill.mus always in Frame", "`in` and `not in` are for which MUs"),
        ("model.class always in Frame", "the model has no `class`"),
        ("Mill.step always in Welding,", "separated by commas"),
        ("Mill.next_destination always in Welding", "an MU's figure"),
    ] {
        let error = expect::Check::parse(typed).expect_err(typed);
        assert!(error.message.contains(part), "{typed}: {error}");
    }
    let expectations = json!([
        { "id": "e1", "text": "Frames.class always in Frame" },
        { "id": "e2", "text": "Mill.class always in Welding" },
        { "id": "e3", "text": "Mill.step always in Frame" },
        { "id": "e4", "text": "Mill.step always in Nowhere" },
        { "id": "e5", "text": "every Welding.step always in Welding, Shipping" },
        { "id": "e6", "text": "Mill.class always not in Big frame, Tote" }
    ]);
    let found = problems(&line(json!("20"), expectations));
    for (place, part) in [(0, "no MU enters a Source"), (1, "`Welding` isn't an MU class"), (2, "`Frame` isn't a class of objects or an instance"), (3, "`Nowhere` isn't")] {
        assert!(found.iter().any(|problem| problem.starts_with(&format!("/expectations/{place}/text")) && problem.contains(part)), "{place}, {part}: {found:#?}");
    }
    assert!(!found.iter().any(|problem| problem.starts_with("/expectations/4") || problem.starts_with("/expectations/5")), "{found:#?}");
}

#[test]
fn a_rename_reaches_the_names_a_line_reads() {
    use crabsim_model::rules::{Line, TimeLine};
    let line = TimeLine::parse("when step = Weld 1, Welding and class = Frame then 30s").unwrap();
    assert_eq!(line.renamed("Weld 1", "Weld A", false).unwrap().text(), "when step = Weld A, Welding and class = Frame then 30s");
    assert_eq!(line.renamed("Welding", "Joining", true).unwrap().text(), "when step = Weld 1, Joining and class = Frame then 30s");
    assert_eq!(line.renamed("Frame", "Panel", true).unwrap().text(), "when step = Weld 1, Welding and class = Panel then 30s");
    assert!(line.renamed("Frame", "Panel", false).is_none());
    let line = Line::parse("when step != Welding then to Weld 1").unwrap();
    assert_eq!(line.renamed("Welding", "Joining", true).unwrap().text(), "when step != Joining then to Weld 1");
}

#[test]
fn a_rename_reaches_the_names_a_limit_lists() {
    let check = expect::Check::parse("every Welding.step always in Welding, Weld 1").unwrap();
    assert_eq!(check.renamed("Weld 1", "Weld A", false).unwrap().text(), "every Welding.step always in Welding, Weld A");
    assert_eq!(check.renamed("Welding", "Joining", true).unwrap().text(), "every Joining.step always in Joining, Weld 1");
    let check = expect::Check::parse("Mill.class always in Frame").unwrap();
    assert!(check.renamed("Frame", "Panel", false).is_none(), "an instance's rename never reaches an MU class");
    assert_eq!(check.renamed("Frame", "Panel", true).unwrap().text(), "Mill.class always in Panel");
    assert_eq!(check.renamed("Mill", "Mill 2", false).unwrap().text(), "Mill 2.class always in Frame");
}

/// Runs the model's expectations as a check does, watching what they name, and judges them.
fn judged(text: &str, deterministic: bool) -> Vec<(Verdict, Option<expect::Violation>, Vec<Option<f64>>)> {
    let model = load(text).unwrap();
    let bound = model.bound_expectations();
    let watches = model.entry_watches(&bound);
    let description = model.to_description();
    let mut run = if deterministic { description.build_deterministic(0) } else { description.build(0) };
    run.watch(watches.list.clone());
    run.init().unwrap();
    let length = SimTime::from_secs(3600);
    run.advance_to(length).unwrap();
    let observed = expect::observe(&bound, &run.state(), &BTreeMap::new(), SimTime::ZERO, length, &watches);
    bound
        .iter()
        .zip(&observed)
        .map(|(bound, observed)| {
            let judgement = expect::judge(bound, std::slice::from_ref(observed), deterministic, length);
            (judgement.verdict, judgement.violation.map(|(_, violation)| violation), observed.values.iter().map(|(_, value)| *value).collect())
        })
        .collect()
}

#[test]
fn a_limit_on_which_mus_an_object_takes_is_watched_and_judged() {
    let text = line(
        json!("20"),
        json!([
            { "id": "frames-only", "text": "Mill.class always in Frame" },
            { "id": "welding", "text": "Weld 1.step always in Welding" },
            { "id": "no-totes", "text": "every Welding.class always not in Tote" },
            { "id": "anything", "text": "Mill.class always in MU" }
        ]),
    );
    let verdicts = judged(&text, true);
    // Totes break the first: one every 100 s after the first frame, the first at 100 s.
    let (verdict, violation, values) = &verdicts[0];
    assert_eq!(*verdict, Verdict::NotMet);
    let violation = violation.as_ref().expect("its first break");
    // Mill works the frame first, from 100 s, so the tote enters at 120 s.
    assert_eq!((violation.time, violation.mu, violation.class.as_deref(), violation.step.as_deref()), (SimTime::from_secs(120), 2, Some("Tote"), Some("none")));
    assert!(values[0].is_some_and(|breaks| breaks >= 30.0), "{values:?}");
    // A tote has no step, so it breaks a step limit too; a frame at Weld 1 is on its step Welding.
    let (verdict, violation, _) = &verdicts[1];
    assert_eq!(*verdict, Verdict::NotMet);
    assert_eq!(violation.as_ref().and_then(|violation| violation.step.as_deref()), Some("none"));
    assert_eq!(verdicts[2].0, Verdict::NotMet);
    // Every class is an MU.
    assert_eq!(verdicts[3], (Verdict::Met, None, vec![Some(0.0)]));

    // Without the totes, every one is met.
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["instances"].as_array_mut().unwrap().remove(1);
    value["connections"].as_array_mut().unwrap().remove(1);
    value["connections"][0]["entrance"] = json!(1);
    let verdicts = judged(&value.to_string(), false);
    assert!(verdicts.iter().all(|(verdict, violation, _)| *verdict == Verdict::Met && violation.is_none()), "{verdicts:?}");

    // A run not told to watch can't judge it.
    let model = load(&text).unwrap();
    let bound = model.bound_expectations();
    let mut run = model.to_description().build(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(600)).unwrap();
    let observed = expect::observe(&bound, &run.state(), &BTreeMap::new(), SimTime::ZERO, SimTime::from_secs(600), &model.entry_watches(&bound));
    assert_eq!(expect::judge(&bound[0], &observed[..1], true, SimTime::from_secs(600)).verdict, Verdict::Error);
    // Nothing to watch costs nothing to work out.
    assert!(load(&line(json!("20"), json!([{ "id": "e1", "text": "Mill.mus always <= 3" }]))).map(|model| model.entry_watches(&model.bound_expectations()).list.is_empty()).unwrap());
}

#[test]
fn the_bounds_take_the_lines_shortest_and_longest_time() {
    // MUs every 10 s from each Source: 720 an hour reach Mill.
    let warned = |process_time: Value, text: &str| {
        let mut value: Value = serde_json::from_str(&line(process_time, json!([{ "id": "e1", "text": text }]))).unwrap();
        value["instances"][0]["attributes"]["interval"] = json!("10");
        value["instances"][1]["attributes"]["interval"] = json!("10");
        let model = load(&value.to_string()).unwrap();
        model.warnings().iter().any(|warning| warning.pointer == "/expectations/0/text" && warning.message.contains("can't be met"))
    };
    // Mill passes at most 180 an hour at 20 s each; a line of 5 s could make it 720, and Weld 1
    // 360.
    let throughput = "Shipping.throughput_per_hour since start >= 300/h";
    assert!(warned(json!("20"), throughput));
    assert!(!warned(json!({ "rules": ["when class = Tote then 5s"], "default": "20" }), throughput));
    // At most 4 s of work an MU, 720 of them can't keep it busy more than 80%; a line of 9 s
    // could.
    let busy = "Mill.working_share since start >= 90%";
    assert!(warned(json!("4"), busy));
    assert!(!warned(json!({ "rules": ["when class = Tote then 9s"], "default": "4" }), busy));
}
