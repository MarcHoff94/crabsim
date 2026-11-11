//! The published JSON Schema: generated from `crabsim_model::schema`, committed as
//! `models/model-file.schema.json`, and kept from drifting away from what the reader accepts.
//!
//! After changing the format, write the file again with:
//! `cargo test -p crabsim-model --test schema -- --ignored write_the_schema_file`

use crabsim::time::format_time_exact;
use crabsim::SimTime;
use crabsim_model::schema::{schema, schema_text};
use crabsim_model::{load, Attribute};
use serde_json::{json, Value};

const SCHEMA_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/model-file.schema.json");

fn validator() -> jsonschema::Validator {
    jsonschema::validator_for(&schema()).expect("the schema is a valid JSON Schema")
}

fn passes_schema(file: &Value) -> bool {
    validator().is_valid(file)
}

/// Files the reader accepts, covering every field and every kind of value.
fn valid_files() -> Vec<Value> {
    let demo: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../models/demo.json")).unwrap()).unwrap();
    let everything = json!({
        "$schema": "https://example.com/model-file.schema.json",
        "format_version": 2,
        "description": "every field there is",
        "classes": [
            { "name": "Mill", "parent": "Station", "attributes": {
                "process_time": "1:30",
                "failures": { "availability": 0.9, "repair_time": { "erlang": { "mean": "2d 06:00:00", "k": 3 } } },
                "starting_mus": 1,
                "dimensions": { "length": 2.5, "width": 1, "height": 1.5, "rotation_degrees": 90 },
                "description": "a mill" } },
            { "name": "Shaky mill", "parent": "Mill", "attributes": {
                "failures": { "availability": 0.5, "repair_time": { "uniform": { "min": "10", "max": "0:00:20.5" } } } } },
            { "name": "Odd mill", "parent": "Mill", "attributes": {
                "failures": { "availability": 0.99, "repair_time": { "normal": { "mean": "1:00", "std": "0" } } } } },
            { "name": "Steady mill", "parent": "Mill", "attributes": {
                "failures": { "availability": 0.8, "repair_time": { "constant": "3d" } } } },
            { "name": "Store", "parent": "Buffer", "attributes": { "capacity": 4 } },
        ],
        "instances": [
            { "name": "In", "class": "Source", "position": { "x": -1.5, "y": 2, "z": 0.5 }, "attributes": { "interval": "0" } },
            { "name": "A", "class": "Mill" },
            { "name": "B", "class": "Shaky mill", "attributes": { "starting_mus": 0 } },
            { "name": "C", "class": "Odd mill" },
            { "name": "D", "class": "Steady mill" },
            { "name": "S", "class": "Store" },
            { "name": "Out", "class": "Drain", "attributes": { "description": "the end" } },
        ],
        "connections": [
            { "from": "In", "to": "A", "exit": 1, "entrance": 1 },
            { "from": "In", "to": "B", "exit": 2, "entrance": 1, "description": "the long way" },
            { "from": "In", "to": "C", "exit": 3, "entrance": 1 },
            { "from": "In", "to": "D", "exit": 4, "entrance": 1 },
            { "from": "A", "to": "S", "exit": 1, "entrance": 4 },
            { "from": "B", "to": "S", "exit": 1, "entrance": 3 },
            { "from": "C", "to": "S", "exit": 1, "entrance": 2 },
            { "from": "D", "to": "S", "exit": 1, "entrance": 1 },
            { "from": "S", "to": "Out", "exit": 1, "entrance": 1 },
        ],
    });
    vec![demo, everything]
}

// ---------- The committed file ----------

#[test]
#[ignore = "writes models/model-file.schema.json: run it after changing the format"]
fn write_the_schema_file() {
    std::fs::write(SCHEMA_FILE, schema_text()).unwrap();
}

#[test]
fn the_committed_schema_is_the_generated_one() {
    let committed = std::fs::read_to_string(SCHEMA_FILE).expect("models/model-file.schema.json exists").replace("\r\n", "\n");
    assert!(
        committed == schema_text(),
        "models/model-file.schema.json is out of date: run `cargo test -p crabsim-model --test schema -- --ignored write_the_schema_file`"
    );
}

#[test]
fn the_demo_file_points_editors_at_the_schema() {
    let demo = &valid_files()[0];
    assert_eq!(demo["$schema"], "./model-file.schema.json");
}

// ---------- The schema and the reader agree ----------

#[test]
fn every_file_the_reader_accepts_passes_the_schema() {
    for file in valid_files() {
        assert!(load(&file.to_string()).is_ok(), "the reader accepts {file}");
        let errors: Vec<String> = validator().iter_errors(&file).map(|error| format!("{} at {}", error, error.instance_path())).collect();
        assert!(errors.is_empty(), "{errors:#?}");
    }
}

#[test]
fn the_schema_has_every_attribute_and_no_others() {
    let schema = schema();
    let mut in_schema: Vec<&str> =
        schema["$defs"]["AttributesFile"]["properties"].as_object().unwrap().keys().map(String::as_str).collect();
    let mut known: Vec<&str> = Attribute::ALL.iter().map(|attribute| attribute.name()).collect();
    in_schema.sort();
    known.sort();
    assert_eq!(in_schema, known);
}

#[test]
fn shape_mistakes_fail_the_schema_as_well_as_the_reader() {
    let base = || valid_files().remove(1);
    let mistakes: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("an unknown field", Box::new(|file| file["seed"] = json!(3))),
        ("an unknown attribute", Box::new(|file| file["classes"][0]["attributes"]["proces_time"] = json!("10"))),
        ("a null instead of leaving a field out", Box::new(|file| file["description"] = Value::Null)),
        ("another format version", Box::new(|file| file["format_version"] = json!(1))),
        ("successors on an instance", Box::new(|file| file["instances"][5]["successors"] = json!(["Out"]))),
        ("an exit of 0", Box::new(|file| file["connections"][0]["exit"] = json!(0))),
        ("a connection without an entrance", Box::new(|file| { file["connections"][0].as_object_mut().unwrap().remove("entrance"); })),
        ("a connection with a field it doesn't have", Box::new(|file| file["connections"][0]["share"] = json!(0.5))),
        ("a missing name", Box::new(|file| { file["instances"][1].as_object_mut().unwrap().remove("name"); })),
        ("a name with a space at the end", Box::new(|file| file["instances"][6]["name"] = json!("Out "))),
        ("a time that isn't one", Box::new(|file| file["classes"][0]["attributes"]["process_time"] = json!("soon"))),
        ("a time as a number", Box::new(|file| file["classes"][0]["attributes"]["process_time"] = json!(90))),
        ("an availability of 1", Box::new(|file| file["classes"][1]["attributes"]["failures"]["availability"] = json!(1))),
        ("two kinds of repair time", Box::new(|file| file["classes"][3]["attributes"]["failures"]["repair_time"]["uniform"] = json!({ "min": "1", "max": "2" }))),
        ("an Erlang k of 0", Box::new(|file| file["classes"][0]["attributes"]["failures"]["repair_time"]["erlang"]["k"] = json!(0))),
        ("a capacity of 0", Box::new(|file| file["classes"][4]["attributes"]["capacity"] = json!(0))),
        ("two starting MUs", Box::new(|file| file["classes"][0]["attributes"]["starting_mus"] = json!(2))),
        ("a length of 0", Box::new(|file| file["classes"][0]["attributes"]["dimensions"]["length"] = json!(0))),
        ("a position without y", Box::new(|file| { file["instances"][0]["position"].as_object_mut().unwrap().remove("y"); })),
    ];
    for (what, change) in mistakes {
        let mut file = base();
        change(&mut file);
        assert!(load(&file.to_string()).is_err(), "the reader refuses {what}");
        assert!(!passes_schema(&file), "the schema refuses {what}");
    }
}

#[test]
fn times_the_reader_takes_match_the_schemas_pattern() {
    let with_interval = |time: &str| {
        json!({ "format_version": 2, "instances": [{ "name": "S", "class": "Station", "attributes": { "process_time": time } }] })
    };
    let saved: Vec<String> = [0, 1, 999, 59_000, 90_000, 3_599_999, 86_400_000, 86_401_250, 400 * 86_400_000 + 59_000]
        .into_iter()
        .map(|millis| format_time_exact(SimTime::from_millis(millis)))
        .collect();
    let typed = ["90", "1:30", "01:02:03", "3d", "2d 06:00:00", "2d0:00:01", "0:00:12.5", " 1:30 "];
    for time in saved.iter().map(String::as_str).chain(typed) {
        let file = with_interval(time);
        assert!(load(&file.to_string()).is_ok(), "the reader takes {time:?}");
        assert!(passes_schema(&file), "the schema takes {time:?}");
    }
    for not_a_time in ["soon", "1:2:3:4", "-5", "", "d", "1.5d"] {
        let file = with_interval(not_a_time);
        assert!(load(&file.to_string()).is_err(), "the reader refuses {not_a_time:?}");
        assert!(!passes_schema(&file), "the schema refuses {not_a_time:?}");
    }
}
