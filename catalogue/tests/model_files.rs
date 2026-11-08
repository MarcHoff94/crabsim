//! The catalogue's model file operations and the models folder's limits, through
//! `call(name, JSON)`, as an agent uses them.

mod common;

use common::{demo_text, Setup, TempFolder, BROKEN, SMALL};
use crabsim_catalogue::{Catalogue, Settings, Source};
use serde_json::{json, Value};
use std::path::Path;

fn problems_of(text: &str) -> Value {
    let problems = crabsim_model::load(text).expect_err("has problems");
    serde_json::to_value(
        problems
            .iter()
            .map(|p| match &p.name {
                Some(name) => json!({ "pointer": p.pointer, "name": name, "message": p.message }),
                None => json!({ "pointer": p.pointer, "message": p.message }),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn every_operation_is_listed_with_a_description_and_object_schemas() {
    let setup = Setup::new("listing");
    let names: Vec<_> = setup.catalogue.operations().map(|operation| operation.name).collect();
    for expected in ["validate_model", "list_models", "read_model", "write_model", "copy_model", "delete_model", "query_results"] {
        assert!(names.contains(&expected), "{expected} missing from {names:?}");
    }
    for operation in setup.catalogue.operations() {
        assert!(operation.description.len() > 40, "{} needs a description an agent can use", operation.name);
        assert_eq!(operation.input_schema["type"], "object", "{}", operation.name);
        assert_eq!(operation.output_schema["type"], "object", "{}", operation.name);
        jsonschema::validator_for(&operation.input_schema).unwrap_or_else(|error| panic!("{}: {error}", operation.name));
        jsonschema::validator_for(&operation.output_schema).unwrap_or_else(|error| panic!("{}: {error}", operation.name));
    }
}

#[test]
fn operations_are_marked_read_only_or_destructive() {
    let setup = Setup::new("effects");
    let effect = |name: &str| serde_json::to_value(setup.catalogue.operations().find(|o| o.name == name).unwrap().effect).unwrap();
    for name in ["validate_model", "list_models", "read_model", "query_results"] {
        assert_eq!(effect(name), "read_only", "{name}");
    }
    for name in ["write_model", "delete_model"] {
        assert_eq!(effect(name), "destructive", "{name}");
    }
    assert_eq!(effect("copy_model"), "additive");
}

#[test]
fn model_file_format_gives_the_model_files_own_schema_and_a_valid_canonical_example() {
    let setup = Setup::new("format");
    let format = setup.ok("model_file_format", json!({}));
    assert_eq!(format["schema"], crabsim_model::schema::schema());
    let example = format["example"].as_str().unwrap();
    let model = crabsim_model::load(example).unwrap_or_else(|problems| panic!("the example has problems: {problems:?}"));
    assert_eq!(model.save(), example, "the example is in canonical form");
    assert!(!model.classes.is_empty(), "the example shows a derived class");
    let validator = jsonschema::validator_for(&format["schema"]).unwrap();
    assert!(validator.is_valid(&serde_json::from_str::<Value>(example).unwrap()), "the example passes the schema");
    assert!(!format["schema"].to_string().contains("docs/"), "the schema points to nothing an agent can't reach");
}

/// The trial found an agent guessing how objects behave, so the format says it.
#[test]
fn model_file_format_explains_how_each_built_in_class_behaves() {
    let setup = Setup::new("semantics");
    let semantics = setup.ok("model_file_format", json!({}))["semantics"].as_str().unwrap().to_string();
    for part in ["MU", "Source", "Station", "Buffer", "Drain", "round-robin", "blocked", "availability", "position"] {
        assert!(semantics.contains(part), "the semantics don't mention {part}: {semantics}");
    }
}

/// A harness may not show an operation's output schema to the model, so what an output means is
/// in the description.
#[test]
fn descriptions_say_what_their_outputs_mean() {
    let setup = Setup::new("descriptions");
    let description = |name: &str| setup.catalogue.operations().find(|o| o.name == name).unwrap().description;
    for (name, parts) in [
        ("run_status", &["`here`"][..]),
        ("run_results", &["added up", "`why`", "busiest", "20 or fewer", "mean_lead_time_s"][..]),
        ("start_run", &["paired", "position", "10 observations"][..]),
    ] {
        for part in parts {
            assert!(description(name).contains(part), "{name} doesn't say {part}: {}", description(name));
        }
    }
}

/// The model file's schema is about 7 KB, so it comes from `model_file_format` alone rather than
/// in every operation a model passes through, which a harness would carry in every session.
#[test]
fn a_model_in_an_operation_is_json_text_pointing_to_model_file_format() {
    let setup = Setup::new("schemas");
    for operation in setup.catalogue.operations() {
        if operation.name == "model_file_format" {
            continue;
        }
        for schema in [&operation.input_schema, &operation.output_schema] {
            let text = schema.to_string();
            assert!(!text.contains("contentSchema") && !text.contains("ModelFile"), "{} carries the model file's schema", operation.name);
        }
    }
    for (name, text) in [
        ("validate_model", &setup.catalogue.operations().find(|o| o.name == "validate_model").unwrap().input_schema["properties"]["text"]),
        ("write_model", &setup.catalogue.operations().find(|o| o.name == "write_model").unwrap().input_schema["properties"]["text"]),
        ("read_model", &setup.catalogue.operations().find(|o| o.name == "read_model").unwrap().output_schema["properties"]["text"]),
    ] {
        assert_eq!(text["type"], "string", "{name}");
        assert_eq!(text["contentMediaType"], "application/json", "{name}");
        assert!(text["description"].as_str().unwrap().contains("model_file_format"), "{name}: {text}");
    }
}

/// A harness may check a tool's input against its schema before calling it, so every input an
/// operation accepts must pass that schema, above all the shortest ones that leave out optional
/// fields.
#[test]
fn the_shortest_inputs_an_operation_accepts_pass_its_input_schema() {
    let setup = Setup::new("short-inputs");
    let inputs = [
        ("validate_model", json!({ "path": "line.json" })),
        ("validate_model", json!({ "text": SMALL })),
        ("list_models", json!({})),
        ("read_model", json!({ "path": "line.json" })),
        ("write_model", json!({ "path": "line.json", "text": SMALL })),
        ("copy_model", json!({ "from": "line.json", "to": "copy.json" })),
        ("delete_model", json!({ "path": "line.json" })),
        ("model_file_format", json!({})),
        ("edit_model", json!({ "path": "line.json", "changes": [{ "change": "remove_instance", "name": "Mill" }] })),
        ("edit_model", json!({ "path": "line.json", "changes": [{ "change": "set_attribute", "target": { "class": "Mill" }, "attribute": "process_time", "value": "1:00" }] })),
        ("get_attributes", json!({ "path": "line.json", "target": { "instance": "Mill" } })),
        ("claim_model", json!({ "path": "line.json", "purpose": "adding a mill" })),
        ("release_model", json!({ "path": "line.json" })),
        ("get_edit_queue", json!({})),
        ("take_model", json!({ "path": "line.json" })),
        ("give_model", json!({ "path": "line.json", "agent": "agent-1" })),
        ("move_in_queue", json!({ "path": "line.json", "agent": "agent-1", "place": 0 })),
        ("remove_from_queue", json!({ "path": "line.json", "agent": "agent-1" })),
        ("block_agent", json!({ "agent": "agent-1" })),
        ("unblock_agent", json!({ "agent": "agent-1" })),
        ("start_run", json!({ "model": "line.json", "length": "1d" })),
        ("run_status", json!({ "run": "run-1" })),
        ("cancel_run", json!({ "run": "run-1" })),
        ("run_results", json!({ "run": "run-1" })),
        ("label_run", json!({ "run": "run-1" })),
        ("forget_run", json!({ "run": "run-1" })),
        ("query_results", json!({ "sql": "SELECT 1" })),
    ];
    for operation in setup.catalogue.operations() {
        assert!(inputs.iter().any(|(name, _)| *name == operation.name), "add the shortest input of {} here", operation.name);
    }
    for (name, input) in inputs {
        let schema = &setup.catalogue.operations().find(|o| o.name == name).unwrap().input_schema;
        let validator = jsonschema::validator_for(schema).unwrap();
        let errors: Vec<_> = validator.iter_errors(&input).map(|error| error.to_string()).collect();
        assert!(errors.is_empty(), "{name} accepts {input}, but its schema refuses it: {errors:?}");
    }
}

#[test]
fn an_unknown_operation_or_an_input_that_doesnt_fit_is_invalid_input() {
    let setup = Setup::new("bad-input");
    let error = setup.err("frobnicate", json!({}));
    assert_eq!(error["kind"], "invalid_input");
    assert!(error["message"].as_str().unwrap().contains("validate_model"), "names what exists: {error}");

    let error = setup.err("read_model", json!({ "file": "x.json" }));
    assert_eq!(error["kind"], "invalid_input");
    let error = setup.err("validate_model", json!({}));
    assert_eq!(error["kind"], "invalid_input");
}

#[test]
fn validate_model_reports_every_problem_by_path_or_by_text() {
    let setup = Setup::new("validate");
    setup.folder.write("broken.json", BROKEN);
    setup.folder.write("plant/small.json", SMALL);

    let by_path = setup.ok("validate_model", json!({ "path": "broken.json" }));
    let by_text = setup.ok("validate_model", json!({ "text": BROKEN }));
    assert_eq!(by_path, by_text);
    assert_eq!(by_path["valid"], false);
    assert_eq!(by_path["problems"], problems_of(BROKEN));
    assert_eq!(by_path["problems"].as_array().unwrap().len(), 3);

    let valid = setup.ok("validate_model", json!({ "path": "plant/small.json" }));
    assert_eq!(valid, json!({ "valid": true, "problems": [] }));

    let missing = setup.err("validate_model", json!({ "path": "nothing.json" }));
    assert_eq!(missing["kind"], "not_found");
}

#[test]
fn list_models_lists_json_files_in_the_folder_and_below_but_never_crabsims_own() {
    let setup = Setup::new("list");
    setup.folder.write("b.json", SMALL);
    setup.folder.write("a.json", SMALL);
    setup.folder.write("plant/hall 2/line.json", SMALL);
    setup.folder.write("notes.txt", "not a model");
    setup.folder.write(".crabsim/sneaky.json", SMALL);
    // The store exists once a store operation has run.
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    assert!(setup.folder.join(".crabsim/results.sqlite").is_file());

    let listed = setup.ok("list_models", json!({}));
    assert_eq!(listed["models"], json!(["a.json", "b.json", "plant/hall 2/line.json"]));
    let under = setup.ok("list_models", json!({ "folder": "plant" }));
    assert_eq!(under["models"], json!(["plant/hall 2/line.json"]));

    assert_eq!(setup.err("list_models", json!({ "folder": ".crabsim" }))["kind"], "outside_models_folder");
    assert_eq!(setup.err("list_models", json!({ "folder": "elsewhere" }))["kind"], "not_found");
}

#[test]
fn read_model_gives_the_files_text_exactly() {
    let setup = Setup::new("read");
    setup.folder.write("small.json", SMALL);
    let read = setup.ok("read_model", json!({ "path": "small.json" }));
    assert_eq!(read, json!({ "path": "small.json", "text": SMALL }));
    assert_eq!(setup.err("read_model", json!({ "path": "gone.json" }))["kind"], "not_found");
    assert_eq!(setup.err("read_model", json!({ "path": "notes.txt" }))["kind"], "invalid_input");
}

#[test]
fn write_model_saves_a_valid_model_canonical_and_an_invalid_one_as_given_with_its_problems() {
    let setup = Setup::new("write");
    let written = setup.ok("write_model", json!({ "path": "new/small.json", "text": SMALL }));
    assert_eq!(written, json!({ "path": "new/small.json", "saved": "canonical", "problems": [] }));
    let canonical = crabsim_model::load(SMALL).unwrap().save();
    assert_ne!(canonical, SMALL, "the test needs a model that isn't canonical yet");
    assert_eq!(setup.folder.read("new/small.json"), canonical);

    let written = setup.ok("write_model", json!({ "path": "draft.json", "text": BROKEN }));
    assert_eq!(written["saved"], "as_given");
    assert_eq!(written["problems"], problems_of(BROKEN));
    assert_eq!(setup.folder.read("draft.json"), BROKEN);

    let not_json = "{ this is work in progress";
    let written = setup.ok("write_model", json!({ "path": "draft.json", "text": not_json }));
    assert_eq!(written["saved"], "as_given");
    assert_eq!(setup.folder.read("draft.json"), not_json, "it replaces what was there");
}

#[test]
fn copy_model_copies_but_never_overwrites() {
    let setup = Setup::new("copy");
    setup.folder.write("small.json", SMALL);
    setup.folder.write("other.json", BROKEN);

    let copied = setup.ok("copy_model", json!({ "from": "small.json", "to": "variants/small 2.json" }));
    assert_eq!(copied, json!({ "path": "variants/small 2.json" }));
    assert_eq!(setup.folder.read("variants/small 2.json"), SMALL);

    let refused = setup.err("copy_model", json!({ "from": "small.json", "to": "other.json" }));
    assert_eq!(refused["kind"], "already_exists");
    assert_eq!(setup.folder.read("other.json"), BROKEN);
    assert_eq!(setup.err("copy_model", json!({ "from": "gone.json", "to": "x.json" }))["kind"], "not_found");
}

#[test]
fn delete_model_deletes_files_only() {
    let setup = Setup::new("delete");
    setup.folder.write("small.json", SMALL);
    setup.folder.write("plant.json/inside.json", SMALL);

    assert_eq!(setup.ok("delete_model", json!({ "path": "small.json" })), json!({ "deleted": "small.json" }));
    assert!(!setup.folder.join("small.json").exists());
    assert_eq!(setup.err("delete_model", json!({ "path": "small.json" }))["kind"], "not_found");
    // A folder whose name ends in .json is still a folder.
    assert_eq!(setup.err("delete_model", json!({ "path": "plant.json" }))["kind"], "not_found");
    assert!(setup.folder.join("plant.json/inside.json").is_file());
}

/// Every model operation, on `path`.
fn every_operation_on(path: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("validate_model", json!({ "path": path })),
        ("read_model", json!({ "path": path })),
        ("write_model", json!({ "path": path, "text": SMALL })),
        ("copy_model", json!({ "from": path, "to": "copy.json" })),
        ("copy_model", json!({ "from": "small.json", "to": path })),
        ("delete_model", json!({ "path": path })),
    ]
}

#[test]
fn no_model_operation_reaches_outside_the_models_folder() {
    let outside = TempFolder::new("outside");
    outside.write("secret.json", SMALL);
    let setup = Setup::new("inside");
    setup.folder.write("small.json", SMALL);
    let absolute = outside.join("secret.json").to_string_lossy().into_owned();
    let escapes = [
        "../secret.json".to_string(),
        "plant/../../secret.json".to_string(),
        format!("../{}/secret.json", outside.path.file_name().unwrap().to_string_lossy()),
        absolute,
        "/secret.json".to_string(),
        "C:secret.json".to_string(),
    ];
    for path in &escapes {
        for (name, input) in every_operation_on(path) {
            let error = setup.err(name, input.clone());
            assert_eq!(error["kind"], "outside_models_folder", "{name} {input}: {error}");
        }
    }
    assert_eq!(outside.read("secret.json"), SMALL, "nothing outside changed");
    assert!(!outside.join("copy.json").exists());
}

#[test]
fn crabsims_own_folder_never_changes_through_model_operations() {
    let setup = Setup::new("own-folder");
    setup.folder.write("small.json", SMALL);
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    setup.folder.write(".crabsim/notes.json", SMALL);
    let store_before = std::fs::read(setup.folder.join(".crabsim/results.sqlite")).unwrap();

    for path in [".crabsim/notes.json", ".crabsim/results.sqlite", ".CRABSIM/notes.json", "./.crabsim/x.json", "plant/../.crabsim/notes.json"] {
        for (name, input) in every_operation_on(path) {
            let error = setup.err(name, input.clone());
            assert!(["outside_models_folder", "invalid_input"].contains(&error["kind"].as_str().unwrap()), "{name} {input}: {error}");
        }
    }
    assert_eq!(setup.folder.read(".crabsim/notes.json"), SMALL);
    assert_eq!(std::fs::read(setup.folder.join(".crabsim/results.sqlite")).unwrap(), store_before);
}

/// Makes `link` a link to the folder `target`: a junction on Windows, which needs no special
/// rights, and a symbolic link elsewhere.
fn link_folder(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "a junction can be made");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn a_link_leading_out_of_the_models_folder_is_refused_and_never_listed() {
    let outside = TempFolder::new("linked-outside");
    outside.write("secret.json", SMALL);
    let setup = Setup::new("links");
    setup.folder.write("small.json", SMALL);
    link_folder(&setup.folder.join("linked"), &outside.path);
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    link_folder(&setup.folder.join("into-own"), &setup.folder.join(".crabsim"));
    assert!(setup.folder.join("linked/secret.json").is_file(), "the link works");

    for path in ["linked/secret.json", "linked/new.json", "into-own/results.sqlite.json"] {
        for (name, input) in every_operation_on(path) {
            let error = setup.err(name, input.clone());
            assert_eq!(error["kind"], "outside_models_folder", "{name} {input}: {error}");
        }
    }
    assert_eq!(setup.ok("list_models", json!({}))["models"], json!(["small.json"]));
    assert_eq!(setup.err("list_models", json!({ "folder": "linked" }))["kind"], "outside_models_folder");
    assert_eq!(outside.read("secret.json"), SMALL);
    assert!(!outside.join("new.json").exists());
}

#[test]
fn a_link_that_stays_inside_the_models_folder_is_followed() {
    let setup = Setup::new("inner-link");
    setup.folder.write("real/small.json", SMALL);
    link_folder(&setup.folder.join("alias"), &setup.folder.join("real"));
    assert_eq!(setup.ok("read_model", json!({ "path": "alias/small.json" }))["text"], SMALL);
}

#[test]
fn a_folder_holding_source_code_is_refused_as_a_models_folder() {
    for (what, file) in [("Cargo.toml", "Cargo.toml"), (".git", ".git/HEAD"), ("a .rs file", "deep/down/main.rs")] {
        let folder = TempFolder::new("source");
        folder.write("model.json", SMALL);
        folder.write(file, "");
        let opened = Catalogue::open(Settings::new(&folder.path, Source::Mcp));
        let error = opened.err().unwrap_or_else(|| panic!("a folder holding {what} must be refused"));
        assert_eq!(serde_json::to_value(error.kind).unwrap(), "invalid_input");
        assert!(error.message.contains("source code"), "{error}");
        assert!(!folder.join(".crabsim").exists(), "nothing was written");
    }
    let missing = Catalogue::open(Settings::new(std::env::temp_dir().join("crabsim-no-such-folder"), Source::Cli));
    assert_eq!(serde_json::to_value(missing.err().unwrap().kind).unwrap(), "not_found");
}

#[test]
fn the_repositorys_own_models_folder_opens_but_the_repository_doesnt() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    assert!(Catalogue::open(Settings::new(repository.join("models"), Source::Cli)).is_ok());
    assert!(Catalogue::open(Settings::new(&repository, Source::Cli)).is_err());
    let catalogue = Catalogue::open(Settings::new(repository.join("models"), Source::Cli)).unwrap();
    let valid = catalogue.call("validate_model", json!({ "text": demo_text() })).unwrap();
    assert_eq!(valid["valid"], true);
}

#[test]
fn crabsims_own_folder_is_refused_even_before_it_exists() {
    let setup = Setup::new("no-own-folder-yet");
    let error = setup.err("write_model", json!({ "path": ".crabsim/results.json", "text": SMALL }));
    assert_eq!(error["kind"], "outside_models_folder");
    assert!(!setup.folder.join(".crabsim").exists());
}

#[test]
fn a_link_round_in_a_circle_doesnt_stop_list_models() {
    let setup = Setup::new("circle");
    setup.folder.write("plant/small.json", SMALL);
    link_folder(&setup.folder.join("plant").join("again"), &setup.folder.path);
    assert_eq!(setup.ok("list_models", json!({}))["models"], json!(["plant/small.json"]));
}

#[test]
fn a_link_round_in_a_circle_doesnt_stop_opening_the_folder() {
    let folder = TempFolder::new("circle-open");
    folder.write("plant/small.json", SMALL);
    link_folder(&folder.join("plant").join("again"), &folder.path);
    assert!(Catalogue::open(Settings::new(&folder.path, Source::Mcp)).is_ok());
}
