//! Work plans, detours and routing by destination in model files (spec issue 148): reading,
//! saving, problems, warnings and what a run of them does.

use crabsim::{OccurrenceKind, Recording, SimTime};
use crabsim_model::{load, DistributionStrategy, Model, WorkStep};
use serde_json::{json, Value};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// A model file of these classes and instances, each instance listing its `successors`, which
/// become connections in the order they come.
fn file(classes: Value, instances: Value) -> String {
    let mut instances = instances;
    let mut connections = Vec::new();
    let mut entrances: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for instance in instances.as_array_mut().into_iter().flatten() {
        let Some(fields) = instance.as_object_mut() else { continue };
        let Some(Value::Array(successors)) = fields.remove("successors") else { continue };
        let from = fields["name"].clone();
        for (place, to) in successors.into_iter().enumerate() {
            let entrance = entrances.entry(to.as_str().unwrap_or_default().to_string()).or_default();
            *entrance += 1;
            connections.push(json!({ "from": from, "to": to, "exit": place + 1, "entrance": *entrance }));
        }
    }
    json!({ "format_version": 2, "classes": classes, "instances": instances, "connections": connections }).to_string()
}

fn problems(text: &str) -> Vec<String> {
    load(text).expect_err("the file has problems").iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

fn assert_problem(text: &str, pointer: &str, part: &str) {
    let found = problems(text);
    assert!(found.iter().any(|problem| problem.starts_with(&format!("{pointer}: ")) && problem.contains(part)), "no problem at {pointer} saying {part:?} in {found:#?}");
}

fn warnings(model: &Model) -> Vec<String> {
    model.warnings().iter().map(|warning| format!("{}: {}", warning.pointer, warning.message)).collect()
}

/// How long each instance worked on each MU, as (instance, seconds), over `length`.
fn work(model: &Model, length: u64) -> Vec<(String, u64)> {
    let description = model.to_description();
    let mut run = description.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(length)).unwrap());
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { at, duration, .. } => Some((description.name(at).unwrap().to_string(), duration.as_secs())),
            _ => None,
        })
        .collect()
}

/// Housing, an MU class following Saw for 40 s, any Mill, then Out, and a line through Saw and
/// Mill A, of class Mill, to Out.
fn housing_line() -> String {
    file(
        json!([
            { "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Saw", "process_time": "0:40" }, { "at": "Mill" }, { "at": "Out" }] } },
            { "name": "Mill", "parent": "Station", "attributes": { "process_time": "20" } }
        ]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "100", "mu_class": "Housing" }, "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "attributes": { "process_time": "10" }, "successors": ["Mill A"] },
            { "name": "Mill A", "class": "Mill", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    )
}

#[test]
fn a_file_s_work_plan_is_followed_by_its_mu_class_s_mus() {
    let model = load(&housing_line()).unwrap();
    assert_eq!(work(&model, 150), vec![("Saw".to_string(), 40), ("Mill A".to_string(), 20)], "a class step reached at its instance, with its own time");
}

#[test]
fn work_plans_detours_and_by_destination_save_and_load_the_same() {
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Saw", "process_time": "0:40" }, { "at": "Out" }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "10", "mu_class": "Housing", "mu_values": { "work_plan": [{ "at": "Out" }] } }, "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "attributes": { "detour": "Rework", "distribution": { "by_destination": {} } }, "successors": ["Rework", "Out"] },
            { "name": "Rework", "class": "Station", "successors": ["Out"] },
            { "name": "Store", "class": "Buffer", "attributes": { "distribution": { "by_destination": { "then": { "weighted": { "by": "process_time" } } } } } },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    let model = load(&text).unwrap();
    let saved = model.save();
    assert_eq!(load(&saved).unwrap(), model);
    let value: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["classes"][0]["attributes"]["work_plan"], json!([{ "at": "Saw", "process_time": "00:00:40" }, { "at": "Out" }]));
    assert_eq!(value["instances"][1]["attributes"]["distribution"], json!({ "by_destination": { "then": "round_robin" } }), "then written out");
    assert_eq!(model.instances[1].attributes.detour.as_deref(), Some("Rework"));
    assert!(matches!(&model.instances[3].attributes.distribution, Some(DistributionStrategy::ByDestination(then)) if then.weighted().is_some()));
    assert_eq!(model.work_plan_of(0), Some(vec![WorkStep::new("Out", None)]), "the Source's own plan, in place of its class's");
}

#[test]
fn the_built_in_mu_s_plan_is_followed_by_every_source_s_mus() {
    let text = file(
        json!([{ "name": "MU", "attributes": { "work_plan": [{ "at": "Saw", "process_time": "5" }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "100" }, "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "attributes": { "process_time": "10" }, "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    assert_eq!(work(&load(&text).unwrap(), 150), vec![("Saw".to_string(), 5)]);
}

#[test]
fn a_class_destination_stands_for_instances_of_classes_derived_from_it() {
    let text = file(
        json!([
            { "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Mill", "process_time": "7" }] } },
            { "name": "Mill", "parent": "Station" },
            { "name": "Big mill", "parent": "Mill" }
        ]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "100", "mu_class": "Housing" }, "successors": ["Big"] },
            { "name": "Big", "class": "Big mill", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    let model = load(&text).unwrap();
    assert_eq!(model.destination_members("Mill"), Some(vec![1]));
    assert_eq!(model.destination_members("Station"), Some(vec![1]), "a built-in class stands for all its instances");
    assert_eq!(model.destination_members("Housing"), None, "an MU class is no place");
    assert_eq!(work(&model, 150), vec![("Big".to_string(), 7)]);
}

#[test]
fn a_step_or_detour_must_name_a_place_in_the_model() {
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Saww" }, { "at": "Housing" }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "10", "mu_class": "Housing" }, "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "attributes": { "detour": "Rewrk" }, "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    assert_problem(&text, "/classes/0/attributes/work_plan/0/at", "nothing is called `Saww` (did you mean `Saw`?)");
    assert_problem(&text, "/classes/0/attributes/work_plan/1/at", "`Housing` is an MU class, not a place");
    assert_problem(&text, "/instances/1/attributes/detour", "nothing is called `Rewrk`");
}

#[test]
fn only_a_step_that_can_be_a_station_has_a_process_time() {
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Saw", "process_time": "5" }, { "at": "Out", "process_time": "5" }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "10", "mu_class": "Housing" }, "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    let found = problems(&text);
    assert_eq!(found, vec!["/classes/0/attributes/work_plan/1/process_time: `Out` is no Station, so nothing works on the MU there: leave out the step's process time".to_string()]);
}

#[test]
fn work_plans_belong_to_mu_classes_and_detours_to_stations() {
    let text = file(
        json!([]),
        json!([
            { "name": "In", "class": "Source", "successors": ["Saw"] },
            { "name": "Saw", "class": "Station", "attributes": { "work_plan": [{ "at": "Out" }] }, "successors": ["Store"] },
            { "name": "Store", "class": "Buffer", "attributes": { "detour": "Saw" }, "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    assert_problem(&text, "/instances/1/attributes/work_plan", "has no attribute `work_plan`");
    assert_problem(&text, "/instances/2/attributes/detour", "has no attribute `detour`");
}

#[test]
fn a_work_plan_that_can_t_be_read_says_how_one_is_written() {
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "process_time": "5" }, { "at": "Saw", "process_time": "5 s" }, { "at": "Saw", "time": "5" }] } }]),
        json!([{ "name": "Saw", "class": "Station" }]),
    );
    assert_problem(&text, "/classes/0/attributes/work_plan/0", "`at` is missing");
    assert_problem(&text, "/classes/0/attributes/work_plan/1/process_time", "is not a time");
    assert_problem(&text, "/classes/0/attributes/work_plan/2/time", "unknown field `time`");
    let text = file(json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": "Saw" } }]), json!([]));
    assert_problem(&text, "/classes/0/attributes/work_plan", "a work plan is a list of steps");
}

#[test]
fn by_destination_chooses_by_round_robin_or_weighted_only() {
    let text = file(
        json!([]),
        json!([{ "name": "Saw", "class": "Station", "attributes": { "distribution": { "by_destination": { "then": { "by_destination": {} } } } } }]),
    );
    assert_problem(&text, "/instances/0/attributes/distribution/by_destination/then", "not by destination again");
}

/// In, routing by destination, to A or B, both into Out, with MUs going to `going`.
fn fork(going: &str) -> Model {
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": going }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "1", "mu_class": "Housing", "distribution": { "by_destination": {} } }, "successors": ["A", "B"] },
            { "name": "A", "class": "Station", "successors": ["Out"] },
            { "name": "B", "class": "Station", "successors": ["Out"] },
            { "name": "Elsewhere", "class": "Station", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    load(&text).unwrap()
}

#[test]
fn validate_warns_of_a_destination_an_object_routing_by_destination_can_t_reach() {
    let found = warnings(&fork("Elsewhere"));
    assert!(
        found.iter().any(|warning| warning.starts_with("/instances/0: it routes by destination, but none of its successors leads to `Elsewhere`")),
        "{found:#?}"
    );
    let found = warnings(&fork("B"));
    assert!(!found.iter().any(|warning| warning.contains("routes by destination")), "{found:#?}");
}

#[test]
fn validate_follows_mus_on_through_their_plans_and_detours() {
    // Inspection detours to Rework, which reaches only Pack: once there, MUs go to Pack next, which
    // the Fork after Rework reaches, so nothing is warned. A Station whose plan step comes after
    // MUs have passed it isn't warned about either.
    let text = file(
        json!([{ "name": "Housing", "parent": "MU", "attributes": { "work_plan": [{ "at": "Inspection" }, { "at": "Pack" }] } }]),
        json!([
            { "name": "In", "class": "Source", "attributes": { "interval": "10", "mu_class": "Housing" }, "successors": ["Inspection"] },
            { "name": "Inspection", "class": "Station", "attributes": { "detour": "Rework", "distribution": { "by_destination": {} } }, "successors": ["Rework", "Pack"] },
            { "name": "Rework", "class": "Station", "attributes": { "distribution": { "by_destination": {} } }, "successors": ["Pack", "Scrap"] },
            { "name": "Pack", "class": "Station", "successors": ["Out"] },
            { "name": "Scrap", "class": "Drain" },
            { "name": "Out", "class": "Drain" }
        ]),
    );
    let found = warnings(&load(&text).unwrap());
    assert!(!found.iter().any(|warning| warning.contains("routes by destination")), "{found:#?}");
}

#[test]
fn a_run_of_a_file_routes_by_destination() {
    let model = fork("B");
    let description = model.to_description();
    let mut run = description.build(0);
    run.init().unwrap();
    run.advance_to(secs(10)).unwrap();
    let mus_in = |name: &str| run.state().objects[model.instances.iter().position(|i| i.name == name).unwrap()].statistics.mus_in;
    assert_eq!(mus_in("A"), 0, "every MU goes to B");
    assert!(mus_in("B") > 0);
}
