//! Bounds (ADR 0030): expectations that can't be met, or contradict another, found without a
//! run, as `validate_model`'s warnings. Each kind of bound warns past its edge and not at it, and
//! none ever warns about what a run of the model files actually reached.

use crabsim::figures::{Figure, Scope};
use crabsim::{ObjectKind, SimTime};
use crabsim_model::{load, CheckStage, Expectation, Model};
use serde_json::{json, Value};

/// Source every 10 s, Press A 8 s, Buffer, Press B 12 s, Drain: Press B is the slowest.
fn line(expectations: Value) -> Value {
    json!({
        "format_version": 2,
        "classes": [{ "name": "Press", "parent": "Station" }],
        "instances": [
            { "name": "Source", "class": "Source", "attributes": { "interval": "10" } },
            { "name": "Press A", "class": "Press", "attributes": { "process_time": "8" } },
            { "name": "Buffer", "class": "Buffer", "attributes": { "capacity": 5 } },
            { "name": "Press B", "class": "Press", "attributes": { "process_time": "12" } },
            { "name": "Drain", "class": "Drain" }
        ],
        "connections": [
            { "from": "Source", "to": "Press A", "exit": 1, "entrance": 1 },
            { "from": "Press A", "to": "Buffer", "exit": 1, "entrance": 1 },
            { "from": "Buffer", "to": "Press B", "exit": 1, "entrance": 1 },
            { "from": "Press B", "to": "Drain", "exit": 1, "entrance": 1 }
        ],
        "expectations": expectations
    })
}

/// The warnings about expectations, as `id: message`.
fn bounds(file: &Value) -> Vec<String> {
    let model = load(&file.to_string()).unwrap_or_else(|problems| panic!("{problems:?}"));
    model.expectation_bounds().iter().map(|warning| format!("{}: {}", warning.name.as_deref().unwrap_or(""), warning.message)).collect()
}

/// Expectations of the verification stage, by id `e1`, `e2` and so on.
fn lines(texts: &[&str]) -> Value {
    Value::Array(texts.iter().enumerate().map(|(k, text)| json!({ "id": format!("e{}", k + 1), "text": text })).collect())
}

/// The ids warned about.
fn warned(file: &Value) -> Vec<String> {
    let mut ids: Vec<String> = bounds(file).iter().map(|warning| warning.split(':').next().unwrap().to_string()).collect();
    ids.dedup();
    ids
}

#[test]
fn a_throughput_or_count_above_what_the_line_can_pass_is_warned_naming_what_limits_it() {
    // Press B finishes at most 86400 / 12 + 1 = 7201 MUs in a day: 7200 after the first, which
    // comes after 20 s at the earliest, so 300.07/h.
    let file = line(lines(&[
        "Drain.throughput_per_hour since start >= 301/h",
        "Drain.throughput_per_hour since start >= 300/h",
        "Drain.mus_in since start >= 7202",
        "Drain.mus_in since start >= 7201",
        "model.throughput_per_hour since start > 300.1/h",
        "Source.mus_out since start = 9000 ± 1%",
        "Press A.mus_in last 1h >= 362",
        "Press A.mus_in last 1h >= 361",
    ]));
    assert_eq!(warned(&file), ["e1", "e3", "e5", "e6", "e7"]);
    let warnings = bounds(&file);
    assert!(warnings[0].contains("Press B (1 MU at a time"), "{}", warnings[0]);
    assert!(warnings[0].contains("7201 MUs"), "{}", warnings[0]);
    // A Source makes one MU every 10 s: 8641 in a day, 361 in an hour.
    assert!(warnings[3].contains("Source (an MU every"), "{}", warnings[3]);
}

#[test]
fn a_station_busy_more_than_the_mus_reaching_it_allow_is_warned() {
    // The Source sends Press A at most 8641 MUs a day, 8 s each: 80%. Press B could get more
    // than it can work, so nothing limits its share below 100%.
    let file = line(lines(&[
        "Press A.working_share since start >= 81%",
        "Press A.working_share since start >= 80%",
        "every Press.working_share since start >= 85%",
        "mean Press.working_share since start >= 95%",
    ]));
    assert_eq!(warned(&file), ["e1", "e3"]);
    assert!(bounds(&file)[1].contains("1 of its 2 instances can't"), "{:?}", bounds(&file));
}

#[test]
fn a_lead_time_shorter_than_the_work_on_the_way_is_warned() {
    let file = line(lines(&[
        "Drain.mean_lead_time_s since start <= 19s",
        "Drain.mean_lead_time_s since start <= 20s",
        "Drain.lead_time_s always <= 15s",
        "model.mean_lead_time_s last 1h < 20s",
    ]));
    assert_eq!(warned(&file), ["e1", "e3", "e4"]);
    assert!(bounds(&file)[0].contains("at least 20s"), "{:?}", bounds(&file));
}

#[test]
fn more_than_an_object_holds_is_warned() {
    let file = line(lines(&["Buffer.max_mus since start >= 6", "Buffer.max_mus since start >= 5", "Press A.mus now > 1", "Buffer.mean_mus since start = 5.5 ± 0.2"]));
    assert_eq!(warned(&file), ["e1", "e3", "e4"]);
}

#[test]
fn what_never_happens_in_a_check_is_warned() {
    let mut file = line(json!([
        { "id": "v-failed", "text": "Press A.failed_share since start > 0%" },
        { "id": "v-available", "text": "Press A.availability since start < 100%" },
        { "id": "no-profile", "text": "Press B.failures since start >= 1", "stage": "validation", "source": "MES" },
        { "id": "profile", "text": "Press A.failures since start >= 1", "stage": "validation", "source": "MES" },
        { "id": "paused", "text": "Press A.paused_share since start > 0%", "stage": "validation", "source": "MES" },
        { "id": "receiving", "text": "Press B.receiving_share since start > 0%" },
        { "id": "share", "text": "Press A.idle_share since start >= 101%" }
    ]));
    file["instances"][1]["attributes"]["failures"] = json!([{ "name": "wear", "availability": 0.9, "repair_time": { "constant": "60" } }]);
    assert_eq!(warned(&file), ["v-failed", "v-available", "no-profile", "paused", "receiving", "share"]);
    let warnings = bounds(&file);
    assert!(warnings[0].contains("verification stage"), "{warnings:?}");
    assert!(warnings[2].contains("no failure profile"), "{warnings:?}");
}

#[test]
fn expectations_asking_what_no_value_gives_together_contradict() {
    let file = line(json!([
        { "id": "busy", "text": "Press A.working_share since start >= 60%" },
        { "id": "not-busy", "text": "Press A.working_share since start <= 50%" },
        { "id": "other-check", "text": "Press A.working_share since start <= 50%", "length": "2:00:00" },
        { "id": "every", "text": "every Press.working_share since start <= 55%" },
        { "id": "state", "text": "Press B.state now = busy" },
        { "id": "not-state", "text": "Press B.state now != busy" },
        { "id": "limit", "text": "Buffer.mus always <= 3" },
        { "id": "most", "text": "Buffer.max_mus since start >= 4" }
    ]));
    let warnings = bounds(&file);
    assert_eq!(warned(&file), ["not-busy", "every", "not-state", "most"], "{warnings:?}");
    assert!(warnings[0].contains("contradicts busy: Press A's working_share since start can't be both >= 60% and <= 50%"), "{warnings:?}");
}

#[test]
fn an_objects_shares_of_time_add_up_to_all_of_it() {
    let file = line(lines(&[
        "Press B.working_share since start >= 60%",
        "Press B.idle_share since start >= 30%",
        "Press B.blocked_share since start >= 15%",
        "Press A.working_share since start <= 30%",
        "Press A.idle_share since start <= 30%",
        "Press A.blocked_share since start <= 30%",
    ]));
    let warnings = bounds(&file);
    // Press A can neither fail, pause nor receive here, so those three leave 10% unaccounted for.
    assert_eq!(warned(&file), ["e3", "e6"], "{warnings:?}");
    assert!(warnings[0].contains("at least 105%"), "{warnings:?}");
}

#[test]
fn a_model_with_code_gets_only_the_bounds_code_cant_change() {
    let mut file = line(lines(&["Drain.mus_in since start >= 100000", "Buffer.max_mus since start >= 6", "Press A.working_share since start >= 101%"]));
    file["instances"][1]["attributes"]["on_entry"] = json!({ "code": { "function": "routing.report" } });
    assert_eq!(warned(&file), ["e2", "e3"]);
}

#[test]
fn a_loop_bounds_its_objects_by_their_own_limits_alone() {
    // A rework loop: Press A sends some MUs back through Rework, so it takes more MUs than the
    // Source makes.
    let mut file = line(lines(&["Press A.mus_in since start >= 9000", "Press A.mus_in since start >= 10803"]));
    file["instances"].as_array_mut().unwrap().push(json!({ "name": "Rework", "class": "Station", "attributes": { "process_time": "1" } }));
    file["connections"].as_array_mut().unwrap().push(json!({ "from": "Press A", "to": "Rework", "exit": 2, "entrance": 1 }));
    file["connections"].as_array_mut().unwrap().push(json!({ "from": "Rework", "to": "Press A", "exit": 1, "entrance": 2 }));
    // 10801 it can finish, and 1 it can hold at the end.
    assert_eq!(warned(&file), ["e2"]);
}

// ---------- Never wrong about what a run reached ----------

/// A share as a line writes it, never above `value`.
fn share_at_most(value: f64) -> String {
    format!("{}%", (value * 100.0 * 1e6).floor() / 1e6)
}

/// For every object and figure a bound reads, what a run reached as an expectation that asks no
/// more of it: at least what it reached of what is bounded from above, at most of what is
/// bounded from below.
fn reached(model: &Model, state: &crabsim::RunState, scope: Scope, from: Option<&crabsim::RunState>) -> Vec<String> {
    let scope_text = match scope {
        Scope::SinceStart => "since start".to_string(),
        Scope::Last(length) => format!("last {}s", length.as_secs()),
        _ => unreachable!(),
    };
    let mut texts = Vec::new();
    let read = |figure: Figure, id: usize| -> Option<f64> {
        let object = &state.objects[id];
        match from {
            Some(earlier) => figure.value_over(&earlier.objects[id], object),
            None => figure.value(object, state.time),
        }
    };
    for (id, instance) in model.instances.iter().enumerate() {
        let name = &instance.name;
        let kind = state.objects[id].kind;
        for figure in [Figure::MusIn, Figure::MusOut, Figure::WorkingShare, Figure::FailedShare, Figure::Failures, Figure::PausedShare, Figure::ReceivingShare] {
            let Some(value) = read(figure, id) else { continue };
            let text = match figure.unit() {
                crabsim::figures::Unit::Share => share_at_most(value),
                _ => format!("{}", value.floor()),
            };
            texts.push(format!("{name}.{} {scope_text} >= {text}", figure.name()));
        }
        if let Some(value) = read(Figure::Availability, id) {
            texts.push(format!("{name}.availability {scope_text} <= {}%", (value * 100.0 * 1e6).ceil() / 1e6));
        }
        if kind == ObjectKind::Drain {
            if let Some(value) = read(Figure::ThroughputPerHour, id) {
                texts.push(format!("{name}.throughput_per_hour {scope_text} >= {}/h", (value * 1e6).floor() / 1e6));
            }
            if let Some(value) = read(Figure::MeanLeadTime, id) {
                texts.push(format!("{name}.mean_lead_time_s {scope_text} <= {}s", (value * 1e3).ceil() / 1e3));
            }
            if from.is_none()
                && let Some(least) = Figure::LeadTime.extreme(&state.objects[id], false)
            {
                texts.push(format!("{name}.lead_time_s always <= {}s", (least * 1e3).ceil() / 1e3));
            }
        }
        if kind == ObjectKind::Conveyor
            && let Some(value) = read(Figure::MeanCrossingTime, id)
        {
            texts.push(format!("{name}.mean_crossing_time_s {scope_text} <= {}s", (value * 1e3).ceil() / 1e3));
        }
        if from.is_none() && matches!(kind, ObjectKind::Buffer | ObjectKind::Conveyor) {
            texts.push(format!("{name}.max_mus since start >= {}", state.objects[id].statistics.max_contents));
        }
    }
    let model_value = match from {
        Some(earlier) => Figure::ThroughputPerHour.model_value_over(&earlier.objects, &state.objects),
        None => Figure::ThroughputPerHour.model_value(&state.objects, state.time),
    };
    if let Some(value) = model_value {
        texts.push(format!("model.throughput_per_hour {scope_text} >= {}/h", (value * 1e6).floor() / 1e6));
    }
    texts
}

fn check_reached(path: &str, deterministic: bool, seed: u64, warm_up: SimTime, length: SimTime) {
    let text = std::fs::read_to_string(format!("{}/../models/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut model = load(&text).unwrap();
    model.expectations.clear();
    model.requests.clear();
    let stage = if deterministic { CheckStage::Verification } else { CheckStage::Validation };
    let settings = model.stages.of_mut(stage);
    settings.length = Some(length);
    settings.warm_up = Some(warm_up);
    let description = model.to_description();
    let mut run = if deterministic { description.build_deterministic(seed) } else { description.build(seed) };
    run.init().unwrap();
    run.advance_to(warm_up).unwrap();
    if warm_up > SimTime::ZERO {
        run.clear_statistics();
    }
    let stretch = SimTime::from_secs(3600);
    run.advance_to((warm_up + length).saturating_sub(stretch)).unwrap();
    let before = run.state();
    run.advance_to(warm_up + length).unwrap();
    let end = run.state();
    let mut texts = reached(&model, &end, Scope::SinceStart, None);
    texts.extend(reached(&model, &end, Scope::Last(stretch), Some(&before)));
    for (k, text) in texts.iter().enumerate() {
        let mut expectation = Expectation::new(format!("e{k}"), text.clone());
        expectation.stage = stage;
        expectation.source = Some("the run".into());
        model.expectations.push(expectation);
    }
    // Read back, so every line is checked against the model as a file's would be.
    let model = load(&model.save()).unwrap_or_else(|problems| panic!("{path}: {problems:?}"));
    let warnings: Vec<String> = model
        .expectation_bounds()
        .iter()
        .map(|warning| format!("{} ({}): {}", warning.name.as_deref().unwrap_or(""), model.expectations.iter().find(|e| Some(&e.id) == warning.name.as_ref()).map_or("", |e| e.text.as_str()), warning.message))
        .collect();
    assert!(warnings.is_empty(), "{path}, seed {seed}, deterministic {deterministic}: what a run reached is warned:\n{}", warnings.join("\n"));
    assert!(texts.len() > 10, "{path}: {}", texts.len());
}

#[test]
fn no_bound_is_below_what_a_run_of_the_model_files_reached() {
    let day = SimTime::from_secs(86_400);
    for path in ["demo.json", "buffered-line.json", "conveyor-line.json", "cross-line.json"] {
        check_reached(path, true, 0, SimTime::ZERO, day);
        check_reached(path, true, 0, SimTime::from_secs(7200), SimTime::from_secs(6 * 3600));
        for seed in [1, 7] {
            check_reached(path, false, seed, SimTime::ZERO, day);
        }
        check_reached(path, false, 3, SimTime::from_secs(3 * 3600), SimTime::from_secs(5 * 3600));
    }
}

#[test]
fn a_short_run_bounds_what_its_starting_mus_can_add() {
    // Two MUs start in the Buffer, so in the first minute Press B can finish 2... at most what
    // it can work: 60 / 12 + 1 = 6, and the Drain gets no more than that.
    let mut file = line(lines(&["Drain.mus_in since start >= 7", "Drain.mus_in since start >= 6"]));
    file["instances"][2]["attributes"]["starting_mus"] = json!(2);
    file["stages"] = json!({ "verification": { "length": "60" } });
    assert_eq!(warned(&file), ["e1"]);
}

#[test]
fn what_objects_hold_as_a_stretch_begins_can_pass_on_in_it() {
    // The Buffer starts with 20 MUs, which Press B, every 5 s, works off faster than the Source,
    // every 10 s, brings more: after a minute's warm-up the Drain gets more in the next minute
    // than the Source makes in it.
    for capacity in [json!(20), json!("unlimited")] {
        let mut file = line(json!([]));
        file["instances"][2]["attributes"] = json!({ "capacity": capacity, "starting_mus": 20 });
        file["instances"][3]["attributes"]["process_time"] = json!("5");
        file["stages"] = json!({ "verification": { "warm_up": "60", "length": "60" } });
        let model = load(&file.to_string()).unwrap();
        let mut run = model.to_description().build_deterministic(0);
        run.init().unwrap();
        run.advance_to(SimTime::from_secs(60)).unwrap();
        run.clear_statistics();
        run.advance_to(SimTime::from_secs(120)).unwrap();
        let reached = run.state().objects[4].statistics.mus_in;
        assert!(reached > 7, "the Source makes at most 7 in a minute, and the Drain got {reached}");
        file["expectations"] = lines(&[&format!("Drain.mus_in since start >= {reached}"), "Drain.mus_in since start >= 14"]);
        assert_eq!(warned(&file), ["e2"], "{capacity}: {:?}", bounds(&file));
    }
}

#[test]
fn a_reference_figure_has_a_tolerance_and_says_where_it_came_from() {
    let file = line(json!([
        { "id": "exact", "text": "Drain.mus_in since start = 7000", "stage": "validation", "source": "MES" },
        { "id": "no-source", "text": "Drain.mus_in since start = 7000 ± 3%", "stage": "validation" },
        { "id": "reference", "text": "Drain.mus_in since start = 7000 ± 3%", "stage": "validation", "source": "MES, October" },
        { "id": "verifying", "text": "Drain.mus_in since start = 7000", "source": "the plan" }
    ]));
    let model = load(&file.to_string()).unwrap();
    let warnings: Vec<String> = model.warnings().iter().filter(|warning| warning.pointer.starts_with("/expectations/")).map(|warning| format!("{} {}: {}", warning.pointer, warning.name.as_deref().unwrap(), warning.message)).collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].starts_with("/expectations/0/text exact: ") && warnings[0].contains("give the reference figure its tolerance"), "{warnings:?}");
    assert!(warnings[1].starts_with("/expectations/1 no-source: ") && warnings[1].contains("`source`"), "{warnings:?}");
}

#[test]
fn a_throughput_since_start_counts_from_the_drains_first_mu() {
    // Ten places of an hour each over two hours: the first ten MUs come at 1:00, the next ten at
    // 2:00, so 19 after the first in the hour since it, 19/h, though the Station does 10/h.
    let file = json!({
        "format_version": 2,
        "instances": [
            { "name": "Source", "class": "Source" },
            { "name": "Oven", "class": "Station", "attributes": { "process_time": "1:00:00", "capacity": 10 } },
            { "name": "Drain", "class": "Drain" }
        ],
        "connections": [
            { "from": "Source", "to": "Oven", "exit": 1, "entrance": 1 },
            { "from": "Oven", "to": "Drain", "exit": 1, "entrance": 1 }
        ],
        "stages": { "verification": { "length": "2:00:00" } },
        "expectations": lines(&["Drain.throughput_per_hour since start >= 19/h", "Drain.throughput_per_hour since start >= 30/h"])
    });
    let model = load(&file.to_string()).unwrap();
    let mut run = model.to_description().build_deterministic(0);
    run.init().unwrap();
    run.advance_to(SimTime::from_secs(7200)).unwrap();
    let reached = Figure::ThroughputPerHour.value(&run.state().objects[2], SimTime::from_secs(7200)).unwrap();
    assert!((reached - 19.0).abs() < 1e-6, "{reached}");
    assert_eq!(warned(&file), ["e2"], "{:?}", bounds(&file));
}
