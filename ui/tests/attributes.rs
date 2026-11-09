//! The inspector's attributes: every attribute of an instance with where its value comes from,
//! changed and saved to the model file, each restored on its own, refused while an agent holds the
//! file unless taken over for the one change, instances moved, and the fields that turn text into a model file's
//! values. No window, as in the other UI tests.

use crabsim::Position;
use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::attributes::{Form, RepairKind};
use crabsim_ui::workspace::{ModelChange, Refusal, ValueFrom, Workspace};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A class of press, P1 of it as it is, and P2 setting its own process time and description.
const PRESSES: &str = r#"{"format_version":1,
  "classes":[{"name":"Press","parent":"Station","attributes":{"process_time":"0:05","description":"Hydraulic press"}}],
  "instances":[
    {"name":"In","class":"Source","position":{"x":0,"y":0},"successors":["P1","P2"],"attributes":{"interval":"10"}},
    {"name":"P1","class":"Press","position":{"x":5,"y":2},"successors":["Out"]},
    {"name":"P2","class":"Press","position":{"x":5,"y":-2},"successors":["Out"],"attributes":{"process_time":"0:07","description":"The old one"}},
    {"name":"Out","class":"Drain","position":{"x":10,"y":0}}]}"#;

struct Folder(PathBuf);

impl Folder {
    /// `presses.json`, saved by crabsim.
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-attributes-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "presses.json", "text": PRESSES })).unwrap();
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("presses.json"));
    workspace
}

/// An attribute's value and where it comes from.
fn of(workspace: &Workspace, instance: &str, attribute: &str) -> (Option<Value>, ValueFrom) {
    let attributes = workspace.attributes(instance).unwrap();
    let found = attributes.into_iter().find(|view| view.attribute == attribute).unwrap_or_else(|| panic!("{instance} has no {attribute}"));
    (found.value, found.from)
}

// ---------- Reading and changing ----------

#[test]
fn every_attribute_comes_with_its_value_and_where_it_comes_from() {
    let folder = Folder::new("read");
    let workspace = open(&folder);
    let names: Vec<String> = workspace.attributes("P1").unwrap().into_iter().map(|view| view.attribute).collect();
    assert_eq!(names, ["process_time", "failures", "starting_mus", "dimensions", "description"], "a Station's attributes, in a fixed order");
    assert_eq!(of(&workspace, "P1", "process_time"), (Some(json!("00:00:05")), ValueFrom::Class("Press".into())));
    assert_eq!(of(&workspace, "P2", "process_time"), (Some(json!("00:00:07")), ValueFrom::Instance));
    assert_eq!(of(&workspace, "P1", "starting_mus"), (Some(json!(0)), ValueFrom::BuiltIn));
    assert_eq!(of(&workspace, "P1", "failures"), (None, ValueFrom::NotSet));
    assert_eq!(of(&workspace, "In", "interval").1, ValueFrom::Instance);
}

#[test]
fn a_changed_attribute_is_saved_to_the_file_and_cuts_inheritance() {
    let folder = Folder::new("set");
    let mut workspace = open(&folder);
    workspace.set_attribute("P1", "process_time", json!("0:30"), false).unwrap();
    assert_eq!(workspace.check(), Some(ModelChange::Changed), "the window picks up the saved file");
    workspace.apply_pending();
    assert_eq!(of(&workspace, "P1", "process_time"), (Some(json!("00:00:30")), ValueFrom::Instance));
    let saved = std::fs::read_to_string(folder.0.join("presses.json")).unwrap();
    assert!(saved.contains("00:00:30"), "{saved}");
}

#[test]
fn each_attribute_is_restored_on_its_own() {
    let folder = Folder::new("restore");
    let mut workspace = open(&folder);
    workspace.restore("P2", "process_time", false).unwrap();
    assert_eq!(of(&workspace, "P2", "process_time"), (Some(json!("00:00:05")), ValueFrom::Class("Press".into())), "back to the class's value");
    assert_eq!(of(&workspace, "P2", "description"), (Some(json!("The old one")), ValueFrom::Instance), "the other attribute set on it stays");
}

#[test]
fn a_value_against_the_model_files_rules_is_refused_with_its_message() {
    let folder = Folder::new("invalid");
    let mut workspace = open(&folder);
    let refused = workspace.set_attribute("P1", "process_time", json!("soon"), false).unwrap_err();
    assert!(matches!(&refused, Refusal::Invalid(message) if message.contains("process_time")), "{refused:?}");
    assert_eq!(workspace.check(), None, "nothing was saved");
}

// ---------- Who may change it ----------

#[test]
fn while_an_agent_holds_the_file_a_change_is_refused_unless_taken_over_and_the_agent_keeps_its_right() {
    let folder = Folder::new("agent");
    let mut workspace = open(&folder);
    let agent = Catalogue::open(Settings::new(&folder.0, Source::Mcp)).unwrap();
    agent.call("claim_model", json!({ "path": "presses.json", "purpose": "tuning the presses" })).unwrap();

    let refused = workspace.set_attribute("P1", "process_time", json!("0:30"), false).unwrap_err();
    assert!(matches!(&refused, Refusal::NotEditor(message) if message.contains("tuning the presses")), "names the agent's purpose: {refused:?}");
    workspace.set_attribute("P1", "process_time", json!("0:30"), true).unwrap();
    let queue = agent.call("get_edit_queue", json!({ "path": "presses.json" })).unwrap();
    assert_eq!(queue["editor"]["agent"], "agent-1", "{queue}");

    let refused = workspace.restore("P2", "description", false).unwrap_err();
    assert!(matches!(refused, Refusal::NotEditor(_)), "a take-over lasts one change");
}

#[test]
fn a_file_changed_outside_crabsim_is_refused_until_adopted() {
    let folder = Folder::new("outside");
    let text = std::fs::read_to_string(folder.0.join("presses.json")).unwrap();
    std::fs::write(folder.0.join("presses.json"), text.replace("Hydraulic press", "Hydraulic press, rebuilt")).unwrap();
    let mut workspace = open(&folder);
    let refused = workspace.set_attribute("P1", "process_time", json!("0:30"), false).unwrap_err();
    assert!(matches!(refused, Refusal::ChangedOutside(_)), "{refused:?}");
    workspace.adopt().unwrap();
    workspace.set_attribute("P1", "process_time", json!("0:30"), false).unwrap();
}

#[test]
fn with_no_model_open_there_are_no_attributes() {
    let folder = Folder::new("none");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.attributes("P1").is_err());
    assert_eq!(workspace.set_attribute("P1", "process_time", json!("0:30"), false), Err(Refusal::NoModel));
}

// ---------- Moving ----------

/// Where the open model places an instance.
fn position_of(workspace: &Workspace, instance: &str) -> Position {
    let model = workspace.description();
    let id = (0..model.object_count()).find(|&id| model.name(id) == Some(instance)).unwrap_or_else(|| panic!("no {instance}"));
    model.position(id).expect("placed")
}

#[test]
fn a_moved_instance_is_saved_to_the_file_to_the_centimetre() {
    let folder = Folder::new("move");
    let mut workspace = open(&folder);
    workspace.move_instance("P1", Position { x: 7.123456, y: -0.996, z: 0.0 }, false).unwrap();
    assert_eq!(workspace.check(), Some(ModelChange::Changed), "the window picks up the saved file");
    workspace.apply_pending();
    assert_eq!(position_of(&workspace, "P1"), Position { x: 7.12, y: -1.0, z: 0.0 });
    assert_eq!(position_of(&workspace, "P2"), Position { x: 5.0, y: -2.0, z: 0.0 }, "nothing else moves");
    let saved = std::fs::read_to_string(folder.0.join("presses.json")).unwrap();
    assert!(saved.contains("7.12") && !saved.contains("7.123"), "{saved}");
}

#[test]
fn while_an_agent_holds_the_file_a_move_is_refused_unless_taken_over() {
    let folder = Folder::new("move-agent");
    let mut workspace = open(&folder);
    let agent = Catalogue::open(Settings::new(&folder.0, Source::Mcp)).unwrap();
    agent.call("claim_model", json!({ "path": "presses.json", "purpose": "rearranging the hall" })).unwrap();
    let to = Position { x: 6.0, y: 3.0, z: 0.0 };
    let refused = workspace.move_instance("P1", to, false).unwrap_err();
    assert!(matches!(&refused, Refusal::NotEditor(message) if message.contains("rearranging the hall")), "{refused:?}");
    assert_eq!(workspace.check(), None, "nothing was saved");
    workspace.move_instance("P1", to, true).unwrap();
    workspace.check();
    workspace.apply_pending();
    assert_eq!(position_of(&workspace, "P1"), to);
    let queue = agent.call("get_edit_queue", json!({ "path": "presses.json" })).unwrap();
    assert_eq!(queue["editor"]["agent"], "agent-1", "the agent keeps its right: {queue}");
}

#[test]
fn a_move_in_a_file_changed_outside_crabsim_is_refused_until_adopted() {
    let folder = Folder::new("move-outside");
    let text = std::fs::read_to_string(folder.0.join("presses.json")).unwrap();
    std::fs::write(folder.0.join("presses.json"), text.replace("Hydraulic press", "Hydraulic press, rebuilt")).unwrap();
    let mut workspace = open(&folder);
    let to = Position { x: 6.0, y: 3.0, z: 0.0 };
    let refused = workspace.move_instance("P1", to, false).unwrap_err();
    assert!(matches!(refused, Refusal::ChangedOutside(_)), "{refused:?}");
    assert_eq!(workspace.check(), None, "nothing was saved");
    workspace.adopt().unwrap();
    workspace.move_instance("P1", to, false).unwrap();
    workspace.check();
    workspace.apply_pending();
    assert_eq!(position_of(&workspace, "P1"), to);
}

// ---------- The fields ----------

fn typed(form: &mut Form, texts: &[&str]) {
    for (part, text) in form.parts.iter_mut().zip(texts) {
        part.text = text.to_string();
    }
}

#[test]
fn a_time_field_gives_the_time_typed_or_says_what_is_wrong() {
    let mut form = Form::new("process_time", Some(&json!("00:00:45")));
    assert_eq!(form.parts[0].text, "00:00:45");
    assert!(form.matches(Some(&json!("00:00:45"))));
    typed(&mut form, &["1:30"]);
    assert_eq!(form.value(), Ok(json!("1:30")));
    assert!(!form.matches(Some(&json!("00:00:45"))), "something waits to be applied");
    typed(&mut form, &["soon"]);
    assert!(form.value().unwrap_err().contains("isn't a time"));
}

#[test]
fn a_count_field_takes_only_whole_numbers() {
    let mut form = Form::new("starting_mus", Some(&json!(0)));
    assert_eq!(form.parts[0].text, "0");
    typed(&mut form, &["1"]);
    assert_eq!(form.value(), Ok(json!(1)));
    typed(&mut form, &["-1"]);
    assert!(form.value().is_err());
}

#[test]
fn failures_go_round_from_a_value_to_fields_and_back() {
    for value in [
        json!({ "availability": 0.9, "repair_time": { "erlang": { "mean": "00:01:00", "k": 2 } } }),
        json!({ "availability": 0.75, "repair_time": { "constant": "00:02:00" } }),
        json!({ "availability": 0.99, "repair_time": { "uniform": { "min": "00:00:30", "max": "00:01:30" } } }),
        json!({ "availability": 0.8, "repair_time": { "normal": { "mean": "00:01:00", "std": "00:00:10" } } }),
    ] {
        let form = Form::new("failures", Some(&value));
        assert_eq!(form.value(), Ok(value.clone()), "{value}");
    }
    let mut form = Form::new("failures", Some(&json!({ "availability": 0.9, "repair_time": { "erlang": { "mean": "00:01:00", "k": 2 } } })));
    assert_eq!(form.parts[0].text, "90", "availability as a percentage");
    form.set_repair(RepairKind::Constant);
    assert_eq!(form.value(), Ok(json!({ "availability": 0.9, "repair_time": { "constant": "00:01:00" } })), "what was typed stays");
    typed(&mut form, &["100"]);
    assert!(form.value().unwrap_err().contains("below 100"));
}

#[test]
fn failures_not_set_start_from_a_typical_profile() {
    let form = Form::new("failures", None);
    assert_eq!(form.repair, Some(RepairKind::Erlang));
    assert_eq!(form.value(), Ok(json!({ "availability": 0.95, "repair_time": { "erlang": { "mean": "00:01:00", "k": 2 } } })));
}

#[test]
fn dimensions_go_round_and_leave_out_no_rotation() {
    let value = json!({ "length": 2.0, "width": 1.5, "height": 1.5 });
    let mut form = Form::new("dimensions", Some(&value));
    assert_eq!(form.value(), Ok(value));
    typed(&mut form, &["3", "1", "2", "90"]);
    assert_eq!(form.value(), Ok(json!({ "length": 3.0, "width": 1.0, "height": 2.0, "rotation_degrees": 90.0 })));
    typed(&mut form, &["wide"]);
    assert!(form.value().is_err());
}
