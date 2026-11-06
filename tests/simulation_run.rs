//! Behaviour of a simulation run, tested at its seam: Init, advance, Reset,
//! then read occurrences and state.

use crabsim::random_numbers::Distribution;
use crabsim::{
    Buffer, Drain, EventType, FailureProfile, MoveableUnit, Occurrence, OccurrenceKind, ProcessState,
    RunError, RunState, SimulationRun, SingleStation, Source, StopCause, ZERO_TIME_EVENT_LIMIT,
};
use std::{cell::RefCell, rc::Rc, time::Duration};

type Shared<T> = Rc<RefCell<T>>;

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn shared<T>(value: T) -> Shared<T> {
    Rc::new(RefCell::new(value))
}

fn no_failures(repair_ms: f64) -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: repair_ms }, 0)
}

fn station(process_secs: u64) -> Shared<SingleStation> {
    shared(SingleStation::new(0, secs(process_secs), no_failures(0.0), None, Vec::new(), Vec::new()))
}

fn station_with_failures(process_secs: u64, profile: FailureProfile) -> Shared<SingleStation> {
    shared(SingleStation::new(0, secs(process_secs), profile, None, Vec::new(), Vec::new()))
}

/// A station whose default state already holds an MU, so it works from Init onwards.
fn preloaded_station(process_secs: u64, mu_id: u32) -> Shared<SingleStation> {
    let mu = MoveableUnit { id: mu_id, location_id: 0 };
    shared(SingleStation::new(0, secs(process_secs), no_failures(0.0), Some(mu), Vec::new(), Vec::new()))
}

fn source(interval_secs: u64) -> Shared<Source> {
    shared(Source::new(0, secs(interval_secs), Vec::new()))
}

fn drain() -> Shared<Drain> {
    shared(Drain::new(0, Vec::new()))
}

fn exit_times(drain: &Shared<Drain>) -> Vec<f64> {
    drain.borrow().exits.iter().map(|d| d.as_secs_f64()).collect()
}

fn fail_at(run: &mut SimulationRun, at: u64, object_id: u32) {
    run.schedule_event(secs(at), EventType::StartFailure, object_id).unwrap();
}

fn moves_from(occurrences: &[Occurrence], object_id: u32) -> Vec<f64> {
    occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { from, .. } if from == object_id))
        .map(|o| o.time.as_secs_f64())
        .collect()
}

fn object(state: &RunState, id: u32) -> &crabsim::ObjectState {
    state.objects.iter().find(|o| o.id == id).expect("object should be reported")
}

/// Source (one MU every 10s) -> Station (60s) -> Drain, returning the run, the station's
/// id and the drain. The first MU enters the station at t=10.
fn line(repair_ms: f64) -> (SimulationRun, u32, Shared<Drain>) {
    let st = station_with_failures(60, no_failures(repair_ms));
    let src = source(10);
    let dr = drain();
    src.borrow_mut().succ = vec![st.clone()];
    st.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    let st_id = run.add_object(st);
    run.add_object(dr.clone());
    (run, st_id, dr)
}

// ---------- Controlling a run ----------

#[test]
fn init_sets_time_zero_and_queues_first_events_without_processing_them() {
    // A zero-interval Source has an event due at t=0, which Init must not process.
    let src = source(0);
    let dr = drain();
    src.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    let src_id = run.add_object(src);
    run.add_object(dr.clone());

    let occurrences = run.init().unwrap();

    assert_eq!(run.time(), Duration::ZERO);
    assert!(exit_times(&dr).is_empty());
    assert_eq!(object(&run.state(), src_id).contents, vec![1]);
    assert_eq!(occurrences[0].kind, OccurrenceKind::RunInitialised);
    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuCreated { mu_id: 1, at: src_id }));
}

#[test]
fn init_twice_is_an_error() {
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    assert_eq!(run.init().unwrap_err(), RunError::AlreadyInitialised);
}

#[test]
fn advancing_before_init_is_an_error() {
    let (mut run, _, _) = line(0.0);
    assert_eq!(run.step().unwrap_err(), RunError::NotInitialised);
    assert_eq!(run.advance_to(secs(10)).unwrap_err(), RunError::NotInitialised);
}

#[test]
fn step_processes_one_event() {
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    run.step().unwrap();
    assert_eq!(run.time(), secs(10), "the first event is the Source's exit at t=10");
}

#[test]
fn advance_to_includes_events_at_that_time_and_leaves_the_clock_there() {
    let (mut run, st_id, dr) = line(0.0);
    run.init().unwrap();
    run.advance_to(secs(70)).unwrap();
    assert_eq!(exit_times(&dr), vec![70.0]);
    assert_eq!(run.time(), secs(70));
    run.advance_to(secs(75)).unwrap();
    assert_eq!(run.time(), secs(75));
    assert_eq!(object(&run.state(), st_id).remaining, Some(secs(55)));
}

#[test]
fn advancing_in_pieces_gives_the_same_result_as_advancing_at_once() {
    let (mut whole, _, _) = line(5_000.0);
    whole.init().unwrap();
    let all = whole.advance_to(secs(200)).unwrap();

    let (mut pieces, _, _) = line(5_000.0);
    pieces.init().unwrap();
    let mut joined = Vec::new();
    for t in [1, 10, 70, 100, 137, 200] {
        joined.extend(pieces.advance_to(secs(t)).unwrap());
    }
    assert_eq!(joined, all);
}

#[test]
fn an_event_error_is_reported_instead_of_swallowed() {
    let src = source(10);
    let mut run = SimulationRun::new(0);
    let src_id = run.add_object(src);
    run.init().unwrap();
    fail_at(&mut run, 1, src_id);
    assert_eq!(
        run.advance_to(secs(5)).unwrap_err(),
        RunError::ObjectFailed { object_id: src_id, time: secs(1) }
    );
}

#[test]
fn an_event_for_an_unknown_object_is_reported() {
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    fail_at(&mut run, 1, 999);
    assert_eq!(run.advance_to(secs(5)).unwrap_err(), RunError::UnknownObject(999));
}

#[test]
fn an_event_cannot_be_scheduled_in_the_past() {
    let (mut run, st_id, _) = line(0.0);
    run.init().unwrap();
    run.advance_to(secs(50)).unwrap();
    assert_eq!(
        run.schedule_event(secs(10), EventType::StartFailure, st_id).unwrap_err(),
        RunError::EventInThePast
    );
}

// ---------- Reset ----------

fn assert_reset_replays_the_run(mut run: SimulationRun, until: u64) {
    run.init().unwrap();
    let first = run.advance_to(secs(until)).unwrap();
    assert!(first.len() > 1, "the model should do something");
    run.reset();
    assert_eq!(run.time(), Duration::ZERO);
    run.init().unwrap();
    let second = run.advance_to(secs(until)).unwrap();
    assert_eq!(first, second);
}

#[test]
fn reset_replays_a_source_and_drain() {
    let src = source(10);
    let dr = drain();
    src.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(3);
    run.add_object(src);
    run.add_object(dr);
    assert_reset_replays_the_run(run, 500);
}

#[test]
fn reset_replays_a_station_with_random_failures() {
    let profile = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
    let st = station_with_failures(25, profile);
    let src = source(10);
    let dr = drain();
    src.borrow_mut().succ = vec![st.clone()];
    st.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(7);
    run.add_object(src);
    run.add_object(st);
    run.add_object(dr);
    assert_reset_replays_the_run(run, 5_000);
}

#[test]
fn reset_replays_a_preloaded_station() {
    let st = preloaded_station(30, 50);
    let blocked_behind = station(40);
    let dr = drain();
    st.borrow_mut().succ = vec![blocked_behind.clone()];
    blocked_behind.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(st);
    run.add_object(blocked_behind);
    run.add_object(dr);
    assert_reset_replays_the_run(run, 200);
}

#[test]
fn reset_replays_a_buffer() {
    let src = source(10);
    let buf = shared(Buffer::new(0, 3, Vec::new(), Vec::new()));
    let dr = drain();
    src.borrow_mut().succ = vec![buf.clone()];
    buf.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(buf);
    run.add_object(dr);
    assert_reset_replays_the_run(run, 200);
}

#[test]
fn reset_mid_run_behaves_like_a_fresh_model() {
    let build = || {
        let profile = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
        let st = station_with_failures(25, profile);
        let src = source(10);
        let dr = drain();
        src.borrow_mut().succ = vec![st.clone()];
        st.borrow_mut().succ = vec![dr.clone()];
        let mut run = SimulationRun::new(11);
        run.add_object(src);
        run.add_object(st);
        run.add_object(dr);
        run
    };
    let mut fresh = build();
    fresh.init().unwrap();
    let expected = fresh.advance_to(secs(2_000)).unwrap();

    let mut used = build();
    used.init().unwrap();
    used.advance_to(secs(777)).unwrap();
    used.reset();
    used.init().unwrap();
    assert_eq!(used.advance_to(secs(2_000)).unwrap(), expected);
}

#[test]
fn reset_deletes_run_mus_and_restores_the_configured_starting_mu() {
    let st = preloaded_station(30, 50);
    let src = source(5);
    let dr = drain();
    src.borrow_mut().succ = vec![st.clone()];
    st.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    let src_id = run.add_object(src);
    let st_id = run.add_object(st);
    let dr_id = run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    assert!(!exit_times(&dr).is_empty());

    run.reset();

    let state = run.state();
    assert_eq!(object(&state, st_id).contents, vec![50]);
    assert_eq!(object(&state, st_id).process, ProcessState::Idle);
    assert!(object(&state, src_id).contents.is_empty());
    assert_eq!(object(&state, dr_id).mus_left, 0);
    assert!(exit_times(&dr).is_empty());
    assert_eq!(run.step().unwrap_err(), RunError::NotInitialised);
}

#[test]
fn reset_discards_everything_still_scheduled() {
    // A replay can't catch leftover events, since they land on the replay's own event times.
    // A failure scheduled by hand, which the next run doesn't repeat, can.
    let (mut run, st_id, _) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 150, st_id);
    run.advance_to(secs(100)).unwrap();
    run.reset();
    assert_eq!(run.next_event_time(), None);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(200)).unwrap();
    assert!(!occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::StopBegan { .. })));
}

#[test]
fn reset_returns_the_exact_state_from_before_init() {
    let waiting = preloaded_station(10, 200);
    let busy = preloaded_station(1_000, 201);
    waiting.borrow_mut().succ = vec![busy.clone()];
    let mut run = SimulationRun::new(0);
    let waiting_id = run.add_object(waiting);
    let busy_id = run.add_object(busy);
    let default_state = run.state();

    run.init().unwrap();
    run.advance_to(secs(20)).unwrap();
    assert_eq!(object(&run.state(), busy_id).blocked_predecessors, vec![waiting_id]);

    run.reset();
    assert_eq!(run.state(), default_state);
}

#[test]
fn reset_starts_round_robin_over() {
    // One move has happened by t=15, leaving the pointer on the second successor.
    let a = station(1);
    let b = station(100);
    let c = station(100);
    let src = source(10);
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b.clone(), c.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(a);
    run.add_object(b);
    run.add_object(c);
    assert_reset_replays_the_run(run, 15);
}

// ---------- MU identity ----------

#[test]
fn mu_ids_are_unique_across_sources_and_starting_mus() {
    let st = preloaded_station(5, 2);
    let first = source(10);
    let second = source(10);
    let dr_a = drain();
    let dr_b = drain();
    st.borrow_mut().succ = vec![dr_a.clone()];
    first.borrow_mut().succ = vec![dr_a.clone()];
    second.borrow_mut().succ = vec![dr_b.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(st);
    run.add_object(first);
    run.add_object(second);
    run.add_object(dr_a);
    run.add_object(dr_b);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(100)).unwrap());

    let mut created: Vec<u32> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuCreated { mu_id, .. } => Some(mu_id),
            _ => None,
        })
        .collect();
    assert!(created.len() >= 10);
    let total = created.len();
    created.sort_unstable();
    created.dedup();
    assert_eq!(created.len(), total, "no MU id may repeat");
    assert!(!created.contains(&2), "the starting MU's id must not be reused");
}

// ---------- Occurrences ----------

#[test]
fn an_mu_passing_through_a_line_reports_each_step_in_order() {
    let (mut run, st_id, _) = line(0.0);
    let src_id = st_id - 1;
    let dr_id = st_id + 1;
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(70)).unwrap());

    let of_mu_1: Vec<(f64, OccurrenceKind)> = occurrences
        .iter()
        .filter(|o| match o.kind {
            OccurrenceKind::MuCreated { mu_id, .. }
            | OccurrenceKind::MuMoved { mu_id, .. }
            | OccurrenceKind::MuLeft { mu_id, .. }
            | OccurrenceKind::WorkStarted { mu_id, .. }
            | OccurrenceKind::WorkFinished { mu_id, .. } => mu_id == 1,
            _ => false,
        })
        .map(|o| (o.time.as_secs_f64(), o.kind.clone()))
        .collect();

    assert_eq!(
        of_mu_1,
        vec![
            (0.0, OccurrenceKind::MuCreated { mu_id: 1, at: src_id }),
            (10.0, OccurrenceKind::MuMoved { mu_id: 1, from: src_id, to: st_id }),
            (10.0, OccurrenceKind::WorkStarted { mu_id: 1, at: st_id }),
            (70.0, OccurrenceKind::WorkFinished { mu_id: 1, at: st_id }),
            (70.0, OccurrenceKind::MuMoved { mu_id: 1, from: st_id, to: dr_id }),
            (70.0, OccurrenceKind::MuLeft { mu_id: 1, at: dr_id }),
        ]
    );
}

#[test]
fn a_stale_exit_produces_no_occurrence() {
    let (mut run, st_id, _) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st_id);
    let occurrences = run.advance_to(secs(74)).unwrap();
    // The original exit at t=70 is stale, so MU 1 has neither finished nor moved on.
    assert!(!occurrences.iter().any(|o| o.kind == OccurrenceKind::WorkFinished { mu_id: 1, at: st_id }));
}

#[test]
fn blocking_is_reported_once_and_unblocking_when_it_ends() {
    let st = preloaded_station(10, 200);
    let busy_successor = preloaded_station(100, 201);
    let dr = drain();
    st.borrow_mut().succ = vec![busy_successor.clone()];
    busy_successor.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    let st_id = run.add_object(st);
    let busy_id = run.add_object(busy_successor);
    run.add_object(dr);
    run.init().unwrap();

    let occurrences = run.advance_to(secs(100)).unwrap();
    let times = |wanted: OccurrenceKind| -> Vec<f64> {
        occurrences.iter().filter(|o| o.kind == wanted).map(|o| o.time.as_secs_f64()).collect()
    };
    assert_eq!(times(OccurrenceKind::Blocked { object_id: st_id }), vec![10.0]);
    assert_eq!(times(OccurrenceKind::Unblocked { object_id: st_id }), vec![100.0]);
    assert_eq!(moves_from(&occurrences, st_id), vec![100.0]);
    assert_eq!(moves_from(&occurrences, busy_id), vec![100.0]);
}

#[test]
fn a_failure_reports_its_stop_beginning_and_ending_with_its_cause() {
    let (mut run, st_id, _) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st_id);
    let occurrences = run.advance_to(secs(30)).unwrap();
    let stops: Vec<(f64, OccurrenceKind)> = occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::StopBegan { .. } | OccurrenceKind::StopEnded { .. }))
        .map(|o| (o.time.as_secs_f64(), o.kind.clone()))
        .collect();
    assert_eq!(
        stops,
        vec![
            (20.0, OccurrenceKind::StopBegan { object_id: st_id, cause: StopCause::Failure }),
            (25.0, OccurrenceKind::StopEnded { object_id: st_id, cause: StopCause::Failure }),
        ]
    );
}

// ---------- State ----------

#[test]
fn state_reports_every_object_with_its_connections() {
    let (mut run, st_id, _) = line(0.0);
    let src_id = st_id - 1;
    let dr_id = st_id + 1;
    run.init().unwrap();
    run.advance_to(secs(30)).unwrap();
    let state = run.state();

    assert_eq!(state.time, secs(30));
    assert_eq!(state.objects.len(), 3);
    assert_eq!(object(&state, src_id).successors, vec![st_id]);
    assert_eq!(object(&state, st_id).successors, vec![dr_id]);
    assert!(object(&state, dr_id).successors.is_empty());
    assert_eq!(object(&state, st_id).contents, vec![1]);
    assert_eq!(object(&state, st_id).process, ProcessState::Processing);
    assert_eq!(object(&state, st_id).remaining, Some(secs(40)));
}

#[test]
fn a_buffer_is_reported_like_any_other_object() {
    let src = source(10);
    let buf = shared(Buffer::new(0, 3, Vec::new(), Vec::new()));
    src.borrow_mut().succ = vec![buf.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    let buf_id = run.add_object(buf);
    run.init().unwrap();
    assert_eq!(object(&run.state(), buf_id).kind, crabsim::ObjectKind::Buffer);
}

#[test]
fn a_drain_has_one_id_that_connections_agree_on() {
    // Regression: the drain used to get a second id, so its predecessors pointed at nothing.
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    let state = run.state();
    let ids: Vec<u32> = state.objects.iter().map(|o| o.id).collect();
    for o in &state.objects {
        for successor in &o.successors {
            assert!(ids.contains(successor), "object {} points at unknown {}", o.id, successor);
        }
    }
}

#[test]
fn remaining_time_is_frozen_during_a_stop() {
    let (mut run, st_id, _) = line(50_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st_id);
    run.advance_to(secs(30)).unwrap();
    assert_eq!(object(&run.state(), st_id).remaining, Some(secs(50)));
    assert_eq!(object(&run.state(), st_id).stop, Some(StopCause::Failure));
    run.advance_to(secs(60)).unwrap();
    assert_eq!(object(&run.state(), st_id).remaining, Some(secs(50)));
    run.advance_to(secs(80)).unwrap();
    assert_eq!(object(&run.state(), st_id).remaining, Some(secs(40)));
    assert_eq!(object(&run.state(), st_id).stop, None);
}

// ---------- Completion time ----------

fn exits_with_failures(failures_at: &[u64], repair_ms: f64) -> Vec<f64> {
    let (mut run, st_id, dr) = line(repair_ms);
    run.init().unwrap();
    for &at in failures_at {
        fail_at(&mut run, at, st_id);
    }
    run.advance_to(secs(200)).unwrap();
    exit_times(&dr)
}

#[test]
fn completion_time_without_failures_is_entry_plus_process_time() {
    assert_eq!(exits_with_failures(&[], 5_000.0), vec![70.0, 130.0, 190.0]);
}

#[test]
fn failure_moves_completion_time_later() {
    // MU 1 enters at t=10 and would complete at t=70. A 5s failure moves that to t=75.
    // MU 2 then enters at t=75 and gets its full 60s, completing at t=135, and so on.
    assert_eq!(exits_with_failures(&[20], 5_000.0), vec![75.0, 135.0, 195.0]);
}

#[test]
fn each_failure_during_processing_moves_completion_time_later() {
    // Two 5s failures while MU 1 is in the station: 10 + 60 + 5 + 5 = 80.
    assert_eq!(exits_with_failures(&[20, 40], 5_000.0), vec![80.0, 140.0, 200.0]);
}

#[test]
fn failure_outlasting_the_original_completion_time_moves_it_by_its_full_length() {
    // A 50s failure runs past the original completion at t=70: 10 + 60 + 50 = 120.
    assert_eq!(exits_with_failures(&[20], 50_000.0), vec![120.0, 180.0]);
}

#[test]
fn a_failed_station_keeps_its_mu_past_the_original_completion_time() {
    let (mut run, st_id, dr) = line(50_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st_id);
    run.advance_to(secs(75)).unwrap();
    assert_eq!(object(&run.state(), st_id).contents, vec![1]);
    assert!(exit_times(&dr).is_empty());
}

#[test]
fn a_failure_on_an_idle_station_denies_entrance_until_repaired() {
    // The Source's first MU is ready at t=10, while the station is down from t=5 to t=65.
    let (mut run, st_id, _) = line(60_000.0);
    run.init().unwrap();
    fail_at(&mut run, 5, st_id);
    let occurrences = run.advance_to(secs(70)).unwrap();
    assert_eq!(moves_from(&occurrences, st_id - 1), vec![65.0]);
}

#[test]
fn an_active_failure_profile_keeps_producing_failures() {
    // Availability 0.5 with a 30s repair means a failure 30s after each repair.
    let profile = FailureProfile::new(0.5, Distribution::CONSTANT { value: 30_000.0 }, 0);
    let st = station_with_failures(10, profile);
    let mut run = SimulationRun::new(0);
    let st_id = run.add_object(st);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(200)).unwrap();
    let began: Vec<f64> = occurrences
        .iter()
        .filter(|o| o.kind == OccurrenceKind::StopBegan { object_id: st_id, cause: StopCause::Failure })
        .map(|o| o.time.as_secs_f64())
        .collect();
    assert_eq!(began, vec![30.0, 90.0, 150.0]);
}

// ---------- Blocking and retries ----------

#[test]
fn starting_mu_is_worked_on_from_init_and_then_leaves() {
    let st = preloaded_station(75, 9999);
    let dr = drain();
    st.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(st);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    assert_eq!(exit_times(&dr), vec![75.0]);
    assert_eq!(dr.borrow().exit_ids, vec![9999]);
}

#[test]
fn a_blocked_station_retries_when_its_successor_frees_up() {
    let a = station(60);
    let b = preloaded_station(100, 9999);
    let src = source(30);
    let dr = drain();
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b.clone()];
    b.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    let a_id = run.add_object(a);
    run.add_object(b);
    run.add_object(dr.clone());
    run.init().unwrap();
    let occurrences = run.advance_to(secs(250)).unwrap();
    // A finishes MU 1 at t=90 but B is busy until t=100, when A's MU moves across.
    assert_eq!(moves_from(&occurrences, a_id)[0], 100.0);
    assert_eq!(exit_times(&dr)[0], 100.0);
}

#[test]
fn a_blocked_station_falls_back_to_a_free_successor() {
    let pred = preloaded_station(10, 42);
    let busy = preloaded_station(1_000, 43);
    let free = station(10);
    pred.borrow_mut().succ = vec![busy.clone(), free.clone()];
    let mut run = SimulationRun::new(0);
    let pred_id = run.add_object(pred.clone());
    let busy_id = run.add_object(busy);
    let free_id = run.add_object(free);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(10)).unwrap();

    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuMoved { mu_id: 42, from: pred_id, to: free_id }));
    assert_eq!(pred.borrow().succ_pointer, 0, "the pointer moves past the chosen successor");
    assert_eq!(object(&run.state(), busy_id).blocked_predecessors, vec![pred_id]);
}

#[test]
fn round_robin_starts_at_the_first_successor() {
    let pred = preloaded_station(10, 1);
    let first = station(10);
    let second = station(10);
    pred.borrow_mut().succ = vec![first.clone(), second.clone()];
    let mut run = SimulationRun::new(0);
    let pred_id = run.add_object(pred.clone());
    let first_id = run.add_object(first);
    run.add_object(second);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(10)).unwrap();
    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuMoved { mu_id: 1, from: pred_id, to: first_id }));
    assert_eq!(pred.borrow().succ_pointer, 1);
}

#[test]
fn a_station_blocks_when_no_successor_accepts() {
    let pred = preloaded_station(10, 200);
    let succ = preloaded_station(1_000, 201);
    pred.borrow_mut().succ = vec![succ.clone()];
    let mut run = SimulationRun::new(0);
    let pred_id = run.add_object(pred.clone());
    let succ_id = run.add_object(succ);
    run.init().unwrap();
    run.advance_to(secs(20)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, pred_id).process, ProcessState::Blocked);
    assert_eq!(object(&state, pred_id).contents, vec![200]);
    assert_eq!(object(&state, succ_id).blocked_predecessors, vec![pred_id]);
    assert!(pred.borrow().entrancelocked);
}

#[test]
fn a_retry_while_the_predecessor_is_working_does_not_move_its_mu() {
    let st1 = station(10);
    let st2 = preloaded_station(17, 99);
    let st3 = station(100);
    let src = source(5);
    let dr = drain();
    src.borrow_mut().succ = vec![st1.clone()];
    st1.borrow_mut().succ = vec![st2.clone(), st3.clone()];
    st2.borrow_mut().succ = vec![dr.clone()];
    st3.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    let st1_id = run.add_object(st1);
    run.add_object(st2);
    run.add_object(st3);
    run.add_object(dr);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(30)).unwrap();
    // St1's first MU finishes at t=15, while st2 is busy, and goes to st3. St1 then works on
    // its next MU until t=25. St2 frees up at t=17 and retries st1, which must not release.
    assert_eq!(moves_from(&occurrences, st1_id), vec![15.0, 25.0]);
}

#[test]
fn the_source_keeps_producing_after_being_blocked() {
    let a = station(60);
    let b = preloaded_station(100, 9999);
    let c = station(30);
    let src = source(30);
    let dr = drain();
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b.clone(), c.clone()];
    b.borrow_mut().succ = vec![dr.clone()];
    c.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(a);
    run.add_object(b);
    run.add_object(c);
    run.add_object(dr);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(1_000)).unwrap());
    let created = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::MuCreated { .. })).count();
    assert!(created >= 3, "expected at least 3 MUs, got {created}");
}

#[test]
fn blocked_list_holds_each_predecessor_once_through_the_station_interface() {
    let mut s = SingleStation::new(1, secs(10), no_failures(0.0), None, Vec::new(), Vec::new());
    s.add_blocked(5);
    s.add_blocked(5);
    assert_eq!(s.blockedlist.len(), 1);
    let mut d = Drain::new(2, Vec::new());
    d.add_blocked(7);
    d.add_blocked(7);
    assert_eq!(d.blockedlist.len(), 1);
}

// ---------- Throughput and ordering ----------

#[test]
fn a_source_feeds_a_drain_in_order() {
    let src = source(10);
    let dr = drain();
    src.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(200)).unwrap();
    let ids = dr.borrow().exit_ids.clone();
    assert_eq!(ids, (1..=20).collect::<Vec<u32>>());
}

#[test]
fn order_is_preserved_across_stations() {
    let a = station(10);
    let b = station(30);
    let src = source(0);
    let dr = drain();
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b.clone()];
    b.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(a);
    run.add_object(b);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(500)).unwrap();
    // First exit at 0 + 10 + 30 = 40, then one every 30s (the slowest station) up to t=490.
    let ids = dr.borrow().exit_ids.clone();
    assert_eq!(ids, (1..=16).collect::<Vec<u32>>());
}

#[test]
fn order_is_preserved_across_identical_parallel_stations() {
    let a = station(10);
    let b1 = station(20);
    let b2 = station(20);
    let src = source(20);
    let dr = drain();
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b1.clone(), b2.clone()];
    b1.borrow_mut().succ = vec![dr.clone()];
    b2.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(a);
    run.add_object(b1);
    run.add_object(b2);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(500)).unwrap();
    // First exit at 20 + 10 + 20 = 50, then one every 20s (the Source's interval) up to t=490.
    let ids = dr.borrow().exit_ids.clone();
    assert_eq!(ids, (1..=23).collect::<Vec<u32>>());
}

#[test]
fn unbalanced_parallel_stations_stay_within_the_best_case_throughput() {
    let a = station(10);
    let b1 = station(30);
    let b2 = station(10);
    let src = source(20);
    let dr = drain();
    src.borrow_mut().succ = vec![a.clone()];
    a.borrow_mut().succ = vec![b1.clone(), b2.clone()];
    b1.borrow_mut().succ = vec![dr.clone()];
    b2.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(a);
    run.add_object(b1);
    run.add_object(b2);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(500)).unwrap();
    // First possible exit at 20 + 10 + 10 = 40, and the Source limits flow to one MU per 20s.
    let drained = dr.borrow().exit_ids.len();
    assert!(drained > 0);
    assert!(drained <= 24, "drained {drained}, above the best case");
}

#[test]
fn every_object_gets_its_own_random_seed_at_init() {
    let src = source(5);
    let st1 = station(10);
    let st2 = station(10);
    let dr = drain();
    let mut run = SimulationRun::new(0);
    run.add_object(src.clone());
    run.add_object(st1.clone());
    run.add_object(st2.clone());
    run.add_object(dr.clone());
    run.init().unwrap();
    let mut seeds = vec![
        src.borrow().base.random_seed,
        st1.borrow().base.random_seed,
        st2.borrow().base.random_seed,
        dr.borrow().base.random_seed,
    ];
    seeds.sort_unstable();
    seeds.dedup();
    assert_eq!(seeds.len(), 4);
}

// ---------- Zero-time loops ----------

/// A Source that creates its next MU in no time, into a Drain: simulation time never moves.
fn zero_time_loop() -> SimulationRun {
    let src = source(0);
    let dr = drain();
    src.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(dr);
    run
}

/// Steps until the run refuses, returning everything it reported and the error.
fn step_until_error(run: &mut SimulationRun) -> (Vec<Occurrence>, u64, RunError) {
    let mut occurrences = Vec::new();
    let mut steps = 0;
    loop {
        match run.step() {
            Ok(new) => {
                occurrences.extend(new);
                steps += 1;
            }
            Err(error) => return (occurrences, steps, error),
        }
    }
}

#[test]
fn a_zero_time_loop_is_an_error_when_advancing_to_a_time() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    assert_eq!(
        run.advance_to(secs(10)).unwrap_err(),
        RunError::ZeroTimeLoop { time: Duration::ZERO }
    );
    assert_eq!(run.time(), Duration::ZERO, "the clock stays where the loop is");
}

#[test]
fn a_zero_time_loop_is_an_error_when_stepping() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    let (_, steps, error) = step_until_error(&mut run);
    assert_eq!(steps, ZERO_TIME_EVENT_LIMIT);
    assert_eq!(error, RunError::ZeroTimeLoop { time: Duration::ZERO });
}

#[test]
fn zero_length_failures_are_a_zero_time_loop() {
    let zero_length = FailureProfile::new(0.5, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut run = SimulationRun::new(0);
    run.add_object(station_with_failures(10, zero_length));
    run.init().unwrap();
    assert_eq!(
        run.advance_to(secs(10)).unwrap_err(),
        RunError::ZeroTimeLoop { time: Duration::ZERO }
    );
}

#[test]
fn a_run_stuck_in_a_zero_time_loop_keeps_refusing() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    run.advance_to(secs(10)).unwrap_err();
    let state = run.state();
    let error = RunError::ZeroTimeLoop { time: Duration::ZERO };
    assert_eq!(run.step().unwrap_err(), error);
    assert_eq!(run.advance_to(secs(10)).unwrap_err(), error);
    assert_eq!(run.state(), state, "refusing processes nothing");
}

#[test]
fn only_events_at_one_time_count_towards_a_zero_time_loop() {
    // More events in total than the limit allows at one time, but time moves between them.
    let src = source(1);
    let dr = drain();
    src.borrow_mut().succ = vec![dr.clone()];
    let mut run = SimulationRun::new(0);
    run.add_object(src);
    run.add_object(dr.clone());
    run.init().unwrap();
    run.advance_to(secs(ZERO_TIME_EVENT_LIMIT + 10)).unwrap();
    assert_eq!(dr.borrow().exit_ids.len() as u64, ZERO_TIME_EVENT_LIMIT + 10);
}

#[test]
fn events_counted_at_an_earlier_time_do_not_count_at_the_time_the_clock_was_moved_to() {
    // A repeated failure on an already failed Station does nothing, so these are harmless
    // events that all land on one time. The repair is far away, past everything here.
    let st = station_with_failures(10, no_failures(1_000_000_000.0));
    let mut run = SimulationRun::new(0);
    let st_id = run.add_object(st);
    run.init().unwrap();
    for _ in 0..ZERO_TIME_EVENT_LIMIT {
        fail_at(&mut run, 5, st_id);
    }
    run.advance_to(secs(10)).unwrap();
    assert_eq!(run.time(), secs(10), "nothing but the clock has reached t=10");

    fail_at(&mut run, 10, st_id);
    run.step().unwrap();
    assert_eq!(run.time(), secs(10));
}

#[test]
fn a_zero_time_loop_is_found_at_the_same_event_however_the_run_is_advanced() {
    let mut stepped = zero_time_loop();
    stepped.init().unwrap();
    let (step_by_step, _, _) = step_until_error(&mut stepped);

    let mut advanced = zero_time_loop();
    advanced.init().unwrap();
    advanced.advance_to(secs(10)).unwrap_err();
    assert_eq!(advanced.take_occurrences(), step_by_step);
}

#[test]
fn reset_clears_the_zero_time_count() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    let (first, _, _) = step_until_error(&mut run);
    run.reset();
    run.init().unwrap();
    let (second, _, _) = step_until_error(&mut run);
    assert_eq!(first, second, "a replay gets the full allowance again");
}

#[test]
fn occurrences_before_a_run_error_can_still_be_collected() {
    let src = source(10);
    let mut run = SimulationRun::new(0);
    let src_id = run.add_object(src);
    run.init().unwrap();
    fail_at(&mut run, 25, src_id);
    run.advance_to(secs(30)).unwrap_err();
    let left_over = run.take_occurrences();
    assert_eq!(
        left_over,
        vec![Occurrence { time: secs(10), kind: OccurrenceKind::Blocked { object_id: src_id } }],
        "the Source has no successor, so it blocks at t=10 and then the failure is refused"
    );
    assert!(run.take_occurrences().is_empty(), "collecting empties them");
}
