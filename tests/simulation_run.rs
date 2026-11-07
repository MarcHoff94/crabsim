//! Behaviour of a simulation run, tested at its seam: Init, advance, Reset,
//! then read occurrences and state.

use crabsim::demo::build_model;
use crabsim::flows::MaterialFlow;
use crabsim::random_numbers::Distribution;
use crabsim::{
    Dimensions, Drain, EventType, FailureProfile, ModelDescription, ObjectKind, Position, ObjectStatistics, Occurrence, OccurrenceKind, OccurrenceType,
    ProcessState, Recording, RunError, RunState, SimTime, SimulationRun, SingleStation, StopCause,
    ZERO_TIME_EVENT_LIMIT,
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn no_failures(repair_ms: f64) -> FailureProfile {
    FailureProfile::new(1.0, Distribution::CONSTANT { value: repair_ms }, 0)
}

fn station(model: &mut ModelDescription, process_secs: u64) -> u32 {
    model.add_station(secs(process_secs), no_failures(0.0))
}

fn station_with_failures(model: &mut ModelDescription, process_secs: u64, profile: FailureProfile) -> u32 {
    model.add_station(secs(process_secs), profile)
}

/// A station whose default state already holds an MU, so it works from Init onwards.
fn preloaded_station(model: &mut ModelDescription, process_secs: u64, mu_id: u32) -> u32 {
    model.add_station_holding(secs(process_secs), no_failures(0.0), mu_id)
}

fn source(model: &mut ModelDescription, interval_secs: u64) -> u32 {
    model.add_source(secs(interval_secs))
}

fn drain(model: &mut ModelDescription) -> u32 {
    model.add_drain()
}

/// A run that keeps every occurrence, which most of these tests read. A run keeps none by default.
fn recorded(mut run: SimulationRun) -> SimulationRun {
    run.set_recording(Recording::ALL);
    run
}

fn station_in(run: &SimulationRun, id: u32) -> &SingleStation {
    run.object(id).and_then(|object| object.as_station()).expect("a station")
}

fn exit_times(occurrences: &[Occurrence]) -> Vec<f64> {
    occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuLeft { .. }))
        .map(|o| o.time.as_secs_f64())
        .collect()
}

fn exit_ids(occurrences: &[Occurrence]) -> Vec<u32> {
    occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuLeft { mu_id, .. } => Some(mu_id),
            _ => None,
        })
        .collect()
}

fn statistics_of(run: &SimulationRun, id: u32) -> ObjectStatistics {
    object(&run.state(), id).statistics
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
/// id and the drain's. The first MU enters the station at t=10.
fn line(repair_ms: f64) -> (SimulationRun, u32, u32) {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = station_with_failures(&mut model, 60, no_failures(repair_ms));
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    (recorded(model.build(0)), st, dr)
}

// ---------- Controlling a run ----------

#[test]
fn init_sets_time_zero_and_queues_first_events_without_processing_them() {
    let mut model = ModelDescription::new();
    // A zero-interval Source has an event due at t=0, which Init must not process.
    let src = source(&mut model, 0);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = recorded(model.build(0));

    let occurrences = run.init().unwrap();

    assert_eq!(run.time(), SimTime::ZERO);
    assert_eq!(statistics_of(&run, dr).mus_in, 0);
    assert_eq!(object(&run.state(), src).contents, vec![1]);
    assert_eq!(occurrences[0].kind, OccurrenceKind::RunInitialised);
    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuCreated { mu_id: 1, at: src, duration: SimTime::ZERO }));
}

#[test]
fn a_run_counts_the_events_it_has_processed() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = recorded(model.build(0));

    run.init().unwrap();
    assert_eq!(run.events_processed(), 0, "Init queues events without processing them");

    run.advance_to(secs(25)).unwrap();
    assert_eq!(run.events_processed(), 2, "the Source's exits at t=10 and t=20");

    run.reset();
    assert_eq!(run.events_processed(), 0);
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
    assert_eq!(run.step().unwrap_err().error, RunError::NotInitialised);
    assert_eq!(run.advance_to(secs(10)).unwrap_err().error, RunError::NotInitialised);
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
    let (mut run, st_id, _) = line(0.0);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(70)).unwrap();
    assert_eq!(exit_times(&occurrences), vec![70.0]);
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
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    fail_at(&mut run, 1, src);
    assert_eq!(
        run.advance_to(secs(5)).unwrap_err().error,
        RunError::ObjectFailed { object_id: src, time: secs(1) }
    );
}

#[test]
fn an_event_for_an_unknown_object_is_reported() {
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    fail_at(&mut run, 1, 999);
    assert_eq!(run.advance_to(secs(5)).unwrap_err().error, RunError::UnknownObject(999));
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

// ---------- Bounded advancing ----------

/// Source (one MU every 10s) -> Drain.
fn source_into_drain() -> SimulationRun {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run
}

#[test]
fn a_bounded_advance_stops_at_its_last_event_when_more_are_due() {
    let mut run = source_into_drain();
    let advance = run.advance_bounded(secs(100), 2).unwrap();
    assert!(!advance.reached);
    assert_eq!(run.time(), secs(20), "the clock stays at the last event it processed");
    assert_eq!(run.next_event_time(), Some(secs(30)));
    assert_eq!(moves_from(&advance.occurrences, 0), vec![10.0, 20.0]);
}

#[test]
fn a_bounded_advance_with_room_to_spare_leaves_the_clock_at_the_target() {
    let mut run = source_into_drain();
    let advance = run.advance_bounded(secs(25), 1_000).unwrap();
    assert!(advance.reached);
    assert_eq!(run.time(), secs(25));
    assert_eq!(moves_from(&advance.occurrences, 0), vec![10.0, 20.0]);
}

#[test]
fn a_bounded_advance_includes_events_at_the_target() {
    let mut run = source_into_drain();
    let advance = run.advance_bounded(secs(20), 2).unwrap();
    assert!(advance.reached, "using up the limit on the last due event still reaches the target");
    assert_eq!(moves_from(&advance.occurrences, 0), vec![10.0, 20.0]);

    let mut run = source_into_drain();
    let advance = run.advance_bounded(secs(20), 1).unwrap();
    assert!(!advance.reached, "the event at the target is still due");
    assert_eq!(run.time(), secs(10));
}

#[test]
fn a_bounded_advance_of_no_events_only_moves_the_clock_if_nothing_is_due() {
    let mut run = source_into_drain();
    assert!(run.advance_bounded(secs(5), 0).unwrap().reached);
    assert_eq!(run.time(), secs(5));
    assert!(!run.advance_bounded(secs(15), 0).unwrap().reached);
    assert_eq!(run.time(), secs(5));
}

#[test]
fn bounded_advances_give_the_same_occurrences_in_the_same_order_as_advance_to() {
    // Blocking and retries put several events on one time, so this also covers `seq` order.
    let model = || {
        let (mut run, st_id, _) = line(5_000.0);
        run.init().unwrap();
        fail_at(&mut run, 15, st_id);
        fail_at(&mut run, 95, st_id);
        run
    };
    let expected = model().advance_to(secs(400)).unwrap();
    assert!(expected.len() > 30, "the model should do something");

    for max_events in [1, 2, 3, 7] {
        let mut run = model();
        let mut occurrences = Vec::new();
        for _ in 0..expected.len() + 1 {
            let advance = run.advance_bounded(secs(400), max_events).unwrap();
            occurrences.extend(advance.occurrences);
            if advance.reached {
                break;
            }
        }
        assert_eq!(run.time(), secs(400));
        assert_eq!(occurrences, expected, "{max_events} events at a time");
    }
}

#[test]
fn a_bounded_advance_before_init_is_an_error() {
    let (mut run, _, _) = line(0.0);
    assert_eq!(run.advance_bounded(secs(10), 1).unwrap_err().error, RunError::NotInitialised);
}

#[test]
fn an_error_from_a_bounded_advance_carries_the_occurrences_before_it() {
    let mut run = source_into_drain();
    run.schedule_event(secs(25), EventType::StartFailure, 0).unwrap();
    let failure = run.advance_bounded(secs(30), 100).unwrap_err();
    assert_eq!(failure.error, RunError::ObjectFailed { object_id: 0, time: secs(25) });
    assert_eq!(moves_from(&failure.occurrences, 0), vec![10.0, 20.0]);
}

#[test]
fn a_run_says_whether_it_is_initialised() {
    let (mut run, _, _) = line(0.0);
    assert!(!run.is_initialised());
    run.init().unwrap();
    assert!(run.is_initialised());
    run.reset();
    assert!(!run.is_initialised());
}

// ---------- Reset ----------

fn assert_reset_replays_the_run(mut run: SimulationRun, until: u64) {
    run.init().unwrap();
    let first = run.advance_to(secs(until)).unwrap();
    assert!(first.len() > 1, "the model should do something");
    run.reset();
    assert_eq!(run.time(), SimTime::ZERO);
    run.init().unwrap();
    let second = run.advance_to(secs(until)).unwrap();
    assert_eq!(first, second);
}

#[test]
fn reset_replays_a_source_and_drain() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let run = recorded(model.build(3));
    assert_reset_replays_the_run(run, 500);
}

#[test]
fn reset_replays_a_station_with_random_failures() {
    let profile = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = station_with_failures(&mut model, 25, profile);
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    let run = recorded(model.build(7));
    assert_reset_replays_the_run(run, 5_000);
}

#[test]
fn reset_replays_a_preloaded_station() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 30, 50);
    let blocked_behind = station(&mut model, 40);
    let dr = drain(&mut model);
    model.connect(st, blocked_behind);
    model.connect(blocked_behind, dr);
    let run = recorded(model.build(0));
    assert_reset_replays_the_run(run, 200);
}

#[test]
fn reset_replays_a_buffer() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let buf = model.add_buffer(3);
    let dr = drain(&mut model);
    model.connect(src, buf);
    model.connect(buf, dr);
    let run = recorded(model.build(0));
    assert_reset_replays_the_run(run, 200);
}

#[test]
fn reset_mid_run_behaves_like_a_fresh_model() {
    let build = || {
        let profile = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
        let mut model = ModelDescription::new();
        let src = source(&mut model, 10);
        let st = station_with_failures(&mut model, 25, profile);
        let dr = drain(&mut model);
        model.connect(src, st);
        model.connect(st, dr);
        let run = recorded(model.build(11));
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
    let mut model = ModelDescription::new();
    let src = source(&mut model, 5);
    let st = preloaded_station(&mut model, 30, 50);
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    assert!(statistics_of(&run, dr).mus_in > 0);

    run.reset();

    let state = run.state();
    assert_eq!(object(&state, st).contents, vec![50]);
    assert_eq!(object(&state, st).process, ProcessState::Idle);
    assert!(object(&state, src).contents.is_empty());
    assert_eq!(object(&state, dr).statistics.mus_in, 0);
    assert_eq!(run.step().unwrap_err().error, RunError::NotInitialised);
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
    assert!(!occurrences.iter().any(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { .. })));
}

#[test]
fn reset_returns_the_exact_state_from_before_init() {
    let mut model = ModelDescription::new();
    let waiting = preloaded_station(&mut model, 10, 200);
    let busy = preloaded_station(&mut model, 1_000, 201);
    model.connect(waiting, busy);
    let mut run = recorded(model.build(0));
    let default_state = run.state();

    run.init().unwrap();
    run.advance_to(secs(20)).unwrap();
    assert_eq!(object(&run.state(), busy).blocked_predecessors, vec![waiting]);

    run.reset();
    assert_eq!(run.state(), default_state);
}

#[test]
fn reset_starts_round_robin_over() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    // One move has happened by t=15, leaving the pointer on the second successor.
    let a = station(&mut model, 1);
    let b = station(&mut model, 100);
    let c = station(&mut model, 100);
    model.connect(src, a);
    model.connect(a, b);
    model.connect(a, c);
    let run = recorded(model.build(0));
    assert_reset_replays_the_run(run, 15);
}

// ---------- MU identity ----------

#[test]
fn mu_ids_are_unique_across_sources_and_starting_mus() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 5, 2);
    let first = source(&mut model, 10);
    let second = source(&mut model, 10);
    let dr_a = drain(&mut model);
    let dr_b = drain(&mut model);
    model.connect(st, dr_a);
    model.connect(first, dr_a);
    model.connect(second, dr_b);
    let mut run = recorded(model.build(0));
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
            (0.0, OccurrenceKind::MuCreated { mu_id: 1, at: src_id, duration: secs(10) }),
            (10.0, OccurrenceKind::MuMoved { mu_id: 1, from: src_id, to: st_id }),
            (10.0, OccurrenceKind::WorkStarted { mu_id: 1, at: st_id, duration: secs(60) }),
            (70.0, OccurrenceKind::WorkFinished { mu_id: 1, at: st_id }),
            (70.0, OccurrenceKind::MuMoved { mu_id: 1, from: st_id, to: dr_id }),
            (70.0, OccurrenceKind::MuLeft { mu_id: 1, at: dr_id, lead_time: secs(60) }),
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
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 10, 200);
    let busy_successor = preloaded_station(&mut model, 100, 201);
    let dr = drain(&mut model);
    model.connect(st, busy_successor);
    model.connect(busy_successor, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    let occurrences = run.advance_to(secs(100)).unwrap();
    let times = |wanted: OccurrenceKind| -> Vec<f64> {
        occurrences.iter().filter(|o| o.kind == wanted).map(|o| o.time.as_secs_f64()).collect()
    };
    assert_eq!(times(OccurrenceKind::Blocked { object_id: st }), vec![10.0]);
    assert_eq!(times(OccurrenceKind::Unblocked { object_id: st }), vec![100.0]);
    assert_eq!(moves_from(&occurrences, st), vec![100.0]);
    assert_eq!(moves_from(&occurrences, busy_successor), vec![100.0]);
}

#[test]
fn a_failure_reports_its_stop_beginning_and_ending_with_its_cause() {
    let (mut run, st_id, _) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st_id);
    let occurrences = run.advance_to(secs(30)).unwrap();
    let stops: Vec<(f64, OccurrenceKind)> = occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { .. } | OccurrenceKind::InterruptionEnded { .. }))
        .map(|o| (o.time.as_secs_f64(), o.kind.clone()))
        .collect();
    assert_eq!(
        stops,
        vec![
            (20.0, OccurrenceKind::InterruptionBegan { object_id: st_id, cause: StopCause::Failure }),
            (25.0, OccurrenceKind::InterruptionEnded { object_id: st_id, cause: StopCause::Failure }),
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
fn state_reports_a_stations_process_time() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = station(&mut model, 25);
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).process_time, Some(secs(25)));
    assert_eq!(object(&state, src).process_time, None);
    assert_eq!(object(&state, dr).process_time, None);
}

#[test]
fn a_buffer_is_reported_like_any_other_object() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let buf = model.add_buffer(3);
    model.connect(src, buf);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    assert_eq!(object(&run.state(), buf).kind, crabsim::ObjectKind::Buffer);
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
    let (mut run, st_id, _) = line(repair_ms);
    run.init().unwrap();
    for &at in failures_at {
        fail_at(&mut run, at, st_id);
    }
    exit_times(&run.advance_to(secs(200)).unwrap())
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
    assert_eq!(statistics_of(&run, dr).mus_in, 0);
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
    let profile = FailureProfile::new(0.5, Distribution::CONSTANT { value: 30_000.0 }, 0);
    let mut model = ModelDescription::new();
    // Availability 0.5 with a 30s repair means a failure 30s after each repair.
    let st = station_with_failures(&mut model, 10, profile);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(200)).unwrap();
    let began: Vec<f64> = occurrences
        .iter()
        .filter(|o| o.kind == OccurrenceKind::InterruptionBegan { object_id: st, cause: StopCause::Failure })
        .map(|o| o.time.as_secs_f64())
        .collect();
    assert_eq!(began, vec![30.0, 90.0, 150.0]);
}

// ---------- Blocking and retries ----------

#[test]
fn starting_mu_is_worked_on_from_init_and_then_leaves() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 75, 9999);
    let dr = drain(&mut model);
    model.connect(st, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(100)).unwrap();
    assert_eq!(exit_times(&occurrences), vec![75.0]);
    assert_eq!(exit_ids(&occurrences), vec![9999]);
}

#[test]
fn a_blocked_station_retries_when_its_successor_frees_up() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 30);
    let a = station(&mut model, 60);
    let b = preloaded_station(&mut model, 100, 9999);
    let dr = drain(&mut model);
    model.connect(src, a);
    model.connect(a, b);
    model.connect(b, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(250)).unwrap();
    // A finishes MU 1 at t=90 but B is busy until t=100, when A's MU moves across.
    assert_eq!(moves_from(&occurrences, a)[0], 100.0);
    assert_eq!(exit_times(&occurrences)[0], 100.0);
}

#[test]
fn a_blocked_station_falls_back_to_a_free_successor() {
    let mut model = ModelDescription::new();
    let pred = preloaded_station(&mut model, 10, 42);
    let busy = preloaded_station(&mut model, 1_000, 43);
    let free = station(&mut model, 10);
    model.connect(pred, busy);
    model.connect(pred, free);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(10)).unwrap();

    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuMoved { mu_id: 42, from: pred, to: free }));
    assert_eq!(station_in(&run, pred).succ_pointer, 0, "the pointer moves past the chosen successor");
    assert!(
        object(&run.state(), busy).blocked_predecessors.is_empty(),
        "its MU has gone elsewhere, so it isn't waiting for this successor"
    );
}

#[test]
fn round_robin_starts_at_the_first_successor() {
    let mut model = ModelDescription::new();
    let pred = preloaded_station(&mut model, 10, 1);
    let first = station(&mut model, 10);
    let second = station(&mut model, 10);
    model.connect(pred, first);
    model.connect(pred, second);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(10)).unwrap();
    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuMoved { mu_id: 1, from: pred, to: first }));
    assert_eq!(station_in(&run, pred).succ_pointer, 1);
}

#[test]
fn a_station_blocks_when_no_successor_accepts() {
    let mut model = ModelDescription::new();
    let pred = preloaded_station(&mut model, 10, 200);
    let succ = preloaded_station(&mut model, 1_000, 201);
    model.connect(pred, succ);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(20)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, pred).process, ProcessState::Blocked);
    assert_eq!(object(&state, pred).contents, vec![200]);
    assert_eq!(object(&state, succ).blocked_predecessors, vec![pred]);
    assert!(station_in(&run, pred).entrancelocked);
}

#[test]
fn a_retry_while_the_predecessor_is_working_does_not_move_its_mu() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 5);
    let st1 = station(&mut model, 10);
    let st2 = preloaded_station(&mut model, 17, 99);
    let st3 = station(&mut model, 100);
    let dr = drain(&mut model);
    model.connect(src, st1);
    model.connect(st1, st2);
    model.connect(st1, st3);
    model.connect(st2, dr);
    model.connect(st3, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(30)).unwrap();
    // St1's first MU finishes at t=15, while st2 is busy, and goes to st3. St1 then works on
    // its next MU until t=25. St2 frees up at t=17 and retries st1, which must not release.
    assert_eq!(moves_from(&occurrences, st1), vec![15.0, 25.0]);
}

#[test]
fn the_source_keeps_producing_after_being_blocked() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 30);
    let a = station(&mut model, 60);
    let b = preloaded_station(&mut model, 100, 9999);
    let c = station(&mut model, 30);
    let dr = drain(&mut model);
    model.connect(src, a);
    model.connect(a, b);
    model.connect(a, c);
    model.connect(b, dr);
    model.connect(c, dr);
    let mut run = recorded(model.build(0));
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(1_000)).unwrap());
    let created = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::MuCreated { .. })).count();
    assert!(created >= 3, "expected at least 3 MUs, got {created}");
}

#[test]
fn blocked_list_holds_each_predecessor_once_through_the_station_interface() {
    let mut s = SingleStation::new(1, secs(10), no_failures(0.0), None, Vec::new());
    s.add_blocked(5);
    s.add_blocked(5);
    assert_eq!(s.blockedlist.len(), 1);
    let mut d = Drain::new(2);
    d.add_blocked(7);
    d.add_blocked(7);
    assert_eq!(d.blockedlist.len(), 1);
}

#[test]
fn the_longest_waiting_predecessor_retries_first() {
    let mut model = ModelDescription::new();
    // Ids run against the waiting order, so neither id order nor reverse id order passes.
    let waits_last = preloaded_station(&mut model, 30, 3);
    let waits_first = preloaded_station(&mut model, 10, 1);
    let waits_second = preloaded_station(&mut model, 20, 2);
    let contested = preloaded_station(&mut model, 100, 4);
    let dr = drain(&mut model);
    for waiting in [waits_last, waits_first, waits_second] {
        model.connect(waiting, contested);
    }
    model.connect(contested, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    let mut occurrences = run.advance_to(secs(50)).unwrap();
    assert_eq!(object(&run.state(), contested).blocked_predecessors, vec![waits_first, waits_second, waits_last]);

    // A retry that is refused keeps its turn: those still waiting stay in waiting order.
    occurrences.extend(run.advance_to(secs(100)).unwrap());
    assert_eq!(object(&run.state(), contested).blocked_predecessors, vec![waits_second, waits_last]);

    occurrences.extend(run.advance_to(secs(300)).unwrap());
    assert_eq!(moves_from(&occurrences, waits_first), vec![100.0]);
    assert_eq!(moves_from(&occurrences, waits_second), vec![200.0]);
    assert_eq!(moves_from(&occurrences, waits_last), vec![300.0]);
}

#[test]
fn a_predecessor_refused_on_every_retry_is_listed_once() {
    let mut model = ModelDescription::new();
    let early = preloaded_station(&mut model, 5, 1);
    let both = preloaded_station(&mut model, 10, 2);
    let late = preloaded_station(&mut model, 15, 3);
    let contested = preloaded_station(&mut model, 50, 4);
    let always_busy = preloaded_station(&mut model, 1_000, 5);
    let dr = drain(&mut model);
    model.connect(early, contested);
    model.connect(late, contested);
    model.connect(both, always_busy);
    model.connect(both, contested);
    model.connect(contested, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    // Contested frees up at t=50. The earliest takes it, and the retries of the other two are
    // refused, which must not list them a second time at anything that already has them.
    run.advance_to(secs(60)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, always_busy).blocked_predecessors, vec![both]);
    assert_eq!(object(&state, contested).blocked_predecessors, vec![both, late]);
}

#[test]
fn a_predecessor_that_moved_on_loses_its_place_in_the_queue() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 20);
    let moves_on = station(&mut model, 10);
    let contested = preloaded_station(&mut model, 100, 91);
    let elsewhere = station(&mut model, 1_000);
    let waits_longer = preloaded_station(&mut model, 40, 92);
    let dr = drain(&mut model);
    model.connect(src, moves_on);
    model.connect(moves_on, contested);
    model.connect(moves_on, elsewhere);
    model.connect(waits_longer, contested);
    model.connect(contested, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    // At t=30 its first MU is refused by contested and goes elsewhere, so it isn't waiting.
    let mut occurrences = run.advance_to(secs(35)).unwrap();
    assert!(object(&run.state(), contested).blocked_predecessors.is_empty());

    // The other starts waiting at t=40. The second MU is refused everywhere at t=50.
    occurrences.extend(run.advance_to(secs(60)).unwrap());
    assert_eq!(object(&run.state(), contested).blocked_predecessors, vec![waits_longer, moves_on]);

    occurrences.extend(run.advance_to(secs(100)).unwrap());
    assert_eq!(moves_from(&occurrences, waits_longer), vec![100.0]);
}

#[test]
fn a_station_with_nothing_left_to_move_is_not_listed_as_waiting() {
    let mut model = ModelDescription::new();
    let pred = preloaded_station(&mut model, 10, 1);
    let frees_first = preloaded_station(&mut model, 20, 2);
    let always_busy = preloaded_station(&mut model, 1_000, 3);
    let frees_second = preloaded_station(&mut model, 20, 4);
    let dr = drain(&mut model);
    model.connect(pred, frees_first);
    model.connect(pred, always_busy);
    model.connect(pred, frees_second);
    model.connect(frees_first, dr);
    model.connect(frees_second, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    // Two successors free up at t=20 and each sends a retry. The first retry moves the MU, so
    // the second finds the station empty, with nothing to wait for.
    run.advance_to(secs(20)).unwrap();
    assert!(object(&run.state(), always_busy).blocked_predecessors.is_empty());
}

#[test]
fn an_mu_finishing_as_a_successor_frees_up_goes_behind_those_already_waiting() {
    let mut model = ModelDescription::new();
    let contested = preloaded_station(&mut model, 100, 1);
    let waiting = preloaded_station(&mut model, 10, 2);
    // Added after contested, so its exit at t=100 is queued behind contested's.
    let finishes_then = preloaded_station(&mut model, 100, 3);
    let dr = drain(&mut model);
    model.connect(contested, dr);
    model.connect(waiting, contested);
    model.connect(finishes_then, contested);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    let occurrences = run.advance_to(secs(100)).unwrap();
    assert_eq!(moves_from(&occurrences, waiting), vec![100.0]);
    assert!(moves_from(&occurrences, finishes_then).is_empty());
    assert_eq!(object(&run.state(), contested).blocked_predecessors, vec![finishes_then]);
}

#[test]
fn a_retry_only_offers_the_successor_that_sent_it() {
    let mut model = ModelDescription::new();
    let frees_first = preloaded_station(&mut model, 100, 1);
    let frees_second = preloaded_station(&mut model, 5, 2);
    let waits_longer = preloaded_station(&mut model, 10, 3);
    let has_both = preloaded_station(&mut model, 20, 4);
    let dr = drain(&mut model);
    model.connect(frees_first, dr);
    model.connect(frees_second, frees_first);
    model.connect(waits_longer, frees_second);
    model.connect(has_both, frees_second);
    model.connect(has_both, frees_first);
    let mut run = recorded(model.build(0));
    run.init().unwrap();

    // At t=100 the first frees up and sends retries to its two waiters. The second moves
    // across, which frees it up while the other retry is still queued. That retry was sent
    // for the first, so it must not take the second from the one that has waited there longer.
    let occurrences = run.advance_to(secs(100)).unwrap();
    assert_eq!(moves_from(&occurrences, waits_longer), vec![100.0]);
    assert!(moves_from(&occurrences, has_both).is_empty());
}

#[test]
fn a_seed_gives_identical_results_every_time() {
    let week = || {
        let mut run = recorded(build_model(7));
        let mut occurrences = run.init().unwrap();
        occurrences.extend(run.advance_to(secs(7 * 86_400)).unwrap());
        occurrences
    };
    assert_eq!(week(), week());
}

// ---------- Long runs ----------

/// On demand, in release, because it takes minutes. It runs headless, so nothing grows with
/// simulated length. Only the error of a failure is kept, in case something does get recorded: the `Debug` output of `AdvanceFailure` includes every occurrence
/// and can reach gigabytes.
///
/// `cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model`
#[test]
#[ignore = "takes minutes, run on demand in release"]
fn the_demo_model_runs_far_past_fifteen_simulated_years() {
    const YEARS: u64 = 25;
    const DAY: u64 = 86_400;
    /// The most predecessors any object has in the demo model.
    const MOST_PREDECESSORS: usize = 3;

    let mut failures = Vec::new();
    for seed in 0..5 {
        let mut run = build_model(seed);
        run.init().unwrap();
        let mut longest_list = 0;
        for day in 1..=YEARS * 365 {
            if let Err(failure) = run.advance_to(secs(day * DAY)) {
                failures.push(format!("seed {seed}: {:?}", failure.error));
                break;
            }
            let state = run.state();
            longest_list = state.objects.iter().map(|o| o.blocked_predecessors.len()).fold(longest_list, usize::max);
        }
        let years = run.time().as_secs_f64() / (365.0 * DAY as f64);
        println!("seed {seed}: reached {years:.1} years, longest blocked list seen {longest_list}");
        if longest_list > MOST_PREDECESSORS {
            failures.push(format!("seed {seed}: a blocked list held {longest_list} entries"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------- Throughput and ordering ----------

#[test]
fn a_source_feeds_a_drain_in_order() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let ids = exit_ids(&run.advance_to(secs(200)).unwrap());
    assert_eq!(ids, (1..=20).collect::<Vec<u32>>());
}

#[test]
fn order_is_preserved_across_stations() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 0);
    let a = station(&mut model, 10);
    let b = station(&mut model, 30);
    let dr = drain(&mut model);
    model.connect(src, a);
    model.connect(a, b);
    model.connect(b, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let ids = exit_ids(&run.advance_to(secs(500)).unwrap());
    // First exit at 0 + 10 + 30 = 40, then one every 30s (the slowest station) up to t=490.
    assert_eq!(ids, (1..=16).collect::<Vec<u32>>());
}

#[test]
fn order_is_preserved_across_identical_parallel_stations() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 20);
    let a = station(&mut model, 10);
    let b1 = station(&mut model, 20);
    let b2 = station(&mut model, 20);
    let dr = drain(&mut model);
    model.connect(src, a);
    model.connect(a, b1);
    model.connect(a, b2);
    model.connect(b1, dr);
    model.connect(b2, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let ids = exit_ids(&run.advance_to(secs(500)).unwrap());
    // First exit at 20 + 10 + 20 = 50, then one every 20s (the Source's interval) up to t=490.
    assert_eq!(ids, (1..=23).collect::<Vec<u32>>());
}

#[test]
fn unbalanced_parallel_stations_stay_within_the_best_case_throughput() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 20);
    let a = station(&mut model, 10);
    let b1 = station(&mut model, 30);
    let b2 = station(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, a);
    model.connect(a, b1);
    model.connect(a, b2);
    model.connect(b1, dr);
    model.connect(b2, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(500)).unwrap();
    // First possible exit at 20 + 10 + 10 = 40, and the Source limits flow to one MU per 20s.
    let drained = statistics_of(&run, dr).mus_in;
    assert!(drained > 0);
    assert!(drained <= 24, "drained {drained}, above the best case");
}

#[test]
fn every_object_gets_its_own_random_seed_at_init() {
    let mut model = ModelDescription::new();
    source(&mut model, 5);
    station(&mut model, 10);
    station(&mut model, 10);
    drain(&mut model);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let mut seeds: Vec<u64> = run.objects().iter().map(|object| object.base().random_seed).collect();
    seeds.sort_unstable();
    seeds.dedup();
    assert_eq!(seeds.len(), 4);
}

// ---------- Zero-time loops ----------

/// A Source that creates its next MU in no time, into a Drain: simulation time never moves.
fn zero_time_loop() -> SimulationRun {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 0);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let run = recorded(model.build(0));
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
            Err(failure) => {
                occurrences.extend(failure.occurrences);
                return (occurrences, steps, failure.error);
            }
        }
    }
}

#[test]
fn a_zero_time_loop_is_an_error_when_advancing_to_a_time() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    assert_eq!(
        run.advance_to(secs(10)).unwrap_err().error,
        RunError::ZeroTimeLoop { time: SimTime::ZERO }
    );
    assert_eq!(run.time(), SimTime::ZERO, "the clock stays where the loop is");
}

#[test]
fn a_zero_time_loop_is_an_error_when_stepping() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    let (_, steps, error) = step_until_error(&mut run);
    assert_eq!(steps, ZERO_TIME_EVENT_LIMIT);
    assert_eq!(error, RunError::ZeroTimeLoop { time: SimTime::ZERO });
}

#[test]
fn zero_length_failures_are_a_zero_time_loop() {
    let zero_length = FailureProfile::new(0.5, Distribution::CONSTANT { value: 0.0 }, 0);
    let mut model = ModelDescription::new();
    station_with_failures(&mut model, 10, zero_length);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    assert_eq!(
        run.advance_to(secs(10)).unwrap_err().error,
        RunError::ZeroTimeLoop { time: SimTime::ZERO }
    );
}

#[test]
fn a_run_stuck_in_a_zero_time_loop_keeps_refusing() {
    let mut run = zero_time_loop();
    run.init().unwrap();
    run.advance_to(secs(10)).unwrap_err();
    let state = run.state();
    let error = RunError::ZeroTimeLoop { time: SimTime::ZERO };
    assert_eq!(run.step().unwrap_err().error, error);
    assert_eq!(run.advance_to(secs(10)).unwrap_err().error, error);
    assert_eq!(run.state(), state, "refusing processes nothing");
}

#[test]
fn only_events_at_one_time_count_towards_a_zero_time_loop() {
    let mut model = ModelDescription::new();
    // More events in total than the limit allows at one time, but time moves between them.
    let src = source(&mut model, 1);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    run.advance_to(secs(ZERO_TIME_EVENT_LIMIT + 10)).unwrap();
    assert_eq!(statistics_of(&run, dr).mus_in, ZERO_TIME_EVENT_LIMIT + 10);
}

#[test]
fn events_counted_at_an_earlier_time_do_not_count_at_the_time_the_clock_was_moved_to() {
    let mut model = ModelDescription::new();
    // A repeated failure on an already failed Station does nothing, so these are harmless
    // events that all land on one time. The repair is far away, past everything here.
    let st = station_with_failures(&mut model, 10, no_failures(1_000_000_000.0));
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    for _ in 0..ZERO_TIME_EVENT_LIMIT {
        fail_at(&mut run, 5, st);
    }
    run.advance_to(secs(10)).unwrap();
    assert_eq!(run.time(), secs(10), "nothing but the clock has reached t=10");

    fail_at(&mut run, 10, st);
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
    let failure = advanced.advance_to(secs(10)).unwrap_err();
    assert_eq!(failure.occurrences, step_by_step);
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
fn an_error_from_advancing_to_a_time_carries_the_occurrences_before_it() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    fail_at(&mut run, 25, src);
    let failure = run.advance_to(secs(30)).unwrap_err();
    assert_eq!(failure.error, RunError::ObjectFailed { object_id: src, time: secs(25) });
    assert_eq!(
        failure.occurrences,
        vec![Occurrence { time: secs(10), kind: OccurrenceKind::Blocked { object_id: src } }],
        "the Source has no successor, so it blocks at t=10 and then the failure is refused"
    );
}

// ---------- Simulation time ----------

#[test]
fn sampled_times_are_rounded_to_the_nearest_millisecond() {
    // Availability 0.5 makes the time to failure the same as the 1500.6 ms repair.
    let profile = FailureProfile::new(0.5, Distribution::CONSTANT { value: 1_500.6 }, 0);
    let mut model = ModelDescription::new();
    let st = station_with_failures(&mut model, 10, profile);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(4)).unwrap();
    let stops: Vec<(u64, OccurrenceKind)> = occurrences.iter().map(|o| (o.time.as_millis(), o.kind.clone())).collect();
    assert_eq!(
        stops,
        vec![
            (1_501, OccurrenceKind::InterruptionBegan { object_id: st, cause: StopCause::Failure }),
            (3_002, OccurrenceKind::InterruptionEnded { object_id: st, cause: StopCause::Failure }),
        ]
    );
}

// ---------- Describing a model ----------

/// Source -> Station with random failures -> Drain.
fn line_with_random_failures() -> ModelDescription {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = model.add_station(secs(25), FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0));
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    model
}

fn occurrences_until(mut run: SimulationRun, until: u64) -> Vec<Occurrence> {
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(until)).unwrap());
    occurrences
}

#[test]
fn one_description_builds_independent_runs() {
    let model = line_with_random_failures();
    let mut first = recorded(model.build(1));
    first.init().unwrap();
    first.advance_to(secs(3_000)).unwrap();

    let second = recorded(model.build(1));
    assert!(!second.is_initialised(), "advancing one run must not touch another");
    assert_eq!(occurrences_until(second, 5_000), occurrences_until(recorded(model.build(1)), 5_000));
}

#[test]
fn replications_differ_only_through_their_seed() {
    let model = line_with_random_failures();
    assert_eq!(occurrences_until(recorded(model.build(1)), 5_000), occurrences_until(recorded(model.build(1)), 5_000));
    assert_ne!(occurrences_until(recorded(model.build(1)), 5_000), occurrences_until(recorded(model.build(2)), 5_000));
}

#[test]
fn successors_are_tried_in_the_order_they_were_connected() {
    let mut model = ModelDescription::new();
    let pred = preloaded_station(&mut model, 10, 1);
    let later_id = station(&mut model, 10);
    let earlier_id = station(&mut model, 10);
    model.connect(pred, earlier_id);
    model.connect(pred, later_id);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let occurrences = run.advance_to(secs(10)).unwrap();
    assert!(occurrences.iter().any(|o| o.kind == OccurrenceKind::MuMoved { mu_id: 1, from: pred, to: earlier_id }));
}

#[test]
#[should_panic(expected = "can't have successors")]
fn a_drain_cannot_be_given_a_successor() {
    let mut model = ModelDescription::new();
    let dr = drain(&mut model);
    let st = station(&mut model, 10);
    model.connect(dr, st);
}

#[test]
#[should_panic(expected = "is not in the model")]
fn a_connection_must_name_an_object_in_the_model() {
    let mut model = ModelDescription::new();
    let st = station(&mut model, 10);
    model.connect(st, 99);
}

#[test]
#[should_panic]
fn a_connection_from_an_object_to_itself_panics_when_used() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 10, 1);
    model.connect(st, st);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    let _ = run.advance_to(secs(10));
}

#[test]
fn a_description_keeps_names_positions_and_dimensions() {
    let mut model = line_with_random_failures();
    let at = Position { x: 12.5, y: -3.0, z: 0.0 };
    let size = Dimensions { length: 4.0, width: 2.5, height: 1.8, rotation_degrees: 90.0 };
    model.set_name(1, "Press");
    model.set_position(1, at);
    model.set_dimensions(1, size);
    assert_eq!(model.object_count(), 3);
    assert_eq!(model.kind(1), ObjectKind::Station);
    assert_eq!(model.name(1), Some("Press"));
    assert_eq!(model.position(1), Some(at));
    assert_eq!(model.dimensions(1), size);
    assert_eq!(model.name(0), None);
    assert_eq!(model.position(0), None);
}

#[test]
fn dimensions_default_to_a_size_for_each_object_type() {
    let model = line_with_random_failures();
    assert_eq!(model.dimensions(0), Dimensions::default_for(ObjectKind::Source));
    assert_eq!(model.dimensions(1), Dimensions::default_for(ObjectKind::Station));
    assert_eq!(model.dimensions(2), Dimensions::default_for(ObjectKind::Drain));
    assert_ne!(Dimensions::default_for(ObjectKind::Station), Dimensions::default_for(ObjectKind::Drain));
}

#[test]
fn names_positions_and_dimensions_change_no_result() {
    let plain = line_with_random_failures();
    let mut placed = line_with_random_failures();
    for id in 0..3 {
        placed.set_name(id, format!("object {id}"));
        placed.set_position(id, Position { x: id as f64 * 100.0, y: 7.0, z: 0.0 });
        placed.set_dimensions(id, Dimensions { length: 30.0, width: 9.0, height: 2.0, rotation_degrees: 45.0 });
    }
    let (mut a, mut b) = (recorded(plain.build(3)), recorded(placed.build(3)));
    assert_eq!(a.init().unwrap(), b.init().unwrap());
    assert_eq!(a.advance_to(secs(20_000)).unwrap(), b.advance_to(secs(20_000)).unwrap());
    assert_eq!(a.state(), b.state());
}

#[test]
#[should_panic(expected = "is not in the model")]
fn a_position_must_name_an_object_in_the_model() {
    let mut model = line_with_random_failures();
    model.set_position(3, Position { x: 0.0, y: 0.0, z: 0.0 });
}

// ---------- Threads ----------

#[test]
fn a_run_can_move_to_another_thread_part_way_through() {
    let model = line_with_random_failures();
    let expected = occurrences_until(recorded(model.build(3)), 6_000);

    let mut run = recorded(model.build(3));
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(2_000)).unwrap());
    let rest = std::thread::spawn(move || run.advance_to(secs(6_000)).unwrap()).join().unwrap();
    occurrences.extend(rest);
    assert_eq!(occurrences, expected);
}

// ---------- Recording ----------

#[test]
fn a_run_records_nothing_unless_asked() {
    let (mut run, _, _) = line(0.0);
    run.set_recording(Recording::NOTHING);
    assert!(run.init().unwrap().is_empty());
    assert!(run.advance_to(secs(200)).unwrap().is_empty());
    assert!(run.step().unwrap().is_empty());
    assert!(run.advance_bounded(secs(400), 2).unwrap().occurrences.is_empty());
}

#[test]
fn a_new_run_records_nothing() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = model.build(0);
    assert!(run.init().unwrap().is_empty());
    assert!(run.advance_to(secs(100)).unwrap().is_empty());
}

#[test]
fn a_run_can_record_only_chosen_kinds() {
    let (mut run, _, dr) = line(0.0);
    run.set_recording(Recording::only(&[OccurrenceType::MuLeft]));
    assert!(run.init().unwrap().is_empty());
    let occurrences = run.advance_to(secs(200)).unwrap();
    assert_eq!(exit_times(&occurrences), vec![70.0, 130.0, 190.0]);
    assert_eq!(occurrences.len(), 3, "and nothing else");
    assert_eq!(occurrences[0].kind, OccurrenceKind::MuLeft { mu_id: 1, at: dr, lead_time: secs(60) });
}

#[test]
fn recording_can_change_at_any_time_and_changes_only_what_is_kept() {
    let (mut whole, _, _) = line(5_000.0);
    whole.init().unwrap();
    let all = whole.advance_to(secs(300)).unwrap();

    let (mut run, _, _) = line(5_000.0);
    run.init().unwrap();
    run.set_recording(Recording::NOTHING);
    assert!(run.advance_to(secs(100)).unwrap().is_empty());
    run.set_recording(Recording::ALL);
    let rest = run.advance_to(secs(300)).unwrap();

    let expected: Vec<Occurrence> = all.into_iter().filter(|o| o.time > secs(100)).collect();
    assert_eq!(rest, expected);
}

#[test]
fn recording_makes_no_difference_to_the_results() {
    let model = line_with_random_failures();
    let mut headless = model.build(5);
    headless.init().unwrap();
    headless.advance_to(secs(5_000)).unwrap();
    let mut recording = recorded(model.build(5));
    recording.init().unwrap();
    recording.advance_to(secs(5_000)).unwrap();
    assert_eq!(headless.state(), recording.state());
    assert_eq!(headless.events_processed(), recording.events_processed());
}

#[test]
fn reset_keeps_the_recording_setting() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = model.build(0);
    run.set_recording(Recording::only(&[OccurrenceType::MuLeft]));
    run.init().unwrap();
    run.advance_to(secs(20)).unwrap();
    run.reset();
    run.init().unwrap();
    assert_eq!(exit_times(&run.advance_to(secs(20)).unwrap()), vec![10.0, 20.0]);
}

// ---------- Headless runs ----------

#[test]
fn a_headless_run_gives_back_its_final_state_with_no_reset() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = station(&mut model, 60);
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    assert!(run.advance_to(secs(200)).unwrap().is_empty());

    let state = run.state();
    assert_eq!(state.time, secs(200));
    assert!(state.initialised);
    assert_eq!(object(&state, dr).statistics.mus_in, 3);
    assert_eq!(object(&state, st).contents, vec![4]);
}

#[test]
fn a_headless_run_still_gives_its_state_after_a_run_error() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let dr = drain(&mut model);
    model.connect(src, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    fail_at(&mut run, 25, src);
    let failure = run.advance_to(secs(100)).unwrap_err();
    assert_eq!(failure.error, RunError::ObjectFailed { object_id: src, time: secs(25) });
    assert!(failure.occurrences.is_empty());

    let state = run.state();
    assert_eq!(state.time, secs(25));
    assert_eq!(object(&state, dr).statistics.mus_in, 2);
    assert_eq!(object(&state, src).statistics.measured_time(), secs(25));
}

// ---------- Statistics ----------

/// Every object's machine-level times against the time measured.
fn assert_times_add_up(run: &SimulationRun) {
    let state = run.state();
    let measured = state.time.saturating_sub(state.statistics_since);
    for o in &state.objects {
        assert_eq!(o.statistics.measured_time(), measured, "object {}", o.id);
    }
}

#[test]
fn everything_is_zero_before_init() {
    let (run, st, dr) = line(0.0);
    assert_eq!(statistics_of(&run, st), ObjectStatistics::default());
    assert_eq!(statistics_of(&run, dr).mus_in, 0);
    assert_eq!(run.state().statistics_since, SimTime::ZERO);
}

#[test]
fn a_line_without_failures_gives_the_expected_numbers() {
    // MUs enter the station at t=10, 70, 130, 190 and leave it at t=70, 130, 190.
    let (mut run, st, dr) = line(0.0);
    let src = st - 1;
    run.init().unwrap();
    run.advance_to(secs(200)).unwrap();
    assert_times_add_up(&run);

    let s = statistics_of(&run, st);
    assert_eq!((s.mus_in, s.mus_out), (4, 3));
    assert_eq!(s.time_busy, secs(190));
    assert_eq!(s.time_idle, secs(10));
    assert_eq!(s.time_blocked, SimTime::ZERO);
    assert_eq!(s.mu_time_busy, secs(190));
    assert_eq!(s.time_full, secs(190));
    assert_eq!(s.max_contents, 1);

    // The Source waits out its interval, then blocks until the station takes the MU.
    let s = statistics_of(&run, src);
    assert_eq!((s.mus_in, s.mus_out), (5, 4));
    assert_eq!(s.time_busy, secs(50));
    assert_eq!(s.time_blocked, secs(150));
    assert_eq!(s.time_idle, SimTime::ZERO);
    assert_eq!(s.mu_time_busy, SimTime::ZERO, "a Source has no MU-level statistics");
    assert_eq!(s.time_full, SimTime::ZERO);

    let s = statistics_of(&run, dr);
    let drain = s.drain.unwrap();
    assert_eq!((s.mus_in, s.mus_out), (3, 0));
    assert_eq!(s.time_idle, secs(200));
    assert_eq!(drain.lead_time_total, secs(180));
    assert_eq!(drain.lead_time_min, Some(secs(60)));
    assert_eq!(drain.lead_time_max, Some(secs(60)));
    assert_eq!(s.lead_time_mean(), Some(secs(60)));
    assert_eq!((drain.first_exit, drain.last_exit), (Some(secs(70)), Some(secs(190))));
    assert_eq!(drain.exit_interval_min, Some(secs(60)));
    assert_eq!(drain.exit_interval_max, Some(secs(60)));
    assert_eq!(s.mus_per_hour(secs(190)), Some(60.0));
}

#[test]
fn only_a_drain_has_drain_statistics() {
    let (run, st, dr) = line(0.0);
    assert!(statistics_of(&run, dr).drain.is_some());
    assert!(statistics_of(&run, st).drain.is_none());
    assert_eq!(statistics_of(&run, st).mus_per_hour(secs(100)), None);
    assert_eq!(statistics_of(&run, st).lead_time_mean(), None);
}

#[test]
fn state_reports_capacity() {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let st = station(&mut model, 10);
    let buf = model.add_buffer(3);
    let dr = drain(&mut model);
    let state = model.build(0).state();
    assert_eq!(object(&state, src).capacity, 0);
    assert_eq!(object(&state, st).capacity, 1);
    assert_eq!(object(&state, buf).capacity, 3);
    assert_eq!(object(&state, dr).capacity, 0);
}

#[test]
fn a_blocked_station_counts_blocked_time_at_both_levels() {
    let mut model = ModelDescription::new();
    let blocked = preloaded_station(&mut model, 10, 1);
    let busy = preloaded_station(&mut model, 100, 2);
    let dr = drain(&mut model);
    model.connect(blocked, busy);
    model.connect(busy, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(50)).unwrap();
    let s = statistics_of(&run, blocked);
    assert_eq!(s.time_busy, secs(10));
    assert_eq!(s.time_blocked, secs(40));
    assert_eq!(s.mu_time_blocked, secs(40));
    assert_eq!(s.mus_in, 1, "a starting MU counts at Init");
    assert_times_add_up(&run);
}

#[test]
fn a_failure_counts_its_time_once_and_failed_beats_everything_else() {
    // MU 1 enters at t=10. Failed from t=20 to 25, so it finishes at t=75 rather than t=70.
    let (mut run, st, _) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    run.advance_to(secs(75)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!(s.time_failed, secs(5));
    assert_eq!(s.time_busy, secs(60));
    assert_eq!(s.time_idle, secs(10));
    assert_eq!(s.mu_time_stopped, secs(5));
    assert_eq!(s.mu_time_busy, secs(60));
    assert_eq!((s.failures, s.stops), (1, 1));
    assert_times_add_up(&run);
}

#[test]
fn a_discarded_second_failure_is_not_counted() {
    let (mut run, st, _) = line(50_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    fail_at(&mut run, 30, st);
    run.advance_to(secs(40)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!((s.failures, s.stops), (1, 1));
    assert_eq!(s.time_failed, secs(20));
}

#[test]
fn a_failure_while_blocked_counts_as_failed_rather_than_blocked() {
    let mut model = ModelDescription::new();
    let st = model.add_station_holding(secs(10), no_failures(50_000.0), 1);
    let busy = preloaded_station(&mut model, 1_000, 2);
    model.connect(st, busy);
    let mut run = model.build(0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    run.advance_to(secs(40)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!(s.time_blocked, secs(10), "from t=10 to the failure");
    assert_eq!(s.time_failed, secs(20));
    assert_eq!(s.mu_time_stopped, secs(20));
    assert_times_add_up(&run);
}

#[test]
fn lead_time_starts_when_an_mu_leaves_its_source() {
    // The Source blocks from t=20, so MU 2 was created at t=10 and only left at t=70. Its lead
    // time doesn't include that wait, which is the Source's blocked time instead.
    let (mut run, _, _) = line(0.0);
    run.init().unwrap();
    let occurrences = run.advance_to(secs(130)).unwrap();
    let lead_times: Vec<SimTime> = occurrences
        .iter()
        .filter_map(|o| match o.kind {
            OccurrenceKind::MuLeft { lead_time, .. } => Some(lead_time),
            _ => None,
        })
        .collect();
    assert_eq!(lead_times, vec![secs(60), secs(60)]);
}

#[test]
fn a_starting_mu_counts_its_lead_time_from_init() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 75, 1);
    let dr = drain(&mut model);
    model.connect(st, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    assert_eq!(statistics_of(&run, dr).drain.unwrap().lead_time_total, secs(75));
}

#[test]
fn lead_times_keep_their_shortest_and_longest() {
    // A failure holds up one MU only, so one lead time stands out.
    let (mut run, st, dr) = line(5_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    run.advance_to(secs(200)).unwrap();
    let s = statistics_of(&run, dr);
    let drain = s.drain.unwrap();
    assert_eq!(drain.lead_time_min, Some(secs(60)));
    assert_eq!(drain.lead_time_max, Some(secs(65)));
    assert_eq!(drain.lead_time_total, secs(185));
}

#[test]
fn a_jam_shows_in_the_rate_and_the_longest_exit_interval() {
    let (mut run, _, dr) = line(0.0);
    run.init().unwrap();
    run.advance_to(secs(190)).unwrap();
    assert_eq!(statistics_of(&run, dr).drain.unwrap().exit_interval_max, Some(secs(60)));
    assert_eq!(statistics_of(&run, dr).mus_per_hour(secs(190)), Some(60.0));
    run.advance_to(secs(249)).unwrap();
    let s = statistics_of(&run, dr);
    assert_eq!(s.drain.unwrap().exit_interval_max, Some(secs(60)));
    assert!(s.mus_per_hour(secs(249)).unwrap() < 60.0);
}

#[test]
fn a_buffer_counts_its_mus_and_contents() {
    // A Buffer's movement is known to be broken, so this only covers what it counts on entry.
    let mut model = ModelDescription::new();
    let src = source(&mut model, 10);
    let buf = model.add_buffer(3);
    model.connect(src, buf);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_to(secs(10)).unwrap();
    let s = statistics_of(&run, buf);
    assert_eq!(s.mus_in, 1);
    assert_eq!(s.max_contents, 1);
    assert_times_add_up(&run);
}

/// The demo model on one seed, advanced in `slices`, returning the final state.
fn demo_state_after(seed: u64, slices: &[u64]) -> RunState {
    let mut run = build_model(seed);
    run.init().unwrap();
    for &t in slices {
        run.advance_to(secs(t)).unwrap();
    }
    run.state()
}

#[test]
fn the_same_seed_gives_identical_statistics() {
    assert_eq!(demo_state_after(4, &[100_000]), demo_state_after(4, &[100_000]));
    assert_ne!(demo_state_after(4, &[100_000]), demo_state_after(5, &[100_000]));
}

#[test]
fn statistics_do_not_depend_on_how_an_advance_was_split() {
    let whole = demo_state_after(4, &[100_000]);
    assert_eq!(demo_state_after(4, &[1, 777, 30_000, 30_001, 99_999, 100_000]), whole);

    let mut bounded = build_model(4);
    bounded.init().unwrap();
    while !bounded.advance_bounded(secs(100_000), 13).unwrap().reached {}
    assert_eq!(bounded.state(), whole);
}

#[test]
fn reading_statistics_changes_nothing() {
    let mut read_often = build_model(4);
    read_often.init().unwrap();
    for t in (1_000..=100_000).step_by(1_000) {
        read_often.advance_to(secs(t)).unwrap();
        read_often.state();
    }
    assert_eq!(read_often.state(), demo_state_after(4, &[100_000]));
}

#[test]
fn machine_level_times_add_up_at_any_time_on_the_demo_model() {
    let mut run = build_model(2);
    run.init().unwrap();
    for t in [1, 59, 3_601, 86_400, 500_000] {
        run.advance_to(secs(t)).unwrap();
        assert_times_add_up(&run);
    }
}

/// A line whose stations fail at random, running long enough to be at steady state.
fn long_line(availability: f32) -> (SimulationRun, Vec<u32>, u32) {
    let failures = FailureProfile::new(availability, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    let src = source(&mut model, 25);
    let stations: Vec<u32> = [10, 15, 20].iter().map(|&t| model.add_station(secs(t), failures.clone())).collect();
    let dr = drain(&mut model);
    model.connect(src, stations[0]);
    model.connect(stations[0], stations[1]);
    model.connect(stations[1], stations[2]);
    model.connect(stations[2], dr);
    let mut run = model.build(3);
    run.init().unwrap();
    run.advance_to(secs(5_000_000)).unwrap();
    (run, stations, dr)
}

#[test]
fn statistics_agree_with_littles_law() {
    // The MUs between leaving the Source and entering the Drain: L = throughput x lead time.
    let (run, stations, dr) = long_line(0.9);
    let contents: f64 = stations.iter().map(|&id| statistics_of(&run, id).average_contents().unwrap()).sum();
    let s = statistics_of(&run, dr);
    let per_second = s.mus_per_hour(run.time()).unwrap() / 3_600.0;
    let expected = per_second * s.lead_time_mean().unwrap().as_secs_f64();
    assert!((contents - expected).abs() / expected < 0.01, "contents {contents}, throughput x lead time {expected}");
}

#[test]
fn achieved_availability_converges_to_the_configured_value() {
    let (run, stations, _) = long_line(0.9);
    for id in stations {
        let achieved = statistics_of(&run, id).achieved_availability().unwrap();
        assert!((achieved - 0.9).abs() < 0.01, "station {id} achieved {achieved}");
    }
}

// ---------- Clearing statistics ----------

#[test]
fn clearing_statistics_starts_measuring_afresh_from_now() {
    let (mut run, st, dr) = line(5_000.0);
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    run.clear_statistics();
    assert_eq!(run.state().statistics_since, secs(100));
    assert_eq!(statistics_of(&run, st).measured_time(), SimTime::ZERO);
    assert_eq!(statistics_of(&run, dr).mus_in, 0);
    assert_eq!(statistics_of(&run, dr).drain, Some(Default::default()));
    assert_eq!(statistics_of(&run, st).max_contents, 1, "the MU it holds is still there");

    run.advance_to(secs(250)).unwrap();
    assert_times_add_up(&run);
    assert_eq!(statistics_of(&run, st).measured_time(), secs(150));
}

#[test]
fn clearing_statistics_changes_nothing_else() {
    let model = line_with_random_failures();
    let mut plain = recorded(model.build(5));
    plain.init().unwrap();
    plain.advance_to(secs(1_000)).unwrap();
    let expected = plain.advance_to(secs(5_000)).unwrap();

    let mut cleared = recorded(model.build(5));
    cleared.init().unwrap();
    cleared.advance_to(secs(1_000)).unwrap();
    cleared.clear_statistics();
    assert_eq!(cleared.advance_to(secs(5_000)).unwrap(), expected);
    assert_eq!(cleared.events_processed(), plain.events_processed());
}

#[test]
fn an_mu_already_inside_when_cleared_counts_out_and_for_lead_time_but_not_in() {
    // MU 1 entered the station at t=10 and leaves at t=70.
    let (mut run, st, dr) = line(0.0);
    run.init().unwrap();
    run.advance_to(secs(30)).unwrap();
    run.clear_statistics();
    run.advance_to(secs(70)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!((s.mus_in, s.mus_out), (1, 1), "MU 2 came in at t=70, and only MU 1 went out");
    let s = statistics_of(&run, dr);
    assert_eq!(s.mus_in, 1);
    assert_eq!(s.drain.unwrap().lead_time_total, secs(60), "counted in full");
}

#[test]
fn no_exit_interval_or_rate_spans_a_clear() {
    let (mut run, _, dr) = line(0.0);
    run.init().unwrap();
    run.advance_to(secs(100)).unwrap();
    run.clear_statistics();
    run.advance_to(secs(130)).unwrap();
    let s = statistics_of(&run, dr);
    let drain = s.drain.unwrap();
    assert_eq!(drain.first_exit, Some(secs(130)));
    assert_eq!(drain.exit_interval_min, None);
    assert_eq!(s.mus_per_hour(secs(130)), None, "one exit since the clear");
    run.advance_to(secs(190)).unwrap();
    assert_eq!(statistics_of(&run, dr).mus_per_hour(secs(190)), Some(60.0));
}

#[test]
fn reset_clears_statistics_and_when_they_began() {
    let (mut run, st, dr) = line(5_000.0);
    let default_state = run.state();
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    run.advance_to(secs(100)).unwrap();
    run.clear_statistics();
    run.advance_to(secs(200)).unwrap();
    assert!(statistics_of(&run, dr).mus_in > 0);

    run.reset();
    assert_eq!(run.state(), default_state);
    assert_eq!(run.state().statistics_since, SimTime::ZERO);
}

// ---------- Pause ----------

fn pause_at(run: &mut SimulationRun, at: u64, object_id: u32) {
    run.schedule_event(secs(at), EventType::PauseOn, object_id).unwrap();
}

fn resume_at(run: &mut SimulationRun, at: u64, object_id: u32) {
    run.schedule_event(secs(at), EventType::PauseOff, object_id).unwrap();
}

#[test]
fn a_pause_moves_completion_time_later_and_freezes_remaining_time() {
    // MU 1 enters at t=10. Paused from t=20 to t=50: 10 + 60 + 30 = 100.
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    pause_at(&mut run, 20, st);
    resume_at(&mut run, 50, st);
    run.advance_to(secs(30)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).remaining, Some(secs(50)));
    assert_eq!(object(&state, st).stop, Some(StopCause::Pause));
    assert_eq!(exit_times(&run.advance_to(secs(100)).unwrap()), vec![100.0]);
}

#[test]
fn a_paused_station_takes_no_mu_and_releases_no_mu() {
    // Paused while empty, from t=5 to t=40: the Source's first MU, ready at t=10, waits.
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    pause_at(&mut run, 5, st);
    resume_at(&mut run, 40, st);
    let occurrences = run.advance_to(secs(50)).unwrap();
    assert_eq!(moves_from(&occurrences, st - 1), vec![40.0]);
}

#[test]
fn a_pause_is_not_a_failure_and_counts_as_paused_time() {
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    pause_at(&mut run, 20, st);
    resume_at(&mut run, 50, st);
    run.advance_to(secs(60)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!(s.time_paused, secs(30));
    assert_eq!(s.time_failed, SimTime::ZERO);
    assert_eq!((s.pauses, s.failures, s.stops), (1, 0, 1));
    assert_eq!(s.mu_time_stopped, secs(30));
    assert_eq!(s.achieved_availability(), Some(1.0), "a pause doesn't count against availability");
    assert_times_add_up(&run);
}

#[test]
fn a_failure_overlapping_a_pause_is_two_interruptions_and_one_stop() {
    // The design's example: a failure from t=20 to 30 and a pause from t=25 to 35.
    let (mut run, st, _) = line(10_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    pause_at(&mut run, 25, st);
    resume_at(&mut run, 35, st);
    let occurrences = run.advance_to(secs(40)).unwrap();

    let s = statistics_of(&run, st);
    assert_eq!(s.time_failed, secs(10));
    assert_eq!(s.time_paused, secs(5));
    assert_eq!((s.failures, s.pauses, s.stops), (1, 1, 1));
    assert_times_add_up(&run);

    let interruptions: Vec<(f64, OccurrenceKind)> = occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { .. } | OccurrenceKind::InterruptionEnded { .. }))
        .map(|o| (o.time.as_secs_f64(), o.kind.clone()))
        .collect();
    assert_eq!(
        interruptions,
        vec![
            (20.0, OccurrenceKind::InterruptionBegan { object_id: st, cause: StopCause::Failure }),
            (25.0, OccurrenceKind::InterruptionBegan { object_id: st, cause: StopCause::Pause }),
            (30.0, OccurrenceKind::InterruptionEnded { object_id: st, cause: StopCause::Failure }),
            (35.0, OccurrenceKind::InterruptionEnded { object_id: st, cause: StopCause::Pause }),
        ]
    );

    // Overlapping stops count once: 10 + 60 + 15 = 85.
    assert_eq!(exit_times(&run.advance_to(secs(90)).unwrap()), vec![85.0]);
}

#[test]
fn state_reports_a_failure_over_a_pause_and_the_pause_once_it_ends() {
    let (mut run, st, _) = line(10_000.0);
    run.init().unwrap();
    pause_at(&mut run, 20, st);
    fail_at(&mut run, 25, st);
    run.advance_to(secs(22)).unwrap();
    assert_eq!(object(&run.state(), st).stop, Some(StopCause::Pause));
    run.advance_to(secs(30)).unwrap();
    assert_eq!(object(&run.state(), st).stop, Some(StopCause::Failure));
    run.advance_to(secs(40)).unwrap();
    assert_eq!(object(&run.state(), st).stop, Some(StopCause::Pause));
    assert_eq!(statistics_of(&run, st).stops, 1);
}

#[test]
fn work_stays_frozen_until_the_last_interruption_ends() {
    // A pause inside a failure, and a failure inside a pause.
    for (first_at, second_at, resume) in [(20, 25, 28), (20, 22, 60)] {
        let (mut run, st, _) = line(10_000.0);
        run.init().unwrap();
        fail_at(&mut run, first_at, st);
        pause_at(&mut run, second_at, st);
        resume_at(&mut run, resume, st);
        run.advance_to(secs(70)).unwrap();
        let stop_end = resume.max(30);
        // 10 + 60, moved later by the time from the failure until the stop ended.
        let expected = 70 + stop_end - first_at;
        let state = run.state();
        if expected > 70 {
            assert_eq!(object(&state, st).remaining, Some(secs(expected - 70)), "stop ending at t={stop_end}");
        }
    }
}

#[test]
fn a_pause_on_an_already_paused_station_or_a_resume_on_a_running_one_does_nothing() {
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    resume_at(&mut run, 15, st);
    pause_at(&mut run, 20, st);
    pause_at(&mut run, 25, st);
    let occurrences = run.advance_to(secs(30)).unwrap();
    let began = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { .. })).count();
    assert_eq!(began, 1);
    assert_eq!(statistics_of(&run, st).pauses, 1);
}

#[test]
fn a_blocked_station_paused_and_resumed_tries_its_move_again() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 10, 1);
    let dr = drain(&mut model);
    let busy = preloaded_station(&mut model, 30, 2);
    model.connect(st, busy);
    model.connect(busy, dr);
    let mut run = recorded(model.build(0));
    run.init().unwrap();
    pause_at(&mut run, 20, st);
    resume_at(&mut run, 50, st);
    let occurrences = run.advance_to(secs(60)).unwrap();
    // The successor frees up at t=30 while this one is paused, so its move waits for t=50.
    assert_eq!(moves_from(&occurrences, st), vec![50.0]);
}

#[test]
fn reset_clears_a_pause() {
    let (mut run, st, _) = line(0.0);
    let default_state = run.state();
    run.init().unwrap();
    pause_at(&mut run, 20, st);
    run.advance_to(secs(30)).unwrap();
    run.reset();
    assert_eq!(run.state(), default_state);
    run.init().unwrap();
    assert_eq!(exit_times(&run.advance_to(secs(70)).unwrap()), vec![70.0]);
}

#[test]
fn a_pause_draws_no_random_numbers() {
    // Pausing an idle station for no time leaves the rest of the run exactly as it was.
    let model = line_with_random_failures();
    let mut plain = recorded(model.build(5));
    plain.init().unwrap();
    let st = 1;
    plain.advance_to(secs(1_000)).unwrap();
    let expected = plain.advance_to(secs(5_000)).unwrap();

    let mut paused = recorded(model.build(5));
    paused.init().unwrap();
    paused.advance_to(secs(1_000)).unwrap();
    paused.schedule_event(secs(1_000), EventType::PauseOn, st).unwrap();
    paused.schedule_event(secs(1_000), EventType::PauseOff, st).unwrap();
    let occurrences: Vec<Occurrence> = paused
        .advance_to(secs(5_000))
        .unwrap()
        .into_iter()
        .filter(|o| !matches!(o.kind, OccurrenceKind::InterruptionBegan { cause: StopCause::Pause, .. } | OccurrenceKind::InterruptionEnded { cause: StopCause::Pause, .. }))
        .collect();
    let failures = |list: &[Occurrence]| list.iter().filter(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { cause: StopCause::Failure, .. })).map(|o| o.time).collect::<Vec<_>>();
    assert_eq!(failures(&occurrences), failures(&expected));
}

// ---------- Forced failure ----------

fn force_failure_at(run: &mut SimulationRun, at: u64, object_id: u32) {
    run.schedule_event(secs(at), EventType::ForceFailureOn, object_id).unwrap();
}

fn end_forced_failure_at(run: &mut SimulationRun, at: u64, object_id: u32) {
    run.schedule_event(secs(at), EventType::ForceFailureOff, object_id).unwrap();
}

#[test]
fn a_forced_failure_stops_a_station_until_it_is_ended() {
    // MU 1 enters at t=10. Forced to fail from t=20 to t=500: 10 + 60 + 480 = 550.
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    force_failure_at(&mut run, 20, st);
    run.advance_to(secs(400)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).stop, Some(StopCause::ForcedFailure));
    assert_eq!(object(&state, st).remaining, Some(secs(50)));
    end_forced_failure_at(&mut run, 500, st);
    assert_eq!(exit_times(&run.advance_to(secs(600)).unwrap()), vec![550.0]);
}

#[test]
fn a_forced_failure_counts_as_a_failure() {
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    force_failure_at(&mut run, 20, st);
    end_forced_failure_at(&mut run, 50, st);
    run.advance_to(secs(60)).unwrap();
    let s = statistics_of(&run, st);
    assert_eq!(s.time_failed, secs(30));
    assert_eq!(s.time_paused, SimTime::ZERO);
    assert_eq!((s.failures, s.pauses, s.stops), (1, 0, 1));
    assert!(s.achieved_availability().unwrap() < 1.0, "a forced failure counts against availability");
    assert_times_add_up(&run);
}

fn random_failures(occurrences: &[Occurrence]) -> Vec<Occurrence> {
    occurrences
        .iter()
        .filter(|o| {
            matches!(
                o.kind,
                OccurrenceKind::InterruptionBegan { cause: StopCause::Failure, .. }
                    | OccurrenceKind::InterruptionEnded { cause: StopCause::Failure, .. }
            )
        })
        .cloned()
        .collect()
}

#[test]
fn random_failures_carry_on_underneath_a_forced_failure() {
    // The random failures begin and end at the same times with or without a forced failure,
    // including those that fall due during it. One that did so used to end them for good.
    let model = line_with_random_failures();
    let mut plain = recorded(model.build(1));
    plain.init().unwrap();
    let without = plain.advance_to(secs(10_000)).unwrap();
    let mut forced = recorded(model.build(1));
    forced.init().unwrap();
    force_failure_at(&mut forced, 1_000, 1);
    end_forced_failure_at(&mut forced, 3_000, 1);
    let with = forced.advance_to(secs(10_000)).unwrap();

    let during = random_failures(&without).iter().filter(|o| o.time > secs(1_000) && o.time < secs(3_000)).count();
    assert!(during > 0, "the test needs a random failure during the forced one");
    assert_eq!(random_failures(&with), random_failures(&without));
}

#[test]
fn a_forced_failure_overlapping_a_random_failure_is_one_stop() {
    // A random failure from t=20 to 30 and a forced one from t=25 to 35: one stop of 15 s.
    let (mut run, st, _) = line(10_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    force_failure_at(&mut run, 25, st);
    end_forced_failure_at(&mut run, 35, st);
    run.advance_to(secs(27)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).stop, Some(StopCause::Failure), "a random failure is reported over a forced one");
    assert_eq!(object(&state, st).interruptions, vec![StopCause::Failure, StopCause::ForcedFailure]);
    run.advance_to(secs(32)).unwrap();
    assert_eq!(object(&run.state(), st).stop, Some(StopCause::ForcedFailure));
    // MU 1 entered at t=10, so it leaves at 10 + 60 + 15.
    assert_eq!(exit_times(&run.advance_to(secs(100)).unwrap()), vec![85.0]);
    let s = statistics_of(&run, st);
    assert_eq!(s.time_failed, secs(15));
    assert_eq!((s.failures, s.stops), (2, 1));
    assert_times_add_up(&run);
}

#[test]
fn state_lists_every_active_interruption() {
    let (mut run, st, _) = line(10_000.0);
    run.init().unwrap();
    fail_at(&mut run, 20, st);
    force_failure_at(&mut run, 21, st);
    pause_at(&mut run, 22, st);
    run.advance_to(secs(25)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).interruptions, vec![StopCause::Failure, StopCause::ForcedFailure, StopCause::Pause]);
    assert_eq!(object(&state, st - 1).interruptions, vec![]);
}

#[test]
fn forcing_twice_or_ending_what_is_not_forced_does_nothing() {
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    end_forced_failure_at(&mut run, 15, st);
    force_failure_at(&mut run, 20, st);
    force_failure_at(&mut run, 25, st);
    end_forced_failure_at(&mut run, 30, st);
    end_forced_failure_at(&mut run, 40, st);
    let occurrences = run.advance_to(secs(100)).unwrap();
    let forced: Vec<(f64, OccurrenceKind)> = occurrences
        .iter()
        .filter(|o| {
            matches!(
                o.kind,
                OccurrenceKind::InterruptionBegan { cause: StopCause::ForcedFailure, .. }
                    | OccurrenceKind::InterruptionEnded { cause: StopCause::ForcedFailure, .. }
            )
        })
        .map(|o| (o.time.as_secs_f64(), o.kind.clone()))
        .collect();
    assert_eq!(
        forced,
        vec![
            (20.0, OccurrenceKind::InterruptionBegan { object_id: st, cause: StopCause::ForcedFailure }),
            (30.0, OccurrenceKind::InterruptionEnded { object_id: st, cause: StopCause::ForcedFailure }),
        ]
    );
    assert_eq!(exit_times(&occurrences), vec![80.0]);
}

#[test]
fn reset_ends_a_forced_failure() {
    let (mut run, st, _) = line(0.0);
    run.init().unwrap();
    force_failure_at(&mut run, 20, st);
    run.advance_to(secs(30)).unwrap();
    run.reset();
    run.init().unwrap();
    run.advance_to(secs(30)).unwrap();
    assert_eq!(object(&run.state(), st).stop, None);
    assert_eq!(object(&run.state(), st).interruptions, vec![]);
}

// ---------- Changing a process time ----------

/// Source every 30 s -> Station of 20 s -> Drain, with no failures. The first MU reaches the
/// Station at t=30 and finishes at t=50, the second arrives at t=60.
fn slow_line() -> (SimulationRun, u32) {
    let mut model = ModelDescription::new();
    let src = source(&mut model, 30);
    let st = station(&mut model, 20);
    let dr = drain(&mut model);
    model.connect(src, st);
    model.connect(st, dr);
    (recorded(model.build(0)), st)
}

#[test]
fn a_new_process_time_applies_from_the_next_mu() {
    let (mut run, st) = slow_line();
    run.init().unwrap();
    run.advance_to(secs(40)).unwrap();
    run.set_process_time(st, secs(5)).unwrap();
    let occurrences = run.advance_to(secs(100)).unwrap();
    let at_station: Vec<(SimTime, OccurrenceKind)> = occurrences
        .into_iter()
        .filter(|o| match o.kind {
            OccurrenceKind::WorkStarted { at, .. } | OccurrenceKind::WorkFinished { at, .. } => at == st,
            OccurrenceKind::MuMoved { from, .. } => from == st,
            _ => false,
        })
        .map(|o| (o.time, o.kind))
        .collect();
    assert_eq!(
        at_station,
        vec![
            // The MU in work keeps its 20 s.
            (secs(50), OccurrenceKind::WorkFinished { mu_id: 1, at: st }),
            (secs(50), OccurrenceKind::MuMoved { mu_id: 1, from: st, to: st + 1 }),
            (secs(60), OccurrenceKind::WorkStarted { mu_id: 2, at: st, duration: secs(5) }),
            (secs(65), OccurrenceKind::WorkFinished { mu_id: 2, at: st }),
            (secs(65), OccurrenceKind::MuMoved { mu_id: 2, from: st, to: st + 1 }),
            (secs(90), OccurrenceKind::WorkStarted { mu_id: 3, at: st, duration: secs(5) }),
            (secs(95), OccurrenceKind::WorkFinished { mu_id: 3, at: st }),
            (secs(95), OccurrenceKind::MuMoved { mu_id: 3, from: st, to: st + 1 }),
        ]
    );
}

#[test]
fn state_reports_the_current_and_the_configured_process_time() {
    let (mut run, st) = slow_line();
    run.init().unwrap();
    run.set_process_time(st, secs(5)).unwrap();
    let state = run.state();
    assert_eq!(object(&state, st).process_time, Some(secs(5)));
    assert_eq!(object(&state, st).configured_process_time, Some(secs(20)));
    assert_eq!(object(&state, st - 1).configured_process_time, None);
}

#[test]
fn reset_returns_to_the_configured_process_time() {
    let (mut run, st) = slow_line();
    run.init().unwrap();
    run.advance_to(secs(40)).unwrap();
    run.set_process_time(st, secs(5)).unwrap();
    run.reset();
    assert_eq!(object(&run.state(), st).process_time, Some(secs(20)));
}

#[test]
fn kept_changes_become_the_default_state() {
    let (mut run, st) = slow_line();
    run.init().unwrap();
    run.advance_to(secs(40)).unwrap();
    run.set_process_time(st, secs(5)).unwrap();
    run.keep_changes_as_default();
    run.reset();
    let state = run.state();
    assert_eq!(object(&state, st).process_time, Some(secs(5)));
    assert_eq!(object(&state, st).configured_process_time, Some(secs(5)));
}

#[test]
fn only_a_station_has_a_process_time_to_change() {
    let (mut run, st) = slow_line();
    run.init().unwrap();
    assert_eq!(run.set_process_time(st - 1, secs(5)), Err(RunError::NotAStation(st - 1)));
    assert_eq!(run.set_process_time(st + 1, secs(5)), Err(RunError::NotAStation(st + 1)));
    assert_eq!(run.set_process_time(99, secs(5)), Err(RunError::UnknownObject(99)));
}

// ---------- Advancing by events, and clones ----------

#[test]
fn advancing_by_events_leaves_the_clock_at_the_last_one() {
    let mut run = source_into_drain();
    let occurrences = run.advance_events(2).unwrap();
    assert_eq!(run.time(), secs(20));
    assert_eq!(moves_from(&occurrences, 0), vec![10.0, 20.0]);
    assert_eq!(run.next_event_time(), Some(secs(30)));
}

#[test]
fn advancing_by_events_with_none_left_leaves_the_clock_where_it_is() {
    let mut model = ModelDescription::new();
    let st = preloaded_station(&mut model, 10, 1);
    let dr = drain(&mut model);
    model.connect(st, dr);
    let mut run = model.build(0);
    run.init().unwrap();
    run.advance_events(100).unwrap();
    assert_eq!(run.time(), secs(10), "only one event existed");
}

#[test]
fn advancing_by_events_before_init_is_an_error() {
    let (mut run, _, _) = line(0.0);
    assert_eq!(run.advance_events(1).unwrap_err().error, RunError::NotInitialised);
}

#[test]
fn a_clone_of_a_run_continues_exactly_as_the_original_would() {
    let model = line_with_random_failures();
    let mut run = recorded(model.build(5));
    run.init().unwrap();
    run.advance_to(secs(1_234)).unwrap();
    let mut copy = run.clone();
    let from_original = run.advance_to(secs(5_000)).unwrap();
    let from_copy = copy.advance_to(secs(5_000)).unwrap();
    assert_eq!(from_copy, from_original);
    assert_eq!(copy.state(), run.state());
}
