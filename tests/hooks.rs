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
