//! Model files changed outside crabsim, `adopt_model`, `expected_hash`, `write_model`'s
//! `overwrite`, and agents kept from the user's runs, through `call(name, JSON)`.

mod common;

use common::{write_models, Setup, BROKEN, SMALL};
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};

fn canonical(text: &str) -> String {
    crabsim_model::load(text).unwrap().save()
}

fn describe(setup: &Setup, path: &str) -> Result<Value, Value> {
    setup
        .catalogue
        .call("edit_model", json!({ "path": path, "changes": [{ "change": "set_model_description", "description": "changed" }] }))
        .map_err(|error| error.to_json())
}

#[test]
fn a_file_crabsim_never_saved_is_read_and_run_but_changed_only_once_adopted() {
    let setup = Setup::new("never-saved");
    setup.folder.write("line.json", &canonical(SMALL));
    let refused = describe(&setup, "line.json").unwrap_err();
    assert_eq!(refused["kind"], "changed_outside");
    assert!(refused["message"].as_str().unwrap().contains("adopt_model"), "{refused}");
    for (name, input) in [
        ("read_model", json!({ "path": "line.json" })),
        ("validate_model", json!({ "path": "line.json" })),
        ("get_attributes", json!({ "path": "line.json", "target": { "instance": "Work" } })),
        ("start_run", json!({ "model": "line.json", "length": "1:00", "observations": 1 })),
    ] {
        assert_eq!(setup.ok(name, input)["changed_outside"], true, "{name} says so");
    }
    let adopted = setup.ok("adopt_model", json!({ "path": "line.json" }));
    assert_eq!(adopted["problems"], json!([]));
    describe(&setup, "line.json").unwrap();
    assert_eq!(setup.ok("read_model", json!({ "path": "line.json" }))["changed_outside"], false);
}

#[test]
fn a_file_another_program_changed_is_refused_until_adopted_and_reads_still_work() {
    let setup = Setup::new("changed");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL }));
    describe(&setup, "line.json").unwrap();
    let mut by_hand = setup.folder.read("line.json");
    by_hand = by_hand.replace("00:00:08", "00:00:09");
    setup.folder.write("line.json", &by_hand);

    assert_eq!(describe(&setup, "line.json").unwrap_err()["kind"], "changed_outside");
    assert_eq!(setup.err("write_model", json!({ "path": "line.json", "text": SMALL, "overwrite": true }))["kind"], "changed_outside");
    assert_eq!(setup.err("delete_model", json!({ "path": "line.json" }))["kind"], "changed_outside");
    assert_eq!(setup.folder.read("line.json"), by_hand, "the change by hand is kept");
    assert_eq!(setup.ok("read_model", json!({ "path": "line.json" }))["text"], by_hand.as_str());

    setup.ok("adopt_model", json!({ "path": "line.json" }));
    describe(&setup, "line.json").unwrap();
    assert!(setup.folder.read("line.json").contains("00:00:09"), "the adopted change stays");
}

#[test]
fn an_invalid_file_is_adopted_with_its_problems_so_write_model_can_fix_it() {
    let setup = Setup::new("adopt-invalid");
    setup.folder.write("broken.json", BROKEN);
    let adopted = setup.ok("adopt_model", json!({ "path": "broken.json" }));
    assert_eq!(adopted["problems"].as_array().unwrap().len(), crabsim_model::load(BROKEN).unwrap_err().len());
    assert_eq!(setup.folder.read("broken.json"), BROKEN, "adopting changes nothing in the file");
    setup.ok("write_model", json!({ "path": "broken.json", "text": SMALL, "overwrite": true }));
    assert_eq!(setup.err("adopt_model", json!({ "path": "gone.json" }))["kind"], "not_found");
}

#[test]
fn a_copy_of_an_unadopted_file_needs_adopting_too() {
    let setup = Setup::new("copies");
    setup.folder.write("raw.json", &canonical(SMALL));
    setup.ok("write_model", json!({ "path": "saved.json", "text": SMALL }));
    setup.ok("copy_model", json!({ "from": "raw.json", "to": "raw-copy.json" }));
    setup.ok("copy_model", json!({ "from": "saved.json", "to": "saved-copy.json" }));
    assert_eq!(describe(&setup, "raw-copy.json").unwrap_err()["kind"], "changed_outside");
    describe(&setup, "saved-copy.json").unwrap();
}

#[test]
fn expected_hash_refuses_a_change_based_on_an_older_reading() {
    let setup = Setup::new("expected-hash");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL }));
    let read = setup.ok("read_model", json!({ "path": "line.json" }));
    let hash = read["hash"].as_str().unwrap().to_string();
    assert_eq!(hash, setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "In" } }))["hash"]);

    let change = |expected: &str| {
        setup.catalogue.call(
            "edit_model",
            json!({ "path": "line.json", "expected_hash": expected, "changes": [{ "change": "move_instance", "name": "In", "position": { "x": 1, "y": 0 } }] }),
        )
    };
    let edited = change(&hash).unwrap();
    let before = setup.folder.read("line.json");
    let refused = change(&hash).unwrap_err().to_json();
    assert_eq!(refused["kind"], "conflict", "the file changed since that reading");
    assert!(refused["message"].as_str().unwrap().contains(edited["hash"].as_str().unwrap()), "names the hash it has now: {refused}");
    assert_eq!(setup.folder.read("line.json"), before);

    let written = setup.err("write_model", json!({ "path": "line.json", "text": SMALL, "overwrite": true, "expected_hash": hash }));
    assert_eq!(written["kind"], "conflict");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL, "overwrite": true, "expected_hash": edited["hash"] }));
    assert_eq!(setup.err("write_model", json!({ "path": "new.json", "text": SMALL, "expected_hash": hash }))["kind"], "conflict", "a file that doesn't exist has no hash");
}

#[test]
fn write_model_overwrites_only_when_asked() {
    let setup = Setup::new("overwrite");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL }));
    let refused = setup.err("write_model", json!({ "path": "line.json", "text": SMALL }));
    assert_eq!(refused["kind"], "already_exists");
    assert!(refused["message"].as_str().unwrap().contains("overwrite"), "{refused}");
    setup.ok("write_model", json!({ "path": "line.json", "text": SMALL, "overwrite": true }));
}

#[test]
fn an_agent_can_read_but_not_forget_or_relabel_the_users_runs() {
    let setup = Setup::new("users-runs");
    write_models(&setup.folder);
    let user = Catalogue::open(Settings::new(&setup.folder.path, Source::Cli)).unwrap();
    let theirs = user.call("start_run", json!({ "model": "small.json", "length": "1:00", "observations": 1 })).unwrap()["run"].as_str().unwrap().to_string();
    let mine = setup.start(json!({ "model": "small.json", "length": "1:00", "observations": 1 }));
    setup.finished(&mine);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while user.call("run_status", json!({ "run": theirs })).unwrap()["state"] != "done" {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    setup.ok("run_results", json!({ "run": theirs }));
    for (name, input) in [("forget_run", json!({ "run": theirs })), ("label_run", json!({ "run": theirs, "label": "mine now" }))] {
        let refused = setup.err(name, input);
        assert_eq!(refused["kind"], "invalid_input", "{name}");
        assert!(refused["message"].as_str().unwrap().contains("the user"), "{name}: {refused}");
    }
    setup.ok("label_run", json!({ "run": mine, "label": "an agent's own" }));
    user.call("label_run", json!({ "run": mine, "label": "the user may" })).unwrap();
    user.call("forget_run", json!({ "run": mine })).unwrap();
}

#[test]
fn adopting_follows_the_editor_rules() {
    let setup = Setup::new("adopt-editor");
    setup.folder.write("line.json", &canonical(SMALL));
    let other = Catalogue::open(Settings::new(&setup.folder.path, Source::Mcp)).unwrap();
    other.call("claim_model", json!({ "path": "line.json", "purpose": "working" })).unwrap();
    assert_eq!(setup.err("adopt_model", json!({ "path": "line.json" }))["kind"], "not_editor");
    other.call("adopt_model", json!({ "path": "line.json" })).unwrap();
}
