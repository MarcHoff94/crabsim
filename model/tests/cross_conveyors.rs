//! Cross conveyors in model files (spec issue 148, PR 2): sides on connections, exits, switch and
//! release times, their problems and warnings, and what a run of them does.

use crabsim::{OccurrenceKind, Recording, SimTime};
use crabsim_model::{load, Attribute, Model, Side};
use serde_json::{json, Value};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

/// The cross conveyor example as a file, with line endings as crabsim writes them.
fn cross_example() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/cross-line.json");
    std::fs::read_to_string(path).expect("models/cross-line.json exists").replace("\r\n", "\n")
}

/// A Source "In", of MU class Part going to "Spur", into a cross conveyor's back, with the Drains
/// "Ahead" at its front and "Spur" at its left, which lists the Spur, plus whatever `change` does.
fn fork(change: impl FnOnce(&mut Value)) -> String {
    let mut file = json!({
        "format_version": 2,
        "classes": [{ "name": "Part", "parent": "MU", "attributes": { "work_plan": [{ "at": "Spur" }] } }],
        "instances": [
            { "name": "In", "class": "Source", "attributes": { "interval": "10", "mu_class": "Part" } },
            { "name": "Cross", "class": "CrossConveyor", "attributes": { "exits": { "left": ["Spur"] } } },
            { "name": "Ahead", "class": "Drain" },
            { "name": "Spur", "class": "Drain" }
        ],
        "connections": [
            { "from": "In", "to": "Cross", "exit": 1, "entrance": 1, "to_side": "back" },
            { "from": "Cross", "to": "Ahead", "exit": 1, "entrance": 1, "from_side": "front" },
            { "from": "Cross", "to": "Spur", "exit": 2, "entrance": 1, "from_side": "left" }
        ]
    });
    change(&mut file);
    file.to_string()
}

fn problems(text: &str) -> Vec<String> {
    load(text).expect_err("the file has problems").iter().map(|problem| format!("{}: {}", problem.pointer, problem.message)).collect()
}

fn assert_problem(text: &str, pointer: &str, part: &str) {
    let found = problems(text);
    assert!(found.iter().any(|problem| problem.starts_with(&format!("{pointer}: ")) && problem.contains(part)), "no problem at {pointer} saying {part:?} in {found:#?}");
}

fn warnings(model: &Model) -> Vec<String> {
    model.warnings().iter().map(|warning| format!("{}: {}", warning.pointer, warning.message)).collect()
}

/// When MUs moved from the instance called `from` to the one called `to`, in seconds.
fn moves(model: &Model, until: u64, from: &str, to: &str) -> Vec<f64> {
    let description = model.to_description();
    let id = |name: &str| model.instances.iter().position(|instance| instance.name == name).unwrap() as u32;
    let mut run = description.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { from: f, to: t, .. } if f == id(from) && t == id(to)))
        .map(|o| o.time.as_secs_f64())
        .collect()
}

#[test]
fn the_cross_example_is_valid_canonical_and_warns_of_nothing() {
    let text = cross_example();
    let model = load(&text).unwrap();
    assert_eq!(model.save(), text, "in canonical form");
    assert!(model.warnings().is_empty(), "{:#?}", model.warnings());
}

#[test]
#[ignore = "writes models/cross-line.json in canonical form"]
fn write_the_cross_example_canonical() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../models/cross-line.json");
    std::fs::write(path, load(&cross_example()).unwrap().save()).unwrap();
}

#[test]
fn the_cross_example_runs_as_the_benchmarks_cross_line() {
    let from_file = load(&cross_example()).unwrap().to_description();
    let in_code = crabsim::demo::cross_line();
    for seed in [0, 7] {
        let run = |description: &crabsim::ModelDescription| {
            let mut run = description.build(seed);
            run.set_recording(Recording::ALL);
            let mut occurrences = run.init().unwrap();
            occurrences.extend(run.advance_to(secs(2 * 86_400)).unwrap());
            (occurrences, run.state())
        };
        let (file_occurrences, file_state) = run(&from_file);
        let (code_occurrences, code_state) = run(&in_code);
        assert_eq!(file_occurrences.len(), code_occurrences.len(), "seed {seed}");
        assert!(file_occurrences == code_occurrences, "seed {seed}: the occurrences differ");
        assert!(file_state == code_state, "seed {seed}: the states differ");
        let turned = file_state.objects[4].cross.as_deref().unwrap();
        assert!(turned.mus_out[Side::Left.index()] > 10_000 && turned.mus_out[Side::Front.index()] > 10_000, "both ways are used: {turned:?}");
    }
}

#[test]
fn a_files_cross_conveyor_turns_mus_where_its_exits_list_their_destination() {
    let model = load(&fork(|_| {})).unwrap();
    assert_eq!(moves(&model, 25, "Cross", "Spur"), [13.2, 23.2], "0.6 s in, 2 s switching, 0.6 s out");
    assert!(moves(&model, 25, "Cross", "Ahead").is_empty());

    let straight = load(&fork(|file| file["classes"][0]["attributes"]["work_plan"][0]["at"] = json!("Ahead"))).unwrap();
    assert_eq!(moves(&straight, 25, "Cross", "Ahead"), [11.2, 21.2]);

    let slow = load(&fork(|file| {
        file["instances"][1]["attributes"]["switch_time"] = json!("5");
        file["instances"][1]["attributes"]["release_time"] = json!("3");
        file["instances"][0]["attributes"]["interval"] = json!("1");
    }))
    .unwrap();
    // 0.6 + 5 + 0.6 s across from 1 s, then 3 s releasing before the next enters at 10.2 s.
    assert_eq!(moves(&slow, 18, "Cross", "Spur"), [7.2, 16.4]);
}

#[test]
fn sides_exits_and_times_save_and_load_the_same() {
    let text = fork(|file| {
        file["instances"][1]["attributes"]["switch_time"] = json!("1.5");
        file["instances"][1]["attributes"]["release_time"] = json!("0:01");
        file["instances"][1]["attributes"]["length"] = json!(0.8);
        file["instances"][1]["attributes"]["speed"] = json!(0.4);
        file["instances"][1]["attributes"]["exits"] = json!({ "right": ["Spur", "Ahead"], "left": ["Spur"] });
    });
    let model = load(&text).unwrap();
    let saved = model.save();
    assert_eq!(load(&saved).unwrap().save(), saved);
    let resolved = model.resolve(1).unwrap();
    assert_eq!(resolved.attributes.switch_time, Some(SimTime::from_millis(1_500)));
    assert_eq!(resolved.attributes.release_time, Some(secs(1)));
    assert_eq!(resolved.attributes.exits.unwrap().into_iter().map(|(side, _)| side).collect::<Vec<_>>(), [Side::Left, Side::Right], "by side, whatever the file's order");
    let size = resolved.attributes.dimensions.unwrap();
    assert_eq!((size.length, size.width), (0.8, 0.6), "its own length and width");
    assert_eq!(model.connections[0].to_side, Some(Side::Back));
    assert!(saved.contains("\"from_side\": \"left\""));
    assert!(saved.contains("\"exits\": {\n"));
}

#[test]
fn a_cross_conveyors_defaults_are_a_conveyors_width_each_way_and_a_switch_time_of_2_s() {
    let model = load(&fork(|_| {})).unwrap();
    let resolved = model.resolve(1).unwrap();
    assert_eq!((resolved.attributes.length, resolved.attributes.width, resolved.attributes.speed), (Some(0.6), Some(0.6), Some(0.5)));
    assert_eq!((resolved.attributes.switch_time, resolved.attributes.release_time), (Some(secs(2)), Some(SimTime::ZERO)));
    assert_eq!(resolved.origin(Attribute::SwitchTime), &crabsim_model::Origin::BuiltIn);
}

#[test]
fn a_connection_names_the_side_of_a_cross_conveyor_and_of_nothing_else() {
    assert_problem(&fork(|file| { file["connections"][0].as_object_mut().unwrap().remove("to_side"); }), "/connections/0", "say which side the connection comes in by with `to_side`");
    assert_problem(&fork(|file| file["connections"][0]["from_side"] = json!("front")), "/connections/0/from_side", "`In` isn't a cross conveyor");
    assert_problem(&fork(|file| file["connections"][0]["to_side"] = json!("up")), "/connections/0/to_side", "a side is \"front\", \"back\", \"left\" or \"right\"");
}

#[test]
fn a_side_has_one_connection_in_or_out() {
    assert_problem(&fork(|file| file["connections"][2]["from_side"] = json!("front")), "/connections/2/from_side", "the front side of `Cross` has connection 1 already");
    assert_problem(&fork(|file| file["connections"][1]["from_side"] = json!("back")), "/connections/1/from_side", "the back side of `Cross` has connection 0 already");
}

#[test]
fn exits_name_places_by_side() {
    assert_problem(&fork(|file| file["instances"][1]["attributes"]["exits"] = json!({ "left": ["Spurr"] })), "/instances/1/attributes/exits/left/0", "did you mean `Spur`");
    assert_problem(&fork(|file| file["instances"][1]["attributes"]["exits"] = json!({ "up": ["Spur"] })), "/instances/1/attributes/exits/up", "`up` is no side");
    assert_problem(&fork(|file| file["instances"][1]["attributes"]["exits"] = json!(["Spur"])), "/instances/1/attributes/exits", "exits list each exit side's destinations");
    assert_problem(&fork(|file| file["instances"][1]["attributes"]["exits"] = json!({ "left": ["Part"] })), "/instances/1/attributes/exits/left/0", "is an MU class");
}

#[test]
fn only_a_cross_conveyor_has_exits_and_switch_and_release_times() {
    for attribute in ["exits", "switch_time", "release_time"] {
        let value = if attribute == "exits" { json!({ "left": ["Spur"] }) } else { json!("1") };
        assert_problem(&fork(|file| file["instances"][0]["attributes"][attribute] = value), &format!("/instances/0/attributes/{attribute}"), &format!("a Source has no attribute `{attribute}`"));
    }
    assert_problem(&fork(|file| file["instances"][1]["attributes"]["gap"] = json!(0.1)), "/instances/1/attributes/gap", "a CrossConveyor has no attribute `gap`");
}

#[test]
fn renaming_a_destination_an_exit_lists_is_checked_as_any_other_name() {
    let text = fork(|file| file["instances"][3]["name"] = json!("Side"));
    assert_problem(&text, "/instances/1/attributes/exits/left/0", "nothing is called `Spur`");
}

#[test]
fn validate_warns_of_an_entry_side_with_nothing_straight_across_and_of_exits_listed_for_a_side_with_no_connection() {
    // MUs going to Ahead, which only the front side lists, but nothing leaves by the front.
    let text = fork(|file| {
        file["connections"][1]["from_side"] = json!("right");
        file["instances"][1]["attributes"]["exits"] = json!({ "left": ["Spur"], "front": ["Ahead"] });
        file["classes"][0]["attributes"]["work_plan"][0]["at"] = json!("Ahead");
    });
    let found = warnings(&load(&text).unwrap());
    assert!(found.iter().any(|w| w.starts_with("/instances/1: ") && w.contains("MUs come in by its back side, and no connection leaves by its front side")), "{found:#?}");
    assert!(found.iter().any(|w| w.starts_with("/instances/1/attributes/exits/front: ") && w.contains("no connection leaves by it")), "{found:#?}");
    assert!(warnings(&load(&fork(|_| {})).unwrap()).is_empty());
    // Every MU turns, so nothing waits for want of a way straight across.
    let corner = fork(|file| file["connections"][1]["from_side"] = json!("right"));
    let found = warnings(&load(&corner).unwrap());
    assert!(!found.iter().any(|w| w.contains("straight across")), "{found:#?}");
}

#[test]
fn validate_warns_of_mus_too_big_for_a_cross_conveyor_they_reach() {
    let text = fork(|file| file["classes"][0]["attributes"]["length"] = json!(0.8));
    let found = warnings(&load(&text).unwrap());
    assert!(found.iter().any(|w| w.starts_with("/instances/0: ") && w.contains("cross conveyor `Cross`, 0.6 by 0.6 m, has no way across them")), "{found:#?}");
    // Coming in by its left or right, its way is its width.
    let fits = fork(|file| {
        file["classes"][0]["attributes"]["length"] = json!(0.8);
        file["instances"][1]["attributes"]["width"] = json!(0.9);
        file["connections"][0]["to_side"] = json!("right");
        file["connections"][2]["from_side"] = json!("back");
        file["instances"][1]["attributes"]["exits"] = json!({ "back": ["Spur"] });
    });
    let found = warnings(&load(&fits).unwrap());
    assert!(!found.iter().any(|w| w.contains("no way across")), "{found:#?}");
}

#[test]
fn validate_warns_of_mus_that_turned_and_lie_crosswise_on_a_conveyor_too_narrow_for_them() {
    // Parts, 0.6 by 0.4 m, turn left onto a 0.5 m belt, where they lie 0.6 m across it; going
    // straight on onto the same belt they would lie 0.4 m across and fit.
    let turned = |exit: &str| {
        fork(|file| {
            file["instances"][3] = json!({ "name": "Spur", "class": "Conveyor", "attributes": { "width": 0.5 } });
            file["instances"].as_array_mut().unwrap().push(json!({ "name": "End", "class": "Drain" }));
            file["connections"].as_array_mut().unwrap().push(json!({ "from": "Spur", "to": "End", "exit": 1, "entrance": 1 }));
            file["connections"][2]["from_side"] = json!(exit);
            file["instances"][1]["attributes"]["exits"] = json!({ exit: ["Spur"] });
        })
    };
    let found = warnings(&load(&turned("left")).unwrap());
    assert!(
        found.iter().any(|w| w.starts_with("/instances/0: ") && w.contains("lying crosswise") && w.contains("0.6 m wide, and conveyor `Spur` only 0.5 m")),
        "{found:#?}"
    );
    // The Spur straight across from the back, at the front, takes them lengthwise.
    let straight = fork(|file| {
        file["instances"][3] = json!({ "name": "Spur", "class": "Conveyor", "attributes": { "width": 0.5 } });
        file["instances"].as_array_mut().unwrap().push(json!({ "name": "End", "class": "Drain" }));
        file["connections"].as_array_mut().unwrap().push(json!({ "from": "Spur", "to": "End", "exit": 1, "entrance": 1 }));
        file["connections"].as_array_mut().unwrap().remove(1);
        file["connections"][1]["from_side"] = json!("front");
        file["connections"][1]["exit"] = json!(1);
        file["instances"][1]["attributes"]["exits"] = json!({ "front": ["Spur"] });
    });
    let found = warnings(&load(&straight).unwrap());
    assert!(!found.iter().any(|w| w.contains("conveyor `Spur`")), "{found:#?}");
}
