//! The Paths tab and Validate, against a real catalogue on a temporary models folder, with no
//! window: routes found through `query_model`, weights and strategies worked out as the canvas
//! draws them, and what Validate lists. See spec issue 96.

mod common;

use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::paths::{strategies, weights, Paths, Strategy, Weigh};
use crabsim_ui::view::ViewState;
use crabsim_ui::workspace::Workspace;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// In into Split, a weighted Station preferring the lightest route, which sends to A (10 s) then
/// C (5 s), or to B (3 s) then D (30 s), all into Out. A Lost Station nothing reaches.
const PLAN: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"10"}},
  {"name":"Split","class":"Station","position":{"x":3,"y":0},"attributes":{"distribution":{"weighted":{"by":"process_time","drains":["Out","Gone"]}}}},
  {"name":"A","class":"Station","position":{"x":6,"y":2},"attributes":{"process_time":"10"}},
  {"name":"B","class":"Station","position":{"x":6,"y":-2},"attributes":{"process_time":"3"}},
  {"name":"C","class":"Station","position":{"x":9,"y":2},"attributes":{"process_time":"5"}},
  {"name":"D","class":"Station","position":{"x":9,"y":-2},"attributes":{"process_time":"30"}},
  {"name":"Out","class":"Drain","position":{"x":12,"y":0}},
  {"name":"Lost","class":"Station","position":{"x":0,"y":6}}],"connections":[
  {"from":"In","to":"Split","exit":1,"entrance":1},
  {"from":"Split","to":"A","exit":1,"entrance":1},
  {"from":"Split","to":"B","exit":2,"entrance":1},
  {"from":"A","to":"C","exit":1,"entrance":1},
  {"from":"B","to":"D","exit":1,"entrance":1},
  {"from":"C","to":"Out","exit":1,"entrance":1},
  {"from":"D","to":"Out","exit":1,"entrance":2},
  {"from":"Lost","to":"Out","exit":1,"entrance":3}]}"#;

const IN: u32 = 0;
const SPLIT: u32 = 1;
const A: u32 = 2;
const B: u32 = 3;
const C: u32 = 4;
const D: u32 = 5;
const OUT: u32 = 6;
const LOST: u32 = 7;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-paths-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "plan.json", "text": PLAN })).unwrap();
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(folder: &Folder) -> (Workspace, ViewState) {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("plan.json"));
    let view = ViewState::new(workspace.description());
    (workspace, view)
}

/// Finds the routes as the window does each frame.
fn find(paths: &mut Paths, workspace: &Workspace, view: &ViewState, model: u64) {
    assert!(paths.needs_finding(model, None), "the inputs changed");
    let answer = workspace.query(paths.query(view.objects(), None).unwrap());
    paths.take_answer(answer, view.objects(), model, None);
    assert!(!paths.needs_finding(model, None), "found once for the same inputs");
}

#[test]
fn a_route_is_found_once_for_its_inputs_and_the_best_is_highlighted() {
    let folder = Folder::new("route");
    let (workspace, view) = open(&folder);
    let mut paths = Paths::default();
    paths.from = Some(IN);
    paths.to = Some(OUT);
    assert!(!paths.needs_finding(1, None), "nothing is found while the route view is off");
    paths.route_on = true;

    // By connections: both routes take four.
    find(&mut paths, &workspace, &view, 1);
    let found = paths.found.clone().unwrap().unwrap();
    assert_eq!(found.routes.len(), 2);
    assert!(found.routes.iter().all(|route| route.total == 4.0));
    assert_eq!(found.count, 2);

    // By process time, largest first: the route through B and D, 33 s, is the critical path.
    paths.weigh = Weigh::Attribute("process_time");
    paths.largest = true;
    find(&mut paths, &workspace, &view, 1);
    let found = paths.found.clone().unwrap().unwrap();
    assert_eq!(found.routes[0].ids, vec![IN, SPLIT, B, D, OUT]);
    assert_eq!(found.routes[0].total, 33.0);
    assert_eq!(found.weightless, vec![IN, OUT]);
    assert_eq!(paths.highlighted(), Some(&[IN, SPLIT, B, D, OUT][..]));
    paths.chosen = 1;
    assert_eq!(paths.highlighted(), Some(&[IN, SPLIT, A, C, OUT][..]), "clicking another highlights it");

    // Another model means finding again; the same one doesn't.
    assert!(paths.needs_finding(2, None));
    paths.route_on = false;
    assert_eq!(paths.highlighted(), None);
}

#[test]
fn a_route_weighed_by_a_live_figure_is_found_again_as_the_figures_change() {
    let folder = Folder::new("live");
    let (workspace, view) = open(&folder);
    let mut paths = Paths::default();
    (paths.route_on, paths.from, paths.to, paths.weigh, paths.largest) = (true, Some(IN), Some(OUT), Weigh::Figure("working_share"), true);
    let shares = |a: f64, b: f64| {
        let mut values = vec![Some(0.0); 8];
        values[A as usize] = Some(a);
        values[B as usize] = Some(b);
        values
    };
    let first = shares(0.9, 0.1);
    let query = paths.query(view.objects(), Some(&first)).unwrap();
    assert_eq!(query["weight"]["values"]["A"], json!(0.9));
    paths.take_answer(workspace.query(query), view.objects(), 1, Some(&first));
    assert_eq!(paths.highlighted(), Some(&[IN, SPLIT, A, C, OUT][..]));
    assert!(!paths.needs_finding(1, Some(&first)));
    let later = shares(0.2, 0.7);
    assert!(paths.needs_finding(1, Some(&later)), "the run went on");
    paths.take_answer(workspace.query(paths.query(view.objects(), Some(&later)).unwrap()), view.objects(), 1, Some(&later));
    assert_eq!(paths.highlighted(), Some(&[IN, SPLIT, B, D, OUT][..]));
}

#[test]
fn weights_and_strategies_are_what_the_canvas_writes_beside_each_object() {
    let folder = Folder::new("overlay");
    let (workspace, _) = open(&folder);
    let by_time = weights(workspace.model(), &Weigh::Attribute("process_time"), None);
    assert_eq!(by_time[A as usize], Some(10.0));
    assert_eq!(by_time[IN as usize], None, "a Source has no process time");
    assert_eq!(by_time[LOST as usize], Some(0.0), "a Station without one is a dummy, 0 s");
    assert_eq!(Weigh::Attribute("process_time").show(10.0), "10 s");
    assert_eq!(weights(workspace.model(), &Weigh::Connections, None), vec![None; 8]);

    let marked = strategies(workspace.model(), workspace.description());
    assert_eq!(marked[OUT as usize], None, "a Drain has no successors to choose from");
    assert_eq!(marked[IN as usize], Some(Strategy { weighted: false, places: Vec::new() }));
    // Split prefers A: 10 + 5 s onward against B's 3 + 30 s.
    assert_eq!(marked[SPLIT as usize], Some(Strategy { weighted: true, places: vec![(A, 1), (B, 2)] }));
}

#[test]
fn validate_lists_problems_and_warnings_each_with_what_it_is_about() {
    let folder = Folder::new("validate");
    let (workspace, _) = open(&folder);
    let issues = workspace.validate().unwrap();
    assert!(issues.iter().all(|issue| issue.warning), "the model runs: {issues:#?}");
    let lost = issues.iter().find(|issue| issue.instance == Some(LOST)).expect("Lost is named");
    assert!(lost.message.contains("no Source reaches it"), "{lost:?}");
    let gone = issues.iter().find(|issue| issue.message.contains("`Gone` isn't a Drain")).expect("the Drain list is checked");
    assert_eq!(gone.instance, Some(SPLIT));
    assert_eq!(gone.pointer, "/instances/1/attributes/distribution/weighted/drains/1");
}

#[test]
fn a_live_figure_comes_from_the_runs_last_full_state() {
    let mut watched = common::Watched::demo();
    assert_eq!(watched.view.live_figure("working_share"), None, "nothing before Init");
    watched.session.start().unwrap();
    watched.at(600);
    watched.details();
    let busy = watched.view.live_figure("working_share").unwrap();
    assert_eq!(busy.len(), watched.view.objects().len());
    let station = busy[common::STATION_1 as usize].unwrap();
    assert!(station > 0.0 && station <= 1.0, "a share: {station}");
    let mus_in = watched.view.live_figure("mus_in").unwrap();
    assert!(mus_in[common::STATION_1 as usize].unwrap() > 0.0);
}
