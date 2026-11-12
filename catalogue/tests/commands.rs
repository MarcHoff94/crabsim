//! The command language and `run_commands` (spec issue 102): lines of commands read against a
//! model, as the window's `:` line and agents use them.

mod common;

use common::Setup;
use crabsim_catalogue::commands::{complete, find, parse, Context, Parsed, WindowCommand};
use serde_json::{json, Value};

/// A Source feeding two Mills of one class, which feed a Drain, all placed.
const LINE: &str = r#"{"format_version":2,"classes":[
  {"name":"Mill","parent":"Station","attributes":{"process_time":"0:45"}}],"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"0:30"}},
  {"name":"Mill 1","class":"Mill","position":{"x":4,"y":2}},
  {"name":"Mill 2","class":"Mill","position":{"x":4,"y":-2}},
  {"name":"Out","class":"Drain","position":{"x":8,"y":0}}],"connections":[
  {"from":"In","to":"Mill 1","exit":1,"entrance":1},
  {"from":"In","to":"Mill 2","exit":2,"entrance":1},
  {"from":"Mill 1","to":"Out","exit":1,"entrance":1},
  {"from":"Mill 2","to":"Out","exit":1,"entrance":2}]}"#;

fn model() -> crabsim_model::Model {
    crabsim_model::load(LINE).unwrap()
}

fn changes(line: &str, context: &Context) -> Vec<Value> {
    match parse(line, &model(), context) {
        Ok(Parsed::Changes(planned)) => planned.into_iter().flat_map(|command| command.changes).collect(),
        other => panic!("{line}: {other:?}"),
    }
}

fn refused(line: &str) -> String {
    parse(line, &model(), &Context::default()).expect_err(line)
}

#[test]
fn each_command_becomes_edit_model_changes() {
    let none = Context::default();
    assert_eq!(changes("connect In > Out", &none), vec![json!({ "change": "connect", "from": "In", "to": "Out" })]);
    assert_eq!(changes("connect In>Out", &none), vec![json!({ "change": "connect", "from": "In", "to": "Out" })], "spaces around > or not");
    assert_eq!(changes("connect Mill 1 >Out", &none), vec![json!({ "change": "connect", "from": "Mill 1", "to": "Out" })]);
    assert_eq!(changes("disconnect mill 1 > out", &none), vec![json!({ "change": "disconnect", "from": "Mill 1", "to": "Out" })], "ignoring case");
    assert_eq!(changes("rename Mill 2 to Drill", &none), vec![json!({ "change": "rename_instance", "name": "Mill 2", "to": "Drill" })]);
    assert_eq!(changes("remove Out", &none), vec![json!({ "change": "remove_instance", "name": "Out" })]);
    assert_eq!(
        changes("set Mill 1.proc 0:20", &none),
        vec![json!({ "change": "set_attribute", "target": { "instance": "Mill 1" }, "attribute": "process_time", "value": "0:20" })],
        "an attribute by part of its name, a time as text"
    );
    assert_eq!(
        changes("set Mill.starting_mus 1", &none),
        vec![json!({ "change": "set_attribute", "target": { "class": "Mill" }, "attribute": "starting_mus", "value": 1 })],
        "a class when no instance is called that, a number as a number"
    );
    assert_eq!(changes("restore Mill 2.process_time", &none), vec![json!({ "change": "restore_inheritance", "target": { "instance": "Mill 2" }, "attribute": "process_time" })]);
    assert_eq!(changes("move Out to 10 1", &none), vec![json!({ "change": "move_instance", "name": "Out", "position": { "x": 10.0, "y": 1.0 } })]);
    assert_eq!(changes("move Out by 2 -1", &none), vec![json!({ "change": "move_instance", "name": "Out", "position": { "x": 10.0, "y": -1.0 } })]);
    assert_eq!(changes("derive Mill as Fast mill", &none), vec![json!({ "change": "add_class", "name": "Fast mill", "parent": "Mill" })]);
    assert_eq!(changes("class rename Mill to Lathe", &none), vec![json!({ "change": "rename_class", "name": "Mill", "to": "Lathe" })]);
    assert_eq!(changes("class duplicate Mill as Mill copy", &none), vec![json!({ "change": "duplicate_class", "name": "Mill", "to": "Mill copy" })]);
    assert_eq!(changes("class remove Mill", &none), vec![json!({ "change": "remove_class", "name": "Mill", "delete_dependents": true })]);
}

#[test]
fn add_places_beside_the_selection_or_the_last_placed_or_where_it_is_told() {
    let beside_mill = Context { selected: Some("Mill 1".into()), at: None };
    // A Mill is 2 m long: 4 + 1 + 2 + 1.
    assert_eq!(
        changes("add Mill", &beside_mill),
        vec![json!({ "change": "add_instance", "name": "Mill 3", "class": "Mill", "position": { "x": 8.0, "y": 2.0 } })]
    );
    let line = changes("add Mill as A; add Mill as B", &beside_mill);
    assert_eq!(line[1]["position"], json!({ "x": 12.0, "y": 2.0 }), "beside the one placed before it");
    let nothing = Context { selected: None, at: Some((20.0, 5.0)) };
    assert_eq!(changes("add Drain", &nothing)[0]["position"], json!({ "x": 20.0, "y": 5.0 }));
    assert_eq!(changes("add Drain at 1 2", &beside_mill)[0]["position"], json!({ "x": 1.0, "y": 2.0 }));
    // Stations are already in use, so a new one gets 10 s on itself, as in the window.
    assert_eq!(changes("add Station", &nothing)[0]["attributes"], json!({ "process_time": "00:00:10" }));
}

#[test]
fn a_line_reads_names_added_and_renamed_earlier_in_it() {
    let line = changes("add Mill as Drill at 6 0; connect In > Drill; rename Drill to Drill 2; set Drill 2.process_time 0:20", &Context::default());
    assert_eq!(line.len(), 4);
    assert_eq!(line[1], json!({ "change": "connect", "from": "In", "to": "Drill" }));
    assert_eq!(line[3]["target"], json!({ "instance": "Drill 2" }));
}

#[test]
fn a_refusal_names_the_command_by_its_place() {
    assert!(refused("connect In > Out; connect Mill > Out").starts_with("2nd command (connect Mill > Out): `Mill` could be `Mill 1`, `Mill 2`"), "{}", refused("connect In > Out; connect Mill > Out"));
    assert!(refused("remove Nobody").contains("no instance is called anything like `Nobody`"));
    assert!(refused("fly In").contains("there is no command `fly`"));
    assert!(refused("set In.speed 4").contains("name.attribute"), "{}", refused("set In.speed 4"));
    assert!(refused("connect In Out").contains("write it as connect … > …"));
    assert!(refused("add Mill; goto 2d").contains("give it a line of its own"), "a line is model changes or one window command");
}

#[test]
fn window_commands_stand_alone() {
    let window = |line: &str| match parse(line, &model(), &Context::default()) {
        Ok(Parsed::Window(command)) => command,
        other => panic!("{line}: {other:?}"),
    };
    assert_eq!(window("goto 2d"), WindowCommand::GoTo(crabsim::SimTime::from_secs(2 * 86_400)));
    assert_eq!(window("speed 100"), WindowCommand::Speed(100.0));
    assert_eq!(window("seed 7"), WindowCommand::Seed(7));
    assert_eq!(window("open line.json"), WindowCommand::Open("line.json".into()));
    assert_eq!(window("validate"), WindowCommand::Validate);
    assert!(parse("goto soon", &model(), &Context::default()).is_err());
}

#[test]
fn names_are_found_exactly_first_then_by_a_part_only_one_holds() {
    let names = ["Mill 1", "Mill 10", "Out"];
    assert_eq!(find(&names, "Mill 1", "instance"), Ok("Mill 1".into()), "exact, though Mill 10 holds it too");
    assert_eq!(find(&names, "10", "instance"), Ok("Mill 10".into()));
    assert!(find(&names, "mill", "instance").unwrap_err().contains("could be"));
}

#[test]
fn completion_offers_commands_names_and_attributes() {
    let model = model();
    assert_eq!(complete("con", &model), vec!["connect ".to_string()]);
    assert!(complete("connect In > O", &model).contains(&"connect In > Out".to_string()));
    assert!(complete("connect In>O", &model).contains(&"connect In>Out".to_string()), "without spaces too");
    assert!(complete("add M", &model).contains(&"add Mill".to_string()), "classes for add");
    assert!(complete("set Mill 1.pro", &model).contains(&"set Mill 1.process_time ".to_string()));
    assert!(complete("add Mill; con", &model).contains(&"add Mill; connect ".to_string()), "the last command of a line");
}

// ---------- run_commands ----------

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    setup
}

#[test]
fn run_commands_applies_a_line_as_one_batch_and_says_what_it_placed() {
    let setup = setup("run-commands");
    let ran = setup.ok("run_commands", json!({ "path": "line.json", "commands": "add Mill as Drill; connect Mill 1 > Drill; connect Drill > Out", "selected": "Mill 1" }));
    common::assert_fits_its_schema(&setup.catalogue, "run_commands", &ran);
    assert_eq!(ran["commands"], 3);
    assert_eq!(ran["placed"], json!(["Drill"]));
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    assert_eq!(model.successors("Mill 1"), ["Out", "Drill"]);
    assert_eq!(model.instances.last().unwrap().position, Some(crabsim::Position { x: 8.0, y: 2.0, z: 0.0 }));

    // All or nothing, named by command.
    let before = setup.folder.read("line.json");
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "connect In > Out; add Mill as In" }));
    assert!(refused["message"].as_str().unwrap().contains("2nd command (add Mill as In): an instance is already called `In`"), "{refused}");
    assert_eq!(setup.folder.read("line.json"), before, "nothing changed");

    let window = setup.err("run_commands", json!({ "path": "line.json", "commands": "goto 2d" }));
    assert!(window["message"].as_str().unwrap().contains("use start_run"), "{window}");
}

#[test]
fn an_agent_needs_its_claim_to_run_commands() {
    let setup = Setup::agent("run-commands-agent");
    setup.ok("claim_model", json!({ "path": "line.json", "goal": "build a line", "doing": "writing it" }));
    setup.ok("write_model", json!({ "path": "line.json", "text": LINE }));
    setup.ok("release_model", json!({ "path": "line.json" }));
    let refused = setup.err("run_commands", json!({ "path": "line.json", "commands": "remove Out" }));
    assert_eq!(refused["kind"], "not_editor", "{refused}");
    setup.ok("claim_model", json!({ "path": "line.json", "goal": "tidy up", "doing": "removing Out" }));
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "remove Out" }));
}
