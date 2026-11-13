//! Changing a model through `edit_model` and reading its values through `get_attributes`, by
//! `call(name, JSON)`: a batch of changes in, the file it produced out, compared in canonical form.

mod common;

use common::Setup;
use serde_json::{json, Value};

/// A Source feeding two mills of one class, which feed a Drain. Instances list their successors
/// for short, which `canonical` turns into connections.
fn line() -> Value {
    json!({
        "format_version": 1,
        "classes": [
            { "name": "Mill", "parent": "Station", "attributes": {
                "process_time": "0:45",
                "failures": { "availability": 0.9, "repair_time": { "constant": "1:00" } } } }
        ],
        "instances": [
            { "name": "In", "class": "Source", "stream": 0, "position": { "x": 0, "y": 0 }, "successors": ["Mill 1", "Mill 2"], "attributes": { "interval": "0:30" } },
            { "name": "Mill 1", "class": "Mill", "stream": 1, "position": { "x": 4, "y": 2 }, "successors": ["Out"] },
            { "name": "Mill 2", "class": "Mill", "stream": 2, "position": { "x": 4, "y": -2 }, "successors": ["Out"], "attributes": { "process_time": "0:50" } },
            { "name": "Out", "class": "Drain", "stream": 3, "position": { "x": 8, "y": 0 } }
        ]
    })
}

/// A model as crabsim saves it, from one whose instances may list their successors for short.
fn canonical(model: &Value) -> String {
    let model = common::connected(model.clone());
    crabsim_model::load(&model.to_string()).unwrap_or_else(|problems| panic!("{problems:?}")).save()
}

/// The connections out of `name` in the saved file, by exit, as (to, exit, entrance).
fn out_of(setup: &Setup, name: &str) -> Vec<(String, u64, u64)> {
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    model.connections_out(name).into_iter().map(|c| (c.to.clone(), c.exit, c.entrance)).collect()
}

/// The connections into `name` in the saved file, by entrance, as (from, exit, entrance).
fn into(setup: &Setup, name: &str) -> Vec<(String, u64, u64)> {
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    model.connections_in(name).into_iter().map(|c| (c.from.clone(), c.exit, c.entrance)).collect()
}

fn three(name: &str, exit: u64, entrance: u64) -> (String, u64, u64) {
    (name.to_string(), exit, entrance)
}

fn setup(name: &str) -> Setup {
    let setup = Setup::new(name);
    setup.ok("write_model", json!({ "path": "line.json", "text": canonical(&line()) }));
    setup
}

/// `line()` with `change` applied to its JSON, as the test expects the file to end up.
fn line_with(change: impl FnOnce(&mut Value)) -> String {
    let mut model = line();
    change(&mut model);
    canonical(&model)
}

fn edit(setup: &Setup, changes: Value) -> Value {
    setup.ok("edit_model", json!({ "path": "line.json", "changes": changes }))
}

fn edit_err(setup: &Setup, changes: Value) -> Value {
    setup.err("edit_model", json!({ "path": "line.json", "changes": changes }))
}

fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---------- edit_model ----------

#[test]
fn a_batch_applies_in_order_and_saves_the_model_canonical() {
    let setup = setup("batch");
    let output = edit(
        &setup,
        json!([
            { "change": "add_class", "name": "Inspection", "parent": "Station", "attributes": { "process_time": "0:20" } },
            { "change": "add_instance", "name": "Check", "class": "Inspection", "position": { "x": 6, "y": 0 }, "successors": ["Out"] },
            { "change": "disconnect", "from": "Mill 1", "to": "Out" },
            { "change": "connect", "from": "Mill 1", "to": "Check" },
            { "change": "set_model_description", "description": "With an inspection" },
        ]),
    );
    let expected = line_with(|model| {
        model["description"] = json!("With an inspection");
        model["classes"].as_array_mut().unwrap().push(json!({ "name": "Inspection", "parent": "Station", "attributes": { "process_time": "0:20" } }));
        model["instances"][1]["successors"] = json!(["Check"]);
        model["instances"].as_array_mut().unwrap().push(json!({ "name": "Check", "class": "Inspection", "position": { "x": 6, "y": 0 }, "successors": ["Out"] }));
    });
    let saved = setup.folder.read("line.json");
    assert_eq!(saved, expected);
    assert_eq!(output["file_hash"], hash(&saved), "the hash of the file as saved");
    assert_eq!(output["changes"], 5);
    assert!(output.get("instance_order_changed").is_none(), "streams keep pairing, whatever the order");
    assert_eq!(output["also_removed"], json!({ "instances": [], "classes": [], "connections": [] }));
}

#[test]
fn a_batch_may_pass_through_invalid_states_when_it_ends_valid() {
    let setup = setup("through-invalid");
    edit(
        &setup,
        json!([
            { "change": "add_instance", "name": "In 2", "class": "Source" },
            { "change": "connect", "from": "In 2", "to": "Mill 2" },
            { "change": "set_attribute", "target": { "instance": "In 2" }, "attribute": "interval", "value": "1:00" },
        ]),
    );
    assert!(setup.folder.read("line.json").contains("In 2"));
}

#[test]
fn a_change_that_cant_apply_refuses_the_batch_naming_its_place_and_changes_nothing() {
    let setup = setup("cant-apply");
    let before = setup.folder.read("line.json");
    for (changes, says) in [
        (json!([{ "change": "set_model_description", "description": "x" }, { "change": "remove_instance", "name": "Nobody" }]), "changes/1 (remove_instance): no instance is called `Nobody`"),
        (json!([{ "change": "add_instance", "name": "Out", "class": "Drain" }]), "changes/0 (add_instance): an instance is already called `Out`"),
        (json!([{ "change": "connect", "from": "In", "to": "Mill 1" }]), "changes/0 (connect): `In` is already connected to `Mill 1`"),
        (json!([{ "change": "disconnect", "from": "Out", "to": "In" }]), "changes/0 (disconnect): `Out` isn't connected to `In`"),
        (json!([{ "change": "restore_inheritance", "target": { "instance": "Mill 1" }, "attribute": "process_time" }]), "changes/0 (restore_inheritance): `process_time` isn't set on instance `Mill 1`, so it already inherits it"),
        (json!([{ "change": "set_attribute", "target": { "class": "Lathe" }, "attribute": "process_time", "value": "1:00" }]), "changes/0 (set_attribute): no class is called `Lathe`"),
        (json!([{ "change": "rename_class", "name": "Mill", "to": "Station" }]), "changes/0 (rename_class): `Station` is a built-in class"),
        (json!([{ "change": "connect", "from": "Mill 1", "to": "Mill 2", "exit": 5 }]), "changes/0 (connect): `Mill 1` has 1 other connection out, so its exit can be 1 to 2, not 5"),
        (json!([{ "change": "connect", "from": "Mill 1", "to": "Mill 2", "entrance": 0 }]), "changes/0 (connect): `Mill 2` has 1 other connection in, so its entrance can be 1 to 2, not 0"),
        (json!([{ "change": "set_connection", "from": "In", "to": "Mill 1", "exit": 3 }]), "changes/0 (set_connection): `In` has 1 other connection out, so its exit can be 1 to 2, not 3"),
        (json!([{ "change": "set_connection", "from": "In", "to": "Out" }]), "changes/0 (set_connection): `In` isn't connected to `Out`"),
    ] {
        let error = edit_err(&setup, changes.clone());
        assert_eq!(error["kind"], "invalid_input", "{changes}");
        assert!(error["message"].as_str().unwrap().contains(says), "{changes}: {error}");
        assert_eq!(setup.folder.read("line.json"), before, "{changes} changed the file");
    }
    assert_eq!(edit_err(&setup, json!([{ "change": "paint", "name": "In" }]))["kind"], "invalid_input");
}

#[test]
fn a_batch_ending_invalid_is_refused_with_every_problem_pointing_into_the_result() {
    let setup = setup("ends-invalid");
    let before = setup.folder.read("line.json");
    let error = edit_err(
        &setup,
        json!([
            { "change": "add_instance", "name": "Extra", "class": "Mill" },
            { "change": "set_attribute", "target": { "class": "Mill" }, "attribute": "failures", "value": { "availability": 1.5, "repair_time": { "constant": "1:00" } } },
            { "change": "set_attribute", "target": { "instance": "Mill 2" }, "attribute": "proces_time", "value": "1:00" },
        ]),
    );
    assert_eq!(error["kind"], "invalid_model");
    let pointers: Vec<&str> = error["problems"].as_array().unwrap().iter().map(|problem| problem["pointer"].as_str().unwrap()).collect();
    assert_eq!(pointers, ["/classes/0/attributes/failures/availability", "/instances/2/attributes/proces_time"]);
    assert!(error["problems"][1]["message"].as_str().unwrap().contains("did you mean `process_time`?"));
    assert_eq!(setup.folder.read("line.json"), before);
}

#[test]
fn a_file_that_is_already_invalid_is_refused_and_write_model_stays_the_way_to_fix_it() {
    let setup = setup("already-invalid");
    setup.folder.write("broken.json", common::BROKEN);
    setup.ok("adopt_model", json!({ "path": "broken.json" }));
    let error = setup.err("edit_model", json!({ "path": "broken.json", "changes": [{ "change": "set_model_description", "description": "x" }] }));
    assert_eq!(error["kind"], "invalid_model");
    assert!(error["message"].as_str().unwrap().contains("write_model"), "{error}");
    assert_eq!(setup.folder.read("broken.json"), common::BROKEN);
    assert_eq!(setup.err("edit_model", json!({ "path": "gone.json", "changes": [] }))["kind"], "not_found");
    assert_eq!(setup.err("edit_model", json!({ "path": "../line.json", "changes": [] }))["kind"], "outside_models_folder");
}

#[test]
fn removing_an_instance_removes_its_connections_and_says_runs_are_no_longer_paired() {
    let setup = setup("remove-instance");
    let output = edit(&setup, json!([{ "change": "remove_instance", "name": "Mill 1" }]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            model["instances"].as_array_mut().unwrap().remove(1);
            model["instances"][0]["successors"] = json!(["Mill 2"]);
        })
    );
    assert_eq!(output["also_removed"]["connections"], json!([{ "from": "In", "to": "Mill 1" }, { "from": "Mill 1", "to": "Out" }]));
}

#[test]
fn removing_a_class_others_depend_on_is_refused_listing_them_unless_their_removal_is_asked_for() {
    let setup = setup("remove-class");
    edit(&setup, json!([
        { "change": "add_class", "name": "Fast mill", "parent": "Mill", "attributes": { "process_time": "0:30" } },
        { "change": "set_class", "name": "Mill 2", "class": "Fast mill" },
    ]));
    let before = setup.folder.read("line.json");
    let error = edit_err(&setup, json!([{ "change": "remove_class", "name": "Mill" }]));
    let message = error["message"].as_str().unwrap();
    for named in ["`Fast mill`", "`Mill 1`", "`Mill 2`", "delete_dependents"] {
        assert!(message.contains(named), "{named} in {message}");
    }
    assert_eq!(setup.folder.read("line.json"), before);

    let output = edit(&setup, json!([{ "change": "remove_class", "name": "Mill", "delete_dependents": true }]));
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    assert!(model.classes.is_empty());
    assert_eq!(model.instances.iter().map(|instance| instance.name.as_str()).collect::<Vec<_>>(), ["In", "Out"]);
    assert_eq!(output["also_removed"]["classes"], json!(["Fast mill"]));
    assert_eq!(output["also_removed"]["instances"], json!(["Mill 1", "Mill 2"]));
    assert_eq!(output["also_removed"]["connections"].as_array().unwrap().len(), 4);
}

#[test]
fn renaming_updates_every_reference_and_keeps_the_order() {
    let setup = setup("rename");
    let output = edit(&setup, json!([
        { "change": "rename_instance", "name": "Out", "to": "Shipping" },
        { "change": "rename_class", "name": "Mill", "to": "CNC mill" },
    ]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            model["classes"][0]["name"] = json!("CNC mill");
            model["instances"][1]["class"] = json!("CNC mill");
            model["instances"][2]["class"] = json!("CNC mill");
            model["instances"][1]["successors"] = json!(["Shipping"]);
            model["instances"][2]["successors"] = json!(["Shipping"]);
            model["instances"][3]["name"] = json!("Shipping");
        })
    );
    let _ = output;
}

#[test]
fn a_duplicate_has_no_connections_or_children_and_goes_at_the_end() {
    let setup = setup("duplicate");
    edit(&setup, json!([
        { "change": "duplicate_instance", "name": "Mill 2", "to": "Mill 3", "position": { "x": 4, "y": -6 } },
        { "change": "duplicate_instance", "name": "Mill 1", "to": "Mill 4" },
        { "change": "duplicate_class", "name": "Mill", "to": "Lathe" },
    ]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            let mill = model["classes"][0].clone();
            let mut lathe = mill;
            lathe["name"] = json!("Lathe");
            model["classes"].as_array_mut().unwrap().push(lathe);
            let instances = model["instances"].as_array_mut().unwrap();
            instances.push(json!({ "name": "Mill 3", "class": "Mill", "position": { "x": 4, "y": -6 }, "attributes": { "process_time": "0:50" } }));
            instances.push(json!({ "name": "Mill 4", "class": "Mill", "position": { "x": 4, "y": 2 } }));
        })
    );
}

#[test]
fn attributes_are_set_and_restored_on_instances_and_classes() {
    let setup = setup("attributes");
    edit(&setup, json!([
        { "change": "set_attribute", "target": { "class": "Mill" }, "attribute": "process_time", "value": "1:00" },
        { "change": "set_attribute", "target": { "instance": "Mill 1" }, "attribute": "description", "value": "the old one" },
        { "change": "restore_inheritance", "target": { "instance": "Mill 2" }, "attribute": "process_time" },
        { "change": "restore_inheritance", "target": { "class": "Mill" }, "attribute": "failures" },
    ]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            model["classes"][0]["attributes"] = json!({ "process_time": "1:00" });
            model["instances"][1]["attributes"] = json!({ "description": "the old one" });
            model["instances"][2].as_object_mut().unwrap().remove("attributes");
        })
    );
}

#[test]
fn a_built_in_classs_values_are_set_for_the_model_and_go_when_the_last_is_restored() {
    let setup = setup("built-in-values");
    edit(&setup, json!([
        { "change": "set_attribute", "target": { "class": "Station" }, "attribute": "process_time", "value": "0:10" },
        { "change": "set_attribute", "target": { "class": "Station" }, "attribute": "description", "value": "ours" },
    ]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            model["classes"].as_array_mut().unwrap().push(json!({ "name": "Station", "attributes": { "process_time": "0:10", "description": "ours" } }));
        })
    );
    // Every class built on it inherits it, and the chain names it once.
    let mill = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill 1" } }));
    assert_eq!(mill["chain"], json!(["Mill", "Station"]));
    let description = mill["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "description").unwrap().clone();
    assert_eq!(description, json!({ "attribute": "description", "value": "ours", "from": "class", "class": "Station" }));
    let station = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "class": "Station" } }));
    assert_eq!(station["chain"], json!(["Station"]));

    // A built-in class keeps its name and parent.
    for change in [
        json!({ "change": "rename_class", "name": "Station", "to": "Machine" }),
        json!({ "change": "set_parent", "name": "Station", "parent": "Source" }),
        json!({ "change": "duplicate_class", "name": "Station", "to": "Machine" }),
    ] {
        let refused = edit_err(&setup, json!([change]));
        assert!(refused["message"].as_str().unwrap().contains("keeps its name and parent"), "{refused}");
    }

    // Restoring the last value removes the model's own class, and Mill still derives from Station.
    edit(&setup, json!([
        { "change": "restore_inheritance", "target": { "class": "Station" }, "attribute": "process_time" },
        { "change": "restore_inheritance", "target": { "class": "Station" }, "attribute": "description" },
    ]));
    assert_eq!(setup.folder.read("line.json"), canonical(&line()));
    let refused = edit_err(&setup, json!([{ "change": "restore_inheritance", "target": { "class": "Station" }, "attribute": "description" }]));
    assert!(refused["message"].as_str().unwrap().contains("has the built-in default"), "{refused}");

    // Removing it takes only its values, never what is built on it.
    edit(&setup, json!([{ "change": "set_attribute", "target": { "class": "Station" }, "attribute": "starting_mus", "value": 1 }]));
    edit(&setup, json!([{ "change": "remove_class", "name": "Station" }]));
    assert_eq!(setup.folder.read("line.json"), canonical(&line()));
    let refused = edit_err(&setup, json!([{ "change": "remove_class", "name": "Station" }]));
    assert!(refused["message"].as_str().unwrap().contains("nothing to remove"), "{refused}");
}

#[test]
fn instances_move_change_class_and_connect_at_a_chosen_place() {
    let setup = setup("move");
    edit(&setup, json!([
        { "change": "move_instance", "name": "Out", "position": { "x": 10, "y": 1, "z": 0.5 } },
        { "change": "add_class", "name": "Lathe", "parent": "Station" },
        { "change": "set_class", "name": "Mill 1", "class": "Lathe" },
        { "change": "set_parent", "name": "Lathe", "parent": "Mill" },
        { "change": "add_instance", "name": "Spare", "class": "Mill", "successors": ["Out"] },
        { "change": "connect", "from": "In", "to": "Spare", "exit": 1 },
    ]));
    assert_eq!(
        setup.folder.read("line.json"),
        line_with(|model| {
            model["classes"].as_array_mut().unwrap().push(json!({ "name": "Lathe", "parent": "Mill" }));
            model["instances"][1]["class"] = json!("Lathe");
            model["instances"][3]["position"] = json!({ "x": 10, "y": 1, "z": 0.5 });
            model["instances"][0]["successors"] = json!(["Spare", "Mill 1", "Mill 2"]);
            model["instances"].as_array_mut().unwrap().push(json!({ "name": "Spare", "class": "Mill", "successors": ["Out"] }));
        })
    );
}

// ---------- Connections and their places ----------

#[test]
fn connecting_at_a_chosen_exit_and_entrance_moves_the_others_up() {
    let setup = setup("connect-places");
    edit(&setup, json!([{ "change": "connect", "from": "In", "to": "Out", "exit": 1, "entrance": 1, "description": "the bypass" }]));
    assert_eq!(out_of(&setup, "In"), [three("Out", 1, 1), three("Mill 1", 2, 1), three("Mill 2", 3, 1)]);
    assert_eq!(into(&setup, "Out"), [three("In", 1, 1), three("Mill 1", 1, 2), three("Mill 2", 1, 3)]);
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    assert_eq!(model.connections_out("In")[0].description.as_deref(), Some("the bypass"));

    // Left out, a connection goes last at both ends.
    edit(&setup, json!([{ "change": "connect", "from": "Mill 1", "to": "Mill 2" }]));
    assert_eq!(out_of(&setup, "Mill 1"), [three("Out", 1, 2), three("Mill 2", 2, 2)]);
}

#[test]
fn disconnecting_closes_up_the_places_at_both_ends() {
    let setup = setup("disconnect-places");
    edit(&setup, json!([{ "change": "disconnect", "from": "Mill 1", "to": "Out" }, { "change": "disconnect", "from": "In", "to": "Mill 1" }]));
    assert_eq!(into(&setup, "Out"), [three("Mill 2", 1, 1)]);
    assert_eq!(out_of(&setup, "In"), [three("Mill 2", 1, 1)]);
}

#[test]
fn set_connection_moves_a_connection_and_sets_or_removes_its_description() {
    let setup = setup("set-connection");
    edit(&setup, json!([
        { "change": "set_connection", "from": "In", "to": "Mill 2", "exit": 1 },
        { "change": "set_connection", "from": "Mill 2", "to": "Out", "entrance": 1, "description": "via the lift" },
    ]));
    assert_eq!(out_of(&setup, "In"), [three("Mill 2", 1, 1), three("Mill 1", 2, 1)]);
    assert_eq!(into(&setup, "Out"), [three("Mill 2", 1, 1), three("Mill 1", 1, 2)]);
    assert!(setup.folder.read("line.json").contains("via the lift"));
    edit(&setup, json!([{ "change": "set_connection", "from": "Mill 2", "to": "Out", "description": "" }]));
    assert!(!setup.folder.read("line.json").contains("via the lift"), "an empty description removes it");
    assert_eq!(into(&setup, "Out"), [three("Mill 2", 1, 1), three("Mill 1", 1, 2)], "and leaves its places alone");
}

#[test]
fn removing_or_renaming_an_instance_updates_its_connections_in_the_same_change() {
    let setup = setup("remove-rename-places");
    edit(&setup, json!([
        { "change": "connect", "from": "In", "to": "Out", "exit": 2, "entrance": 2 },
        { "change": "rename_instance", "name": "Mill 2", "to": "Lathe" },
    ]));
    assert_eq!(out_of(&setup, "In"), [three("Mill 1", 1, 1), three("Out", 2, 2), three("Lathe", 3, 1)]);
    assert_eq!(into(&setup, "Out"), [three("Mill 1", 1, 1), three("In", 2, 2), three("Lathe", 1, 3)]);
    let output = edit(&setup, json!([{ "change": "remove_instance", "name": "Mill 1" }]));
    assert_eq!(out_of(&setup, "In"), [three("Out", 1, 1), three("Lathe", 2, 1)]);
    assert_eq!(into(&setup, "Out"), [three("In", 1, 1), three("Lathe", 1, 2)]);
    assert_eq!(output["also_removed"]["connections"], json!([{ "from": "In", "to": "Mill 1" }, { "from": "Mill 1", "to": "Out" }]));
}

// ---------- get_attributes ----------

#[test]
fn get_attributes_gives_every_attribute_with_its_value_and_origin() {
    let setup = setup("get-instance");
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill 2" } }));
    assert_eq!(got["built_in_class"], "Station");
    assert_eq!(got["chain"], json!(["Mill", "Station"]));
    assert_eq!(got["position"], json!({ "x": 4.0, "y": -2.0 }));
    assert_eq!(got["successors"], json!(["Out"]));
    assert_eq!(got["predecessors"], json!(["In"]));
    assert_eq!(got["file_hash"], hash(&setup.folder.read("line.json")));
    let attributes: Vec<(String, Value)> = got["attributes"].as_array().unwrap().iter().map(|a| (a["attribute"].as_str().unwrap().to_string(), a.clone())).collect();
    let names: Vec<&str> = attributes.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["process_time", "failures", "starting_mus", "dimensions", "description", "state_notes", "distribution"], "every attribute a Station has");
    let of = |name: &str| attributes.iter().find(|(n, _)| n == name).unwrap().1.clone();
    assert_eq!(of("distribution"), json!({ "attribute": "distribution", "from": "not_set" }), "round robin by default, set nowhere");
    assert_eq!(of("process_time"), json!({ "attribute": "process_time", "value": "00:00:50", "from": "instance" }));
    assert_eq!(of("failures"), json!({ "attribute": "failures", "value": { "availability": 0.9, "repair_time": { "constant": "00:01:00" } }, "from": "class", "class": "Mill" }));
    assert_eq!(of("starting_mus"), json!({ "attribute": "starting_mus", "value": 0, "from": "built_in" }));
    assert_eq!(of("dimensions")["from"], "built_in");
    assert_eq!(of("description"), json!({ "attribute": "description", "from": "not_set" }));
    assert_eq!(of("state_notes"), json!({ "attribute": "state_notes", "from": "not_set" }));
}

#[test]
fn state_notes_are_set_on_a_class_and_checked_against_its_built_in_class() {
    let setup = setup("state-notes");
    let notes = json!({ "blocked": "waits for the crane", "failed": "the spindle broke" });
    setup.ok("edit_model", json!({ "path": "line.json", "changes": [
        { "change": "set_attribute", "target": { "class": "Mill" }, "attribute": "state_notes", "value": notes } ] }));
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill 2" } }));
    let state_notes = got["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "state_notes").unwrap().clone();
    assert_eq!(state_notes, json!({ "attribute": "state_notes", "value": notes, "from": "class", "class": "Mill" }));

    let error = edit_err(&setup, json!([
        { "change": "set_attribute", "target": { "instance": "Out" }, "attribute": "state_notes", "value": { "failed": "never" } } ]));
    assert_eq!(error["problems"][0]["message"], "a Drain is never failed");
}

#[test]
fn get_attributes_of_a_class_and_of_a_source_without_a_value() {
    let setup = setup("get-class");
    let mill = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "class": "Mill" } }));
    assert_eq!(mill["chain"], json!(["Station"]));
    assert!(mill.get("position").is_none() && mill.get("successors").is_none() && mill.get("predecessors").is_none());
    let process_time = mill["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "process_time").unwrap().clone();
    assert_eq!(process_time, json!({ "attribute": "process_time", "value": "00:00:45", "from": "class", "class": "Mill" }));

    let source = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "class": "Source" } }));
    let interval = source["attributes"].as_array().unwrap().iter().find(|a| a["attribute"] == "interval").unwrap().clone();
    assert_eq!(interval, json!({ "attribute": "interval", "value": "00:00:00", "from": "built_in" }), "a Source's interval is 0 by default");

    assert_eq!(setup.err("get_attributes", json!({ "path": "line.json", "target": { "instance": "Nobody" } }))["kind"], "not_found");
    assert_eq!(setup.err("get_attributes", json!({ "path": "line.json", "target": { "class": "Lathe" } }))["kind"], "not_found");
}

#[test]
fn the_hash_after_an_edit_is_what_get_attributes_then_gives() {
    let setup = setup("hashes");
    let before = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "In" } }))["file_hash"].clone();
    let edited = edit(&setup, json!([{ "change": "move_instance", "name": "In", "position": { "x": -1, "y": 0 } }]));
    let after = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "In" } }))["file_hash"].clone();
    assert_ne!(before, after);
    assert_eq!(edited["file_hash"], after);
}

// ---------- Cost ----------

/// The cost of loading, changing and saving a whole file per call, on the benchmark's model of
/// 429 copies of the demo (6,006 objects), with unique names. Timing only, so on demand in
/// release: `cargo test --release -p crabsim-catalogue --test edit_model -- --ignored --nocapture`.
/// It also checks that an undo and a redo never take longer than the edit they reverse.
#[test]
#[ignore]
fn get_attributes_and_edit_model_on_6006_objects() {
    let demo = crabsim_model::load(&common::demo_text()).unwrap();
    let mut model = crabsim_model::Model { description: Some("429 copies of the demo model".into()), classes: demo.classes.clone(), ..Default::default() };
    for copy in 0..429 {
        let rename = |name: &str| format!("Copy {copy} / {name}");
        for instance in &demo.instances {
            let mut instance = instance.clone();
            instance.name = rename(&instance.name);
            instance.stream = instance.stream.map(|stream| stream + 100 * copy as u64);
            if let Some(position) = &mut instance.position {
                position.y += 20.0 * copy as f64;
            }
            model.instances.push(instance);
        }
        for connection in &demo.connections {
            let mut connection = connection.clone();
            connection.from = rename(&connection.from);
            connection.to = rename(&connection.to);
            model.connections.push(connection);
        }
    }
    let text = model.save();
    let setup = Setup::new("large");
    setup.ok("write_model", json!({ "path": "large.json", "text": text }));
    println!("6,006 objects: {} bytes of model file", text.len());
    let time = |what: &str, input: Value, name: &str| {
        let mut times = Vec::new();
        for _ in 0..7 {
            let start = std::time::Instant::now();
            setup.ok(name, input.clone());
            times.push(start.elapsed());
        }
        times.sort();
        println!("{what}: median {:?} of 7, fastest {:?}", times[3], times[0]);
    };
    time("get_attributes", json!({ "path": "large.json", "target": { "instance": "Copy 428 / Station 4" } }), "get_attributes");
    time(
        "edit_model, one set_attribute",
        json!({ "path": "large.json", "changes": [{ "change": "set_attribute", "target": { "instance": "Copy 200 / Station 2" }, "attribute": "process_time", "value": "0:12" }] }),
        "edit_model",
    );
    time("validate_model", json!({ "path": "large.json" }), "validate_model");
    // Undo against the edit it reverses (spec issue 107): seven edits that each change the file,
    // so each is kept in the history, then each undone and redone.
    let median = |what: &str, mut times: Vec<std::time::Duration>| {
        times.sort();
        println!("{what}: median {:?} of 7, fastest {:?}", times[3], times[0]);
        times[3]
    };
    let timed = |name: &str, input: &dyn Fn(usize) -> Value| -> Vec<std::time::Duration> {
        (0..7)
            .map(|n| {
                let start = std::time::Instant::now();
                setup.ok(name, input(n));
                start.elapsed()
            })
            .collect()
    };
    let step = |_: usize| json!({ "path": "large.json" });
    let edits = timed("edit_model", &|n| {
        json!({ "path": "large.json", "changes": [{ "change": "set_attribute", "target": { "instance": "Copy 200 / Station 2" }, "attribute": "process_time", "value": format!("0:{}", 20 + n) }] })
    });
    let edit = median("edit_model, one set_attribute kept in the history", edits);
    let undo = median("undo_model", timed("undo_model", &step));
    let redo = median("redo_model", timed("redo_model", &step));
    time("get_history", json!({ "path": "large.json" }), "get_history");
    let history = std::fs::metadata(setup.folder.join(".crabsim/history.sqlite")).map_or(0, |file| file.len());
    println!("the history of 7 changes: {history} bytes");
    assert!(undo <= edit && redo <= edit, "an undo or redo never takes longer than the edit it reverses: edit {edit:?}, undo {undo:?}, redo {redo:?}");
}

// ---------- Streams ----------

fn streams_in(setup: &Setup) -> Vec<(String, u64)> {
    let model = crabsim_model::load(&setup.folder.read("line.json")).unwrap();
    model.instances.iter().map(|instance| (instance.name.clone(), instance.stream.unwrap())).collect()
}

#[test]
fn a_new_or_duplicated_instance_gets_a_new_stream_and_a_removed_ones_leaves_a_gap() {
    let setup = setup("streams");
    edit(&setup, json!([
        { "change": "remove_instance", "name": "Mill 1" },
        { "change": "duplicate_instance", "name": "Mill 2", "to": "Mill 3" },
        { "change": "add_instance", "name": "Spare", "class": "Drain" },
    ]));
    let streams = streams_in(&setup);
    let of = |name: &str| streams.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!((of("In"), of("Mill 2"), of("Out")), (0, 2, 3), "nobody else's stream changes");
    assert_eq!((of("Mill 3"), of("Spare")), (4, 5), "one more than the highest, and not the original's");
    let got = setup.ok("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill 3" } }));
    assert_eq!(got["stream"], 4, "get_attributes shows an instance's stream");
}

#[test]
fn only_the_user_sets_a_stream_and_it_must_be_unique() {
    let setup = setup("set-stream");
    let agent = crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&setup.folder.path, crabsim_catalogue::Source::Mcp)).unwrap();
    agent.call("claim_model", json!({ "path": "line.json", "goal": "check the streams", "doing": "renumbering streams" })).unwrap();
    let refused = agent.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_stream", "name": "Mill 1", "stream": 40 }] })).unwrap_err().to_json();
    assert_eq!(refused["kind"], "invalid_input");
    assert!(refused["message"].as_str().unwrap().contains("only for the user"), "{refused}");

    agent.call("release_model", json!({ "path": "line.json" })).unwrap();
    let user = &setup.catalogue;
    user.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_stream", "name": "Mill 1", "stream": 40 }] })).unwrap();
    assert!(streams_in(&setup).contains(&("Mill 1".to_string(), 40)));
    let taken = user.call("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_stream", "name": "Mill 2", "stream": 40 }] })).unwrap_err().to_json();
    assert_eq!(taken["kind"], "invalid_model");
    assert_eq!(taken["problems"][0]["pointer"], "/instances/2/stream");
}
