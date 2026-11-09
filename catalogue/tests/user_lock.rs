//! The UI as the user: a catalogue opened as `Source::Ui` counts as the user, only one UI may have
//! a models folder open, and while one does, the CLI may read and run but not change models or the
//! who edits them (spec issue 63, ADR 0011, ADR 0013). Agents keep working through the UI's folder.

mod common;

use common::{TempFolder, SMALL};
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

fn open(folder: &TempFolder, source: Source) -> Catalogue {
    Catalogue::open(Settings::new(&folder.path, source)).unwrap_or_else(|error| panic!("{source:?} can't open the folder: {error}"))
}

fn ok(catalogue: &Catalogue, name: &str, input: Value) -> Value {
    catalogue.call(name, input.clone()).unwrap_or_else(|error| panic!("{name} {input} failed: {error}"))
}

fn err(catalogue: &Catalogue, name: &str, input: Value) -> Value {
    match catalogue.call(name, input.clone()) {
        Ok(output) => panic!("{name} {input} should fail, but gave {output}"),
        Err(error) => error.to_json(),
    }
}

/// A models folder with a model crabsim saved, and another file to copy and delete.
fn folder(name: &str) -> TempFolder {
    let folder = TempFolder::new(name);
    let user = open(&folder, Source::Cli);
    ok(&user, "write_model", json!({ "path": "line.json", "text": SMALL }));
    ok(&user, "write_model", json!({ "path": "spare.json", "text": SMALL }));
    folder
}

#[test]
fn the_ui_counts_as_the_user() {
    let folder = folder("ui-is-user");
    let ui = open(&folder, Source::Ui);
    ok(&ui, "take_model", json!({ "path": "line.json" }));
    let editors = ok(&ui, "get_editors", json!({ "path": "line.json" }));
    assert_eq!(editors["files"][0]["editor"], "user", "{editors}");
    ok(&ui, "release_model", json!({ "path": "line.json" }));
    ok(&ui, "edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_stream", "name": "Work", "stream": 40 }] }));

    let run = ok(&ui, "start_run", json!({ "model": "line.json", "length": "1:00", "observations": 1, "wait": 30 }))["run"].as_str().unwrap().to_string();
    let source = ok(&ui, "query_results", json!({ "sql": format!("SELECT source FROM runs WHERE id = '{run}'") }));
    assert_eq!(source["rows"], json!([["ui"]]), "runs from the UI are recorded as the user's, from `ui`");
    let agent = open(&folder, Source::Mcp);
    let refused = err(&agent, "forget_run", json!({ "run": run }));
    assert!(refused["message"].as_str().unwrap().contains("user"), "an agent can't forget the user's runs: {refused}");
}

#[test]
fn a_second_ui_on_the_folder_is_refused_until_the_first_closes() {
    let folder = folder("second-ui");
    let first = open(&folder, Source::Ui);
    let second = Catalogue::open(Settings::new(&folder.path, Source::Ui)).err().expect("a second UI is refused").to_json();
    assert_eq!(second["kind"], "folder_in_use");
    assert!(second["message"].as_str().unwrap().contains("copy of the folder"), "{second}");

    drop(first);
    open(&folder, Source::Ui);
}

#[test]
fn while_a_ui_has_the_folder_the_cli_reads_and_runs_but_changes_nothing() {
    let folder = folder("cli-refused");
    let ui = open(&folder, Source::Ui);
    let cli = open(&folder, Source::Cli);

    for (name, input) in [
        ("edit_model", json!({ "path": "line.json", "changes": [{ "change": "move_instance", "name": "Work", "position": { "x": 1, "y": 1 } }] })),
        ("write_model", json!({ "path": "new.json", "text": SMALL })),
        ("copy_model", json!({ "from": "line.json", "to": "copy.json" })),
        ("delete_model", json!({ "path": "spare.json" })),
        ("adopt_model", json!({ "path": "line.json" })),
        ("take_model", json!({ "path": "line.json" })),
        ("release_model", json!({ "path": "line.json" })),
        ("block_agent", json!({ "agent": "agent-1" })),
        ("unblock_agent", json!({ "agent": "agent-1" })),
        ("revoke_claim", json!({ "path": "line.json" })),
    ] {
        let refused = err(&cli, name, input);
        assert_eq!(refused["kind"], "folder_in_use", "{name}: {refused}");
        assert!(refused["message"].as_str().unwrap().contains("window"), "{name}: {refused}");
    }
    assert!(folder.join("spare.json").is_file() && !folder.join("new.json").exists() && !folder.join("copy.json").exists());

    for (name, input) in [
        ("read_model", json!({ "path": "line.json" })),
        ("validate_model", json!({ "path": "line.json" })),
        ("list_models", json!({})),
        ("get_attributes", json!({ "path": "line.json", "target": { "instance": "Work" } })),
        ("describe_model", json!({ "path": "line.json" })),
        ("get_editors", json!({})),
        ("start_run", json!({ "model": "line.json", "length": "1:00", "observations": 1, "wait": 30 })),
        ("export_model", json!({ "path": "line.json", "format": "graphml" })),
    ] {
        ok(&cli, name, input);
    }

    drop(ui);
    ok(&cli, "edit_model", json!({ "path": "line.json", "changes": [{ "change": "move_instance", "name": "Work", "position": { "x": 1, "y": 1 } }] }));
}

#[test]
fn agents_keep_changing_models_while_a_ui_has_the_folder() {
    let folder = folder("agents-with-ui");
    let _ui = open(&folder, Source::Ui);
    let agent = open(&folder, Source::Mcp);
    ok(&agent, "claim_model", json!({ "path": "line.json", "goal": "a tidier layout", "doing": "moving Work" }));
    ok(&agent, "edit_model", json!({ "path": "line.json", "changes": [{ "change": "move_instance", "name": "Work", "position": { "x": 2, "y": 0 } }] }));
}

#[test]
fn the_ui_may_change_a_model_past_an_agent_that_holds_it_and_the_agent_keeps_its_right() {
    let folder = folder("ui-takes-over");
    let ui = open(&folder, Source::Ui);
    let agent = open(&folder, Source::Mcp);
    ok(&agent, "claim_model", json!({ "path": "line.json", "goal": "a tidier layout", "doing": "moving Work" }));
    let change = json!({ "path": "line.json", "changes": [{ "change": "move_instance", "name": "Work", "position": { "x": 3, "y": 0 } }] });
    assert_eq!(err(&ui, "edit_model", change.clone())["kind"], "not_editor");
    ui.set_take_over(true);
    ok(&ui, "edit_model", change);
    let editors = ok(&ui, "get_editors", json!({ "path": "line.json" }));
    assert_eq!(editors["files"][0]["editor"], "agent-1", "{editors}");
}
