//! Building with cross conveyors in the window (spec issue 148, PR 3), through the workspace on a
//! real catalogue: each cross conveyor's exits as the view labels them, connecting at a side, and
//! moving a connection to another side. No window, as in the other UI tests.

use crabsim::Side;
use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::view::ViewState;
use crabsim_ui::workspace::{Refusal, SideChoice, Workspace};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// A models folder holding `cross-line.json`, the example, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-cross-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "cross-line.json", "text": include_str!("../../models/cross-line.json") })).unwrap();
        Folder(path)
    }

    fn model(&self) -> crabsim_model::Model {
        crabsim_model::load(&std::fs::read_to_string(self.0.join("cross-line.json")).unwrap()).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("cross-line.json"));
    workspace
}

fn picked_up(workspace: &mut Workspace) {
    assert!(workspace.check().is_some(), "the file changed");
    workspace.apply_pending();
}

fn id_of(workspace: &Workspace, name: &str) -> u32 {
    let model = workspace.description();
    (0..model.object_count()).find(|&id| model.name(id) == Some(name)).unwrap()
}

#[test]
fn each_cross_conveyor_s_exits_are_labelled_with_the_destinations_its_file_lists() {
    let folder = Folder::new("labels");
    let workspace = open(&folder);
    let lists = workspace.exit_lists();
    assert_eq!(lists.len(), 3, "the Fork, the Corner and the Join: {lists:?}");
    let fork = lists.iter().find(|(id, _)| *id == id_of(&workspace, "Fork")).unwrap();
    assert_eq!(fork.1, vec![(Side::Left, vec!["Mill".to_string()])]);

    let mut view = ViewState::new(workspace.description());
    view.set_exit_lists(lists);
    let labels: Vec<String> = view.exit_labels().into_iter().map(|(_, text)| text).collect();
    assert_eq!(labels, vec!["Mill", "Out", "Out"]);
}

#[test]
fn a_connection_at_a_cross_conveyor_shows_its_side_and_the_free_ones() {
    let folder = Folder::new("view");
    let workspace = open(&folder);
    let spur = workspace.connection("Fork", "Spur").unwrap();
    assert_eq!(spur.from_side, Some(SideChoice { side: Side::Left, free: vec![Side::Right] }));
    assert_eq!(spur.to_side, None, "the Spur is a conveyor");
    let feed = workspace.connection("Feed", "Fork").unwrap();
    assert_eq!(feed.to_side.map(|choice| choice.side), Some(Side::Back));
    assert_eq!(workspace.free_sides("Saw"), Vec::<Side>::new(), "a Station has no sides");
}

#[test]
fn a_connection_moves_to_another_side_and_a_new_one_takes_the_side_given() {
    let folder = Folder::new("sides");
    let mut workspace = open(&folder);
    workspace.set_connection_side("Fork", "Spur", true, Side::Right, false).unwrap();
    picked_up(&mut workspace);
    let saved = folder.model();
    let spur = saved.connections.iter().find(|connection| connection.from == "Fork" && connection.to == "Spur").unwrap();
    assert_eq!(spur.from_side, Some(Side::Right));
    assert_eq!(workspace.free_sides("Fork"), vec![Side::Left]);

    workspace.connect_at("Saw", None, "Fork", Some(Side::Left), false).unwrap();
    picked_up(&mut workspace);
    let saved = folder.model();
    let added = saved.connections.iter().find(|connection| connection.from == "Saw" && connection.to == "Fork").unwrap();
    assert_eq!(added.to_side, Some(Side::Left));
    assert_eq!(workspace.free_sides("Fork"), Vec::<Side>::new());
    // The view draws it meeting the left side's middle.
    let view = ViewState::new(workspace.description());
    let fork = id_of(&workspace, "Fork");
    let drawn = view.connections().iter().find(|connection| connection.to == fork && connection.to_side == Some(Side::Left)).unwrap();
    let middle = view.objects()[fork as usize].side_middle(Side::Left);
    assert!((drawn.end.x - middle.x).abs() < 1e-9 && (drawn.end.y - middle.y).abs() < 1e-9);
}

#[test]
fn a_connection_to_a_cross_conveyor_needs_a_free_side() {
    let folder = Folder::new("refused");
    let mut workspace = open(&folder);
    let Err(Refusal::Invalid(why)) = workspace.connect_at("Saw", None, "Fork", Some(Side::Back), false) else { panic!("the back has a connection") };
    assert!(why.contains("back"), "{why}");
    assert!(workspace.connect_at("Saw", None, "Fork", None, false).is_err(), "a cross conveyor's end names its side");
}

#[test]
fn the_names_a_destination_can_be_are_the_object_classes_then_the_instances() {
    let folder = Folder::new("names");
    let workspace = open(&folder);
    let destinations = workspace.names().destinations;
    // Housing and Cover are MU classes, which no MU goes to.
    assert_eq!(destinations.first().map(String::as_str), Some("In A"));
    assert!(destinations.contains(&"Mill".to_string()) && destinations.contains(&"Fork".to_string()));
    assert!(!destinations.contains(&"Housing".to_string()));
}

#[test]
fn a_work_plan_a_detour_and_exits_edited_in_their_forms_are_saved_as_given() {
    let folder = Folder::new("forms");
    let mut workspace = open(&folder);
    let names = workspace.names();
    let value_of = |workspace: &Workspace, class: Option<&str>, name: &str, attribute: &str| {
        let views = match class {
            Some(class) => workspace.class_attributes(class).unwrap(),
            None => workspace.attributes(name).unwrap(),
        };
        views.into_iter().find(|view| view.attribute == attribute).and_then(|view| view.value)
    };

    // The Housing's plan gets the Paint step, with a time, before Out.
    let plan = value_of(&workspace, Some("Housing"), "", "work_plan");
    let mut form = crabsim_ui::attributes::Form::for_attribute("work_plan", plan.as_ref(), &[], &names);
    assert_eq!(form.choices, names.destinations);
    let rows = form.plan.as_mut().unwrap();
    rows.add("Paint");
    rows.rows[3].time = "0:20".into();
    rows.swap_with_next(2);
    workspace.set_class_attribute("Housing", "work_plan", form.value().unwrap(), false).unwrap();
    picked_up(&mut workspace);
    let saved = value_of(&workspace, Some("Housing"), "", "work_plan");
    assert_eq!(saved, Some(json!([{ "at": "Saw" }, { "at": "Mill" }, { "at": "Paint", "process_time": "00:00:20" }, { "at": "Out" }])));

    // The Mill sends its MUs to Paint first.
    let mut detour = crabsim_ui::attributes::Form::for_attribute("detour", None, &[], &names);
    detour.parts[0].text = "Paint".into();
    workspace.set_attribute("Mill", "detour", detour.value().unwrap(), false).unwrap();
    picked_up(&mut workspace);
    assert_eq!(value_of(&workspace, None, "Mill", "detour"), Some(json!("Paint")));

    // The Fork's front lists Paint too.
    let exits = value_of(&workspace, None, "Fork", "exits");
    let mut form = crabsim_ui::attributes::Form::for_attribute("exits", exits.as_ref(), &[], &names);
    let front = form.exits.as_mut().unwrap().sides.iter_mut().find(|(side, _)| *side == Side::Front).unwrap();
    front.1.iter_mut().find(|(name, _)| name == "Paint").unwrap().1 = true;
    workspace.set_attribute("Fork", "exits", form.value().unwrap(), false).unwrap();
    picked_up(&mut workspace);
    let saved = value_of(&workspace, None, "Fork", "exits");
    assert_eq!(saved, Some(json!({ "front": ["Paint"], "left": ["Mill"] })));
    assert!(form.matches(saved.as_ref()), "nothing waits once saved");
    assert_eq!(workspace.exit_lists().into_iter().find(|(id, _)| *id == id_of(&workspace, "Fork")).unwrap().1, vec![(Side::Front, vec!["Paint".to_string()]), (Side::Left, vec!["Mill".to_string()])]);
}
