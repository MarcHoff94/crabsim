//! The workspace: a models folder opened through the catalogue as the user, its model list, a
//! model opened and laid out, a model with problems, a second window refused, and the seed. Plus
//! the end time and the seed as the view state offers them. No window, as in the other UI tests.

mod common;

use common::{FakeClock, Watched};
use crabsim::{ModelDescription, ObjectKind, Position, RunState, SessionOutput, SessionStatus, SimTime};
use crabsim_ui::view::{Control, EndTimeProblem, ViewState};
use crabsim_ui::workspace::Workspace;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A models folder of its own, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Folder(path)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Placed by hand, as the demo is.
const PLACED: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","position":{"x":5,"y":0},"successors":["Out"],"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain","position":{"x":10,"y":0}}]}"#;

/// Nothing placed: a Source, a split into two Stations that join again, a Drain, and a Station
/// nothing reaches.
const UNPLACED: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","successors":["Split"],"attributes":{"interval":"10"}},
  {"name":"Split","class":"Station","successors":["A","B"],"attributes":{"process_time":"1"}},
  {"name":"A","class":"Station","successors":["Join"],"attributes":{"process_time":"5"}},
  {"name":"B","class":"Station","successors":["Join"],"attributes":{"process_time":"5"}},
  {"name":"Join","class":"Station","successors":["Out"],"attributes":{"process_time":"2"}},
  {"name":"Out","class":"Drain"},
  {"name":"Orphan","class":"Station","attributes":{"process_time":"2"}}]}"#;

/// Two placed and two not.
const PARTLY_PLACED: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"successors":["Work"],"attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","successors":["Check"],"attributes":{"process_time":"8"}},
  {"name":"Check","class":"Station","successors":["Out"],"attributes":{"process_time":"3"}},
  {"name":"Out","class":"Drain","position":{"x":20,"y":4}}]}"#;

/// Three problems: an unknown successor, a Source with no interval, a Drain with a process time.
const BROKEN: &str = r#"{ "format_version": 1, "instances": [
    { "name": "In", "class": "Source", "successors": ["Nowhere"] },
    { "name": "Out", "class": "Drain", "attributes": { "process_time": "10" } } ] }"#;

fn demo_text() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../models/demo.json")).unwrap()
}

fn folder(name: &str) -> Folder {
    let folder = Folder::new(name);
    folder.write("placed.json", PLACED);
    folder.write("unplaced.json", UNPLACED);
    folder.write("plant/partly.json", PARTLY_PLACED);
    folder.write("plant/broken.json", BROKEN);
    folder.write("demo.json", &demo_text());
    folder
}

fn id_of(model: &ModelDescription, name: &str) -> u32 {
    (0..model.object_count()).find(|&id| model.name(id) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
}

/// Where an object stands, its centre.
fn at(model: &ModelDescription, name: &str) -> (f64, f64) {
    let position = model.position(id_of(model, name)).unwrap_or_else(|| panic!("{name} has no position"));
    (position.x, position.y)
}

/// An object's name and the min and max corners of its footprint.
type Footprint = (String, (f64, f64), (f64, f64));

/// Each object's footprint. Rotation is 0 in these models.
fn footprints(model: &ModelDescription) -> Vec<Footprint> {
    (0..model.object_count())
        .map(|id| {
            let position = model.position(id).expect("every object is placed");
            let size = model.dimensions(id);
            let (half_x, half_y) = (size.length / 2.0, size.width / 2.0);
            (model.name(id).unwrap().to_string(), (position.x - half_x, position.y - half_y), (position.x + half_x, position.y + half_y))
        })
        .collect()
}

fn assert_no_overlap(model: &ModelDescription) {
    let prints = footprints(model);
    for (i, a) in prints.iter().enumerate() {
        for b in &prints[i + 1..] {
            let apart = a.2.0 <= b.1.0 || b.2.0 <= a.1.0 || a.2.1 <= b.1.1 || b.2.1 <= a.1.1;
            assert!(apart, "{} and {} overlap: {prints:?}", a.0, b.0);
        }
    }
}

// ---------- Opening a folder ----------

#[test]
fn until_a_model_is_chosen_none_is_open_and_the_view_offers_no_control() {
    let folder = folder("none-yet");
    let workspace = Workspace::open(&folder.0);
    assert_eq!(workspace.opened(), None);
    assert_eq!(workspace.description().object_count(), 0);
    let view = ViewState::new(workspace.description());
    assert!(!view.has_model());
    for control in [Control::Init, Control::Start, Control::Halt, Control::Reset, Control::StepBack, Control::StepForward] {
        assert!(!view.allows(control), "{control:?} with no model");
    }
}

#[test]
fn a_folder_lists_its_models_marking_those_with_problems() {
    let folder = folder("list");
    let workspace = Workspace::open(&folder.0);
    assert_eq!(workspace.banner(), None);
    assert!(workspace.has_folder());
    let listed: Vec<(&str, usize)> = workspace.models().iter().map(|entry| (entry.path.as_str(), entry.problems)).collect();
    assert_eq!(listed, [("demo.json", 0), ("placed.json", 0), ("plant/broken.json", 3), ("plant/partly.json", 0), ("unplaced.json", 0)]);
    assert_eq!(workspace.opened(), None, "nothing is opened until one is chosen");
}

#[test]
fn opening_a_model_builds_it_with_its_names_and_positions() {
    let folder = folder("open");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("placed.json"));
    assert_eq!(workspace.opened(), Some("placed.json"));
    assert_eq!(workspace.problems(), None);
    let model = workspace.description();
    assert_eq!(model.object_count(), 3);
    assert_eq!(model.kind(id_of(model, "Work")), ObjectKind::Station);
    assert_eq!(at(model, "Work"), (5.0, 0.0), "a placed instance stays where it is");
}

// ---------- The automatic layout ----------

#[test]
fn a_model_with_nothing_placed_is_laid_out_in_columns_by_flow() {
    let folder = folder("layout");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("unplaced.json"));
    let model = workspace.description();
    let x = |name: &str| at(model, name).0;
    let y = |name: &str| at(model, name).1;
    assert!(x("In") < x("Split") && x("Split") < x("A") && x("A") < x("Join") && x("Join") < x("Out"), "columns left to right by the fewest connections from a Source");
    assert_eq!(x("A"), x("B"), "A and B are as far from the Source");
    assert!(y("A") > y("B"), "a column is stacked in file order, first at the top");
    assert!(x("Orphan") > x("Out"), "what nothing reaches goes in a column after the rest");
    assert_no_overlap(model);
}

#[test]
fn the_few_unplaced_instances_go_in_a_row_below_the_placed_ones() {
    let folder = folder("layout-partly");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("plant/partly.json"));
    let model = workspace.description();
    assert_eq!(at(model, "In"), (0.0, 0.0));
    assert_eq!(at(model, "Out"), (20.0, 4.0));
    let lowest_placed = footprints(model).iter().filter(|print| print.0 == "In" || print.0 == "Out").map(|print| print.1.1).fold(f64::MAX, f64::min);
    for name in ["Work", "Check"] {
        let top = footprints(model).into_iter().find(|print| print.0 == name).unwrap().2.1;
        assert!(top < lowest_placed, "{name} is below the placed instances");
    }
    assert_eq!(at(model, "Work").1, at(model, "Check").1, "in one row");
    assert!(at(model, "Work").0 < at(model, "Check").0, "in file order");
    assert_no_overlap(model);
}

#[test]
fn moving_one_instance_of_a_model_laid_out_by_the_window_keeps_the_others_where_they_were_drawn() {
    let folder = folder("move-unplaced");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("unplaced.json"));
    workspace.adopt().unwrap();
    let drawn = workspace.description().clone();
    let out = drawn.position(id_of(&drawn, "Out")).unwrap();
    workspace.move_instance("Out", Position { x: out.x + 4.0, y: out.y - 3.0, z: 0.0 }, false).unwrap();
    assert!(workspace.check().is_some());
    workspace.apply_pending();
    let moved = workspace.description();
    let round = |metres: f64| (metres * 100.0).round() / 100.0;
    for id in 0..moved.object_count() {
        let name = moved.name(id).unwrap();
        let expected = if name == "Out" { (round(out.x + 4.0), round(out.y - 3.0)) } else { at(&drawn, name) };
        let expected = (round(expected.0), round(expected.1));
        assert_eq!(at(moved, name), expected, "{name}");
    }
    let saved = std::fs::read_to_string(folder.0.join("unplaced.json")).unwrap();
    assert_eq!(saved.matches("\"position\"").count(), 7, "every instance is placed in the file now: {saved}");
}

// ---------- A model with problems ----------

#[test]
fn a_model_with_problems_shows_them_and_the_open_model_stays() {
    let folder = folder("problems");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("placed.json"));
    assert!(!workspace.open_model("plant/broken.json"));
    assert_eq!(workspace.opened(), Some("placed.json"), "the model open before stays open");
    assert_eq!(workspace.description().object_count(), 3);
    let report = workspace.problems().expect("the problems are shown");
    assert_eq!(report.path, "plant/broken.json");
    assert_eq!(report.problems.len(), 3);
    assert!(report.problems.iter().any(|problem| problem.contains("Nowhere")), "{:?}", report.problems);
    assert!(report.summary.iter().any(|line| line.contains("2 instances")), "describe_model's summary: {:?}", report.summary);

    assert!(workspace.open_model("placed.json"));
    assert_eq!(workspace.problems(), None, "opening a model that runs puts the problems away");
}

// ---------- A second window ----------

#[test]
fn a_second_window_on_the_folder_is_refused_and_opens_nothing() {
    let folder = folder("second");
    let first = Workspace::open(&folder.0);
    let second = Workspace::open(&folder.0);
    let banner = second.banner().expect("the second window says why");
    assert!(banner.contains("Another crabsim window") && banner.contains("copy"), "{banner}");
    assert!(!second.has_folder());
    assert!(second.models().is_empty());
    assert_eq!(second.description().object_count(), 0, "no model, not the demo");

    drop(first);
    drop(second);
    assert_eq!(Workspace::open(&folder.0).banner(), None, "the folder is free once the first window closes");
}

#[test]
fn a_folder_that_cant_be_opened_says_so_and_opens_nothing() {
    let missing = std::env::temp_dir().join("crabsim-ui-no-such-folder-at-all");
    let workspace = Workspace::open(&missing);
    assert!(workspace.banner().is_some());
    assert!(!workspace.has_folder());
    assert_eq!(workspace.description().object_count(), 0);
}

// ---------- The seed ----------

/// The state of `model` at simulation time `t` in a headless run with `seed`.
fn headless(model: &ModelDescription, seed: u64, t: SimTime) -> RunState {
    let mut run = model.build(seed);
    run.init().unwrap();
    run.advance_to(t).unwrap();
    run.state()
}

/// Runs the workspace's session to `t` and gives its state there.
fn session_state(workspace: &Workspace, t: SimTime) -> RunState {
    let clock = FakeClock::default();
    let session = workspace.session(clock.clone());
    session.set_speed(1_000.0).unwrap();
    session.set_end_time(Some(t)).unwrap();
    session.start().unwrap();
    session.sync().unwrap();
    clock.set(Duration::from_secs(1_000_000));
    loop {
        session.sync().unwrap();
        let reached = session.outputs().try_iter().any(|output| output == SessionOutput::Status(SessionStatus::ReachedEnd));
        if reached {
            break;
        }
    }
    session.request_state().unwrap();
    session.sync().unwrap();
    session.outputs().try_iter().find_map(|output| match output {
        SessionOutput::State(state) => Some(state),
        _ => None,
    }).expect("a state")
}

#[test]
fn the_seed_gives_the_runs_random_numbers_and_is_kept_when_another_model_opens() {
    let folder = folder("seed");
    let mut workspace = Workspace::open(&folder.0);
    assert_eq!(workspace.seed(), 0);
    assert!(workspace.open_model("demo.json"));
    workspace.set_seed(7);
    let t = SimTime::from_secs(10_000);
    let state = session_state(&workspace, t);
    assert_eq!(state, headless(workspace.description(), 7, t));
    assert_ne!(state, headless(workspace.description(), 0, t), "seed 7 isn't seed 0 for a model with failures");

    assert!(workspace.open_model("placed.json"));
    assert_eq!(workspace.seed(), 7, "the seed stays for the window");
}

#[test]
fn the_seed_can_change_only_before_init_or_after_reset() {
    let mut watched = Watched::demo();
    assert!(watched.view.allows_seed());
    watched.session.step_forward().unwrap();
    watched.frame();
    assert!(!watched.view.allows_seed(), "not once Init has run");
    watched.session.start().unwrap();
    watched.at(10);
    assert!(!watched.view.allows_seed(), "not while advancing");
    watched.session.reset().unwrap();
    watched.frame();
    assert!(watched.view.allows_seed(), "again after Reset");
}

// ---------- The end time ----------

#[test]
fn the_end_time_must_be_a_time_later_than_shown_time() {
    let mut watched = Watched::demo();
    assert_eq!(watched.view.end_time_target("30d"), Ok(SimTime::from_secs(30 * 86_400)));
    assert_eq!(watched.view.end_time_target("soon"), Err(EndTimeProblem::NotATime));
    assert_eq!(watched.view.end_time_target("0"), Err(EndTimeProblem::NotLater));
    watched.session.set_speed(10.0).unwrap();
    watched.session.start().unwrap();
    watched.at(10);
    assert_eq!(watched.view.end_time_target("50"), Err(EndTimeProblem::NotLater), "shown time is 100 s");
    assert_eq!(watched.view.end_time_target("101"), Ok(SimTime::from_secs(101)));
}

#[test]
fn at_the_end_time_the_view_offers_neither_start_nor_stepping_forward() {
    let mut watched = Watched::demo();
    let end = SimTime::from_secs(60);
    watched.session.set_end_time(Some(end)).unwrap();
    watched.view.set_end_time(Some(end));
    assert_eq!(watched.view.end_time(), Some(end));
    watched.session.start().unwrap();
    watched.at(1_000);
    assert_eq!(watched.view.status(), SessionStatus::ReachedEnd);
    assert_eq!(watched.view.shown_time(), "00:01:00");
    assert!(!watched.view.allows(Control::Start) && !watched.view.allows(Control::StepForward));
    assert!(watched.view.allows(Control::Reset) && watched.view.allows(Control::StepBack));
}

#[test]
fn a_new_view_keeps_the_speed_time_mode_and_end_time_of_the_old() {
    let model = crabsim::demo::model();
    let mut old = ViewState::new(&model);
    old.set_speed(500.0);
    old.set_time_mode(crabsim::TimeMode::SKIPPING);
    old.set_end_time(Some(SimTime::from_secs(3_600)));
    let new = ViewState::with_settings_of(&model, &old);
    assert_eq!(new.speed(), 500.0);
    assert_eq!(new.time_mode(), crabsim::TimeMode::SKIPPING);
    assert_eq!(new.end_time(), Some(SimTime::from_secs(3_600)));
}
