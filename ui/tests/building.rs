//! Building a model in the window, through the workspace on a real catalogue: new and duplicated
//! model files, placing an instance from a class, connecting, moving and describing connections,
//! removing and renaming instances, and the class actions, each one `edit_model` batch. No window,
//! as in the other UI tests.

use crabsim::Position;
use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::workspace::{ClassEntry, ConnectionView, Dependents, End, Refusal, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A Source feeding two Stations of one class, which feed a Drain, with a class derived from a
/// derived class and a Buffer class, which the panel leaves out.
const LINE: &str = r#"{"format_version":2,"classes":[
  {"name":"Mill","parent":"Station","attributes":{"process_time":"0:45"}},
  {"name":"Fast mill","parent":"Mill","attributes":{"process_time":"0:30"}},
  {"name":"Store","parent":"Buffer","attributes":{"capacity":4}}],"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"10"}},
  {"name":"Station 1","class":"Mill","position":{"x":5,"y":2}},
  {"name":"Station 4","class":"Mill","position":{"x":5,"y":-2},"attributes":{"process_time":"0:50","description":"the old one"}},
  {"name":"Out","class":"Drain","position":{"x":10,"y":0}}],"connections":[
  {"from":"In","to":"Station 1","exit":1,"entrance":1},
  {"from":"In","to":"Station 4","exit":2,"entrance":1},
  {"from":"Station 1","to":"Out","exit":1,"entrance":1},
  {"from":"Station 4","to":"Out","exit":1,"entrance":2}]}"#;

/// A models folder holding `line.json`, saved by crabsim, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-building-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        Folder(path)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.0.join(relative)).unwrap()
    }

    /// The model file as it is now.
    fn model(&self) -> crabsim_model::Model {
        crabsim_model::load(&self.read("line.json")).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    workspace
}

/// Picks up the saved file as the window does with no run in progress.
fn picked_up(workspace: &mut Workspace) {
    assert!(workspace.check().is_some(), "the file changed");
    workspace.apply_pending();
}

fn names(model: &crabsim_model::Model) -> Vec<&str> {
    model.instances.iter().map(|instance| instance.name.as_str()).collect()
}

// ---------- Model files ----------

#[test]
fn a_new_model_is_an_empty_file_saved_by_crabsim_and_opened() {
    let folder = Folder::new("new");
    let mut workspace = open(&folder);
    assert_eq!(workspace.new_model("plant/press line").unwrap(), "plant/press line.json", ".json is added");
    assert_eq!(workspace.opened(), Some("plant/press line.json"));
    assert_eq!(workspace.description().object_count(), 0);
    assert!(workspace.models().iter().any(|entry| entry.path == "plant/press line.json" && entry.problems == 0));
    assert!(!workspace.changed_outside(), "crabsim saved it, so it can be built on at once");
    workspace.add_instance("Source", Position { x: 0.0, y: 0.0, z: 0.0 }, false).unwrap();
    assert!(matches!(workspace.new_model("plant/press line.json"), Err(Refusal::Invalid(_))), "an existing file is refused");
}

#[test]
fn duplicating_a_model_copies_it_under_a_new_name_and_lists_it() {
    let folder = Folder::new("duplicate");
    let mut workspace = open(&folder);
    assert_eq!(workspace.duplicate_model("line.json", "line variant").unwrap(), "line variant.json");
    assert_eq!(folder.read("line variant.json"), folder.read("line.json"));
    assert!(workspace.models().iter().any(|entry| entry.path == "line variant.json"));
    assert_eq!(workspace.opened(), Some("line.json"), "the model open stays open");
    assert!(workspace.duplicate_model("line.json", "line variant").is_err(), "a file already there is refused");
}

// ---------- Classes ----------

#[test]
fn the_classes_panel_is_a_tree_by_inheritance_without_buffers() {
    let folder = Folder::new("classes");
    let workspace = open(&folder);
    let tree: Vec<(String, usize, bool)> = workspace.classes().into_iter().map(|ClassEntry { name, depth, built_in, .. }| (name, depth, built_in)).collect();
    assert_eq!(
        tree,
        [
            ("Source".to_string(), 0, true),
            ("Station".to_string(), 0, true),
            ("Mill".to_string(), 1, false),
            ("Fast mill".to_string(), 2, false),
            ("Drain".to_string(), 0, true),
        ],
        "Buffer and the classes derived from it wait for build step 7"
    );
    let mill = workspace.class_attributes("Fast mill").unwrap();
    let process_time = mill.iter().find(|view| view.attribute == "process_time").unwrap();
    assert_eq!(process_time.value, Some(json!("00:00:30")));
}

#[test]
fn a_class_dropped_on_the_canvas_adds_an_instance_named_after_it_with_the_next_number() {
    let folder = Folder::new("drop");
    let mut workspace = open(&folder);
    assert_eq!(workspace.next_name("Mill"), "Mill 1");
    assert_eq!(workspace.next_name("Station"), "Station 5", "after the highest, Station 4");
    let name = workspace.add_instance("Station", Position { x: 7.123, y: -3.456, z: 0.0 }, false).unwrap();
    assert_eq!(name, "Station 5");
    let model = folder.model();
    let added = model.instances.last().unwrap();
    assert_eq!((added.name.as_str(), added.class.as_str()), ("Station 5", "Station"));
    assert_eq!(added.position, Some(Position { x: 7.12, y: -3.46, z: 0.0 }), "to the centimetre");
    assert_eq!(added.stream, Some(4), "a stream of its own, one more than the highest");
    assert!(added.attributes.is_empty(), "nothing set: it inherits everything");
    picked_up(&mut workspace);
    assert_eq!(workspace.next_name("Station"), "Station 6");
    // A Source needs nothing set first: its interval is 0 by default.
    assert_eq!(workspace.add_instance("Source", Position { x: 0.0, y: 5.0, z: 0.0 }, false).unwrap(), "Source 1");
}

// ---------- Connections ----------

#[test]
fn connecting_dragging_from_one_object_to_another_adds_the_last_exit_and_entrance() {
    let folder = Folder::new("connect");
    let mut workspace = open(&folder);
    workspace.connect("Station 1", "Station 4", false).unwrap();
    picked_up(&mut workspace);
    let view = workspace.connection("Station 1", "Station 4").unwrap();
    assert_eq!(view, ConnectionView { from: "Station 1".into(), to: "Station 4".into(), exit: 2, exits: 2, entrance: 2, entrances: 2, description: None });
    assert_eq!(view.exit_text(), "2nd of Station 1's 2 exits");
    assert_eq!(view.entrance_text(), "2nd of Station 4's 2 entrances");
}

#[test]
fn a_refused_connection_says_why_as_the_file_check_does() {
    let folder = Folder::new("refused");
    let mut workspace = open(&folder);
    let before = folder.read("line.json");
    let Err(Refusal::Invalid(why)) = workspace.connect("Out", "Station 1", false) else { panic!("a Drain can't pass MUs on") };
    assert!(why.contains("`Out` is a Drain"), "{why}");
    workspace.add_instance("Source", Position { x: 0.0, y: 5.0, z: 0.0 }, false).unwrap();
    picked_up(&mut workspace);
    let Err(Refusal::Invalid(why)) = workspace.connect("Source 1", "Out", false) else { panic!("a 0-interval Source into a Drain") };
    assert!(why.contains("create MUs forever"), "{why}");
    assert!(workspace.connect("In", "Station 1", false).is_err(), "already connected");
    assert_ne!(folder.read("line.json"), before, "only the Source was added");
}

#[test]
fn a_connection_moves_up_and_down_its_exits_and_entrances_and_takes_a_description() {
    let folder = Folder::new("move-connection");
    let mut workspace = open(&folder);
    workspace.move_connection("In", "Station 4", End::Exit, 1, false).unwrap();
    workspace.move_connection("Station 4", "Out", End::Entrance, 1, false).unwrap();
    workspace.describe_connection("Station 4", "Out", "via the lift", false).unwrap();
    picked_up(&mut workspace);
    let model = folder.model();
    assert_eq!(model.successors("In"), ["Station 4", "Station 1"]);
    assert_eq!(model.predecessors("Out"), ["Station 4", "Station 1"]);
    assert_eq!(workspace.connection("Station 4", "Out").unwrap().description.as_deref(), Some("via the lift"));
    workspace.describe_connection("Station 4", "Out", "", false).unwrap();
    assert_eq!(folder.model().connections_in("Out")[0].description, None, "an empty description removes it");
}

#[test]
fn disconnecting_removes_the_connection_and_closes_up_the_others() {
    let folder = Folder::new("disconnect");
    let mut workspace = open(&folder);
    workspace.disconnect("In", "Station 1", false).unwrap();
    let model = folder.model();
    assert_eq!(model.connections_out("In").iter().map(|c| (c.to.as_str(), c.exit)).collect::<Vec<_>>(), [("Station 4", 1)]);
}

// ---------- Instances ----------

#[test]
fn removing_an_instance_takes_its_connections_which_are_named_first() {
    let folder = Folder::new("remove");
    let mut workspace = open(&folder);
    assert_eq!(
        workspace.instance_connections("Station 1"),
        [("In".to_string(), "Station 1".to_string()), ("Station 1".to_string(), "Out".to_string())]
    );
    assert_eq!(workspace.instance_connections("Nobody"), Vec::<(String, String)>::new());
    workspace.remove_instance("Station 1", false).unwrap();
    let model = folder.model();
    assert_eq!(names(&model), ["In", "Station 4", "Out"]);
    assert_eq!(model.connections.len(), 2);
    assert_eq!(model.predecessors("Out"), ["Station 4"]);
}

#[test]
fn renaming_an_instance_renames_it_in_its_connections_and_refuses_a_name_in_use() {
    let folder = Folder::new("rename");
    let mut workspace = open(&folder);
    workspace.rename_instance("Station 4", "Old mill", false).unwrap();
    let model = folder.model();
    assert_eq!(model.successors("In"), ["Station 1", "Old mill"]);
    assert!(workspace.rename_instance("Station 1", "Out", false).is_err());
    assert!(workspace.rename_instance("Station 1", " Padded ", false).is_err(), "the file's rules on names");
}

// ---------- Class actions ----------

#[test]
fn classes_are_derived_duplicated_and_renamed_through_their_references() {
    let folder = Folder::new("class-actions");
    let mut workspace = open(&folder);
    workspace.derive_class("Station", "Lathe", false).unwrap();
    workspace.duplicate_class("Mill", "Mill copy", false).unwrap();
    workspace.rename_class("Mill", "CNC mill", false).unwrap();
    let model = folder.model();
    assert_eq!(model.class("Lathe").unwrap().parent, "Station");
    assert!(model.class("Lathe").unwrap().attributes.is_empty());
    assert_eq!(model.class("Mill copy").unwrap().attributes, model.class("CNC mill").unwrap().attributes);
    assert_eq!(model.class("Fast mill").unwrap().parent, "CNC mill");
    assert_eq!(model.instances[1].class, "CNC mill");
    assert!(workspace.derive_class("Station", "Source", false).is_err(), "a built-in class's name");
}

#[test]
fn removing_a_class_lists_what_depends_on_it_then_takes_them_all_in_one_change() {
    let folder = Folder::new("remove-class");
    let mut workspace = open(&folder);
    assert_eq!(
        workspace.class_dependents("Mill"),
        Dependents { classes: vec!["Fast mill".into()], instances: vec!["Station 1".into(), "Station 4".into()] }
    );
    assert_eq!(workspace.class_dependents("Store"), Dependents::default());
    workspace.remove_class("Mill", false).unwrap();
    let model = folder.model();
    assert!(model.class("Mill").is_none() && model.class("Fast mill").is_none());
    assert_eq!(names(&model), ["In", "Out"]);
    assert!(model.connections.is_empty());
}

#[test]
fn a_class_made_from_a_tuned_instance_takes_its_values_and_the_instance_follows_it() {
    let folder = Folder::new("make-class");
    let mut workspace = open(&folder);
    workspace.make_class("Station 4", "Slow mill", false).unwrap();
    let model = folder.model();
    let class = model.class("Slow mill").unwrap();
    assert_eq!(class.parent, "Mill", "derived from the instance's class");
    assert_eq!(class.attributes.process_time, Some(crabsim::SimTime::from_secs(50)));
    assert_eq!(class.attributes.description.as_deref(), Some("the old one"));
    let instance = &model.instances[2];
    assert_eq!(instance.class, "Slow mill");
    assert!(instance.attributes.is_empty(), "its values moved to the class");
    assert_eq!(model.resolve(2).unwrap().attributes.process_time, Some(crabsim::SimTime::from_secs(50)), "and it behaves as before");

    // One change: a name in use refuses all of it.
    let before = folder.read("line.json");
    assert!(workspace.make_class("Station 1", "Fast mill", false).is_err());
    assert_eq!(folder.read("line.json"), before);
}
