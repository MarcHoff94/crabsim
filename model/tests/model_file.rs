//! Model files at their seam: text in, a model, its problems or a run out.

use crabsim::{demo, ModelDescription, OccurrenceKind, Recording, SimTime, SimulationRun};
use crabsim_model::{load, Attribute, Capacity, Class, ExitStrategy, Instance, Model, Origin, Problem, StartMode, State};
use serde_json::{json, Value};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// The demo model's file, with line endings as crabsim writes them, whatever git did.
fn demo_file() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/demo.json");
    std::fs::read_to_string(path).expect("models/demo.json exists").replace("\r\n", "\n")
}

/// A model file holding these instances. For short tests an instance may list its `successors`,
/// which become connections, numbered in the order they come.
fn file(instances: Value) -> String {
    let (instances, connections) = connected(instances);
    json!({ "format_version": 2, "instances": instances, "connections": connections }).to_string()
}

/// `instances` without their `successors`, and the connections they list: each successor's exit
/// is its place in the list, and each object's entrances follow the order connections come in.
fn connected(mut instances: Value) -> (Value, Value) {
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
    (instances, Value::Array(connections))
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
  "format_version": 2,
  "instances": [
    {
      "name": "In",
      "class": "Source",
      "stream": 0,
      "position": {
        "x": 1.0,
        "y": 2.0
      },
      "attributes": {
        "interval": "00:01:30"
      }
    },
    {
      "name": "Out",
      "class": "Drain",
      "stream": 1
    }
  ],
  "connections": [
    {
      "from": "In",
      "to": "Out",
      "exit": 1,
      "entrance": 1
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
    let mut model =
        Model { description: Some(format!("{copies} copies of the demo model")), classes: demo.classes.clone(), ..Model::default() };
    for copy in 0..copies {
        let rename = |name: &str| format!("Copy {copy} / {name}");
        for instance in &demo.instances {
            let mut instance: Instance = instance.clone();
            instance.name = rename(&instance.name);
            instance.stream = instance.stream.map(|stream| stream + 100 * copy as u64);
            if let Some(position) = &mut instance.position {
                position.y += 20.0 * copy as f64;
            }
            model.instances.push(instance);
        }
        for connection in &demo.connections {
            let mut connection = connection.clone();
            connection.from = rename(&connection.from);
            connection.to = rename(&connection.to);
            model.connections.push(connection);
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
    }
    for connection in &mut dressed.connections {
        connection.from = format!("{} renamed", connection.from);
        connection.to = format!("{} renamed", connection.to);
        connection.description = Some("described, which changes nothing either".into());
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
        { "name": "In", "class": "Source", "successors": ["Nowhere"], "attributes": { "interval": "10" } },
        { "name": "Mill", "class": "Station", "attributes": { "proces_time": "10", "starting_mus": 2 } },
        { "name": "Mill", "class": "Drain", "successors": ["In"] },
        { "name": "B", "class": "Buffer" },
    ]));
    assert_eq!(
        found(&text),
        vec![
            "/instances/1/attributes/proces_time: unknown attribute `proces_time`: did you mean `process_time`?",
            "/instances/1/attributes/starting_mus: it starts with 2 MUs but holds only 1",
            "/instances/2/name: instance 1 already has this name",
            "/connections/0/to: no instance is called `Nowhere`",
            "/connections/1/to: `In` is a Source, which creates MUs and can't take any in",
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
        "/instances/0/attributes/process_time \"Station 3\": \"1:75\" is not a time, which is written such as \"90\", \"1:30\", \"8:00:00\" or \"2d 06:00:00\""
    );
}

#[test]
fn a_time_with_a_unit_letter_or_not_written_as_text_says_so() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "interval": "10s" } },
        { "name": "Mill", "class": "Station", "attributes": { "process_time": 10 } },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_eq!(
        found(&text),
        vec![
            "/instances/0/attributes/interval: \"10s\" is not a time: a time has no unit letters, apart from d for days, and is written such as \"10\", \"0:10\", \"8:00:00\" or \"2d 06:00:00\"",
            "/instances/1/attributes/process_time: 10 is not a time: write it as text, such as \"90\", \"1:30\", \"8:00:00\" or \"2d 06:00:00\"",
        ]
    );
}

#[test]
fn a_name_one_or_two_edits_from_a_known_one_gets_a_did_you_mean() {
    let text = with_classes(
        json!([
            { "name": "CNC mill", "parent": "Statin" },
        ]),
        json!([
            { "name": "In", "class": "Source", "clas": "Source", "attributes": { "interval": "10" } },
            { "name": "Station 2", "class": "Statoin", "successors": ["Out"] },
            { "name": "Station 3", "class": "Station", "successors": ["Otu"], "attributes": { "proces_time": "10" } },
            { "name": "Out", "class": "Drain" },
        ]),
    );
    assert_eq!(
        found(&text),
        vec![
            "/classes/0/parent: unknown class `Statin` (did you mean `Station`?): its attributes weren't checked",
            "/instances/0/clas: unknown field `clas` in an instance: did you mean `class`?",
            "/instances/1/class: unknown class `Statoin` (did you mean `Station`?): its attributes weren't checked",
            "/instances/2/attributes/proces_time: unknown attribute `proces_time`: did you mean `process_time`?",
            "/connections/1/to: no instance is called `Otu`: did you mean `Out`?",
        ]
    );
    // Nothing close: no guess.
    assert_found(&file(json!([{ "name": "M", "class": "Machine" }])), "/instances/0/class", "unknown class `Machine`: its attributes weren't checked");
    assert!(!found(&file(json!([{ "name": "In", "class": "Source", "successors": ["Elsewhere"], "attributes": { "interval": "10" } }]))).iter().any(|problem| problem.contains("did you mean")));
}

#[test]
fn a_file_that_isnt_json_or_has_the_wrong_version_is_refused() {
    assert_found("{ \"format_version\": 2, ", "", "this isn't JSON");
    assert_found(&json!({ "instances": [] }).to_string(), "", "`format_version` is missing");
    assert_found(&json!({ "format_version": 3, "instances": [] }).to_string(), "/format_version", "format version 3 isn't supported");
    assert_found(&json!({ "format_version": 2 }).to_string(), "", "`instances` is missing");
    assert_found(&json!({ "format_version": 2, "instances": [], "seed": 3 }).to_string(), "/seed", "unknown field `seed`");
}

#[test]
fn a_version_1_file_is_refused_with_one_problem_saying_what_changed() {
    let version_1 = json!({ "format_version": 1, "instances": [
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "interval": "10" } },
        { "name": "Out", "class": "Drain" },
    ] });
    let problems = problems(&version_1.to_string());
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(problems[0].pointer, "/format_version");
    assert!(problems[0].message.contains("connections are their own list"), "{}", problems[0].message);
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
    assert_found(&failures(1.0, json!({ "constant": "1:00" })), "/instances/0/attributes/failures/0/availability", "above 0 and below 1");
    assert_found(&failures(0.0, json!({ "constant": "1:00" })), "/instances/0/attributes/failures/0/availability", "above 0 and below 1");
    assert_found(&failures(0.9, json!({ "constant": "0" })), "/instances/0/attributes/failures/0/repair_time/constant", "has to take some time");
    assert_found(&failures(0.9, json!({ "erlang": { "mean": "1:00", "k": 0 } })), "/instances/0/attributes/failures/0/repair_time/erlang/k", "at least 1");
    assert_found(&failures(0.9, json!({ "uniform": { "min": "2:00", "max": "1:00" } })), "/instances/0/attributes/failures/0/repair_time/uniform", "`min` can't be more than `max`");
    assert_found(&failures(0.9, json!({ "constant": "1:00", "normal": { "mean": "1:00", "std": "0" } })), "/instances/0/attributes/failures/repair_time", "exactly one of");
    assert_found(&station(json!({ "dimensions": { "length": 0, "width": 1, "height": 1 } })), "/instances/0/attributes/dimensions/length", "above 0");
    assert_found(&file(json!([{ "name": "B", "class": "Buffer", "attributes": { "capacity": 0 } }])), "/instances/0/attributes/capacity", "at least 1");
    let buffer = |attributes: Value| file(json!([{ "name": "B", "class": "Buffer", "attributes": attributes }]));
    assert_found(&buffer(json!({ "capacity": -2 })), "/instances/0/attributes/capacity", "a whole number from 1, or \"unlimited\"");
    assert_found(&buffer(json!({ "capacity": "lots" })), "/instances/0/attributes/capacity", "a whole number from 1, or \"unlimited\"");
    assert_found(&buffer(json!({ "exit_strategy": "random" })), "/instances/0/attributes/exit_strategy", "\"first_in_first_out\" or \"last_in_first_out\"");
    assert_found(&buffer(json!({ "dwelling_time": 5 })), "/instances/0/attributes/dwelling_time", "is not a time");
    assert_found(&buffer(json!({ "capacity": 2, "starting_mus": 3 })), "/instances/0/attributes/starting_mus", "it starts with 3 MUs but holds only 2");
    assert_found(&buffer(json!({ "starting_mus": 1_000_001 })), "/instances/0/attributes/starting_mus", "at most 1,000,000");
    assert_found(&station(json!({ "dwelling_time": "5" })), "/instances/0/attributes/dwelling_time", "a Station has no attribute `dwelling_time`");
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

/// A file with three instances and exactly these connections.
fn with_connections(connections: Value) -> String {
    json!({ "format_version": 2, "instances": [
        { "name": "In", "class": "Source", "attributes": { "interval": "5" } },
        { "name": "Mill", "class": "Station" },
        { "name": "Out", "class": "Drain" },
    ], "connections": connections })
    .to_string()
}

#[test]
fn illegal_connections_are_refused() {
    let text = with_connections(json!([
        { "from": "In", "to": "Mill", "exit": 1, "entrance": 1 },
        { "from": "In", "to": "Mill", "exit": 2, "entrance": 2 },
        { "from": "Mill", "to": "Mill", "exit": 1, "entrance": 3 },
        { "from": "Mill", "to": "In", "exit": 2, "entrance": 1 },
        { "from": "Out", "to": "Mill", "exit": 1, "entrance": 4 },
        { "from": "Mill", "to": "Elsewhere", "exit": 3, "entrance": 1 },
    ]));
    assert_found(&text, "/connections/1", "connection 0 already connects `In` to `Mill`");
    assert_found(&text, "/connections/2/to", "can't be its own successor");
    assert_found(&text, "/connections/3/to", "`In` is a Source");
    assert_found(&text, "/connections/4/from", "`Out` is a Drain");
    assert_found(&text, "/connections/5/to", "no instance is called `Elsewhere`");
    assert_eq!(problems(&text)[0].name.as_deref(), Some("In → Mill"), "a problem names the connection by its ends");
}

#[test]
fn exits_and_entrances_run_from_1_with_no_gaps_or_repeats() {
    let text = with_connections(json!([
        { "from": "In", "to": "Mill", "exit": 1, "entrance": 1 },
        { "from": "Mill", "to": "Out", "exit": 2, "entrance": 1 },
    ]));
    assert_eq!(found(&text), vec!["/connections/1/exit: 1 connection leads out of `Mill`, so its exits are numbered 1, not 2"]);

    let mut model = Model::default();
    for (name, class) in [("A", "Source"), ("B", "Source"), ("X", "Station"), ("Y", "Station"), ("Out", "Drain")] {
        let mut instance = Instance::new(name, class);
        if class == "Source" {
            instance.attributes.interval = Some(secs(5));
        }
        model.instances.push(instance);
    }
    // The example in ADR 0014: no order of the list gives these four, so both are written.
    for (from, to, exit, entrance) in [("A", "X", 1, 2), ("A", "Y", 2, 1), ("B", "Y", 1, 2), ("B", "X", 2, 1)] {
        model.connections.push(crabsim_model::Connection::new(from, to, exit, entrance));
    }
    model.connect("X", "Out");
    model.connect("Y", "Out");
    assert_eq!(model.check(), vec![]);
    assert_eq!(model.successors("B"), vec!["Y", "X"]);
    assert_eq!(model.predecessors("X"), vec!["B", "A"]);
    assert_eq!(model.predecessors("Out"), vec!["X", "Y"]);

    let mut repeated = model.clone();
    repeated.connections[1].exit = 1;
    repeated.connections[3].entrance = 2;
    let found: Vec<String> = repeated.check().iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect();
    assert_eq!(
        found,
        vec![
            "/connections/1/exit: connection 0 out of `A` already has exit 1",
            "/connections/3/entrance: connection 0 into `X` already has entrance 2",
        ]
    );
    for (field, value) in [("exit", json!(0)), ("entrance", json!("first"))] {
        let mut broken = json!({ "from": "In", "to": "Mill", "exit": 1, "entrance": 1 });
        broken[field] = value;
        let text = with_connections(json!([broken, { "from": "Mill", "to": "Out", "exit": 1, "entrance": 1 }]));
        assert_found(&text, &format!("/connections/0/{field}"), "is not a place: write a whole number, 1 or more");
    }
}

#[test]
fn the_list_of_connections_can_come_in_any_order_and_saves_in_one() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["A", "B"], "attributes": { "interval": "5" } },
        { "name": "A", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "8" } },
        { "name": "B", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "9" } },
        { "name": "Out", "class": "Drain" },
    ]));
    let mut shuffled: Value = serde_json::from_str(&text).unwrap();
    shuffled["connections"].as_array_mut().unwrap().reverse();
    let shuffled = shuffled.to_string();
    assert_eq!(load(&shuffled).unwrap().save(), load(&text).unwrap().save());
    assert_eq!(run_for(&built(&shuffled), 1, secs(3_600)), run_for(&built(&text), 1, secs(3_600)));
}

#[test]
fn a_connection_keeps_its_description_and_the_exit_order_is_the_order_mus_are_offered() {
    let model = load(&with_connections(json!([
        { "from": "In", "to": "Out", "exit": 2, "entrance": 2 },
        { "from": "In", "to": "Mill", "exit": 1, "entrance": 1, "description": "via the lift" },
        { "from": "Mill", "to": "Out", "exit": 1, "entrance": 1 },
    ])))
    .unwrap();
    assert_eq!(model.connections_out("In")[0].description.as_deref(), Some("via the lift"));
    assert!(model.save().contains("\"description\": \"via the lift\""));
    let description = model.to_description();
    let (mill, out) = (1, 2);
    assert_eq!(description.successors(0), &[mill, out], "Mill first: its exit is 1, though it comes second in the list");
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
    // A Source's interval is 0 by default, so one placed and connected straight to a Drain is
    // refused until it gets one.
    let default_into_a_drain = file(json!([
        { "name": "Feeder", "class": "Source", "successors": ["Out"] },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_found(&default_into_a_drain, "/instances/0", "reaches Drain `Out`, so nothing ever blocks it");
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
        "{\"format_version\": 2, \"instances\": {}}",
        "{\"format_version\": 2, \"instances\": [null, 3, []]}",
        "{\"format_version\": 2, \"instances\": [{\"name\": \"A\", \"class\": \"Station\", \"attributes\": [], \"position\": 3}]}",
        "{\"format_version\": 2, \"instances\": [{\"name\": \"A\", \"class\": \"Station\", \"attributes\": {\"failures\": {\"repair_time\": {}}, \"dimensions\": {}}}]}",
        "{\"format_version\": 2, \"classes\": 3, \"instances\": []}",
        "{\"format_version\": 2, \"instances\": [], \"connections\": 3}",
        "{\"format_version\": 2, \"instances\": [], \"connections\": [null, 3, {\"from\": 1, \"exit\": -1}]}",
    ] {
        assert!(!problems(text).is_empty(), "{text}");
    }
}

#[test]
fn problems_can_be_sent_as_json() {
    let text = file(json!([{ "name": "Out", "class": "Drain", "successors": ["Out"] }]));
    let as_json = serde_json::to_value(problems(&text)).unwrap();
    assert_eq!(as_json, json!([{ "pointer": "/connections/0/to", "name": "Out → Out", "message": "an object can't be its own successor" }]));
}

// ---------- Classes and inheritance ----------

/// A file with these classes and instances.
fn with_classes(classes: Value, instances: Value) -> String {
    let (instances, connections) = connected(instances);
    json!({ "format_version": 2, "classes": classes, "instances": instances, "connections": connections }).to_string()
}

/// Mills: a Mill, a CNC mill derived from it, and instances of both.
fn mills() -> String {
    with_classes(
        json!([
            // A child may come before its parent.
            { "name": "CNC mill", "parent": "Mill", "attributes": { "process_time": "45", "description": "5-axis" } },
            { "name": "Mill", "parent": "Station", "attributes": {
                "process_time": "30",
                "failures": { "availability": 0.95, "repair_time": { "constant": "2:00" } } } },
        ]),
        json!([
            { "name": "In", "class": "Source", "successors": ["A", "B", "C"], "attributes": { "interval": "5" } },
            { "name": "A", "class": "CNC mill", "successors": ["Out"] },
            { "name": "B", "class": "CNC mill", "successors": ["Out"], "attributes": { "process_time": "50" } },
            { "name": "C", "class": "Mill", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" },
        ]),
    )
}

#[test]
fn values_are_inherited_through_a_chain_of_classes_and_say_where_they_come_from() {
    let model = load(&mills()).unwrap();
    let class = |name: &str| Origin::Class(name.to_string());

    let a = model.resolve(1).unwrap();
    assert_eq!(a.built_in, "Station");
    assert_eq!(a.attributes.process_time, Some(secs(45)));
    assert_eq!(a.origin(Attribute::ProcessTime), &class("CNC mill"));
    assert_eq!(a.attributes.failures.as_ref().map(|f| f[0].availability), Some(0.95));
    assert_eq!(a.origin(Attribute::Failures), &class("Mill"));
    assert_eq!(a.attributes.description.as_deref(), Some("5-axis"));
    assert_eq!(a.attributes.starting_mus, Some(0));
    assert_eq!(a.origin(Attribute::StartingMus), &Origin::BuiltIn);
    assert_eq!(a.origin(Attribute::Dimensions), &Origin::BuiltIn);

    let b = model.resolve(2).unwrap();
    assert_eq!((b.attributes.process_time, b.origin(Attribute::ProcessTime)), (Some(secs(50)), &Origin::Instance), "set here, so cut");
    assert_eq!(b.origin(Attribute::Failures), &class("Mill"), "cut only for what it sets");

    let c = model.resolve(3).unwrap();
    assert_eq!((c.attributes.process_time, c.origin(Attribute::ProcessTime)), (Some(secs(30)), &class("Mill")));
    assert_eq!((c.attributes.description.clone(), c.origin(Attribute::Description)), (None, &Origin::BuiltIn));
}

#[test]
fn a_class_change_reaches_exactly_the_instances_that_inherit_it() {
    // 20 CNC mills, of which two set their own process time.
    let mut instances = vec![json!({ "name": "In", "class": "Source", "attributes": { "interval": "1" },
        "successors": (0..20).map(|i| format!("Mill {i}")).collect::<Vec<_>>() })];
    for i in 0..20 {
        let own = if i == 3 || i == 11 { json!({ "process_time": "7" }) } else { json!({}) };
        instances.push(json!({ "name": format!("Mill {i}"), "class": "CNC mill", "successors": ["Out"], "attributes": own }));
    }
    instances.push(json!({ "name": "Out", "class": "Drain" }));
    let classes = json!([{ "name": "CNC mill", "parent": "Station", "attributes": { "process_time": "45" } }]);
    let mut model = load(&with_classes(classes, json!(instances))).unwrap();

    model.classes[0].attributes.process_time = Some(secs(60));

    let mut run = model.to_description().build(0);
    run.init().unwrap();
    let times: Vec<u64> = run.state().objects[1..=20].iter().map(|o| o.process_time.unwrap().as_secs()).collect();
    let expected: Vec<u64> = (0..20).map(|i| if i == 3 || i == 11 { 7 } else { 60 }).collect();
    assert_eq!(times, expected);
}

#[test]
fn a_value_nobody_set_is_the_built_in_default() {
    let classes = json!([{ "name": "Store", "parent": "Buffer", "attributes": { "capacity": 5 } }]);
    let instances = json!([{ "name": "Shelf", "class": "Store" }]);
    let model = load(&with_classes(classes, instances)).unwrap();
    assert_eq!(model.resolve(0).unwrap().attributes.capacity, Some(Capacity::Limited(5)));
    assert_eq!(model.resolve(0).unwrap().origin(Attribute::Capacity), &Origin::Class("Store".into()));
    // A Buffer nobody gave a capacity has no limit.
    let unset = with_classes(json!([{ "name": "Store", "parent": "Buffer" }]), json!([{ "name": "Shelf", "class": "Store" }]));
    let model = load(&unset).unwrap();
    let shelf = model.resolve(0).unwrap();
    assert_eq!(shelf.attributes.capacity, Some(Capacity::Unlimited));
    assert_eq!(shelf.origin(Attribute::Capacity), &Origin::BuiltIn);
    assert_eq!((shelf.attributes.dwelling_time, shelf.attributes.exit_strategy, shelf.attributes.starting_mus), (Some(SimTime::ZERO), Some(ExitStrategy::FirstInFirstOut), Some(0)));
    // A Source's interval is 0 by default.
    let source = with_classes(json!([]), json!([{ "name": "In", "class": "Source" }]));
    let model = load(&source).unwrap();
    assert_eq!(model.resolve(0).unwrap().attributes.interval, Some(SimTime::ZERO));
    assert_eq!(model.resolve(0).unwrap().origin(Attribute::Interval), &Origin::BuiltIn);
}

#[test]
fn class_problems_are_reported_with_the_class() {
    let text = with_classes(
        json!([
            { "name": "Loop A", "parent": "Loop B" },
            { "name": "Loop B", "parent": "Loop A" },
            { "name": "Orphan", "parent": "Nothing" },
            { "name": "Station", "parent": "Station" },
            { "name": "Exit", "parent": "Drain", "attributes": { "process_time": "10" } },
            { "name": "Exit", "parent": "Drain" },
            { "name": "Flaky", "parent": "Station", "attributes": { "failures": { "availability": 1.5, "repair_time": { "constant": "1:00" } } } },
            { "name": " Padded", "parent": "Station" },
        ]),
        json!([
            { "name": "L", "class": "Loop A" },
            { "name": "M", "class": "Machine" },
        ]),
    );
    assert_eq!(
        found(&text),
        vec![
            "/classes/0/parent: the classes derive from each other in a circle: Loop A \u{2192} Loop B \u{2192} Loop A",
            "/classes/1/parent: the classes derive from each other in a circle: Loop B \u{2192} Loop A \u{2192} Loop B",
            "/classes/2/parent: unknown class `Nothing`: its attributes weren't checked",
            "/classes/3/parent: a class named `Station` is this model's own values for the built-in class, so it has no parent",
            "/classes/4/attributes/process_time: a Drain has no attribute `process_time`",
            "/classes/5/name: class 4 already has this name",
            "/classes/6/attributes/failures/0/availability: an availability must be above 0 and below 1, not 1.5: leave out `failures` for one that never fails",
            "/classes/7/name: a name can't start or end with spaces",
            "/instances/1/class: unknown class `Machine`: its attributes weren't checked",
        ]
    );
    assert_eq!(problems(&text)[0].name.as_deref(), Some("Loop A"));
}

#[test]
fn a_class_named_like_a_built_in_is_the_models_own_values_for_it() {
    let text = with_classes(
        json!([
            { "name": "Station", "attributes": { "process_time": "10", "description": "ours" } },
            { "name": "Mill", "parent": "Station", "attributes": { "process_time": "45" } },
            { "name": "Station", "attributes": { "starting_mus": 1 } },
        ]),
        json!([{ "name": "M", "class": "Mill" }, { "name": "S", "class": "Station" }]),
    );
    assert_eq!(found(&text), vec!["/classes/2/name: class 0 already has this name"]);
    let text = with_classes(
        json!([
            { "name": "Station", "attributes": { "process_time": "10", "description": "ours" } },
            { "name": "Mill", "parent": "Station", "attributes": { "process_time": "45" } },
        ]),
        json!([{ "name": "M", "class": "Mill" }, { "name": "S", "class": "Station" }]),
    );
    let model = load(&text).unwrap();
    // An instance of the built-in class, and one of a class derived from it, both inherit it.
    let station = model.resolve(1).unwrap();
    assert_eq!(station.attributes.process_time, Some(secs(10)));
    assert_eq!(station.origin(Attribute::ProcessTime), &Origin::Class("Station".to_string()));
    let mill = model.resolve(0).unwrap();
    assert_eq!(mill.attributes.process_time, Some(secs(45)));
    assert_eq!(mill.attributes.description.as_deref(), Some("ours"));
    assert_eq!(mill.origin(Attribute::Description), &Origin::Class("Station".to_string()));
    assert_eq!(model.chain("Mill").map(|(chain, built_in)| (chain.len(), built_in)), Some((2, "Station")));
    // Saved without a parent, and read back the same.
    let saved = model.save();
    assert!(saved.contains("\"name\": \"Station\",
      \"attributes\""), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    // Its attributes are checked against the built-in class it refines.
    let wrong = with_classes(json!([{ "name": "Drain", "attributes": { "process_time": "10" } }]), json!([]));
    assert_eq!(found(&wrong), vec!["/classes/0/attributes/process_time: a Drain has no attribute `process_time`"]);
}

#[test]
fn a_distribution_strategy_is_read_inherited_as_a_whole_and_saved() {
    use crabsim_model::{DistributionStrategy, WeightedStrategy};
    use crabsim::distribution::{Order, Rank};
    let text = with_classes(
        json!([
            { "name": "Router", "parent": "Station", "attributes": { "distribution": { "weighted": { "by": "process_time", "order": "largest", "drains": ["Out"] } } } },
        ]),
        json!([
            { "name": "R", "class": "Router" },
            { "name": "Plain", "class": "Router", "attributes": { "distribution": "round_robin" } },
            { "name": "S", "class": "Station" },
        ]),
    );
    let model = load(&text).unwrap();
    let router = DistributionStrategy::Weighted(WeightedStrategy {
        by: Attribute::ProcessTime,
        order: Order::Largest,
        rank: Rank::Route,
        drains: Some(vec!["Out".into()]),
    });
    assert_eq!(model.resolve(0).unwrap().attributes.distribution, Some(router));
    assert_eq!(model.resolve(0).unwrap().origin(Attribute::Distribution), &Origin::Class("Router".into()));
    assert_eq!(model.resolve(1).unwrap().attributes.distribution, Some(DistributionStrategy::RoundRobin), "replaced as a whole");
    assert_eq!(model.resolve(2).unwrap().attributes.distribution, None, "round robin by default");
    // Saved with its defaults written out, and read back the same.
    let saved = model.save();
    assert!(saved.contains(r#""rank": "route""#), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
}

#[test]
fn a_distribution_is_checked_where_it_is_set() {
    let text = with_classes(
        json!([]),
        json!([
            { "name": "A", "class": "Station", "attributes": { "distribution": "random" } },
            { "name": "B", "class": "Station", "attributes": { "distribution": { "weighted": { "by": "proces_time" } } } },
            { "name": "C", "class": "Station", "attributes": { "distribution": { "weighted": { "by": "process_time", "order": "most", "drains": [] } } } },
            { "name": "D", "class": "Drain", "attributes": { "distribution": "round_robin" } },
            { "name": "E", "class": "Station", "attributes": { "distribution": { "weighted": { "by": "interval", "drains": ["Nowhere"] } } } },
        ]),
    );
    let found = found(&text);
    assert!(found[0].starts_with("/instances/0/attributes/distribution: unknown distribution `random`"), "{found:#?}");
    assert!(found[1].contains("/instances/1/attributes/distribution/weighted/by") && found[1].contains("did you mean `process_time`"), "{found:#?}");
    assert!(found.iter().any(|problem| problem.contains("/instances/2/attributes/distribution/weighted/order: `order` is `smallest` or `largest`")), "{found:#?}");
    assert!(found.iter().any(|problem| problem.contains("/instances/2/attributes/distribution/weighted/drains: a list of drains can't be empty")), "{found:#?}");
    assert!(found.iter().any(|problem| problem == "/instances/3/attributes/distribution: a Drain has no attribute `distribution`"), "{found:#?}");
    assert!(!found.iter().any(|problem| problem.starts_with("/instances/4")), "a Drain list naming one that isn't there is only a warning: {found:#?}");
}

#[test]
fn a_weighted_distribution_reaches_the_run() {
    // In into Split, which sends to Fast (2 s) then Slow (30 s) then Out, or to Mid (10 s) then
    // Out: the lightest route is through Mid.
    let text = file(json!([
        { "name": "In", "class": "Source", "attributes": { "interval": "20" } },
        { "name": "Split", "class": "Station", "attributes": { "distribution": { "weighted": { "by": "process_time" } } } },
        { "name": "Fast", "class": "Station", "attributes": { "process_time": "2" } },
        { "name": "Slow", "class": "Station", "attributes": { "process_time": "30" } },
        { "name": "Mid", "class": "Station", "attributes": { "process_time": "10" } },
        { "name": "Out", "class": "Drain" },
    ]));
    let mut model = load(&text).unwrap();
    for (from, to) in [("In", "Split"), ("Split", "Fast"), ("Split", "Mid"), ("Fast", "Slow"), ("Slow", "Out"), ("Mid", "Out")] {
        model.connect(from, to);
    }
    let description = model.to_description();
    assert_eq!(description.ranking(1).unwrap().groups, vec![vec![1], vec![0]], "Mid, the second exit, first");
    let (_, state) = run_for(&description, 0, secs(3600));
    assert_eq!(state.objects[2].statistics.mus_in, 0, "nothing to Fast");
    assert_eq!(state.objects[4].statistics.mus_in, 180);
}

#[test]
fn classes_and_instances_are_separate_namespaces() {
    let text = with_classes(json!([{ "name": "Out", "parent": "Drain" }]), json!([{ "name": "Out", "class": "Out" }]));
    assert!(load(&text).is_ok());
}

#[test]
fn the_endless_source_check_follows_inherited_values() {
    let text = with_classes(
        json!([
            { "name": "Feeder", "parent": "Source", "attributes": { "interval": "0" } },
            { "name": "Trigger", "parent": "Station" },
        ]),
        json!([
            { "name": "In", "class": "Feeder", "successors": ["T"] },
            { "name": "T", "class": "Trigger", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" },
        ]),
    );
    assert_found(&text, "/instances/0", "reaches Drain `Out` through zero-time objects (T)");
}

#[test]
fn a_model_with_classes_round_trips() {
    let model = load(&mills()).unwrap();
    let saved = model.save();
    assert!(saved.find("\"classes\"").unwrap() < saved.find("\"instances\"").unwrap(), "classes are saved first");
    assert_eq!(load(&saved).unwrap(), model, "the classes keep the order they were written in");
    assert_eq!(load(&saved).unwrap().save(), saved);
}

#[test]
fn a_class_built_in_code_is_checked_like_one_in_a_file() {
    let mut model = Model::default();
    model.classes.push(Class::new("Mill", "Station"));
    model.instances.push(Instance::new("A", "Mill"));
    assert_eq!(model.check(), vec![]);
    model.classes[0].parent = "Mill".into();
    assert_eq!(model.check().len(), 1);
}

// ---------- Streams ----------

fn streams(text: &str) -> Vec<u64> {
    load(text).expect("valid").instances.iter().map(|instance| instance.stream.expect("loading gives every instance a stream")).collect()
}

#[test]
fn a_file_without_streams_numbers_its_instances_by_place_so_old_results_stay() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Work"], "attributes": { "interval": "10" } },
        { "name": "Work", "class": "Station", "successors": ["Out"] },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_eq!(streams(&text), [0, 1, 2]);
    let saved = load(&text).unwrap().save();
    assert!(saved.contains("\"stream\": 1"), "saving writes every stream: {saved}");
    assert_eq!(streams(&saved), [0, 1, 2]);
}

#[test]
fn an_instance_without_a_stream_gets_one_more_than_the_highest() {
    let text = file(json!([
        { "name": "In", "class": "Source", "stream": 5, "successors": ["A"], "attributes": { "interval": "10" } },
        { "name": "A", "class": "Station", "successors": ["B"] },
        { "name": "B", "class": "Station", "stream": 2, "successors": ["Out"] },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_eq!(streams(&text), [5, 6, 2, 7]);
}

#[test]
fn two_instances_with_one_stream_or_a_stream_that_isnt_a_number_are_problems() {
    let text = file(json!([
        { "name": "In", "class": "Source", "stream": 4, "successors": ["Out"], "attributes": { "interval": "10" } },
        { "name": "Out", "class": "Drain", "stream": 4 },
        { "name": "Odd", "class": "Drain", "stream": -1 },
    ]));
    assert_found(&text, "/instances/1/stream", "instance 0 already has stream 4");
    assert_found(&text, "/instances/2/stream", "a whole number, 0 or more");
}

#[test]
fn a_stations_random_numbers_follow_its_stream_through_a_file() {
    let failures = |instances: Value| {
        let description = built(&file(instances));
        let mut run = description.build(3);
        run.init().unwrap();
        run.advance_to(secs(86_400)).unwrap();
        let state = run.state();
        state.objects.iter().zip(0..).find(|(_, id)| description.name(*id) == Some("Mill")).unwrap().0.statistics.failures
    };
    let mill = json!({ "name": "Mill", "class": "Station", "stream": 9, "successors": ["Out"],
        "attributes": { "process_time": "5", "failures": { "availability": 0.8, "repair_time": { "erlang": { "mean": "20", "k": 2 } } } } });
    let alone = failures(json!([
        { "name": "In", "class": "Source", "stream": 0, "successors": ["Mill"], "attributes": { "interval": "10" } },
        mill.clone(),
        { "name": "Out", "class": "Drain", "stream": 1 },
    ]));
    let with_one_in_front = failures(json!([
        { "name": "In", "class": "Source", "stream": 0, "successors": ["Pass"], "attributes": { "interval": "10" } },
        { "name": "Pass", "class": "Station", "stream": 2, "successors": ["Mill"] },
        mill,
        { "name": "Out", "class": "Drain", "stream": 1 },
    ]));
    assert!(alone > 10, "{alone}");
    assert_eq!(with_one_in_front, alone, "the Mill moved to another place but kept its stream");
}

// ---------- State notes ----------

/// A robot class noting what Blocked means for it, an instance of it with its own note on Failed,
/// and another that inherits the class's notes.
fn robots() -> String {
    with_classes(
        json!([{ "name": "Robot", "parent": "Station", "attributes": {
            "process_time": "30",
            "state_notes": { "blocked": "waits for the conveyor to clear", "busy": "welding" } } }]),
        json!([
            { "name": "In", "class": "Source", "successors": ["R1", "R2"], "attributes": { "interval": "5", "state_notes": { "blocked": "the pallet store is full" } } },
            { "name": "R1", "class": "Robot", "successors": ["Out"], "attributes": { "state_notes": { "failed": "the gripper needs changing" } } },
            { "name": "R2", "class": "Robot", "successors": ["Out"] },
            { "name": "Out", "class": "Drain" },
        ]),
    )
}

#[test]
fn state_notes_are_inherited_as_a_whole_and_an_instance_can_set_its_own() {
    let model = load(&robots()).unwrap();
    let notes = |index: usize| {
        let resolved = model.resolve(index).unwrap();
        let notes: Vec<(&'static str, String)> =
            resolved.attributes.state_notes.clone().unwrap_or_default().into_iter().map(|(state, note)| (state.name(), note)).collect();
        (notes, resolved.origin(Attribute::StateNotes).clone())
    };
    assert_eq!(notes(2), (vec![("busy", "welding".to_string()), ("blocked", "waits for the conveyor to clear".to_string())], Origin::Class("Robot".into())));
    assert_eq!(notes(1), (vec![("failed", "the gripper needs changing".to_string())], Origin::Instance), "set here, so the class's notes are cut");
    assert_eq!(notes(3), (vec![], Origin::BuiltIn), "a Drain with none");
}

#[test]
fn state_notes_survive_saving_and_change_no_result() {
    let model = load(&robots()).unwrap();
    let saved = model.save();
    assert!(saved.contains("the gripper needs changing"), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    let plain = robots().replace(r#","state_notes":{"blocked":"the pallet store is full"}"#, "");
    assert_ne!(plain, robots(), "the note was taken out");
    assert_eq!(run_for(&built(&robots()), 3, secs(3_600)), run_for(&built(&plain), 3, secs(3_600)));
}

#[test]
fn a_note_on_forced_failure_reads_as_stopped_and_is_saved_as_stopped() {
    // "forced_failure" was Stopped's name before ADR 0020, so files written then keep working.
    let old = robots().replace(r#""failed":"the gripper needs changing""#, r#""forced_failure":"someone pressed the red button""#);
    assert_ne!(old, robots(), "the note was replaced");
    let model = load(&old).unwrap();
    let notes = model.resolve(1).unwrap().attributes.state_notes.clone().unwrap_or_default();
    assert_eq!(notes.get(&State::Stopped).map(String::as_str), Some("someone pressed the red button"));
    let saved = model.save();
    assert!(saved.contains(r#""stopped""#) && !saved.contains("forced_failure"), "{saved}");
}

#[test]
fn a_state_note_must_name_a_state_its_object_can_be_in_and_say_something() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "interval": "5", "state_notes": { "holding": "never", "blokked": "typo" } } },
        { "name": "Out", "class": "Drain", "attributes": { "state_notes": { "idle": "", "busy": 3 } } },
    ]));
    assert_eq!(
        found(&text),
        vec![
            "/instances/0/attributes/state_notes/blokked: unknown state `blokked`: did you mean `blocked`?",
            "/instances/0/attributes/state_notes/holding: a Source is never holding",
            "/instances/1/attributes/state_notes/busy: this should be text",
            "/instances/1/attributes/state_notes/idle: a note can't be empty: leave the state out instead",
        ]
    );
}

// ---------- Buffers ----------

/// A Station starting with an MU and a stack starting with two feed a slow Station.
fn stacked() -> String {
    with_classes(
        json!([{ "name": "Stack", "parent": "Buffer", "attributes": { "exit_strategy": "last_in_first_out", "dwelling_time": "0:04", "capacity": -1 } }]),
        json!([
            { "name": "Pre", "class": "Station", "successors": ["Pile"], "attributes": { "process_time": "1", "starting_mus": 1 } },
            { "name": "Pile", "class": "Stack", "successors": ["Slow"], "attributes": { "starting_mus": 2 } },
            { "name": "Slow", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "10" } },
            { "name": "Out", "class": "Drain" },
        ]),
    )
}

#[test]
fn a_buffers_attributes_are_read_inherited_saved_and_built() {
    let model = load(&stacked()).unwrap();
    let pile = model.resolve(1).unwrap();
    assert_eq!(pile.attributes.capacity, Some(Capacity::Unlimited));
    assert_eq!(pile.attributes.exit_strategy, Some(ExitStrategy::LastInFirstOut));
    assert_eq!(pile.attributes.dwelling_time, Some(secs(4)));
    assert_eq!(pile.origin(Attribute::ExitStrategy), &Origin::Class("Stack".into()));
    assert_eq!(pile.origin(Attribute::StartingMus), &Origin::Instance);
    let saved = model.save();
    assert!(saved.contains(r#""capacity": "unlimited""#), "-1 is saved as \"unlimited\": {saved}");
    assert!(saved.contains(r#""exit_strategy": "last_in_first_out""#) && saved.contains(r#""dwelling_time": "00:00:04""#), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    assert_eq!(load(&saved).unwrap().save(), saved, "canonical");

    // Starting MUs are numbered in file order: 1 in Pre, 2 and 3 in Pile, which entered in that
    // order. At 4 their dwelling time is over, and the stack passes on the newest that may leave.
    let (occurrences, _) = run_for(&model.to_description(), 0, secs(100));
    let into_slow: Vec<(u32, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            crabsim::OccurrenceKind::MuMoved { mu_id, to: 2, .. } => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect();
    assert_eq!(into_slow, [(3, 4), (1, 14), (2, 24)], "MU 1 came in at 1 and may leave from 5");
}

#[test]
fn a_buffer_is_holding_not_busy_in_state_notes() {
    let buffer = |notes: Value| file(json!([{ "name": "B", "class": "Buffer", "attributes": { "state_notes": notes } }]));
    assert!(load(&buffer(json!({ "holding": "cooling down", "blocked": "the oven is full", "idle": "empty" }))).is_ok());
    assert_found(&buffer(json!({ "busy": "cooling down" })), "/instances/0/attributes/state_notes/busy", "a Buffer is never busy");
    let station = file(json!([{ "name": "S", "class": "Station", "attributes": { "state_notes": { "holding": "no" } } }]));
    assert_found(&station, "/instances/0/attributes/state_notes/holding", "a Station is never holding");
}

#[test]
fn a_source_with_an_interval_of_0_must_not_reach_a_buffer_with_no_limit() {
    let line = |buffer: Value| {
        file(json!([
            { "name": "Feeder", "class": "Source", "successors": ["Gate"], "attributes": { "interval": "0" } },
            { "name": "Gate", "class": "Station", "successors": ["Store"] },
            { "name": "Store", "class": "Buffer", "successors": ["Slow"], "attributes": buffer },
            { "name": "Slow", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "10" } },
            { "name": "Out", "class": "Drain" },
        ]))
    };
    assert_eq!(
        found(&line(json!({ "dwelling_time": "1:00" }))),
        vec![
            "/instances/0/attributes/interval: with an interval of 0 this Source reaches `Store`, a Buffer with no limit, through zero-time \
             objects (Gate), so nothing ever blocks it and it would create MUs forever without time passing"
        ]
    );
    // With a capacity it fills, and then the Source waits.
    let model = load(&line(json!({ "capacity": 3 }))).unwrap();
    let (_, state) = run_for(&model.to_description(), 0, secs(100));
    assert_eq!(state.objects[2].contents.len(), 3);
    // A Buffer with a capacity and no dwelling time is itself a zero-time object on the way.
    let through = file(json!([
        { "name": "Feeder", "class": "Source", "successors": ["Store"], "attributes": { "interval": "0" } },
        { "name": "Store", "class": "Buffer", "successors": ["Out"], "attributes": { "capacity": 3 } },
        { "name": "Out", "class": "Drain" },
    ]));
    assert_found(&through, "/instances/0/attributes/interval", "reaches Drain `Out` through zero-time objects (Store)");
    let dwelling = through.replace(r#""capacity":3"#, r#""capacity":3,"dwelling_time":"5""#);
    assert_ne!(dwelling, through);
    assert!(load(&dwelling).is_ok(), "MUs stay 5 s, so it fills");
}

#[test]
fn a_buffer_with_no_limit_that_fills_faster_than_it_empties_gets_a_warning() {
    let line = |interval: &str, buffer: Value| {
        file(json!([
            { "name": "In", "class": "Source", "successors": ["Store"], "attributes": { "interval": interval } },
            { "name": "Store", "class": "Buffer", "successors": ["Slow"], "attributes": buffer },
            { "name": "Slow", "class": "Station", "successors": ["Out"], "attributes": { "process_time": "10" } },
            { "name": "Out", "class": "Drain" },
        ]))
    };
    let warnings = |text: String| -> Vec<String> { load(&text).unwrap().warnings().iter().map(|w| format!("{}: {}", w.pointer, w.message)).collect() };
    assert_eq!(
        warnings(line("5", json!({}))),
        vec![
            "/instances/1: it has no limit and can get 720 MUs an hour but pass on only 360, so it would grow for as long as the run lasts: \
             give it a capacity, or make sure that is meant"
        ]
    );
    assert_eq!(warnings(line("5", json!({ "capacity": "unlimited" })))[0].split(':').next(), Some("/instances/1/attributes/capacity"));
    assert_eq!(warnings(line("5", json!({ "capacity": 50 }))), Vec::<String>::new(), "it has a limit");
    assert_eq!(warnings(line("10", json!({}))), Vec::<String>::new(), "it empties as fast as it fills");
    let two_at_once = line("5", json!({})).replace(r#""process_time":"10""#, r#""process_time":"10","capacity":2"#);
    assert!(two_at_once.contains(r#""capacity":2"#));
    assert_eq!(warnings(two_at_once), Vec::<String>::new(), "a Station of capacity 2 passes on 720 an hour");
}

#[test]
fn a_buffers_histogram_bar_width_is_read_saved_built_and_checked() {
    let buffer = |attributes: Value| file(json!([{ "name": "B", "class": "Buffer", "attributes": attributes }]));
    let model = load(&buffer(json!({ "histogram_bar_width": 5 }))).unwrap();
    assert_eq!(model.resolve(0).unwrap().attributes.histogram_bar_width, Some(5));
    assert_eq!(load(&buffer(json!({}))).unwrap().resolve(0).unwrap().attributes.histogram_bar_width, None, "automatic");
    let saved = model.save();
    assert!(saved.contains(r#""histogram_bar_width": 5"#), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    let mut run = model.to_description().build(0);
    run.init().unwrap();
    assert_eq!(run.state().objects[0].fill.as_ref().unwrap().histogram.width, 5);
    assert_found(&buffer(json!({ "histogram_bar_width": 0 })), "/instances/0/attributes/histogram_bar_width", "at least 1 MU");
    assert_found(&buffer(json!({ "histogram_bar_width": "wide" })), "/instances/0/attributes/histogram_bar_width", "whole number");
    let station = file(json!([{ "name": "S", "class": "Station", "attributes": { "histogram_bar_width": 2 } }]));
    assert_found(&station, "/instances/0/attributes/histogram_bar_width", "a Station has no attribute");
}

// ---------- Interruptions of every type (spec issue 122) ----------

#[test]
fn a_source_buffer_or_drain_can_have_failures_and_they_fail() {
    let failures = json!({ "availability": 0.8, "repair_time": { "constant": "0:30" } });
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Store"], "attributes": { "interval": "1", "failures": failures } },
        { "name": "Store", "class": "Buffer", "successors": ["Out"], "attributes": { "failures": failures } },
        { "name": "Out", "class": "Drain", "attributes": { "failures": failures } },
    ]));
    let model = load(&text).unwrap();
    assert_eq!(load(&model.save()).unwrap(), model, "saved and read again");
    let mut run = model.to_description().build(2);
    run.init().unwrap();
    run.advance_to(secs(100_000)).unwrap();
    let state = run.state();
    for object in &state.objects {
        assert!(object.statistics.failures > 100, "object {} failed {} times", object.id, object.statistics.failures);
    }
    let bad = file(json!([{ "name": "Out", "class": "Drain", "attributes": { "failures": { "availability": 1.5, "repair_time": { "constant": "0:30" } } } }]));
    assert_found(&bad, "/instances/0/attributes/failures/0/availability", "above 0 and below 1");
}

#[test]
fn a_buffers_dwelling_in_a_stop_is_read_saved_built_and_checked() {
    let buffer = |attributes: Value| file(json!([{ "name": "B", "class": "Buffer", "attributes": attributes }]));
    let model = load(&buffer(json!({ "stop_freezes_dwelling": true }))).unwrap();
    assert_eq!(model.resolve(0).unwrap().attributes.stop_freezes_dwelling, Some(true));
    let saved = model.save();
    assert!(saved.contains(r#""stop_freezes_dwelling": true"#), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    assert_found(&buffer(json!({ "stop_freezes_dwelling": "yes" })), "/instances/0/attributes/stop_freezes_dwelling", "true or false");
    let station = file(json!([{ "name": "S", "class": "Station", "attributes": { "stop_freezes_dwelling": true } }]));
    assert_found(&station, "/instances/0/attributes/stop_freezes_dwelling", "a Station has no attribute");
}

#[test]
fn every_type_has_notes_for_its_interruptions() {
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "interval": "5", "state_notes": { "paused": "the supplier's break" } } },
        { "name": "Out", "class": "Drain", "attributes": { "state_notes": { "failed": "the dock is jammed", "stopped": "loading by hand" } } },
    ]));
    assert!(load(&text).is_ok(), "{:?}", load(&text).err());
}

// ---------- Station capacity ----------

/// An oven of capacity 3 with batch start, inherited from a class, starting with 2 MUs, fed by a
/// Station starting with one.
fn oven_line() -> String {
    with_classes(
        json!([{ "name": "Oven", "parent": "Station", "attributes": { "capacity": 3, "start": "batch", "process_time": "0:20" } }]),
        json!([
            { "name": "Pre", "class": "Station", "successors": ["Bake"], "attributes": { "process_time": "5", "starting_mus": 1 } },
            { "name": "Bake", "class": "Oven", "successors": ["Out"], "attributes": { "starting_mus": 2 } },
            { "name": "Out", "class": "Drain" },
        ]),
    )
}

#[test]
fn a_stations_capacity_and_start_are_read_inherited_saved_and_built() {
    let model = load(&oven_line()).unwrap();
    let bake = model.resolve(1).unwrap();
    assert_eq!(bake.attributes.capacity, Some(Capacity::Limited(3)));
    assert_eq!(bake.attributes.start, Some(StartMode::Batch));
    assert_eq!(bake.origin(Attribute::Start), &Origin::Class("Oven".into()));
    let pre = model.resolve(0).unwrap();
    assert_eq!((pre.attributes.capacity, pre.attributes.start), (Some(Capacity::Limited(1)), Some(StartMode::Individual)), "the defaults");
    assert_eq!(pre.origin(Attribute::Capacity), &Origin::BuiltIn);
    let saved = model.save();
    assert!(saved.contains(r#""capacity": 3"#) && saved.contains(r#""start": "batch""#), "{saved}");
    assert_eq!(load(&saved).unwrap(), model);
    assert_eq!(load(&saved).unwrap().save(), saved, "canonical");

    // MU 1 starts in Pre, 2 and 3 in Bake, whose batch begins when MU 1 joins them at 5.
    let (occurrences, state) = run_for(&model.to_description(), 0, secs(100));
    let started: Vec<(u32, u64)> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::WorkStarted { mu_id, at: 1, .. } => Some((mu_id, o.time.as_secs())),
            _ => None,
        })
        .collect();
    assert_eq!(started, [(2, 5), (3, 5), (1, 5)]);
    assert_eq!(state.objects[1].capacity, Capacity::Limited(3));
    assert_eq!(state.objects[2].statistics.mus_in, 3);
}

#[test]
fn a_stations_capacity_is_a_whole_number_from_1_and_holds_its_starting_mus() {
    let station = |attributes: Value| file(json!([{ "name": "S", "class": "Station", "attributes": attributes }]));
    assert!(load(&station(json!({ "capacity": 4, "starting_mus": 4, "start": "individual" }))).is_ok());
    assert_found(&station(json!({ "capacity": 0 })), "/instances/0/attributes/capacity", "at least 1");
    assert_found(&station(json!({ "capacity": "unlimited" })), "/instances/0/attributes/capacity", "only a Buffer can be unlimited");
    assert_found(&station(json!({ "capacity": -1 })), "/instances/0/attributes/capacity", "only a Buffer can be unlimited");
    assert_found(&station(json!({ "capacity": 10_001 })), "/instances/0/attributes/capacity", "at most 10,000");
    assert_found(&station(json!({ "capacity": 2, "starting_mus": 3 })), "/instances/0/attributes/starting_mus", "it starts with 3 MUs but holds only 2");
    assert_found(&station(json!({ "start": "all_at_once" })), "/instances/0/attributes/start", "use \"individual\" or \"batch\"");
    let buffer = file(json!([{ "name": "B", "class": "Buffer", "attributes": { "start": "batch" } }]));
    assert_found(&buffer, "/instances/0/attributes/start", "a Buffer has no attribute `start`");
}

// ---------- The second example ----------

fn buffered_line_file() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/buffered-line.json");
    std::fs::read_to_string(path).expect("models/buffered-line.json exists").replace("\r\n", "\n")
}

#[test]
fn the_buffered_line_file_builds_the_same_run_as_the_example_in_code() {
    let from_file = built(&buffered_line_file());
    let from_code = demo::buffered_line();
    for id in 0..from_code.object_count() {
        assert_eq!((from_file.kind(id), from_file.name(id), from_file.position(id)), (from_code.kind(id), from_code.name(id), from_code.position(id)));
    }
    for seed in [0, 7] {
        assert_eq!(run_for(&from_file, seed, secs(86_400)), run_for(&from_code, seed, secs(86_400)), "seed {seed}");
    }
}

#[test]
fn the_buffered_line_file_is_in_canonical_form_with_no_warnings() {
    let text = buffered_line_file();
    let model = load(&text).unwrap();
    assert_eq!(model.save(), text);
    assert!(model.warnings().is_empty(), "{:?}", model.warnings());
}

// ---------- Failure profiles ----------

fn mill_with(failures: Value) -> String {
    file(json!([
        { "name": "In", "class": "Source", "stream": 0, "successors": ["Mill"], "attributes": { "interval": "10" } },
        { "name": "Mill", "class": "Station", "stream": 9, "successors": ["Out"], "attributes": { "process_time": "5", "failures": failures } },
        { "name": "Out", "class": "Drain", "stream": 1 },
    ]))
}

fn profile(name: &str, availability: f64, stream: Option<u64>) -> Value {
    let mut fields = json!({ "name": name, "availability": availability, "repair_time": { "constant": "0:20" } });
    if let Some(stream) = stream {
        fields["stream"] = json!(stream);
    }
    fields
}

#[test]
fn a_single_failures_object_reads_as_a_list_of_one_named_failures_and_saves_as_a_list() {
    let text = mill_with(json!({ "availability": 0.8, "repair_time": { "constant": "0:20" } }));
    let model = load(&text).unwrap();
    let list = model.instances[1].attributes.failures.clone().unwrap();
    assert_eq!((list.len(), list[0].name.as_str(), list[0].stream), (1, "Failures", None));
    let saved = model.save();
    assert!(saved.contains("\"name\": \"Failures\""), "{saved}");
    assert_eq!(load(&saved).unwrap().instances[1].attributes.failures, Some(list));
}

#[test]
fn a_single_profile_from_an_older_file_fails_exactly_as_the_same_profile_named_in_a_list() {
    let old = built(&mill_with(json!({ "availability": 0.8, "repair_time": { "constant": "0:20" } })));
    let new = built(&mill_with(json!([profile("Failures", 0.8, None)])));
    assert_eq!(run_for(&old, 3, secs(86_400)), run_for(&new, 3, secs(86_400)));
}

#[test]
fn profiles_are_named_uniquely_and_checked_one_by_one() {
    assert_found(&mill_with(json!([profile("Jam", 0.9, None), profile("Jam", 0.9, None)])), "/instances/1/attributes/failures/1/name", "already a failure profile named \"Jam\"");
    assert_found(&mill_with(json!([profile("Jam", 0.9, None), profile("Wear", 1.0, None)])), "/instances/1/attributes/failures/1/availability", "above 0 and below 1");
    assert_found(&mill_with(json!([profile("", 0.9, None)])), "/instances/1/attributes/failures/0/name", "can't be empty");
    assert_found(&mill_with(json!([{ "availability": 0.9, "repair_time": { "constant": "0:20" } }])), "/instances/1/attributes/failures/0", "needs a `name`");
    assert_found(&mill_with(json!([{ "name": "A", "stream": -1, "availability": 0.9, "repair_time": { "constant": "0:20" } }])), "/instances/1/attributes/failures/0/stream", "whole number");
}

#[test]
fn two_profiles_reach_the_run_each_failing_on_its_own_and_adding_one_leaves_the_other_alone() {
    let wear = profile("Tool wear", 0.9, Some(21));
    let jam = profile("Jam", 0.8, Some(22));
    let failures_of = |list: Value| {
        let description = built(&mill_with(list));
        let mut run = description.build(3);
        run.init().unwrap();
        run.advance_to(secs(86_400)).unwrap();
        let state = run.state();
        state.objects.into_iter().find(|object| description.name(object.id) == Some("Mill")).unwrap().failure_profiles
    };
    let alone = failures_of(json!([wear.clone()]));
    let both = failures_of(json!([wear, jam]));
    assert_eq!(both.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Tool wear", "Jam"]);
    assert!(both[1].failures > 10, "{both:?}");
    assert_eq!(alone[0].failures, both[0].failures, "its own stream, so another profile doesn't change when it fails");
}

#[test]
fn a_class_hands_its_whole_list_to_its_instances() {
    let text = json!({ "format_version": 2,
        "classes": [{ "name": "Lathe", "parent": "Station", "attributes": { "failures": [profile("Wear", 0.9, Some(5)), profile("Jam", 0.8, Some(6))] } }],
        "instances": [{ "name": "L1", "class": "Lathe", "stream": 0 }], "connections": [] })
    .to_string();
    let model = load(&text).unwrap();
    let resolved = model.resolve(0).unwrap();
    assert_eq!(resolved.attributes.failures.map(|list| list.len()), Some(2));
}

#[test]
fn two_profiles_of_one_object_cant_share_a_stream_counting_a_place_as_one() {
    assert_found(&mill_with(json!([profile("Wear", 0.9, Some(4)), profile("Jam", 0.9, Some(4))])), "/instances/1/attributes/failures/1/stream", "\"Wear\" already has stream 4");
    assert_found(&mill_with(json!([profile("Wear", 0.9, Some(1)), profile("Jam", 0.9, None)])), "/instances/1/attributes/failures/1/stream", "already has stream 1");
}

#[test]
fn a_classs_profiles_fail_apart_in_each_instance() {
    let text = json!({ "format_version": 2,
        "classes": [{ "name": "Lathe", "parent": "Station", "attributes": { "process_time": "5", "failures": [{ "name": "Jam", "stream": 3, "availability": 0.8, "repair_time": { "erlang": { "mean": "0:20", "k": 2 } } }] } }],
        "instances": [
            { "name": "In", "class": "Source", "stream": 0, "attributes": { "interval": "1" } },
            { "name": "L1", "class": "Lathe", "stream": 1 }, { "name": "L2", "class": "Lathe", "stream": 2 },
            { "name": "Out", "class": "Drain", "stream": 3 }],
        "connections": [
            { "from": "In", "to": "L1", "exit": 1, "entrance": 1 }, { "from": "In", "to": "L2", "exit": 2, "entrance": 1 },
            { "from": "L1", "to": "Out", "exit": 1, "entrance": 1 }, { "from": "L2", "to": "Out", "exit": 1, "entrance": 2 }] })
    .to_string();
    let description = built(&text);
    let (_, state) = run_for(&description, 5, secs(86_400));
    let jam = |name: &str| state.objects.iter().find(|object| description.name(object.id) == Some(name)).unwrap().failure_profiles[0].clone();
    assert_ne!(jam("L1").time_failed, jam("L2").time_failed, "each instance's own stream");
}

// ---------- Time bases ----------

#[test]
fn a_profiles_time_basis_is_read_saved_only_when_not_simulation_and_reaches_the_run() {
    let mut wear = profile("Wear", 0.9, Some(0));
    wear["time_basis"] = json!("processing");
    let jam = profile("Jam", 0.9, Some(1));
    let model = load(&mill_with(json!([wear, jam]))).unwrap();
    let list = model.instances[1].attributes.failures.clone().unwrap();
    assert_eq!((list[0].time_basis, list[1].time_basis), (crabsim_model::TimeBasis::Processing, crabsim_model::TimeBasis::Simulation));
    let saved = model.save();
    assert_eq!(saved.matches("\"time_basis\"").count(), 1, "simulation time is the default, left out: {saved}");
    let (_, state) = run_for(&model.to_description(), 3, secs(86_400));
    let mill = state.objects.iter().find(|object| object.failure_profiles.len() == 2).unwrap();
    assert_eq!(mill.failure_profiles[0].time_basis, crabsim::TimeBasis::Processing);
}

#[test]
fn a_time_basis_is_one_of_three_and_a_buffer_or_drain_has_no_processing_time() {
    let mut odd = profile("Wear", 0.9, None);
    odd["time_basis"] = json!("calendar");
    assert_found(&mill_with(json!([odd])), "/instances/1/attributes/failures/0/time_basis", "is not a time basis");
    let mut processing = profile("Full", 0.9, None);
    processing["time_basis"] = json!("processing");
    let text = file(json!([
        { "name": "In", "class": "Source", "successors": ["Out"], "attributes": { "failures": [processing.clone()] } },
        { "name": "Out", "class": "Drain", "attributes": { "failures": [processing.clone()] } },
    ]));
    let found = found(&text);
    assert!(found.iter().any(|problem| problem.starts_with("/instances/1/attributes/failures/0/time_basis: a Drain is never busy")), "{found:#?}");
    assert!(!found.iter().any(|problem| problem.starts_with("/instances/0/")), "a Source has processing time: {found:#?}");
    let mut working = profile("Full", 0.9, None);
    working["time_basis"] = json!("working");
    assert!(load(&file(json!([{ "name": "B", "class": "Buffer", "attributes": { "failures": [working] } }]))).is_ok());
}
