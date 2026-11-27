//! User code's hooks (ADR 0029, spec issue 156), tested at the run's seam with hooks written in
//! Rust: Init, advance, Reset, then read occurrences, state and what the code was called with.

use std::sync::{Arc, Mutex};

use crabsim::random_numbers::Distribution;
use crabsim::{
    CallKind, Capacity, Decision, FailureProfile, Hook, HookCall, HookError, HookRun, Hooks, Interruption, Lifecycle, ModelDescription, ObjectHooks, Occurrence,
    OccurrenceKind, ProcessState, Recording, RunError, SimTime, SimulationRun, TriggerPoint, Triggers, ZERO_TIME_EVENT_LIMIT,
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0))
}

/// What a script keeps between calls: the code's own memory, cleared at Reset.
#[derive(Clone, Default)]
struct Memory {
    counter: u64,
}

type Script = dyn Fn(&HookCall, &mut HookRun<'_>, &mut Memory) -> Result<Option<Decision>, HookError> + Send + Sync;

/// User code written in Rust for a test. Every call, and every clearing of its memory, goes into
/// a log the test reads, shared by every copy.
#[derive(Clone)]
struct Code {
    script: Arc<Script>,
    memory: Memory,
    log: Arc<Mutex<Vec<Logged>>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Logged {
    Call(SimTime, HookCall),
    Cleared,
}

impl Code {
    fn new(script: impl Fn(&HookCall, &mut HookRun<'_>, &mut Memory) -> Result<Option<Decision>, HookError> + Send + Sync + 'static) -> Code {
        Code { script: Arc::new(script), memory: Memory::default(), log: Arc::default() }
    }

    fn log(&self) -> Arc<Mutex<Vec<Logged>>> {
        self.log.clone()
    }
}

impl Hooks for Code {
    fn call(&mut self, call: &HookCall, run: &mut HookRun<'_>) -> Result<Option<Decision>, HookError> {
        self.log.lock().unwrap().push(Logged::Call(run.time(), call.clone()));
        (self.script)(call, run, &mut self.memory)
    }
    fn clear(&mut self) {
        self.memory = Memory::default();
        self.log.lock().unwrap().push(Logged::Cleared);
    }
    fn clone_hooks(&self) -> Box<dyn Hooks> {
        Box::new(self.clone())
    }
}

fn recorded(mut run: SimulationRun) -> SimulationRun {
    run.set_recording(Recording::ALL);
    run
}

fn left_at(occurrences: &[Occurrence], drain: u32) -> usize {
    occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::MuLeft { at, .. } if at == drain)).count()
}

fn moved_into(occurrences: &[Occurrence], object: u32) -> usize {
    occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { to, .. } if to == object)).count()
}

const DISTRIBUTE: u32 = 1;
const ON_ENTRY: u32 = 2;
const ON_EXIT: u32 = 3;
const TIMER: u32 = 4;
const LIFECYCLE: u32 = 5;

/// A Source every 10 s, a Station of 1 s, and two Drains after it.
struct Fork {
    model: ModelDescription,
    source: u32,
    fork: u32,
    a: u32,
    b: u32,
}

fn fork() -> Fork {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let fork = station(&mut model, 1);
    let a = model.add_drain();
    let b = model.add_drain();
    model.connect(source, fork);
    model.connect(fork, a);
    model.connect(fork, b);
    Fork { model, source, fork, a, b }
}

fn distribution(function: u32) -> ObjectHooks {
    ObjectHooks { distribution: Some(function), ..ObjectHooks::default() }
}

#[test]
fn a_distribution_hook_sends_every_mu_where_it_chooses() {
    let Fork { mut model, fork, a, b, .. } = fork();
    model.set_hooks(Box::new(Code::new(move |_, _, _| Ok(Some(Decision::Successor(b))))));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(100)).unwrap();
    assert_eq!(left_at(&occurrences, a), 0);
    assert_eq!(left_at(&occurrences, b), 9, "every MU, leaving its Source at 10 s, 20 s and so on, and the Fork 1 s later");
}

#[test]
fn the_mu_waits_for_the_chosen_successor_alone() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let fork = station(&mut model, 0);
    let slow = station(&mut model, 10);
    let fast = station(&mut model, 1);
    let drain = model.add_drain();
    for (from, to) in [(source, fork), (fork, slow), (fork, fast), (slow, drain), (fast, drain)] {
        model.connect(from, to);
    }
    model.set_hooks(Box::new(Code::new(move |_, _, _| Ok(Some(Decision::Successor(slow))))));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(35)).unwrap();
    assert_eq!(moved_into(&occurrences, fast), 0, "the free Station is never offered the MU");
    assert_eq!(moved_into(&occurrences, slow), 4, "at 1 s, 11 s, 21 s and 31 s");
    assert_eq!(run.state().objects[fork as usize].process, ProcessState::Blocked);
}

#[test]
fn hold_keeps_the_mu_until_code_moves_it() {
    let Fork { mut model, fork, a, .. } = fork();
    let code = Code::new(move |call, run, _| match call.kind {
        CallKind::Distribution { .. } => Ok(Some(Decision::Hold)),
        CallKind::Lifecycle(Lifecycle::AfterInit) => {
            run.call_at(secs(25), TIMER, 0).unwrap();
            Ok(None)
        }
        CallKind::Timer { .. } => {
            let held = run.state(fork).unwrap().contents[0];
            run.move_mu(held, fork, a).unwrap();
            Ok(None)
        }
        _ => Ok(None),
    });
    model.set_hooks(Box::new(code));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(20)).unwrap();
    assert_eq!(left_at(&occurrences, a), 0);
    let state = run.state();
    assert_eq!(state.objects[fork as usize].process, ProcessState::Blocked, "a held MU shows as blocked");
    assert_eq!(state.objects[fork as usize].contents.len(), 1);
    let held = run.state().objects[fork as usize].contents.clone();
    let occurrences = run.advance_to(secs(25)).unwrap();
    assert_eq!(left_at(&occurrences, a), 1, "the timer moved it");
    let now_held = &run.state().objects[fork as usize].contents;
    assert!(now_held.len() == 1 && *now_held != held, "the Fork took the Source's waiting MU as the held one went: {held:?}, then {now_held:?}");
}

#[test]
fn default_hands_the_choice_to_the_objects_own_strategy_and_changes_nothing() {
    let plain = crabsim::demo::model();
    let mut coded = crabsim::demo::model();
    coded.set_hooks(Box::new(Code::new(|_, _, _| Ok(Some(Decision::Default)))));
    for id in 0..coded.object_count() {
        if !coded.successors(id).is_empty() {
            coded.set_object_hooks(id, distribution(DISTRIBUTE));
        }
    }
    let run_for_an_hour = |model: &ModelDescription| {
        let mut run = recorded(model.build(7));
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(3600)).unwrap());
        (occurrences, run.state())
    };
    let (plain_occurrences, plain_state) = run_for_an_hour(&plain);
    let (coded_occurrences, coded_state) = run_for_an_hour(&coded);
    assert_eq!(coded_occurrences, plain_occurrences);
    assert_eq!(coded_state, plain_state);
}

#[test]
fn a_moves_hooks_run_in_order_distribution_exit_then_entry() {
    let Fork { mut model, source, fork, a, .. } = fork();
    let code = Code::new(move |call, _, _| Ok(matches!(call.kind, CallKind::Distribution { .. }).then_some(Decision::Successor(a))));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(source, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    model.set_object_hooks(fork, ObjectHooks { on_exit: Some(Hook::new(ON_EXIT)), distribution: Some(DISTRIBUTE), ..ObjectHooks::default() });
    model.set_object_hooks(a, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(11)).unwrap();
    let calls: Vec<(f64, u32, String)> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|logged| match logged {
            Logged::Call(time, call) => Some((time.as_secs_f64(), call.function, match &call.kind {
                CallKind::Entry { object, at, .. } => format!("entry {object} {at:?}"),
                CallKind::Exit { object, to, .. } => format!("exit {object} to {to}"),
                CallKind::Distribution { object, .. } => format!("distribution {object}"),
                other => format!("{other:?}"),
            })),
            _ => None,
        })
        .collect();
    let expected = [
        (0.0, ON_ENTRY, format!("entry {source} BookingPoint")),
        (10.0, ON_ENTRY, format!("entry {source} BookingPoint")),
        (11.0, DISTRIBUTE, format!("distribution {fork}")),
        (11.0, ON_EXIT, format!("exit {fork} to {a}")),
        (11.0, ON_ENTRY, format!("entry {a} BookingPoint")),
    ];
    assert_eq!(calls, expected);
}

#[test]
fn a_hook_runs_once_for_each_trigger_point_set() {
    let Fork { mut model, fork, .. } = fork();
    let code = Code::new(|_, _, _| Ok(None));
    let log = code.log();
    model.set_hooks(Box::new(code));
    let all = Triggers { front: true, booking_point: true, rear: true };
    model.set_object_hooks(fork, ObjectHooks { on_entry: Some(Hook { function: ON_ENTRY, triggers: all }), ..ObjectHooks::default() });
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(10)).unwrap();
    let points: Vec<TriggerPoint> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|logged| match logged {
            Logged::Call(_, HookCall { kind: CallKind::Entry { at, .. }, .. }) => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(points, [TriggerPoint::Front, TriggerPoint::BookingPoint, TriggerPoint::Rear]);
}

#[test]
fn lifecycle_hooks_run_around_init_and_reset_and_reset_clears_the_codes_memory() {
    let Fork { mut model, source, fork, .. } = fork();
    let code = Code::new(move |call, run, memory| {
        memory.counter += 1;
        if call.kind == CallKind::Lifecycle(Lifecycle::BeforeReset) {
            let refused = run.set_process_time(fork, secs(5)).unwrap_err();
            return Err(HookError(format!("refused: {}; counted {}", refused.0, memory.counter)));
        }
        Ok(None)
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    for moment in Lifecycle::ALL {
        model.set_lifecycle_hook(moment, Some(LIFECYCLE));
    }
    model.set_object_hooks(source, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(5)).unwrap();
    run.reset();
    let seen: Vec<String> = log
        .lock()
        .unwrap()
        .iter()
        .map(|logged| match logged {
            Logged::Call(_, call) => format!("{:?}", call.kind).split([' ', '(']).next().unwrap().to_string()
                + &match call.kind {
                    CallKind::Lifecycle(moment) => format!(" {moment:?}"),
                    _ => String::new(),
                },
            Logged::Cleared => "Cleared".to_string(),
        })
        .collect();
    assert_eq!(seen, ["Lifecycle BeforeInit", "Entry", "Lifecycle AfterInit", "Lifecycle BeforeReset", "Cleared", "Lifecycle AfterReset"]);
    // The hook before Reset was refused its action, failed on purpose, and the next Init says so,
    // with what its memory held before Reset cleared it.
    match run.init() {
        Err(RunError::Hook(failure)) => assert_eq!(failure.message, "refused: the hook before Reset may only read and log; counted 4"),
        other => panic!("expected the hook's failure, got {other:?}"),
    }
    assert!(run.init().is_ok(), "reported once");
}

#[test]
fn code_moves_the_fourth_mu_out_of_a_buffer() {
    let mut model = ModelDescription::new();
    let buffer = model.add_buffer(Capacity::Unlimited);
    let busy = station(&mut model, 1000);
    let drain = model.add_drain();
    model.connect(buffer, busy);
    model.connect(busy, drain);
    model.set_starting_mus(buffer, &[1, 2, 3, 4, 5, 6]);
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        if call.kind == CallKind::Lifecycle(Lifecycle::AfterInit) {
            let fourth = run.state(buffer).unwrap().contents[3];
            run.move_mu(fourth, buffer, drain).unwrap();
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    let occurrences = run.init().unwrap();
    assert!(occurrences.contains(&Occurrence { time: SimTime::ZERO, kind: OccurrenceKind::MuMoved { mu_id: 4, from: buffer, to: drain } }));
    assert!(occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::MuLeft { mu_id: 4, at, .. } if at == drain)));
    run.advance_to(SimTime::ZERO).unwrap();
    assert_eq!(run.state().objects[buffer as usize].contents, [2, 3, 5, 6], "MU 1 went on into the Station");
}

#[test]
fn moving_an_mu_in_work_stops_its_work_and_frees_the_station() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let work = station(&mut model, 10);
    let aside = model.add_drain();
    let drain = model.add_drain();
    model.connect(source, work);
    model.connect(work, drain);
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        match call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) => run.call_at(secs(4), TIMER, 0).unwrap(),
            CallKind::Timer { .. } => {
                let in_work = run.state(work).unwrap().contents[0];
                run.move_mu(in_work, work, aside).unwrap();
            }
            _ => {}
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(12)).unwrap();
    let mu_moved_aside = occurrences.iter().find_map(|o| match o.kind {
        OccurrenceKind::MuMoved { mu_id, to, .. } if to == aside => Some(mu_id),
        _ => None,
    });
    let mu = mu_moved_aside.expect("moved aside at 4 s");
    assert!(!occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::WorkFinished { mu_id, .. } if mu_id == mu)), "its work stopped");
    // The Source's MU, waiting since 2 s, went in at once, and finishes at 14 s.
    let next_in = occurrences.iter().find(|o| o.time == secs(4) && matches!(o.kind, OccurrenceKind::MuMoved { from, to, .. } if from == source && to == work));
    assert!(next_in.is_some(), "the Station took the next MU when the first went");
    assert_eq!(left_at(&occurrences, drain), 0, "nothing finished by 12 s: the stale exit at 11 s did nothing");
    assert_eq!(run.state().objects[work as usize].remaining, Some(secs(2)));
}

#[test]
fn a_refused_move_leaves_the_mu_where_it_was_and_tells_the_code_why() {
    let Fork { mut model, source, fork, .. } = fork();
    let code = Code::new(move |call, run, _| {
        if let CallKind::Entry { mu, .. } = &call.kind {
            let refusal = run.move_mu(mu.id, source, fork).unwrap_err();
            return Err(HookError(refusal.0));
        }
        Ok(None)
    });
    model.set_hooks(Box::new(code));
    model.set_object_hooks(source, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    model.set_object_hooks(fork, ObjectHooks::default());
    // The Fork holds an MU from the start, so it takes no other.
    model.set_starting_mus(fork, &[100]);
    let mut run = model.build(0);
    match run.init() {
        Err(RunError::Hook(failure)) => assert_eq!(failure.message, format!("object {fork} doesn't take an MU now")),
        other => panic!("expected the refusal, got {other:?}"),
    }
    assert_eq!(run.state().objects[source as usize].contents.len(), 1, "the MU stayed in the Source");
}

#[test]
fn a_hook_on_entry_gives_the_mu_its_own_process_time() {
    let Fork { mut model, fork, .. } = fork();
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        if let CallKind::Entry { object, mu, .. } = &call.kind {
            run.set_work_time(*object, mu.id, SimTime::from_millis(250)).unwrap();
        }
        Ok(None)
    })));
    model.set_object_hooks(fork, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(21)).unwrap();
    let finished: Vec<SimTime> = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::WorkFinished { .. })).map(|o| o.time).collect();
    assert_eq!(finished, [SimTime::from_millis(10_250), SimTime::from_millis(20_250)], "MUs come in at 10 s and 20 s");
}

#[test]
fn a_timer_calls_the_code_again_later() {
    let Fork { mut model, .. } = fork();
    let code = Code::new(|call, run, memory| {
        match call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) | CallKind::Timer { .. } => {
                memory.counter += 1;
                run.call_at(secs(3600), TIMER, memory.counter as u32).unwrap();
            }
            _ => {}
        }
        Ok(None)
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(5 * 3600)).unwrap();
    let timers: Vec<(u64, u32)> = log
        .lock()
        .unwrap()
        .iter()
        .filter_map(|logged| match logged {
            Logged::Call(time, HookCall { function: TIMER, kind: CallKind::Timer { data } }) => Some((time.as_secs(), *data)),
            _ => None,
        })
        .collect();
    assert_eq!(timers, [(3600, 1), (7200, 2), (10800, 3), (14400, 4), (18000, 5)]);
}

#[test]
fn code_pauses_an_object() {
    let Fork { mut model, fork, .. } = fork();
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        if call.kind == CallKind::Lifecycle(Lifecycle::AfterInit) {
            run.pause(fork, true).unwrap();
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    let occurrences = run.init().unwrap();
    assert!(occurrences.contains(&Occurrence { time: SimTime::ZERO, kind: OccurrenceKind::InterruptionBegan { object_id: fork, cause: Interruption::Paused, profile: None } }));
    run.advance_to(secs(30)).unwrap();
    assert_eq!(run.state().objects[fork as usize].interrupted, Some(Interruption::Paused));
}

#[test]
fn hooks_moving_an_mu_back_and_forth_end_in_a_zero_time_loop() {
    let mut model = ModelDescription::new();
    let here = model.add_buffer(Capacity::Unlimited);
    let there = model.add_buffer(Capacity::Unlimited);
    model.set_starting_mus(here, &[1]);
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        match &call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) => run.move_mu(1, here, there).unwrap(),
            CallKind::Entry { object, mu, .. } => {
                let other = if *object == here { there } else { here };
                run.move_mu(mu.id, *object, other).unwrap();
            }
            _ => {}
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    for buffer in [here, there] {
        model.set_object_hooks(buffer, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    }
    let mut run = model.build(0);
    assert_eq!(run.init(), Err(RunError::ZeroTimeLoop { time: SimTime::ZERO }), "after {ZERO_TIME_EVENT_LIMIT} calls");
}

#[test]
fn a_failing_hook_ends_the_run_with_its_own_message() {
    let Fork { mut model, fork, .. } = fork();
    model.set_hooks(Box::new(Code::new(|_, _, _| Err(HookError("index out of bounds".to_string())))));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = model.build(0);
    run.init().unwrap();
    let error = run.advance_to(secs(20)).unwrap_err().error;
    let text = error.to_string();
    match error {
        RunError::Hook(failure) => {
            assert_eq!((failure.call.function, failure.time, failure.message.as_str()), (DISTRIBUTE, secs(11), "index out of bounds"));
            assert!(text.contains(&format!("the distribution hook of object {fork}, for MU 1")), "{text}");
        }
        other => panic!("expected the hook's failure, got {other:?}"),
    }
}

#[test]
fn choosing_an_object_that_isnt_a_successor_is_a_run_error() {
    let Fork { mut model, source, fork, .. } = fork();
    model.set_hooks(Box::new(Code::new(move |_, _, _| Ok(Some(Decision::Successor(source))))));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = model.build(0);
    run.init().unwrap();
    match run.advance_to(secs(20)).unwrap_err().error {
        RunError::Hook(failure) => assert_eq!(failure.message, format!("it chose object {source}, which isn't a successor of object {fork}")),
        other => panic!("expected the hook's failure, got {other:?}"),
    }
}

#[test]
fn a_checkpoint_continues_exactly_as_the_original_with_the_codes_memory() {
    let Fork { mut model, fork, a, b, .. } = fork();
    // Every third MU to B, counted in the code's own memory.
    model.set_hooks(Box::new(Code::new(move |_, _, memory| {
        memory.counter += 1;
        Ok(Some(Decision::Successor(if memory.counter % 3 == 0 { b } else { a })))
    })));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(55)).unwrap();
    let mut checkpoint = run.clone();
    let original = run.advance_to(secs(300)).unwrap();
    let from_checkpoint = checkpoint.advance_to(secs(300)).unwrap();
    assert_eq!(from_checkpoint, original);
    assert_eq!(left_at(&original, b), 8, "MUs 6, 9, ... 30 of the 25 leaving from 61 s to 291 s");
}

#[test]
fn going_back_by_reset_and_replaying_gives_the_same_run() {
    let Fork { mut model, fork, a, b, .. } = fork();
    model.set_hooks(Box::new(Code::new(move |_, _, memory| {
        memory.counter += 1;
        Ok(Some(Decision::Successor(if memory.counter % 2 == 0 { b } else { a })))
    })));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    let mut first = run.init().unwrap();
    first.extend(run.advance_to(secs(200)).unwrap());
    run.reset();
    let mut second = run.init().unwrap();
    second.extend(run.advance_to(secs(200)).unwrap());
    assert_eq!(second, first);
}

#[test]
fn an_event_in_the_queue_stays_32_bytes() {
    assert_eq!(size_of::<crabsim::event::ScheduledEvent>(), 32);
}

// Conveyors and cross conveyors (spec issue 156, PR 1b): the trigger points spread over time where
// geometry counts, the rear event, a distribution at a conveyor's end and as a cross conveyor lets
// an MU in, a distribution's moment moved, and moving MUs onto and off belts.

fn every_point(function: u32) -> Option<Hook> {
    Some(Hook { function, triggers: Triggers { front: true, booking_point: true, rear: true } })
}

fn both(function_in: u32, function_out: u32) -> ObjectHooks {
    ObjectHooks { on_entry: every_point(function_in), on_exit: every_point(function_out), ..ObjectHooks::default() }
}

/// The calls logged, as (time in seconds, what, object, trigger point), leaving out lifecycle and
/// timer calls.
fn calls_of(log: &Mutex<Vec<Logged>>) -> Vec<(f64, &'static str, u32, Option<TriggerPoint>)> {
    log.lock()
        .unwrap()
        .iter()
        .filter_map(|logged| match logged {
            Logged::Call(time, call) => match &call.kind {
                CallKind::Entry { object, at, .. } => Some((time.as_secs_f64(), "entry", *object, Some(*at))),
                CallKind::Exit { object, at, .. } => Some((time.as_secs_f64(), "exit", *object, Some(*at))),
                CallKind::Distribution { object, .. } => Some((time.as_secs_f64(), "distribution", *object, None)),
                _ => None,
            },
            Logged::Cleared => None,
        })
        .collect()
}

/// A Source every 100 s, of MUs 1 m long with their booking point in the middle, onto two
/// conveyors of 5 m at 1 m/s, one after the other, then a Drain.
struct Belts {
    model: ModelDescription,
    source: u32,
    first: u32,
    second: u32,
    drain: u32,
}

fn belts() -> Belts {
    let mut model = ModelDescription::new();
    let metre = model.add_mu_kind(crabsim::MuKind { length: 1.0, width: 0.5, booking_point: 0.5 });
    let source = model.add_source(secs(100));
    model.set_mu_kind(source, metre);
    let first = model.add_conveyor(5.0, 0.6, 1.0);
    let second = model.add_conveyor(5.0, 0.6, 1.0);
    let drain = model.add_drain();
    for (from, to) in [(source, first), (first, second), (second, drain)] {
        model.connect(from, to);
    }
    Belts { model, source, first, second, drain }
}

#[test]
fn on_a_conveyor_front_booking_point_and_rear_each_run_when_they_cross() {
    use TriggerPoint::{BookingPoint, Front, Rear};
    let Belts { mut model, source, first, second, drain } = belts();
    let code = Code::new(|_, _, _| Ok(None));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(source, ObjectHooks { on_exit: every_point(ON_EXIT), ..ObjectHooks::default() });
    model.set_object_hooks(first, both(ON_ENTRY, ON_EXIT));
    model.set_object_hooks(second, both(ON_ENTRY, ON_EXIT));
    model.set_object_hooks(drain, ObjectHooks { on_entry: every_point(ON_ENTRY), ..ObjectHooks::default() });
    let mut run = model.build(0);
    run.init().unwrap();
    // The Source's first MU, made at 0 s, is ready to leave at 100 s.
    run.advance_to(secs(130)).unwrap();
    let calls = calls_of(&log);
    let expected = [
        // Onto the first conveyor from the Source: its booking point at the start, its front half
        // a metre ahead, its rear half a metre behind, crossing half a second later.
        (100.0, "exit", source, Some(Front)),
        (100.0, "exit", source, Some(BookingPoint)),
        (100.0, "entry", first, Some(Front)),
        (100.0, "entry", first, Some(BookingPoint)),
        (100.5, "exit", source, Some(Rear)),
        (100.5, "entry", first, Some(Rear)),
        // Its front reaches the end of the first at 104.5 s, where the second takes it, its booking
        // point crosses at 105 s and its rear at 105.5 s.
        (104.5, "exit", first, Some(Front)),
        (104.5, "entry", second, Some(Front)),
        (105.0, "exit", first, Some(BookingPoint)),
        (105.0, "entry", second, Some(BookingPoint)),
        (105.5, "exit", first, Some(Rear)),
        (105.5, "entry", second, Some(Rear)),
        // The same into the Drain, an object without geometry, from 109.5 s.
        (109.5, "exit", second, Some(Front)),
        (109.5, "entry", drain, Some(Front)),
        (110.0, "exit", second, Some(BookingPoint)),
        (110.0, "entry", drain, Some(BookingPoint)),
        (110.5, "exit", second, Some(Rear)),
        (110.5, "entry", drain, Some(Rear)),
    ];
    assert_eq!(calls, expected);
}

#[test]
fn hooks_at_the_booking_point_alone_ask_for_no_rear_event_and_change_nothing() {
    let Belts { mut model, first, second, .. } = belts();
    let plain = model.build(0);
    model.set_hooks(Box::new(Code::new(|_, _, _| Ok(None))));
    model.set_object_hooks(first, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), on_exit: Some(Hook::new(ON_EXIT)), ..ObjectHooks::default() });
    model.set_object_hooks(second, ObjectHooks { on_entry: Some(Hook::new(ON_ENTRY)), ..ObjectHooks::default() });
    let hooked = model.build(0);
    let run_for_a_day = |mut run: SimulationRun| {
        run.set_recording(Recording::ALL);
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(86_400)).unwrap());
        (occurrences, run.state())
    };
    assert_eq!(run_for_a_day(hooked), run_for_a_day(plain));
}

#[test]
fn default_on_conveyors_and_cross_conveyors_changes_nothing() {
    for (name, plain) in [("conveyor line", crabsim::demo::conveyor_line()), ("cross line", crabsim::demo::cross_line())] {
        let mut coded = plain.clone();
        coded.set_hooks(Box::new(Code::new(|_, _, _| Ok(Some(Decision::Default)))));
        for id in 0..coded.object_count() {
            if !coded.successors(id).is_empty() {
                coded.set_object_hooks(id, distribution(DISTRIBUTE));
            }
        }
        let run_for_a_day = |model: &ModelDescription| {
            let mut run = recorded(model.build(7));
            let mut occurrences = run.init().unwrap();
            occurrences.extend(run.advance_to(secs(86_400)).unwrap());
            (occurrences, run.state())
        };
        let (plain_occurrences, plain_state) = run_for_a_day(&plain);
        let (coded_occurrences, coded_state) = run_for_a_day(&coded);
        assert_eq!(coded_occurrences.len(), plain_occurrences.len(), "{name}");
        assert!(coded_occurrences == plain_occurrences, "{name}: the occurrences differ");
        assert_eq!(coded_state, plain_state, "{name}");
    }
}

/// A Source every 10 s onto a conveyor of 5 m at 1 m/s, which forks to two Drains.
fn conveyor_fork() -> Fork {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let fork = model.add_conveyor(5.0, 0.6, 1.0);
    let a = model.add_drain();
    let b = model.add_drain();
    model.connect(source, fork);
    model.connect(fork, a);
    model.connect(fork, b);
    Fork { model, source, fork, a, b }
}

#[test]
fn a_conveyors_distribution_runs_as_its_front_mu_reaches_the_end() {
    let Fork { mut model, fork, a, b, .. } = conveyor_fork();
    let code = Code::new(move |_, _, _| Ok(Some(Decision::Successor(b))));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(100)).unwrap();
    assert_eq!(left_at(&occurrences, a), 0);
    assert_eq!(left_at(&occurrences, b), 9);
    // The built-in MU is 0.6 m long with its booking point in the middle: it enters at 10 s with
    // its front 0.3 m on, which reaches the end 4.7 s later.
    let asked: Vec<f64> = calls_of(&log).iter().filter(|call| call.1 == "distribution").map(|call| call.0).collect();
    assert_eq!(asked[..3], [14.7, 24.7, 34.7]);
    // Asked and answered at one instant, the MU never stood.
    assert!(!occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::Blocked { object_id } if object_id == fork)));
}

#[test]
fn hold_keeps_a_conveyors_mu_at_its_end_until_code_moves_it_off() {
    let Fork { mut model, fork, a, b, .. } = conveyor_fork();
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        match &call.kind {
            CallKind::Distribution { .. } => return Ok(Some(Decision::Hold)),
            CallKind::Lifecycle(Lifecycle::AfterInit) => run.call_at(secs(30), TIMER, 0).unwrap(),
            CallKind::Timer { .. } => {
                let held = run.state(fork).unwrap().contents[0];
                run.move_mu(held, fork, b).unwrap();
            }
            _ => {}
        }
        Ok(None)
    })));
    model.set_object_hooks(fork, distribution(DISTRIBUTE));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(29)).unwrap();
    assert_eq!(left_at(&occurrences, a) + left_at(&occurrences, b), 0);
    let state = run.state();
    let held = &state.objects[fork as usize];
    assert_eq!(held.contents.len(), 2, "the second MU closed up behind the held one");
    assert!((held.positions[0].front - 5.0).abs() < 1e-9 && held.positions[0].speed == 0.0, "the held MU stands at the end: {:?}", held.positions[0]);
    let occurrences = run.advance_to(secs(31)).unwrap();
    assert_eq!(left_at(&occurrences, b), 1, "code moved it off");
    let state = run.state();
    let after = &state.objects[fork as usize];
    assert_eq!(after.contents.len(), 2, "the next MU took its place at the end, and another came on at 30 s");
    assert_eq!(after.statistics.mus_out, 1);
}

#[test]
fn a_cross_conveyors_distribution_chooses_its_exit_as_it_lets_the_mu_in() {
    use crabsim::Side;
    let mut model = ModelDescription::new();
    let small = model.add_mu_kind(crabsim::MuKind { length: 0.4, width: 0.4, booking_point: 0.5 });
    let source = model.add_source(secs(20));
    model.set_mu_kind(source, small);
    let feed = model.add_conveyor(3.0, 0.6, 1.0);
    let cross = model.add_cross_conveyor(0.6, 0.6, 1.0);
    let straight = model.add_drain();
    let left = model.add_drain();
    model.connect(source, feed);
    model.connect_at(feed, None, cross, Some(Side::Back));
    model.connect_at(cross, Some(Side::Front), straight, None);
    model.connect_at(cross, Some(Side::Left), left, None);
    let code = Code::new(move |_, _, memory| {
        memory.counter += 1;
        Ok(Some(match memory.counter % 3 {
            0 => Decision::Default,
            1 => Decision::Successor(left),
            _ => Decision::Successor(straight),
        }))
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(cross, distribution(DISTRIBUTE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(20 * 30)).unwrap();
    let asked = calls_of(&log).iter().filter(|call| call.1 == "distribution").count();
    // Every MU is asked once, the left gets every third from the first, default sends the rest
    // straight on, where no exit lists them.
    assert_eq!(asked, 29);
    assert_eq!(left_at(&occurrences, left), 10);
    assert_eq!(left_at(&occurrences, straight), 19);
    assert_eq!(run.state().objects[cross as usize].cross.as_ref().unwrap().mus_out[Side::Left.index()], 10);
}

#[test]
fn a_distribution_moved_to_entry_is_asked_as_the_mu_comes_in_and_kept_until_it_leaves() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let buffer = model.add_buffer(Capacity::Unlimited);
    model.set_dwelling_time(buffer, secs(35));
    let a = model.add_drain();
    let b = model.add_drain();
    model.connect(source, buffer);
    model.connect(buffer, a);
    model.connect(buffer, b);
    let code = Code::new(move |call, _, _| match &call.kind {
        // Odd MUs to A, even ones to B, decided as each comes in.
        CallKind::Distribution { mu, .. } => Ok(Some(Decision::Successor(if mu.id % 2 == 1 { a } else { b }))),
        _ => Ok(None),
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(buffer, ObjectHooks { distribution: Some(DISTRIBUTE), distribution_on_entry: Some(TriggerPoint::BookingPoint), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(200)).unwrap();
    let asked: Vec<f64> = calls_of(&log).iter().filter(|call| call.1 == "distribution").map(|call| call.0).collect();
    assert_eq!(asked, [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0, 200.0], "once each, as it came in");
    let to_a: Vec<u32> = occurrences.iter().filter_map(|o| match o.kind { OccurrenceKind::MuLeft { mu_id, at, .. } if at == a => Some(mu_id), _ => None }).collect();
    let to_b: Vec<u32> = occurrences.iter().filter_map(|o| match o.kind { OccurrenceKind::MuLeft { mu_id, at, .. } if at == b => Some(mu_id), _ => None }).collect();
    assert_eq!(to_a, [1, 3, 5, 7, 9, 11, 13, 15]);
    assert_eq!(to_b, [2, 4, 6, 8, 10, 12, 14, 16]);
    assert_eq!(run.state().objects[buffer as usize].contents.len(), 4, "MUs 17 to 20, still dwelling");
}

#[test]
fn a_distribution_moved_to_a_sources_entry_is_asked_once_per_mu() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let a = model.add_drain();
    let b = model.add_drain();
    model.connect(source, a);
    model.connect(source, b);
    let code = Code::new(move |_, _, _| Ok(Some(Decision::Successor(b))));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(source, ObjectHooks { distribution: Some(DISTRIBUTE), distribution_on_entry: Some(TriggerPoint::BookingPoint), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(55)).unwrap();
    let asked: Vec<f64> = calls_of(&log).iter().filter(|call| call.1 == "distribution").map(|call| call.0).collect();
    // Made at 0 s, it waits for its interval, then leaves as answered at 10 s, when the next is made.
    assert_eq!(asked, [0.0, 10.0, 20.0, 30.0, 40.0, 50.0], "as each MU is made, never again as it leaves");
    assert_eq!(left_at(&occurrences, b), 5);
    assert_eq!(left_at(&occurrences, a), 0);
}

#[test]
fn code_moves_an_mu_onto_a_conveyor_it_isnt_connected_to() {
    let mut model = ModelDescription::new();
    let buffer = model.add_buffer(Capacity::Unlimited);
    let belt = model.add_conveyor(5.0, 0.6, 1.0);
    let drain = model.add_drain();
    model.connect(belt, drain);
    model.set_starting_mus(buffer, &[1]);
    let code = Code::new(move |call, run, _| {
        if call.kind == CallKind::Lifecycle(Lifecycle::AfterInit) {
            run.move_mu(1, buffer, belt).unwrap();
        }
        Ok(None)
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    model.set_object_hooks(buffer, ObjectHooks { on_exit: every_point(ON_EXIT), ..ObjectHooks::default() });
    model.set_object_hooks(belt, ObjectHooks { on_entry: every_point(ON_ENTRY), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(10)).unwrap());
    assert!(occurrences.contains(&Occurrence { time: SimTime::ZERO, kind: OccurrenceKind::MuMoved { mu_id: 1, from: buffer, to: belt } }));
    // The built-in MU, 0.6 m long, enters with its booking point at the start and its front 0.3 m
    // on: its front reaches the end at 4.7 s, its booking point 0.3 s later.
    let left = occurrences.iter().find(|o| matches!(o.kind, OccurrenceKind::MuLeft { mu_id: 1, at, .. } if at == drain)).expect("it travelled to the Drain");
    assert_eq!(left.time, SimTime::from_millis(5_000));
    use TriggerPoint::{BookingPoint, Front, Rear};
    // Taken out by code, it leaves the Buffer whole at once, and its rear crosses the conveyor's
    // start 0.3 s after it is put on.
    assert_eq!(calls_of(&log), [
        (0.0, "exit", buffer, Some(Front)),
        (0.0, "exit", buffer, Some(BookingPoint)),
        (0.0, "exit", buffer, Some(Rear)),
        (0.0, "entry", belt, Some(Front)),
        (0.0, "entry", belt, Some(BookingPoint)),
        (0.3, "entry", belt, Some(Rear)),
    ]);
}

#[test]
fn code_takes_an_mu_off_a_conveyor_and_those_behind_move_up() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(1));
    let belt = model.add_conveyor(10.0, 0.6, 1.0);
    let slow = station(&mut model, 1000);
    let aside = model.add_drain();
    model.connect(source, belt);
    model.connect(belt, slow);
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        match call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) => run.call_at(secs(60), TIMER, 0).unwrap(),
            CallKind::Timer { .. } => {
                // The MU second from the end, in an accumulated queue.
                let second = run.state(belt).unwrap().contents[1];
                run.move_mu(second, belt, aside).unwrap();
            }
            _ => {}
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(59)).unwrap();
    let before = run.state().objects[belt as usize].clone();
    assert_eq!(before.process, ProcessState::Blocked, "full and standing behind the busy Station");
    let occurrences = run.advance_to(secs(61)).unwrap();
    assert_eq!(left_at(&occurrences, aside), 1);
    let after = run.state().objects[belt as usize].clone();
    assert_eq!(after.contents.len(), before.contents.len() - 1 + 1, "one off, and the Source's waiting MU came on in the room");
    assert!(!after.contents.contains(&before.contents[1]));
    assert_eq!(after.statistics.mus_out, 1 + 1, "the one in the Station, and the one taken off");
    let behind = after.positions.iter().find(|position| position.mu_id == before.contents[2]).unwrap();
    assert!((behind.front - 9.4).abs() < 1e-6, "the MU behind closed up to the front one: {behind:?}");
}

#[test]
fn an_mu_being_handed_over_cant_be_moved() {
    let Belts { mut model, first, second, drain, .. } = belts();
    let refusals = Arc::new(Mutex::new(Vec::new()));
    let seen = refusals.clone();
    model.set_hooks(Box::new(Code::new(move |call, run, _| {
        match call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) => {
                // Between the second taking it at 104.5 s and its booking point crossing at 105 s,
                // then with its rear still on the first, until 105.5 s.
                run.call_at(SimTime::from_millis(104_700), TIMER, first).unwrap();
                run.call_at(SimTime::from_millis(105_200), TIMER, second).unwrap();
            }
            CallKind::Timer { data } => {
                let mu = run.state(data).unwrap().contents[0];
                seen.lock().unwrap().push(run.move_mu(mu, data, drain).unwrap_err().0);
            }
            _ => {}
        }
        Ok(None)
    })));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(120)).unwrap();
    assert_eq!(*refusals.lock().unwrap(), [
        format!("MU 1 is being handed over to object {second}, and a transfer once begun always finishes"),
        format!("MU 1 still has its rear on object {first}, and a transfer once begun always finishes"),
    ]);
}

#[test]
fn conveyor_hooks_at_the_rear_go_back_exactly_by_checkpoint_and_by_reset() {
    let Belts { mut model, source, first, second, drain } = belts();
    let code = Code::new(|_, _, _| Ok(None));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(source, ObjectHooks { on_exit: every_point(ON_EXIT), ..ObjectHooks::default() });
    model.set_object_hooks(first, both(ON_ENTRY, ON_EXIT));
    model.set_object_hooks(second, both(ON_ENTRY, ON_EXIT));
    model.set_object_hooks(drain, ObjectHooks { on_entry: every_point(ON_ENTRY), ..ObjectHooks::default() });
    let calls_since = |from: usize| log.lock().unwrap()[from..].to_vec();
    let mut run = recorded(model.build(0));
    let mut first_run = run.init().unwrap();
    // Between a handover and the rear crossing, onto the second conveyor and into the Drain, with
    // the calls at the rear kept.
    first_run.extend(run.advance_to(SimTime::from_millis(205_200)).unwrap());
    let first_calls = calls_since(0);
    let mut checkpoint = run.clone();
    let mark = log.lock().unwrap().len();
    let rest = run.advance_to(secs(1_000)).unwrap();
    let rest_calls = calls_since(mark);
    let mark = log.lock().unwrap().len();
    assert_eq!(checkpoint.advance_to(secs(1_000)).unwrap(), rest);
    assert_eq!(calls_since(mark), rest_calls, "the checkpoint made the same calls, those at the rear kept included");
    first_run.extend(rest);
    run.reset();
    let mark = log.lock().unwrap().len();
    let mut second_run = run.init().unwrap();
    second_run.extend(run.advance_to(SimTime::from_millis(205_200)).unwrap());
    let second_calls: Vec<Logged> = calls_since(mark).into_iter().filter(|logged| *logged != Logged::Cleared).collect();
    assert_eq!(second_calls, first_calls, "no call kept at a rear outlives Reset");
    second_run.extend(run.advance_to(secs(1_000)).unwrap());
    assert_eq!(second_run, first_run);
}

#[test]
fn a_distribution_moved_to_the_rear_isnt_asked_where_the_mu_has_left_by_then() {
    let mut model = ModelDescription::new();
    let source = model.add_source(secs(10));
    let belt = model.add_conveyor(5.0, 0.6, 1.0);
    // It passes each MU on at once, before its rear has crossed the conveyor's end.
    let fork = station(&mut model, 0);
    let a = model.add_drain();
    let b = model.add_drain();
    for (from, to) in [(source, belt), (belt, fork), (fork, a), (fork, b)] {
        model.connect(from, to);
    }
    let code = Code::new(move |_, _, _| Ok(Some(Decision::Successor(b))));
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_object_hooks(fork, ObjectHooks { distribution: Some(DISTRIBUTE), distribution_on_entry: Some(TriggerPoint::Rear), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(50)).unwrap();
    // Asked as each MU wants to leave, at its handover, and never again once it has gone.
    let asked: Vec<f64> = calls_of(&log).iter().filter(|call| call.1 == "distribution").map(|call| call.0).collect();
    assert_eq!(asked, [15.0, 25.0, 35.0, 45.0]);
    assert_eq!(left_at(&occurrences, b), 4);
}

#[test]
#[should_panic(expected = "a cross conveyor's distribution runs as it lets an MU in, a moment that can't move")]
fn a_cross_conveyors_distribution_moment_cant_move() {
    let mut model = ModelDescription::new();
    let cross = model.add_cross_conveyor(0.6, 0.6, 1.0);
    model.set_object_hooks(cross, ObjectHooks { distribution: Some(DISTRIBUTE), distribution_on_entry: Some(TriggerPoint::Front), ..ObjectHooks::default() });
}

#[test]
fn an_mu_moved_off_a_conveyor_by_code_leaves_it_whole_at_once() {
    use TriggerPoint::{BookingPoint, Front, Rear};
    let Fork { mut model, fork, a, b, .. } = conveyor_fork();
    let code = Code::new(move |call, run, _| {
        match call.kind {
            CallKind::Lifecycle(Lifecycle::AfterInit) => run.call_at(SimTime::from_millis(12_500), TIMER, 0).unwrap(),
            CallKind::Timer { .. } => {
                // Half way along the conveyor, its rear well inside it.
                let mu = run.state(fork).unwrap().contents[0];
                run.move_mu(mu, fork, b).unwrap();
            }
            _ => {}
        }
        Ok(None)
    });
    let log = code.log();
    model.set_hooks(Box::new(code));
    model.set_lifecycle_hook(Lifecycle::AfterInit, Some(LIFECYCLE));
    model.set_object_hooks(fork, ObjectHooks { on_exit: every_point(ON_EXIT), ..ObjectHooks::default() });
    model.set_object_hooks(b, ObjectHooks { on_entry: every_point(ON_ENTRY), ..ObjectHooks::default() });
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(19)).unwrap();
    assert_eq!(calls_of(&log), [
        (12.5, "exit", fork, Some(Front)),
        (12.5, "exit", fork, Some(BookingPoint)),
        (12.5, "exit", fork, Some(Rear)),
        (12.5, "entry", b, Some(Front)),
        (12.5, "entry", b, Some(BookingPoint)),
        (12.5, "entry", b, Some(Rear)),
    ], "every point at the move, and nothing later");
    assert!(occurrences.iter().any(|o| o.time == SimTime::from_millis(12_500) && matches!(o.kind, OccurrenceKind::MuLeft { at, .. } if at == b)));
    assert_eq!(left_at(&occurrences, a), 0);
    assert!(run.state().objects[fork as usize].contents.is_empty(), "nothing of it stays on the conveyor");
}
