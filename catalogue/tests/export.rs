//! `export_model`, by `call(name, JSON)`: a model written as a Cypher script, Neo4j's CSV import
//! or GraphML into `exports/`, checked structurally, and the scope rule that keeps model
//! operations out of `exports/` and exports in it.

mod common;

use common::Setup;
use serde_json::{json, Value};
use std::collections::HashMap;

/// A Source, a Station splitting three ways, three presses of a class and a class derived from it,
/// a join, and two Drains, one with a name that needs escaping in every format.
const LINE: &str = r#"{"format_version":1,"description":"Split, press and join",
  "classes":[
    {"name":"Press","parent":"Station","attributes":{"process_time":"0:05","description":"Hydraulic press",
       "failures":{"availability":0.9,"repair_time":{"erlang":{"mean":"1:00","k":2}}}}},
    {"name":"Fast press","parent":"Press","attributes":{"process_time":"0:03"}}],
  "instances":[
    {"name":"In","class":"Source","position":{"x":0,"y":1.5},"successors":["Split"],"attributes":{"interval":"10"}},
    {"name":"Split","class":"Station","successors":["P1","P2","P3"],"attributes":{"process_time":"1"}},
    {"name":"P1","class":"Press","successors":["Join"]},
    {"name":"P2","class":"Fast press","successors":["Join"]},
    {"name":"P3","class":"Press","successors":["Scrap 'A', \"bad\" & <worse>"],"attributes":{"process_time":"0:40"}},
    {"name":"Join","class":"Station","successors":["Out"],"attributes":{"process_time":"2"}},
    {"name":"Out","class":"Drain"},
    {"name":"Scrap 'A', \"bad\" & <worse>","class":"Drain"}]}"#;

const INSTANCES: usize = 8;
/// The model's own two, and the four built-in classes.
const CLASSES: usize = 6;
const CONNECTIONS: usize = 8;
/// Each derived class derives from one other.
const DERIVATIONS: usize = 2;
const ODD_NAME: &str = "Scrap 'A', \"bad\" & <worse>";

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.folder.write("plant/line.json", LINE);
    setup
}

fn export(setup: &Setup, format: &str) -> Value {
    setup.ok("export_model", json!({ "path": "plant/line.json", "format": format }))
}

/// The files an export says it wrote, each with its size checked against the file.
fn written(setup: &Setup, output: &Value) -> Vec<String> {
    output["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            let path = file["path"].as_str().unwrap().to_string();
            let bytes = std::fs::metadata(setup.folder.join(&path)).unwrap_or_else(|_| panic!("{path} wasn't written")).len();
            assert_eq!(file["bytes"], bytes, "{path}");
            path
        })
        .collect()
}

// ---------- Cypher ----------

/// The statements of a Cypher script that start with `start`.
fn statements<'t>(script: &'t str, start: &str) -> Vec<&'t str> {
    script.lines().filter(|line| line.starts_with(start)).collect()
}

#[test]
fn a_cypher_script_creates_every_instance_class_and_connection() {
    let setup = setup("cypher");
    let output = export(&setup, "cypher");
    assert_eq!(written(&setup, &output), ["exports/plant/line.cypher"]);
    assert_eq!((output["instances"].clone(), output["classes"].clone(), output["connections"].clone()), (json!(INSTANCES), json!(CLASSES), json!(CONNECTIONS)));
    let script = setup.folder.read("exports/plant/line.cypher");
    assert_eq!(statements(&script, "CREATE (:Instance:").len(), INSTANCES);
    assert_eq!(statements(&script, "CREATE (:Instance:Station ").len(), 5, "labelled with the built-in class");
    assert_eq!(statements(&script, "CREATE (:Class ").len(), CLASSES);
    let feeds: Vec<_> = script.lines().filter(|line| line.contains("-[:FEEDS ")).collect();
    assert_eq!(feeds.len(), CONNECTIONS);
    assert_eq!(script.lines().filter(|line| line.contains("-[:IS_A]->")).count(), INSTANCES);
    assert_eq!(script.lines().filter(|line| line.contains("-[:DERIVES_FROM]->")).count(), DERIVATIONS);
    assert!(feeds.iter().any(|line| line.contains("{name: 'Split'}") && line.contains("{name: 'P3'}") && line.contains("{order: 2}")), "{feeds:?}");
    for line in script.lines().filter(|line| !line.trim().is_empty() && !line.starts_with("//")) {
        assert!(line.ends_with(';'), "every statement ends with a semicolon: {line}");
    }

    let p3 = statements(&script, "CREATE (:Instance:Station {name: 'P3'").pop().expect("P3 is created");
    for property in ["class: 'Press'", "process_time_s: 40.0", "availability: 0.9", "repair_distribution: 'erlang'", "repair_mean_s: 60.0", "repair_k: 2", "description: 'Hydraulic press'", "starting_mus: 0", "stream: 4"] {
        assert!(p3.contains(property), "{property} in {p3}");
    }
    let source = statements(&script, "CREATE (:Instance:Source ").pop().unwrap();
    assert!(source.contains("interval_s: 10.0") && source.contains("x: 0.0") && source.contains("y: 1.5"), "{source}");
    assert!(!source.contains("process_time_s"), "only attributes its built-in class has: {source}");
    let press = statements(&script, "CREATE (:Class {name: 'Press'").pop().unwrap();
    assert!(press.contains("built_in: false") && press.contains("process_time_s: 5.0"), "{press}");
    assert!(script.contains(r#"{name: 'Scrap \'A\', "bad" & <worse>'"#), "quotes escaped in a name");
}

// ---------- Neo4j's CSV import ----------

/// A CSV file's header and rows, each row as header to value.
fn csv(setup: &Setup, path: &str) -> (Vec<String>, Vec<HashMap<String, String>>) {
    let mut reader = csv::Reader::from_path(setup.folder.join(path)).unwrap();
    let header: Vec<String> = reader.headers().unwrap().iter().map(str::to_string).collect();
    let rows = reader
        .records()
        .map(|record| header.iter().cloned().zip(record.unwrap().iter().map(str::to_string)).collect())
        .collect();
    (header, rows)
}

#[test]
fn neo4j_csv_gives_one_file_per_node_and_edge_type_with_neo4j_headers() {
    let setup = setup("neo4j-csv");
    let output = export(&setup, "neo4j-csv");
    let files = written(&setup, &output);
    let folder = "exports/plant/line";
    let names = ["instances.csv", "classes.csv", "feeds.csv", "is_a.csv", "derives_from.csv"];
    assert_eq!(files, names.map(|name| format!("{folder}/{name}")));

    let (header, instances) = csv(&setup, &format!("{folder}/instances.csv"));
    assert_eq!(header[..2], ["name:ID(Instance)", ":LABEL"]);
    assert!(header.contains(&"stream:long".to_string()) && header.contains(&"process_time_s:double".to_string()), "{header:?}");
    assert_eq!(instances.len(), INSTANCES);
    let p3 = instances.iter().find(|row| row["name:ID(Instance)"] == "P3").unwrap();
    assert_eq!((p3[":LABEL"].as_str(), p3["class"].as_str(), p3["process_time_s:double"].as_str()), ("Instance;Station", "Press", "40"));
    let source = instances.iter().find(|row| row["name:ID(Instance)"] == "In").unwrap();
    assert_eq!(source["process_time_s:double"], "", "left empty where an instance hasn't the attribute");
    assert!(instances.iter().any(|row| row["name:ID(Instance)"] == ODD_NAME), "a name with quotes and a comma survives");

    let (header, classes) = csv(&setup, &format!("{folder}/classes.csv"));
    assert_eq!(header[..2], ["name:ID(Class)", ":LABEL"]);
    assert_eq!(classes.len(), CLASSES);
    assert!(header.contains(&"built_in:boolean".to_string()));

    let (header, feeds) = csv(&setup, &format!("{folder}/feeds.csv"));
    assert_eq!(header, [":START_ID(Instance)", ":END_ID(Instance)", "order:int", ":TYPE"]);
    assert_eq!(feeds.len(), CONNECTIONS);
    assert!(feeds.iter().any(|row| row[":START_ID(Instance)"] == "Split" && row[":END_ID(Instance)"] == "P3" && row["order:int"] == "2" && row[":TYPE"] == "FEEDS"));

    let (header, is_a) = csv(&setup, &format!("{folder}/is_a.csv"));
    assert_eq!(header, [":START_ID(Instance)", ":END_ID(Class)", ":TYPE"]);
    assert_eq!(is_a.len(), INSTANCES);
    let (header, derives) = csv(&setup, &format!("{folder}/derives_from.csv"));
    assert_eq!(header, [":START_ID(Class)", ":END_ID(Class)", ":TYPE"]);
    assert_eq!(derives.len(), DERIVATIONS);
    assert!(derives.iter().any(|row| row[":START_ID(Class)"] == "Fast press" && row[":END_ID(Class)"] == "Press"));
}

// ---------- GraphML ----------

#[test]
fn graphml_is_xml_with_typed_keys_and_every_node_and_edge() {
    let setup = setup("graphml");
    let output = export(&setup, "graphml");
    assert_eq!(written(&setup, &output), ["exports/plant/line.graphml"]);
    let text = setup.folder.read("exports/plant/line.graphml");
    let document = roxmltree::Document::parse(&text).expect("GraphML is well-formed XML");
    let root = document.root_element();
    assert_eq!(root.tag_name().name(), "graphml");
    let keys: HashMap<&str, (&str, &str, &str)> = root
        .children()
        .filter(|node| node.has_tag_name("key"))
        .map(|key| (key.attribute("id").unwrap(), (key.attribute("for").unwrap(), key.attribute("attr.name").unwrap(), key.attribute("attr.type").unwrap())))
        .collect();
    assert!(keys.values().any(|key| *key == ("node", "process_time_s", "double")), "{keys:?}");
    assert!(keys.values().any(|key| *key == ("node", "stream", "long")), "{keys:?}");
    assert!(keys.values().any(|key| *key == ("edge", "order", "int")), "{keys:?}");
    let graph = root.children().find(|node| node.has_tag_name("graph")).unwrap();
    assert_eq!(graph.attribute("edgedefault"), Some("directed"));
    let nodes: Vec<_> = graph.children().filter(|node| node.has_tag_name("node")).collect();
    let edges: Vec<_> = graph.children().filter(|node| node.has_tag_name("edge")).collect();
    assert_eq!(nodes.len(), INSTANCES + CLASSES);
    assert_eq!(edges.len(), CONNECTIONS + INSTANCES + DERIVATIONS);

    // A node's data, by the name its key gives.
    let data = |node: &roxmltree::Node| -> HashMap<String, String> {
        node.children()
            .filter(|child| child.has_tag_name("data"))
            .map(|data| (keys[data.attribute("key").unwrap()].1.to_string(), data.text().unwrap_or_default().to_string()))
            .collect()
    };
    let p3 = nodes.iter().find(|node| data(node).get("name").map(String::as_str) == Some("P3") && data(node)["labels"].contains("Instance")).unwrap();
    let p3 = data(p3);
    assert_eq!((p3["labels"].as_str(), p3["class"].as_str(), p3["process_time_s"].as_str()), (":Instance:Station", "Press", "40"));
    assert!(nodes.iter().any(|node| data(node)["name"] == ODD_NAME), "a name with markup characters survives");
    let id_of = |name: &str, label: &str| nodes.iter().find(|node| data(node)["name"] == name && data(node)["labels"].contains(label)).unwrap().attribute("id").unwrap();
    let feeds = edges
        .iter()
        .find(|edge| edge.attribute("source") == Some(id_of("Split", "Instance")) && edge.attribute("target") == Some(id_of("P3", "Instance")))
        .expect("Split feeds P3");
    let feeds = data(feeds);
    assert_eq!((feeds["type"].as_str(), feeds["order"].as_str()), ("FEEDS", "2"));
}

// ---------- A run's model, overwriting, refusals ----------

#[test]
fn a_runs_model_is_exported_under_the_runs_name() {
    let setup = setup("export-run");
    let run = setup.ok("start_run", json!({ "model": "plant/line.json", "length": "1:00", "observations": 1, "wait": 30 }))["run"].as_str().unwrap().to_string();
    let output = setup.ok("export_model", json!({ "run": run, "format": "graphml" }));
    assert_eq!(written(&setup, &output), [format!("exports/{run}.graphml")]);
    let csv = setup.ok("export_model", json!({ "run": run, "format": "neo4j-csv" }));
    assert_eq!(written(&setup, &csv)[0], format!("exports/{run}/instances.csv"));
}

#[test]
fn exporting_again_overwrites_and_the_operation_is_marked_destructive() {
    let setup = setup("overwrite");
    export(&setup, "cypher");
    setup.folder.write("exports/plant/line.cypher", "// changed by hand");
    export(&setup, "cypher");
    assert!(setup.folder.read("exports/plant/line.cypher").contains("CREATE (:Instance:"));
    let effect = setup.catalogue.operations().find(|operation| operation.name == "export_model").unwrap().effect;
    assert_eq!(serde_json::to_value(effect).unwrap(), "destructive");
}

#[test]
fn a_model_with_problems_or_an_unknown_format_is_refused() {
    let setup = setup("export-refused");
    setup.folder.write("broken.json", common::BROKEN);
    let broken = setup.err("export_model", json!({ "path": "broken.json", "format": "cypher" }));
    assert_eq!(broken["kind"], "invalid_model");
    assert!(!setup.folder.join("exports/broken.cypher").exists());
    let format = setup.err("export_model", json!({ "path": "plant/line.json", "format": "dot" }));
    assert_eq!(format["kind"], "invalid_input");
    let both = setup.err("export_model", json!({ "path": "plant/line.json", "run": "run-1", "format": "cypher" }));
    assert_eq!(both["kind"], "invalid_input");
}

// ---------- The scope rule ----------

#[test]
fn model_operations_cant_reach_into_exports() {
    let setup = setup("scope");
    setup.folder.write("exports/hidden.json", common::SMALL);
    for (name, input) in [
        ("write_model", json!({ "path": "exports/new.json", "text": common::SMALL })),
        ("read_model", json!({ "path": "exports/hidden.json" })),
        ("read_model", json!({ "path": "EXPORTS/hidden.json" })),
        ("validate_model", json!({ "path": "exports/hidden.json" })),
        ("copy_model", json!({ "from": "plant/line.json", "to": "exports/copy.json" })),
        ("describe_model", json!({ "path": "exports/hidden.json" })),
        ("list_models", json!({ "folder": "exports" })),
    ] {
        let refused = setup.err(name, input.clone());
        assert_eq!(refused["kind"], "outside_models_folder", "{name} {input}: {refused}");
    }
    let listed = setup.ok("list_models", json!({}));
    assert_eq!(listed["models"], json!(["plant/line.json"]), "list_models never shows exports/");
}

#[test]
fn a_model_called_exports_elsewhere_is_still_a_model() {
    let setup = setup("scope-nested");
    setup.folder.write("plant/exports/line.json", common::SMALL);
    setup.ok("read_model", json!({ "path": "plant/exports/line.json" }));
    setup.ok("export_model", json!({ "path": "plant/exports/line.json", "format": "cypher" }));
    assert!(setup.folder.join("exports/plant/exports/line.cypher").is_file());
}

#[test]
fn a_path_into_exports_is_refused_before_any_export_exists() {
    let setup = setup("scope-fresh");
    let refused = setup.err("write_model", json!({ "path": "exports/new.json", "text": common::SMALL }));
    assert_eq!(refused["kind"], "outside_models_folder");
    assert!(!setup.folder.join("exports").exists());
}

/// Makes `link` a link to the folder `target`: a junction on Windows, which needs no special
/// rights, and a symbolic link elsewhere.
fn link_folder(link: &std::path::Path, target: &std::path::Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "a junction can be made");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn a_link_cant_carry_a_model_path_into_exports() {
    let setup = setup("scope-link-in");
    setup.folder.write("exports/hidden.json", common::SMALL);
    link_folder(&setup.folder.join("alias"), &setup.folder.join("exports"));
    let refused = setup.err("read_model", json!({ "path": "alias/hidden.json" }));
    assert_eq!(refused["kind"], "outside_models_folder");
}

#[test]
fn a_link_cant_carry_an_export_out_of_exports() {
    // A link inside `exports` to another folder of models.
    let setup = setup("scope-link-out");
    std::fs::create_dir_all(setup.folder.join("exports")).unwrap();
    link_folder(&setup.folder.join("exports").join("plant"), &setup.folder.join("plant"));
    let refused = setup.err("export_model", json!({ "path": "plant/line.json", "format": "cypher" }));
    assert_eq!(refused["kind"], "outside_models_folder");
    assert!(!setup.folder.join("plant/line.cypher").exists());

    // `exports` itself a link out of the models folder.
    let elsewhere = common::TempFolder::new("exports-elsewhere");
    let away = self::setup("scope-link-away");
    link_folder(&away.folder.join("exports"), &elsewhere.path);
    let refused = away.err("export_model", json!({ "path": "plant/line.json", "format": "graphml" }));
    assert_eq!(refused["kind"], "outside_models_folder");
    assert!(!elsewhere.join("plant").exists());
}

// ---------- Cost ----------

/// What an export costs on the benchmark's model of 429 copies of the demo (6,006 objects), and
/// how big it is. Timing only, so on demand in release:
/// `cargo test --release -p crabsim-catalogue --test export -- --ignored --nocapture`.
#[test]
#[ignore]
fn exports_of_6006_objects() {
    let demo = crabsim_model::load(&common::demo_text()).unwrap();
    let mut model = crabsim_model::Model { description: Some("429 copies of the demo model".into()), classes: demo.classes.clone(), ..Default::default() };
    for copy in 0..429 {
        let rename = |name: &str| format!("Copy {copy} / {name}");
        for instance in &demo.instances {
            let mut instance = instance.clone();
            instance.name = rename(&instance.name);
            instance.stream = instance.stream.map(|stream| stream + 100 * copy as u64);
            instance.successors = instance.successors.iter().map(|name| rename(name)).collect();
            model.instances.push(instance);
        }
    }
    let setup = Setup::new("export-large");
    setup.folder.write("large.json", &model.save());
    for format in ["cypher", "neo4j-csv", "graphml"] {
        let mut times = Vec::new();
        let mut bytes = 0;
        for _ in 0..7 {
            let start = std::time::Instant::now();
            let output = setup.ok("export_model", json!({ "path": "large.json", "format": format }));
            times.push(start.elapsed());
            bytes = output["files"].as_array().unwrap().iter().map(|file| file["bytes"].as_u64().unwrap()).sum::<u64>();
        }
        times.sort();
        println!("{format}: median {:?} of 7, fastest {:?}, {bytes} bytes", times[3], times[0]);
    }
}
