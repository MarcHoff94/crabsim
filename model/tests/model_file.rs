//! Model files at their seam: text in, a model, its problems or a run out.

use crabsim::{demo, ModelDescription, OccurrenceKind, Recording, SimTime, SimulationRun};
use crabsim_model::{load, Instance, Model, Problem};
use serde_json::{json, Value};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// The demo model's file, with line endings as crabsim writes them, whatever git did.
fn demo_file() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/demo.json");
    std::fs::read_to_string(path).expect("models/demo.json exists").replace("\r\n", "\n")
}

/// A model file holding these instances.
fn file(instances: Value) -> String {
    json!({ "format_version": 1, "instances": instances }).to_string()
}

fn problems(text: &str) -> Vec<Problem> {
    load(text).expect_err("the file has problems")
}

/// Each problem as `pointer: message`, for comparing.
fn found(text: &str) -> Vec<String> {
    problems(text).iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

fn assert_found(text: &str, pointer: &str, message_part: &str) {
    let found = found(text);
    assert!(
        found.iter().any(|problem| problem.starts_with(&format!("{pointer}: ")) && problem.contains(message_part)),
        "no problem at {pointer} saying {message_part:?} in {found:#?}"
    );
}

/// Everything a run records over `length`, and its state at the end.
fn run_for(description: &ModelDescription, seed: u64, length: SimTime) -> (Vec<crabsim::Occurrence>, crabsim::RunState) {
    let mut run: SimulationRun = description.build(seed);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(length).unwrap());
    (occurrences, run.state())
}

fn built(text: &str) -> ModelDescription {
    load(text).expect("a valid model").to_description()
}

// ---------- The demo model as a file ----------

#[test]
fn the_demo_file_builds_the_same_run_as_the_demo_model() {
    let from_file = built(&demo_file());
    let from_code = demo::model();
    for seed in [0, 7] {
        assert_eq!(run_for(&from_file, seed, secs(86_400)), run_for(&from_code, seed, secs(86_400)), "seed {seed}");
    }
}

#[test]
fn the_demo_file_has_the_demo_models_names_positions_and_connections() {
    let from_file = built(&demo_file());
    let from_code = demo::model();
    assert_eq!(from_file.object_count(), from_code.object_count());
    for id in 0..from_code.object_count() {
        assert_eq!(from_file.kind(id), from_code.kind(id));
        assert_eq!(from_file.name(id), from_code.name(id));
        assert_eq!(from_file.position(id), from_code.position(id));
        assert_eq!(from_file.dimensions(id), from_code.dimensions(id));
        assert_eq!(from_file.successors(id), from_code.successors(id));
    }
}

// ---------- Saving ----------

#[test]
fn the_demo_file_is_in_canonical_form() {
    let text = demo_file();
    assert_eq!(load(&text).unwrap().save(), text);
}

#[test]
fn a_hand_written_file_is_saved_in_canonical_form_and_then_stays_the_same() {
    let text = file(json!([
        { "attributes": { "interval": "90" }, "class": "Source", "name": "In", "successors": ["Out"],
          "position": { "x": 1, "y": 2, "z": 0 } },
        { "name": "Out", "class": "Drain", "successors": [], "attributes": {} },
    ]));
    let saved = load(&text).unwrap().save();
    let expected = r#"{
  "format_version": 1,
  "instances": [
    {
      "name": "In",
      "class": "Source",
      "position": {
        "x": 1.0,
        "y": 2.0
      },
      "successors": [
        "Out"
      ],
      "attributes": {
        "interval": "00:01:30"
      }
    },
    {
      "name": "Out",
      "class": "Drain"
    }
  ]
}
"#;
    assert_eq!(saved, expected);
    assert_eq!(load(&saved).unwrap().save(), saved);
}

#[test]
fn milliseconds_and_days_survive_saving() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "interval": "2d 0:00:01.25" } },
        { "name": "Out", "class": "Drain" },
    ]));
    let saved = load(&text).unwrap().save();
    assert!(saved.contains("\"2d 00:00:01.250\""), "{saved}");
    assert_eq!(load(&saved).unwrap().instances[0].attributes.interval, Some(SimTime::from_millis(2 * 86_400_000 + 1_250)));
}

/// The benchmark's large model: 429 copies of the demo side by side, each with its own names.
fn copies_of_the_demo(copies: usize) -> Model {
    let demo = load(&demo_file()).unwrap();
    let mut model = Model { description: Some(format!("{copies} copies of the demo model")), instances: Vec::new() };
    for copy in 0..copies {
        let rename = |name: &str| format!("Copy {copy} / {name}");
        for instance in &demo.instances {
            let mut instance: Instance = instance.clone();
            instance.name = rename(&instance.name);
            instance.successors = instance.successors.iter().map(|name| rename(name)).collect();
            if let Some(position) = &mut instance.position {
                position.y += 20.0 * copy as f64;
            }
            model.instances.push(instance);
        }
    }
    model
}

#[test]
fn the_benchmarks_6006_object_model_round_trips_as_a_file() {
    let model = copies_of_the_demo(429);
    assert_eq!(model.instances.len(), 6_006);
    assert_eq!(model.check(), vec![]);
    let saved = model.save();
    let loaded = load(&saved).unwrap();
    assert_eq!(loaded, model);
    assert_eq!(loaded.save(), saved);
    let (_, state) = run_for(&loaded.to_description(), 0, secs(600));
    assert_eq!(state.objects.len(), 6_006);
}

// ---------- What a file means ----------

#[test]
fn a_dummy_station_passes_mus_on_without_time_passing() {
    let description = built(&file(json!([
        { "name": "In", "class": "Source", "successors": ["Trigger"], "attributes": { "interval": "10" } },
        { "name": "Trigger", "class": "Station", "successors": ["Out"] },
        { "name": "Out", "class": "Drain" },
    ])));
    let (occurrences, _) = run_for(&description, 0, secs(35));
    let left: Vec<(SimTime, u32)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuLeft { mu_id, .. } => Some((o.time, mu_id)),
            _ => None,
        })
        .collect();
    assert_eq!(left, vec![(secs(10), 1), (secs(20), 2), (secs(30), 3)], "each MU leaves the moment the Source lets it go");
}

#[test]
fn a_source_with_an_interval_of_0_waits_for_a_busy_successor() {
    let description = built(&file(json!([
        { "name": "In", "class": "Source", "successors": ["Mill"], "attributes": { "interval": "0" } },
        { "name": "Mill", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "10" } },
        { "name": "Out", "class": "Drain" },
    ])));
    let (_, state) = run_for(&description, 0, secs(100));
    assert_eq!(state.objects[2].statistics.mus_in, 10, "one MU every 10 s, the Mill's pace");
}

#[test]
fn starting_mus_are_numbered_in_file_order() {
    let description = built(&file(json!([
        { "name": "A", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "100", "starting_mus": 1 } },
        { "name": "B", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "100", "starting_mus": 1 } },
        { "name": "C", "class": "Station", "successors": ["Out"], "attributes": { "starting_mus": 0 } },
        { "name": "Out", "class": "Drain" },
    ])));
    let (_, state) = run_for(&description, 0, secs(1));
    let contents: Vec<Vec<u32>> = state.objects.iter().map(|object| object.contents.clone()).collect();
    assert_eq!(contents, vec![vec![1], vec![2], vec![], vec![]]);
}

#[test]
fn names_positions_dimensions_and_descriptions_change_no_result() {
    let plain = load(&demo_file()).unwrap();
    let mut dressed = plain.clone();
    dressed.description = Some("the same model, drawn differently".into());
    for (index, instance) in dressed.instances.iter_mut().enumerate() {
        instance.position = None;
        instance.attributes.dimensions = Some(crabsim::Dimensions { length: 3.0, width: 3.0, height: 1.0, rotation_degrees: 45.0 });
        instance.attributes.description = Some(format!("object {index}"));
    }
    let renamed: Vec<String> = dressed.instances.iter().map(|instance| format!("{} renamed", instance.name)).collect();
    for instance in &mut dressed.instances {
        instance.name = format!("{} renamed", instance.name);
        instance.successors = instance.successors.iter().map(|name| format!("{name} renamed")).collect();
    }
    assert!(renamed.iter().all(|name| name.ends_with("renamed")));
    let (plain_run, _) = run_for(&plain.to_description(), 3, secs(86_400));
    let (dressed_run, _) = run_for(&dressed.to_description(), 3, secs(86_400));
    assert_eq!(plain_run, dressed_run);
}

// ---------- Validation ----------

#[test]
fn every_problem_comes_back_at_once_in_file_order() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Nowhere"] },
        { "name": "Mill", "class": "Station", "attributes": { "proces_time": "10", "starting_mus": 2 } },
        { "name": "Mill", "class": "Drain", "successors": ["In"] },
    ]));
    assert_eq!(
        found(&text),
        vec![
            "/instances/0: a Source needs an `interval`: the time between creating an MU and offering it on",
            "/instances/0/successors/0: no instance is called `Nowhere`",
            "/instances/1/attributes/proces_time: unknown attribute `proces_time`",
            "/instances/1/attributes/starting_mus: a Station can start with 0 or 1 MUs, until Stations have a capacity",
            "/instances/2/name: instance 1 already has this name",
            "/instances/2/successors: a Drain takes MUs out of the model, so it can't have successors",
        ]
    );
}

#[test]
fn a_problem_names_its_instance_and_prints_on_one_line() {
    let text = file(json!([{ "name": "Station 3", "class": "Station", "attributes": { "process_time": "1:75" } }]));
    let problem = &problems(&text)[0];
    assert_eq!(problem.name.as_deref(), Some("Station 3"));
    assert_eq!(
        problem.to_string(),
        "/instances/0/attributes/process_time \"Station 3\": \"1:75\" is not a time: write it as text, such as \"90\", \"1:30\", \"8:00:00\" or \"2d 06:00:00\""
    );
}

#[test]
fn a_file_that_isnt_json_or_has_the_wrong_version_is_refused() {
    assert_found("{ \"format_version\": 1, ", "", "this isn't JSON");
    assert_found(&json!({ "instances": [] }).to_string(), "", "`format_version` is missing");
    assert_found(&json!({ "format_version": 2, "instances": [] }).to_string(), "/format_version", "format version 2 isn't supported");
    assert_found(&json!({ "format_version": 1 }).to_string(), "", "`instances` is missing");
    assert_found(&json!({ "format_version": 1, "instances": [], "seed": 3 }).to_string(), "/seed", "unknown field `seed`");
}

#[test]
fn derived_classes_wait_for_the_next_slice() {
    let text = json!({ "format_version": 1, "classes": [{ "name": "CNC mill", "parent": "Station" }], "instances": [] }).to_string();
    assert_found(&text, "/classes/0", "derived classes aren't supported yet");
    assert_found(&file(json!([{ "name": "M", "class": "CNC mill" }])), "/instances/0/class", "unknown class `CNC mill`");
}

#[test]
fn names_must_be_present_unique_and_without_spaces_at_either_end() {
    assert_found(&file(json!([{ "class": "Drain" }])), "/instances/0", "`name` is missing");
    assert_found(&file(json!([{ "name": "", "class": "Drain" }])), "/instances/0/name", "can't be empty");
    assert_found(&file(json!([{ "name": " Out", "class": "Drain" }])), "/instances/0/name", "can't start or end with spaces");
    assert_found(&file(json!([{ "name": 3, "class": "Drain" }])), "/instances/0/name", "`name` should be text");
    // Names compare exactly, so these two are different.
    assert!(load(&file(json!([{ "name": "Out", "class": "Drain" }, { "name": "out", "class": "Drain" }]))).is_ok());
}

#[test]
fn values_out_of_range_are_refused() {
    let station = |attributes: Value| file(json!([{ "name": "S", "class": "Station", "attributes": attributes }]));
    let failures = |availability: f64, repair_time: Value| station(json!({ "failures": { "availability": availability, "repair_time": repair_time } }));
    assert_found(&failures(1.0, json!({ "constant": "1:00" })), "/instances/0/attributes/failures/availability", "above 0 and below 1");
    assert_found(&failures(0.0, json!({ "constant": "1:00" })), "/instances/0/attributes/failures/availability", "above 0 and below 1");
    assert_found(&failures(0.9, json!({ "constant": "0" })), "/instances/0/attributes/failures/repair_time/constant", "has to take some time");
    assert_found(&failures(0.9, json!({ "erlang": { "mean": "1:00", "k": 0 } })), "/instances/0/attributes/failures/repair_time/erlang/k", "at least 1");
    assert_found(&failures(0.9, json!({ "uniform": { "min": "2:00", "max": "1:00" } })), "/instances/0/attributes/failures/repair_time/uniform", "`min` can't be more than `max`");
    assert_found(&failures(0.9, json!({ "constant": "1:00", "normal": { "mean": "1:00", "std": "0" } })), "/instances/0/attributes/failures/repair_time", "exactly one of");
    assert_found(&station(json!({ "dimensions": { "length": 0, "width": 1, "height": 1 } })), "/instances/0/attributes/dimensions/length", "above 0");
    assert_found(&file(json!([{ "name": "B", "class": "Buffer", "attributes": { "capacity": 0 } }])), "/instances/0/attributes/capacity", "at least 1");
    assert_found(&file(json!([{ "name": "B", "class": "Buffer" }])), "/instances/0", "a Buffer needs a `capacity`");
    assert_found(&station(json!({ "starting_mus": -1 })), "/instances/0/attributes/starting_mus", "a whole number");
}

#[test]
fn an_attribute_its_class_doesnt_have_is_refused() {
    let text = file(json!([{ "name": "Out", "class": "Drain", "attributes": { "process_time": "10", "description": "the end" } }]));
    assert_eq!(found(&text), vec!["/instances/0/attributes/process_time: a Drain has no attribute `process_time`"]);
}

#[test]
fn a_value_that_cant_be_read_isnt_also_reported_missing() {
    let text = file(json!([{ "name": "In", "class": "Source", "attributes": { "interval": "soon" } }]));
    assert_eq!(found(&text).len(), 1, "{:#?}", found(&text));
}

#[test]
fn illegal_connections_are_refused() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Mill", "Mill"], "attributes": { "interval": "5" } },
        { "name": "Mill", "class": "Station", "successors": ["Mill", "In", "Out"] },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_found(&text, "/instances/0/successors/1", "`Mill` is listed twice");
    assert_found(&text, "/instances/1/successors/0", "can't be its own successor");
    assert_found(&text, "/instances/1/successors/1", "`In` is a Source");
}

#[test]
fn a_source_that_would_create_mus_forever_is_refused() {
    let text = file(json!([
        { "name": "Feeder", "class": "Source", "successors": ["Slow", "Trigger"], "attributes": { "interval": "0" } },
        { "name": "Slow", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "10" } },
        { "name": "Trigger", "class": "Station", "successors": ["Check"] },
        { "name": "Check", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "0" } },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_eq!(
        found(&text),
        vec![
            "/instances/0/attributes/interval: with an interval of 0 this Source reaches Drain `Out` through zero-time objects \
             (Trigger, Check), so nothing ever blocks it and it would create MUs forever without time passing"
        ]
    );
    let straight_into_a_drain = file(json!([
        { "name": "Feeder", "class": "Source", "successors": ["Out"], "attributes": { "interval": "0" } },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_found(&straight_into_a_drain, "/instances/0/attributes/interval", "reaches Drain `Out`, so nothing ever blocks it");
}

#[test]
fn odd_shapes_are_problems_not_panics() {
    for text in [
        "null",
        "[]",
        "{\"format_version\": 1, \"instances\": {}}",
        "{\"format_version\": 1, \"instances\": [null, 3, []]}",
        "{\"format_version\": 1, \"instances\": [{\"name\": \"A\", \"class\": \"Station\", \"attributes\": [], \"position\": 3, \"successors\": \"B\"}]}",
        "{\"format_version\": 1, \"instances\": [{\"name\": \"A\", \"class\": \"Station\", \"attributes\": {\"failures\": {\"repair_time\": {}}, \"dimensions\": {}}}]}",
        "{\"format_version\": 1, \"classes\": 3, \"instances\": []}",
    ] {
        assert!(!problems(text).is_empty(), "{text}");
    }
}

#[test]
fn problems_can_be_sent_as_json() {
    let text = file(json!([{ "name": "Out", "class": "Drain", "successors": ["Out"] }]));
    let as_json = serde_json::to_value(problems(&text)).unwrap();
    assert_eq!(
        as_json,
        json!([{ "pointer": "/instances/0/successors", "name": "Out", "message": "a Drain takes MUs out of the model, so it can't have successors" }])
    );
}
