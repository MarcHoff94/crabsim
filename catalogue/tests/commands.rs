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
    let beside_mill = Context { selected: Some("Mill 1".into()), at: None, macros: Vec::new() };
    // A Mill is 2 m long: 4 + 1 + 2 + 1.
    assert_eq!(
        changes("add Mill", &beside_mill),
        vec![json!({ "change": "add_instance", "name": "Mill 3", "class": "Mill", "position": { "x": 8.0, "y": 2.0 } })]
    );
    let line = changes("add Mill as A; add Mill as B", &beside_mill);
    assert_eq!(line[1]["position"], json!({ "x": 12.0, "y": 2.0 }), "beside the one placed before it");
    let nothing = Context { selected: None, at: Some((20.0, 5.0)), macros: Vec::new() };
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
    assert_eq!(complete("con", &model, &[]), vec!["connect ".to_string()]);
    assert!(complete("connect In > O", &model, &[]).contains(&"connect In > Out".to_string()));
    assert!(complete("connect In>O", &model, &[]).contains(&"connect In>Out".to_string()), "without spaces too");
    assert!(complete("add M", &model, &[]).contains(&"add Mill".to_string()), "classes for add");
    assert!(complete("set Mill 1.pro", &model, &[]).contains(&"set Mill 1.process_time ".to_string()));
    assert!(complete("add Mill; con", &model, &[]).contains(&"add Mill; connect ".to_string()), "the last command of a line");
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

// ---------- macros ----------

fn with_macros(macros: &[(&str, &[&str])], selected: Option<&str>) -> Context {
    Context {
        selected: selected.map(str::to_string),
        at: Some((20.0, 0.0)),
        macros: macros.iter().map(|(name, commands)| (name.to_string(), commands.iter().map(|command| command.to_string()).collect())).collect(),
    }
}

#[test]
fn a_macro_runs_its_commands_with_its_arguments_selected_and_last_filled_in() {
    let context = with_macros(&[("feed", &["add $1", "connect $selected > $last"])], Some("Mill 1"));
    let line = match parse("@feed Drain", &model(), &context) {
        Ok(Parsed::Changes(planned)) => planned,
        other => panic!("{other:?}"),
    };
    assert_eq!(line[0].placed.as_deref(), Some("Drain 1"));
    assert_eq!(line[1].changes[0], json!({ "change": "connect", "from": "Mill 1", "to": "Drain 1" }));

    // Repeated in one line, $last follows each add, so a line builds on.
    let context = with_macros(&[("next", &["add Mill", "connect $selected > $last"])], Some("Mill 1"));
    let line = match parse("@next; @next", &model(), &context) {
        Ok(Parsed::Changes(planned)) => planned,
        other => panic!("{other:?}"),
    };
    assert_eq!(line[3].changes[0], json!({ "change": "connect", "from": "Mill 1", "to": "Mill 4" }));

    // Arguments with spaces are split at commas.
    let context = with_macros(&[("join", &["connect $1 > $2"])], None);
    match parse("@join Mill 1, Out", &model(), &context) {
        Ok(Parsed::Changes(planned)) => assert_eq!(planned[0].changes[0], json!({ "change": "connect", "from": "Mill 1", "to": "Out" })),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_macro_is_refused_without_its_arguments_or_selection_and_when_it_runs_itself() {
    let context = with_macros(&[("feed", &["add $1", "connect $selected > $last"]), ("loop", &["@loop"])], None);
    assert!(parse("@feed", &model(), &context).unwrap_err().contains("@feed needs at least 1 argument"));
    assert!(parse("@feed Drain", &model(), &context).unwrap_err().contains("select an instance first"));
    assert!(parse("@loop", &model(), &context).unwrap_err().contains("more than 8 deep"));
    assert!(parse("@nothing", &model(), &context).unwrap_err().contains("there is no macro `nothing`"));
    assert!(parse("connect In > $last", &model(), &context).unwrap_err().contains("none has yet"));
}

#[test]
fn a_macro_run_alone_may_hold_window_commands_which_run_in_order() {
    let context = with_macros(&[("tour", &["add Mill as A", "add Mill as B", "goto 1d", "remove A"])], None);
    assert_eq!(
        parse("@tour", &model(), &context),
        Ok(Parsed::Sequence(vec!["add Mill as A; add Mill as B".into(), "goto 1d".into(), "remove A".into()]))
    );
    assert!(parse("remove Out; @tour", &model(), &context).unwrap_err().contains("give it a line of its own"));
}

#[test]
fn macro_commands_save_remove_and_keep() {
    use crabsim_catalogue::commands::MacroCommand;
    let none = Context::default();
    assert_eq!(
        parse("macro line = add Station; connect $selected > $last", &model(), &none),
        Ok(Parsed::Macro(MacroCommand::Save { name: "line".into(), commands: vec!["add Station".into(), "connect $selected > $last".into()] }))
    );
    assert_eq!(parse("macro remove line", &model(), &none), Ok(Parsed::Macro(MacroCommand::Remove { name: "line".into() })));
    assert_eq!(parse("macro keep line", &model(), &none), Ok(Parsed::Macro(MacroCommand::Keep { name: "line".into() })));
    assert!(complete("@li", &model(), &["line".to_string(), "loop".to_string()]).contains(&"@line ".to_string()));
}

#[test]
fn macros_are_saved_listed_run_and_counted_through_operations() {
    let setup = setup("macros");
    let saved = setup.ok("save_macro", json!({ "name": "feed", "commands": ["add Mill", "connect $selected > $last"] }));
    common::assert_fits_its_schema(&setup.catalogue, "save_macro", &saved);
    assert_eq!(saved["author"], json!({ "kind": "user" }));
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "@feed; @feed", "selected": "Mill 1" }));
    let listed = setup.ok("list_macros", json!({}));
    common::assert_fits_its_schema(&setup.catalogue, "list_macros", &listed);
    assert_eq!(listed["macros"][0]["runs"], 2, "two runs in one line");
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    assert_eq!(model.successors("Mill 1"), ["Out", "Mill 3", "Mill 4"]);

    // The file sits beside the models, readable, and is never taken for one.
    assert!(setup.folder.read("macros.json").contains("\"macros_version\""));
    let models = setup.ok("list_models", json!({}));
    assert_eq!(models["models"], json!(["line.json"]));
    let refused = setup.err("write_model", json!({ "path": "macros.json", "text": "{}" }));
    assert!(refused["message"].as_str().unwrap().contains("holds the macros"), "{refused}");

    // Through the command line too.
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "macro twice = add Drain; add Drain" }));
    setup.ok("run_commands", json!({ "path": "line.json", "commands": "macro remove feed" }));
    let names: Vec<String> = setup.ok("list_macros", json!({}))["macros"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["twice"]);
    let refused = setup.err("save_macro", json!({ "name": "two words", "commands": ["add Mill"] }));
    assert!(refused["message"].as_str().unwrap().contains("be a macro"), "{refused}");
}

#[test]
fn an_agent_keeps_its_own_macros_for_its_work_and_cant_touch_the_users() {
    let user = setup("macros-agents");
    user.ok("save_macro", json!({ "name": "mine", "commands": ["add Mill"] }));
    {
        let agent = Agent(crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&user.folder.path, crabsim_catalogue::Source::Mcp)).unwrap());
        agent.ok("claim_model", json!({ "path": "line.json", "goal": "a longer line", "doing": "adding mills" }));
        let saved = agent.ok("save_macro", json!({ "name": "theirs", "commands": ["add Station"] }));
        assert_eq!(saved["author"]["kind"], "agent");
        assert_eq!(saved["author"]["goals"], json!(["a longer line"]));
        let refused = agent.err("save_macro", json!({ "name": "mine", "commands": ["remove Out"] }));
        assert!(refused["message"].as_str().unwrap().contains("the user"), "{refused}");
        let refused = agent.err("remove_macro", json!({ "name": "mine" }));
        assert!(refused["message"].as_str().unwrap().contains("the user"), "{refused}");
        let refused = agent.err("run_commands", json!({ "path": "line.json", "commands": "macro keep theirs" }));
        assert!(refused["message"].as_str().unwrap().contains("ask the user"), "{refused}");
        // While the agent works, everyone sees and can run its macro.
        let names: Vec<String> = user.ok("list_macros", json!({}))["macros"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["mine", "theirs"]);
    }
    // The agent's process ended: its macro left the list for the archive.
    let listed = user.ok("list_macros", json!({ "archived": true }));
    let names: Vec<&str> = listed["macros"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["mine"]);
    let archived = listed["archived"].as_array().unwrap();
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0]["name"], "theirs");
    assert!(user.folder.read(".crabsim/agent-macros.jsonl").contains("\"theirs\""));
    // The user keeps it.
    let kept = user.ok("keep_macro", json!({ "archived": archived[0]["id"] }));
    assert_eq!(kept["author"], json!({ "kind": "user" }));
    let names: Vec<String> = user.ok("list_macros", json!({}))["macros"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["mine", "theirs"]);
}

/// A second catalogue on the same folder, as an agent's MCP server opens it, ending with its
/// process when dropped.
struct Agent(crabsim_catalogue::Catalogue);

impl Agent {
    fn ok(&self, name: &str, input: Value) -> Value {
        self.0.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"))
    }

    fn err(&self, name: &str, input: Value) -> Value {
        self.0.call(name, input).expect_err("should fail").to_json()
    }
}
