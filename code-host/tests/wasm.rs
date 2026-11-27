//! Code modules run in a simulation run: the example module's hooks as WebAssembly and natively,
//! the sandbox's caps, a module that doesn't fit, and checkpoints and Reset with state kept in
//! code.

mod common;

use crabsim::hooks::{Hook, Lifecycle, ObjectHooks};
use crabsim::random_numbers::Distribution;
use crabsim::{FailureProfile, ModelDescription, Occurrence, OccurrenceKind, Recording, RunError, SimTime, SimulationRun};
use crabsim_code::{Code, Library, LogLine, Options};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn never_fails() -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0)
}

/// Source → Fork → Left, Right → Drain, with names.
struct Fork {
    model: ModelDescription,
    fork: u32,
    left: u32,
    right: u32,
    drain: u32,
}

fn fork_model() -> Fork {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let fork = model.add_station(secs(5), never_fails());
    let left = model.add_station(secs(20), never_fails());
    let right = model.add_station(secs(12), never_fails());
    let drain = model.add_drain();
    for (id, name) in [(source, "Source"), (fork, "Fork"), (left, "Left"), (right, "Right"), (drain, "Drain")] {
        model.set_name(id, name);
    }
    model.connect(source, fork);
    model.connect(fork, left);
    model.connect(fork, right);
    model.connect(left, drain);
    model.connect(right, drain);
    Fork { model, fork, left, right, drain }
}

fn names(model: &ModelDescription) -> impl Fn(&str) -> Option<u32> + '_ {
    move |name| (0..model.object_count()).find(|&id| model.name(id) == Some(name))
}

fn library_with(name: &str, bytes: &[u8]) -> Library {
    let mut library = Library::new();
    library.load_wasm(name, bytes).unwrap();
    library
}

/// Gives the model code: the functions with their values, the n-th being hook function n.
fn give_code(model: &mut ModelDescription, library: Library, functions: &[(&str, serde_json::Value)], options: Options) {
    let bindings = functions
        .iter()
        .map(|(function, with)| library.bind(function, with.as_object().unwrap(), &names(model)).unwrap_or_else(|problems| panic!("{problems:?}")))
        .collect();
    let code = Code::new(Arc::new(library), bindings, model, options).unwrap();
    model.set_hooks(Box::new(code));
}

fn mus_in(run: &SimulationRun, id: u32) -> u64 {
    run.state().objects[id as usize].statistics.mus_in
}

fn every_third(library: Library) -> Fork {
    let mut fork = fork_model();
    give_code(&mut fork.model, library, &[("routing.every_nth", json!({"n": 3, "left": "Left", "right": "Right"}))], Options::default());
    fork.model.set_object_hooks(fork.fork, ObjectHooks { distribution: Some(0), ..ObjectHooks::default() });
    fork
}

fn moves_to(occurrences: &[Occurrence], from: u32) -> Vec<u32> {
    occurrences
        .iter()
        .filter_map(|occurrence| match occurrence.kind {
            OccurrenceKind::MuMoved { from: moved_from, to, .. } if moved_from == from => Some(to),
            _ => None,
        })
        .collect()
}

#[test]
fn a_module_declares_its_functions_and_parameters() {
    let library = library_with("routing", common::routing());
    let module = library.module("routing").unwrap();
    let declared: Vec<String> = module.functions().iter().map(ToString::to_string).collect();
    assert_eq!(declared, ["cycle()", "every_nth(n: int, left: object, right: object)", "pass()", "random_exit()", "report(drain: object, every: time)", "shortest_queue()"]);
    assert_eq!(module.hash().len(), 64, "a SHA-256 in hex");
}

#[test]
fn a_distribution_in_webassembly_sends_every_third_mu_left() {
    let fork = every_third(library_with("routing", common::routing()));
    let mut run = fork.model.build(0);
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(3600)).unwrap());
    let sent = moves_to(&occurrences, fork.fork);
    assert!(sent.len() > 300);
    for (n, to) in sent.iter().enumerate() {
        assert_eq!(*to, if (n + 1) % 3 == 0 { fork.left } else { fork.right }, "MU {} out of the fork", n + 1);
    }
}

#[test]
fn the_same_functions_natively_give_the_same_occurrences() {
    let run_with = |library: Library| {
        let fork = every_third(library);
        let mut run = fork.model.build(7);
        run.set_recording(Recording::ALL);
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(7200)).unwrap());
        (occurrences, run.state())
    };
    let mut native = Library::new();
    native.add_native(crabsim_guest::native_module!("routing", [routing::every_nth, routing::shortest_queue, routing::random_exit, routing::report, routing::pass, routing::cycle])).unwrap();
    let (wasm_occurrences, wasm_state) = run_with(library_with("routing", common::routing()));
    let (native_occurrences, native_state) = run_with(native);
    assert_eq!(wasm_occurrences, native_occurrences);
    assert_eq!(wasm_state, native_state);
}

#[test]
fn a_checkpoint_carries_the_modules_memory_and_reset_clears_it() {
    let fork = every_third(library_with("routing", common::routing()));
    let mut run = fork.model.build(0);
    run.set_recording(Recording::ALL);
    run.init().unwrap();
    run.advance_to(secs(1010)).unwrap();
    // 100 MUs have left the fork, so its count is part way between two sent left: a copy that
    // started counting afresh would send the next MUs elsewhere.
    let mut checkpoint = run.clone();
    let ahead = run.advance_to(secs(5000)).unwrap();
    let again = checkpoint.advance_to(secs(5000)).unwrap();
    assert_eq!(ahead, again);
    // Reset clears the count, so a run after Reset is a fresh run.
    let fresh = {
        let mut fresh = fork.model.build(0);
        fresh.set_recording(Recording::ALL);
        let mut occurrences = fresh.init().unwrap();
        occurrences.extend(fresh.advance_to(secs(5000)).unwrap());
        occurrences
    };
    run.reset();
    let mut after_reset = run.init().unwrap();
    after_reset.extend(run.advance_to(secs(5000)).unwrap());
    assert_eq!(after_reset, fresh);
}

#[test]
fn shortest_queue_reads_the_successors_contents() {
    let mut fork = fork_model();
    give_code(&mut fork.model, library_with("routing", common::routing()), &[("routing.shortest_queue", json!({}))], Options::default());
    fork.model.set_object_hooks(fork.fork, ObjectHooks { distribution: Some(0), ..ObjectHooks::default() });
    let mut run = fork.model.build(0);
    run.set_process_time(fork.left, secs(100)).unwrap();
    run.set_process_time(fork.right, secs(8)).unwrap();
    run.init().unwrap();
    run.advance_to(secs(3600)).unwrap();
    // Left is mostly busy, so the empty Right gets most, and Left one whenever both are empty,
    // as the first of them.
    assert!(mus_in(&run, fork.right) > 5 * mus_in(&run, fork.left), "{} right, {} left", mus_in(&run, fork.right), mus_in(&run, fork.left));
    assert!(mus_in(&run, fork.left) > 0);
}

#[test]
fn random_numbers_from_code_follow_the_seed() {
    let run_with = |seed: u64| {
        let mut fork = fork_model();
        give_code(&mut fork.model, library_with("routing", common::routing()), &[("routing.random_exit", json!({}))], Options::default());
        fork.model.set_object_hooks(fork.fork, ObjectHooks { distribution: Some(0), ..ObjectHooks::default() });
        let mut run = fork.model.build(seed);
        run.set_recording(Recording::ALL);
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(3600)).unwrap());
        moves_to(&occurrences, fork.fork)
    };
    assert_eq!(run_with(3), run_with(3));
    assert_ne!(run_with(3), run_with(4));
}

#[test]
fn a_timer_logs_every_hour_with_the_time_and_function() {
    let lines: Arc<Mutex<Vec<LogLine>>> = Arc::default();
    let sink = lines.clone();
    let options = Options { log: Some(Arc::new(move |line: &LogLine| sink.lock().unwrap().push(line.clone()))), ..Options::default() };
    let mut fork = fork_model();
    give_code(&mut fork.model, library_with("routing", common::routing()), &[("routing.report", json!({"drain": "Drain", "every": "1:00:00"}))], options);
    fork.model.set_lifecycle_hook(Lifecycle::AfterInit, Some(0));
    let mut run = fork.model.build(0);
    run.init().unwrap();
    run.advance_to(secs(3 * 3600)).unwrap();
    let lines = lines.lock().unwrap();
    let times: Vec<SimTime> = lines.iter().map(|line| line.time).collect();
    assert_eq!(times, [secs(0), secs(3600), secs(7200), secs(10800)]);
    assert_eq!(lines[0].function, "routing.report");
    assert_eq!(lines[0].message, "Drain has taken 0 MUs");
    assert_eq!(lines[3].message, format!("Drain has taken {} MUs", mus_in(&run, fork.drain)));
}

fn fails_with(function: &str, with: serde_json::Value, options: Options) -> String {
    let mut fork = fork_model();
    give_code(&mut fork.model, library_with("faults", common::faults()), &[(function, with)], options);
    fork.model.set_object_hooks(fork.fork, ObjectHooks { on_entry: Some(Hook::new(0)), ..ObjectHooks::default() });
    let mut run = fork.model.build(0);
    run.init().unwrap();
    match run.advance_to(secs(100)).unwrap_err().error {
        error @ RunError::Hook(_) => error.to_string(),
        other => panic!("not a hook's failure: {other:?}"),
    }
}

#[test]
fn a_call_that_never_ends_runs_out_of_fuel() {
    let message = fails_with("faults.spin", json!({}), Options { fuel_per_call: 1_000_000, ..Options::default() });
    assert!(message.contains("faults.spin: ran out of fuel: a call may run 1000000 instructions"), "{message}");
    assert!(message.contains("object 1"), "names the object: {message}");
}

#[test]
fn code_that_takes_too_much_memory_is_stopped() {
    let message = fails_with("faults.hog", json!({"megabytes": 8}), Options { memory_limit: 16 << 20, ..Options::default() });
    assert!(message.contains("faults.hog: "), "{message}");
    assert!(message.contains("memory"), "{message}");
}

#[test]
fn a_panic_ends_the_run_with_its_message() {
    let message = fails_with("faults.boom", json!({"message": "no partner for this MU"}), Options::default());
    assert!(message.contains("faults.boom: panicked: no partner for this MU"), "{message}");
    assert!(message.contains("for MU 1"), "names the MU: {message}");
}

#[test]
fn an_answer_that_is_no_successor_ends_the_run() {
    let mut fork = fork_model();
    give_code(&mut fork.model, library_with("faults", common::faults()), &[("faults.nonsense", json!({}))], Options::default());
    fork.model.set_object_hooks(fork.fork, ObjectHooks { distribution: Some(0), ..ObjectHooks::default() });
    let mut run = fork.model.build(0);
    run.init().unwrap();
    let error = run.advance_to(secs(100)).unwrap_err().error;
    assert!(error.to_string().contains("9999"), "{error}");
}

#[test]
fn an_action_refused_tells_the_code_why() {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = lines.clone();
    let options = Options { log: Some(Arc::new(move |line: &LogLine| sink.lock().unwrap().push(line.message.clone()))), ..Options::default() };
    let mut fork = fork_model();
    // Every MU entering Left is sent to the Drain, which takes it, and from the Drain the MU is
    // gone.
    give_code(&mut fork.model, library_with("faults", common::faults()), &[("faults.send", json!({"to": "Drain"}))], options);
    fork.model.set_object_hooks(fork.left, ObjectHooks { on_entry: Some(Hook::new(0)), ..ObjectHooks::default() });
    fork.model.set_object_hooks(fork.drain, ObjectHooks { on_entry: Some(Hook::new(0)), ..ObjectHooks::default() });
    let mut run = fork.model.build(0);
    run.init().unwrap();
    run.advance_to(secs(200)).unwrap();
    let lines = lines.lock().unwrap();
    assert!(lines.iter().any(|line| line.starts_with("moved MU ") && line.ends_with(" to Drain")), "{lines:?}");
    assert!(lines.iter().any(|line| line.starts_with("refused: ")), "the Drain's own hook can't move the MU: {lines:?}");
    let left = &run.state().objects[fork.left as usize];
    assert!(left.contents.is_empty() && left.statistics.mus_in > 0, "every MU that came into Left was moved on at once");
    assert_eq!(run.state().objects[fork.drain as usize].statistics.mus_in, left.statistics.mus_in + run.state().objects[fork.right as usize].statistics.mus_out);
}

#[test]
fn the_hook_before_reset_may_not_act_and_state_goes_at_reset() {
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = lines.clone();
    let options = Options { log: Some(Arc::new(move |line: &LogLine| sink.lock().unwrap().push(line.message.clone()))), ..Options::default() };
    let mut fork = fork_model();
    give_code(&mut fork.model, library_with("faults", common::faults()), &[("faults.count", json!({}))], options);
    for moment in Lifecycle::ALL {
        fork.model.set_lifecycle_hook(moment, Some(0));
    }
    let mut run = fork.model.build(0);
    run.init().unwrap();
    run.reset();
    run.init().unwrap();
    // Before and after Init, then before Reset with the count still there, then after Reset
    // counting afresh.
    assert_eq!(*lines.lock().unwrap(), ["call 1", "call 2", "call 3", "call 1", "call 2", "call 3"]);
}

// Modules that don't fit, written by hand.

fn refused(wat: &str) -> String {
    let bytes = wat::parse_str(wat).unwrap();
    Library::new().load_wasm("bad", &bytes).unwrap_err()
}

const DECLARES_F: &str = r#"(@custom "crabsim-functions" "1|f()\n")"#;

#[test]
fn a_module_of_another_interface_version_is_refused() {
    let message = refused(r#"(module (@custom "crabsim-functions" "2|f()\n") (memory (export "memory") 1) (func (export "crabsim:f") (param i32 i32 i32 i32) (result i32) i32.const -1))"#);
    assert!(message.contains("interface version 2, and this crabsim offers version 1"), "{message}");
}

#[test]
fn a_module_importing_anything_but_crabsim_is_refused() {
    let message = refused(&format!(r#"(module {DECLARES_F} (import "wasi_snapshot_preview1" "clock_time_get" (func (param i32 i64 i32) (result i32))) (memory (export "memory") 1) (func (export "crabsim:f") (param i32 i32 i32 i32) (result i32) i32.const -1))"#));
    assert!(message.contains("imports `wasi_snapshot_preview1::clock_time_get`, which crabsim doesn't offer"), "{message}");
}

#[test]
fn a_module_without_crabsims_functions_is_refused() {
    let message = refused(r#"(module (memory (export "memory") 1))"#);
    assert!(message.contains("isn't a crabsim code module"), "{message}");
    let message = refused(&format!(r#"(module {DECLARES_F} (memory (export "memory") 1))"#));
    assert!(message.contains("exports no `crabsim:f`"), "{message}");
}

#[test]
fn a_module_keeping_state_in_hidden_globals_is_refused() {
    let message = refused(&format!(
        r#"(module {DECLARES_F} (global (mut i32) (i32.const 0)) (global (mut i32) (i32.const 0)) (memory (export "memory") 1) (func (export "crabsim:f") (param i32 i32 i32 i32) (result i32) i32.const -1))"#
    ));
    assert!(message.contains("globals it doesn't export"), "{message}");
}

#[test]
fn parameters_are_checked_against_the_function() {
    let library = library_with("routing", common::routing());
    let model = fork_model().model;
    let problems = library.bind("routing.every_nth", json!({"n": "three", "left": "Lfet", "colour": 1}).as_object().unwrap(), &names(&model)).unwrap_err();
    let messages: Vec<&str> = problems.iter().map(|problem| problem.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "`routing.every_nth` has no parameter `colour`; its parameters are every_nth(n: int, left: object, right: object)",
            "parameter `n` of `routing.every_nth`: \"three\" isn't a whole number",
            "parameter `left` of `routing.every_nth`: there is no instance `Lfet`",
            "`routing.every_nth` needs its parameter `right`, of type object",
        ]
    );
    assert!(library.bind("routing.every_second", &serde_json::Map::new(), &names(&model)).unwrap_err()[0].message.contains("has no function `every_second`; it has cycle, every_nth, pass, random_exit, report, shortest_queue"));
    assert!(library.bind("routes.every_nth", &serde_json::Map::new(), &names(&model)).unwrap_err()[0].message.contains("there is no code module `routes`"));
}

