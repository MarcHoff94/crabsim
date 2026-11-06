//! Behaviour of a live session, tested at its seam: commands in, observations out.
//!
//! Nearly every test uses a fake clock with `sync`, so what it sees follows from its commands.
//! Real time still plays a part in a few places. The catch-up tests expect a command to land
//! before a very large catch-up finishes, and one test runs on the real clock. Deadlines turn
//! a hang into a failure, so an extremely overloaded machine could fail a test that is fine.

use crabsim::{
    Clock, FailureProfile, LiveSession, ModelDescription, Observation, Occurrence, OccurrenceKind, Recording, RunState,
    EventType, ObjectState, RunError, SessionError, SessionSettings, SessionStatus, SimTime, SimulationRun, StopCause,
    TimeMode, WallClock, OBSERVATION_BACKLOG_LIMIT, ZERO_TIME_EVENT_LIMIT,
};
use crabsim::random_numbers::Distribution;
use std::{
    sync::{mpsc::channel, Arc, Mutex, Once},
    thread,
    time::{Duration, Instant},
};

fn secs(s: u64) -> SimTime {
    SimTime::from_secs(s)
}

fn wall_secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// A model with nothing in it.
fn empty_model() -> SimulationRun {
    ModelDescription::new().build(0)
}

#[derive(Clone, Default)]
struct FakeClock(Arc<Mutex<Duration>>);

impl FakeClock {
    fn set(&self, elapsed: Duration) {
        *self.0.lock().unwrap() = elapsed;
    }
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        *self.0.lock().unwrap()
    }
}

/// A session with its fake clock, and everything seen since the last look.
struct Watched {
    session: LiveSession,
    clock: FakeClock,
    state: Option<RunState>,
    shown: SimTime,
    unread: Seen,
}

#[derive(Default)]
struct Seen {
    occurrences: Vec<Occurrence>,
    states: usize,
    shown_times: usize,
    statuses: Vec<SessionStatus>,
    errors: Vec<SessionError>,
}

impl Watched {
    fn new(build: impl FnOnce() -> SimulationRun + Send + 'static) -> Watched {
        Watched::with_settings(build, SessionSettings::default())
    }

    fn with_settings(build: impl FnOnce() -> SimulationRun + Send + 'static, settings: SessionSettings) -> Watched {
        arm_watchdog();
        let clock = FakeClock::default();
        let session = LiveSession::with_settings(build, clock.clone(), settings);
        Watched { session, clock, state: None, shown: SimTime::ZERO, unread: Seen::default() }
    }

    /// Moves the clock, once the session has handled every command given so far. A command
    /// reads the clock when the session gets to it, not when it was given.
    fn at(&mut self, elapsed_secs: u64) {
        self.settle();
        self.clock.set(wall_secs(elapsed_secs));
    }

    /// Waits for the session to catch up, reading whatever it needs read to get there.
    fn settle(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.session.sync() {
                Ok(()) => return,
                Err(SessionError::Backlogged) => {
                    assert!(Instant::now() < deadline, "the session never caught up");
                    self.collect();
                }
                Err(error) => panic!("sync failed: {error}"),
            }
        }
    }

    fn collect(&mut self) {
        for observation in self.session.observations().try_iter() {
            match observation {
                Observation::Occurrences(new) => self.unread.occurrences.extend(new),
                Observation::ShownTime(shown) => {
                    self.unread.shown_times += 1;
                    self.shown = shown;
                }
                Observation::State(state) => {
                    self.unread.states += 1;
                    self.shown = state.time;
                    self.state = Some(state);
                }
                Observation::Status(status) => self.unread.statuses.push(status),
                Observation::Error(error) => self.unread.errors.push(error),
            }
        }
    }

    /// Waits for the session to catch up, then hands over what it reported.
    fn look(&mut self) -> Seen {
        self.settle();
        self.collect();
        std::mem::take(&mut self.unread)
    }

    fn look_without_errors(&mut self) -> Vec<Occurrence> {
        let seen = self.look();
        assert_eq!(seen.errors, vec![]);
        seen.occurrences
    }

    fn time(&mut self) -> SimTime {
        let seen = self.look();
        assert_eq!(seen.errors, vec![]);
        self.shown
    }

    /// The full state at shown time, asked for.
    fn state_now(&mut self) -> RunState {
        self.session.request_state().unwrap();
        self.look();
        self.state.clone().expect("the session should have reported state")
    }

    /// Waits until the session reports an occurrence after t=0, which it only does part way
    /// through a catch-up that is too big for one slice.
    fn wait_for_catch_up_to_begin(&self) {
        wait_for_occurrence(&self.session, |o| o.time > SimTime::ZERO);
    }
}

/// How long a test waits for something that must happen, far longer than it ever takes.
/// Only a broken session runs into this, so no test depends on timing.
const PATIENCE: Duration = Duration::from_secs(30);

/// How long this whole test binary may take. `sync` and a dropped session's join have no
/// timeout of their own, so a broken session could otherwise hang the run instead of failing it.
const WHOLE_BINARY_PATIENCE: Duration = Duration::from_secs(120);

/// The most rounds a pattern of controls gets to reach the common time.
const MAX_ROUNDS: usize = 10_000;

fn arm_watchdog() {
    static ARMED: Once = Once::new();
    ARMED.call_once(|| {
        thread::spawn(|| {
            thread::sleep(WHOLE_BINARY_PATIENCE);
            eprintln!("tests/live_session.rs is still running after {WHOLE_BINARY_PATIENCE:?}: something hangs");
            std::process::exit(101);
        });
    });
}

/// A session on a fake clock or any other, with the hang protection every test needs.
fn session_on<C: Clock>(build: impl FnOnce() -> SimulationRun + Send + 'static, clock: C) -> LiveSession {
    arm_watchdog();
    LiveSession::new(build, clock)
}

/// Waits for an observation, giving up at a deadline even if others keep arriving.
fn wait_for_observation<T>(session: &LiveSession, mut wanted: impl FnMut(Observation) -> Option<T>) -> T {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let observation = session.observations().recv_timeout(left).expect("the awaited observation never came");
        if let Some(found) = wanted(observation) {
            return found;
        }
    }
}

fn wait_for_occurrence(session: &LiveSession, wanted: impl Fn(&Occurrence) -> bool) {
    wait_for_observation(session, |observation| match observation {
        Observation::Occurrences(occurrences) if occurrences.iter().any(&wanted) => Some(()),
        _ => None,
    });
}

/// More events than the session handles between looks at its commands, by far.
const HEAVY_CATCH_UP: u64 = 2_000_000;

fn finishes(what: &str, work: impl FnOnce() + Send + 'static) {
    let (done, wait) = channel();
    thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    wait.recv_timeout(PATIENCE).unwrap_or_else(|_| panic!("{what} did not finish"));
}

fn source_every(interval: SimTime) -> ModelDescription {
    let mut model = ModelDescription::new();
    let src = model.add_source(interval);
    let dr = model.add_drain();
    model.connect(src, dr);
    model
}

/// Source (one MU every 10s) -> Drain.
fn source_into_drain() -> SimulationRun {
    source_every(secs(10)).build(0)
}

/// Source (one MU every second) -> Drain, for catching up on many events.
fn busy_source() -> SimulationRun {
    source_every(secs(1)).build(0)
}

/// A Source that creates its next MU in no time: simulation time never moves.
fn zero_time_loop() -> SimulationRun {
    source_every(SimTime::ZERO).build(0)
}

/// A model that fails at t=25 of its first run, because a Source can't handle a failure.
/// Init is what schedules the failure, so a run after Reset goes on past t=25.
fn fails_at_25() -> SimulationRun {
    let mut run = source_into_drain();
    run.init().unwrap();
    run.schedule_event(secs(25), EventType::StartFailure, 0).unwrap();
    run
}

/// Source -> Drain with an MU every second, and an edge into a Source that panics at t=1000.
fn panics_at_1000() -> SimulationRun {
    let mut model = source_every(secs(1));
    let first = model.add_source(secs(1000));
    let second = model.add_source(secs(1000));
    model.connect(first, second);
    model.build(0)
}

/// Source -> Station with random failures -> Drain.
fn line_with_random_failures() -> SimulationRun {
    let failures = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    let src = model.add_source(secs(10));
    let st = model.add_station(secs(8), failures);
    let dr = model.add_drain();
    model.connect(src, st);
    model.connect(st, dr);
    model.build(7)
}

// ---------- Start ----------

#[test]
fn start_from_the_default_state_does_init() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    let seen = watched.look();
    assert_eq!(seen.occurrences[0].kind, OccurrenceKind::RunInitialised);
    assert!(watched.state.unwrap().initialised);
}

#[test]
fn nothing_happens_before_start() {
    let mut watched = Watched::new(source_into_drain);
    watched.at(100);
    let seen = watched.look();
    assert!(seen.occurrences.is_empty());
    assert_eq!(seen.states, 0);
}

#[test]
fn simulation_time_follows_the_clock() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.look();
    watched.at(25);
    let seen = watched.look();
    assert_eq!(watched.shown, secs(25));
    let moved: Vec<_> = seen
        .occurrences
        .iter()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { .. }))
        .map(|o| o.time)
        .collect();
    assert_eq!(moved, vec![secs(10), secs(20)]);
}

#[test]
fn time_on_the_clock_before_start_does_not_count() {
    let mut watched = Watched::new(source_into_drain);
    watched.at(100);
    watched.session.start().unwrap();
    assert_eq!(watched.time(), SimTime::ZERO);
    watched.at(105);
    assert_eq!(watched.time(), secs(5));
}

// ---------- Speed ----------

#[test]
fn simulation_time_follows_the_clock_times_the_speed() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.set_speed(4.0).unwrap();
    watched.session.start().unwrap();
    watched.at(5);
    assert_eq!(watched.time(), secs(20));
}

#[test]
fn a_speed_below_one_slows_simulation_time_down() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.set_speed(0.5).unwrap();
    watched.session.start().unwrap();
    watched.at(30);
    assert_eq!(watched.time(), secs(15));
}

#[test]
fn changing_speed_does_not_make_simulation_time_jump() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(100);
    assert_eq!(watched.time(), secs(100));

    watched.session.set_speed(10.0).unwrap();
    assert_eq!(watched.time(), secs(100), "the new speed only applies from now on");
    watched.at(103);
    assert_eq!(watched.time(), secs(130));

    watched.session.set_speed(1.0).unwrap();
    assert_eq!(watched.time(), secs(130));
    watched.at(105);
    assert_eq!(watched.time(), secs(132));
}

#[test]
fn time_since_the_last_look_counts_at_the_old_speed() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.look();
    watched.at(10);
    watched.session.set_speed(5.0).unwrap();
    assert_eq!(watched.time(), secs(10));
}

#[test]
fn an_invalid_speed_is_an_error_and_changes_nothing() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.set_speed(2.0).unwrap();
    for speed in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        match watched.session.set_speed(speed) {
            Err(SessionError::InvalidSpeed(_)) => {}
            other => panic!("speed {speed} gave {other:?}"),
        }
    }
    watched.session.start().unwrap();
    watched.at(10);
    assert_eq!(watched.time(), secs(20), "the last valid speed still applies");
}

#[test]
fn an_enormous_speed_does_not_panic() {
    let mut watched = Watched::new(empty_model);
    watched.session.set_speed(f64::MAX).unwrap();
    watched.session.start().unwrap();
    watched.at(1_000_000);
    assert_eq!(watched.time(), SimTime::MAX, "an empty model has nothing to slow it down");
}

// ---------- Halt ----------

#[test]
fn halt_freezes_simulation_time_while_the_clock_moves() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(15);
    assert_eq!(watched.time(), secs(15));
    watched.session.halt().unwrap();
    watched.look();

    watched.at(1_000);
    let seen = watched.look();
    assert!(seen.occurrences.is_empty());
    assert_eq!(seen.shown_times, 0);
    assert_eq!(watched.shown, secs(15));
}

#[test]
fn halt_takes_effect_at_the_clock_time_it_is_given() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(15);
    watched.session.halt().unwrap();
    assert_eq!(watched.time(), secs(15));
}

#[test]
fn start_continues_from_where_halt_left_off_without_counting_the_halted_time() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(15);
    watched.look();
    watched.session.halt().unwrap();
    watched.at(1_000);
    watched.look();

    watched.session.start().unwrap();
    let seen = watched.look();
    assert!(
        !seen.occurrences.iter().any(|o| o.kind == OccurrenceKind::RunInitialised),
        "continuing must not repeat Init"
    );
    assert_eq!(watched.shown, secs(15));
    watched.at(1_010);
    assert_eq!(watched.time(), secs(25));
}

#[test]
fn a_speed_set_during_halt_applies_after_start() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(10);
    watched.look();
    watched.session.halt().unwrap();
    watched.session.set_speed(3.0).unwrap();
    watched.at(20);
    assert_eq!(watched.time(), secs(10));
    watched.session.start().unwrap();
    watched.at(30);
    assert_eq!(watched.time(), secs(40));
}

#[test]
fn starting_twice_and_halting_twice_change_nothing() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.halt().unwrap();
    watched.session.start().unwrap();
    watched.at(10);
    watched.session.start().unwrap();
    assert_eq!(watched.time(), secs(10));
    watched.at(20);
    assert_eq!(watched.time(), secs(20));
    watched.session.halt().unwrap();
    watched.session.halt().unwrap();
    watched.at(30);
    assert_eq!(watched.time(), secs(20));
}

// ---------- Reset ----------

#[test]
fn reset_returns_to_the_default_state_and_stays_there() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.reset().unwrap();
    watched.look();
    let default_state = watched.state.clone().unwrap();

    watched.session.start().unwrap();
    watched.at(25);
    watched.look();
    watched.session.reset().unwrap();
    watched.look();
    assert_eq!(watched.state.as_ref().unwrap(), &default_state);

    watched.at(500);
    let seen = watched.look();
    assert!(seen.occurrences.is_empty(), "after Reset nothing runs until Start");
    assert_eq!(seen.shown_times, 0);
}

#[test]
fn reset_while_running_then_start_replays_the_run() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    let first = watched.look().occurrences;
    assert!(first.len() > 1);

    watched.session.reset().unwrap();
    watched.at(100);
    watched.look();
    watched.session.start().unwrap();
    watched.at(125);
    assert_eq!(watched.look().occurrences, first);
}

#[test]
fn reset_during_halt_works_and_start_replays() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    let first = watched.look().occurrences;
    watched.session.halt().unwrap();
    watched.session.reset().unwrap();
    watched.session.start().unwrap();
    watched.at(50);
    assert_eq!(watched.look().occurrences, first);
}

#[test]
fn reset_keeps_the_speed() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.set_speed(2.0).unwrap();
    watched.session.start().unwrap();
    watched.at(10);
    watched.look();
    watched.session.reset().unwrap();
    watched.session.start().unwrap();
    watched.at(20);
    assert_eq!(watched.time(), secs(20));
}

// ---------- Observations ----------

#[test]
fn a_state_on_request_reflects_shown_time() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    watched.look();
    let state = watched.state_now();
    let drain = state.objects.iter().find(|o| o.id == 1).unwrap();
    assert_eq!(drain.statistics.mus_in, 2);
}

#[test]
fn the_real_clock_advances_a_run_without_anyone_asking() {
    let session = session_on(source_into_drain, WallClock::new());
    session.set_speed(1_000.0).unwrap();
    session.start().unwrap();
    wait_for_occurrence(&session, |o| matches!(o.kind, OccurrenceKind::MuLeft { .. }));
}

// ---------- Status ----------

#[test]
fn a_new_session_reports_no_status_until_it_changes() {
    let mut watched = Watched::new(source_into_drain);
    assert_eq!(watched.look().statuses, vec![]);
}

#[test]
fn start_and_halt_report_the_status_they_change() {
    use SessionStatus::{Advancing, Halted};
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    assert_eq!(watched.look().statuses, vec![Advancing]);
    watched.session.halt().unwrap();
    assert_eq!(watched.look().statuses, vec![Halted]);
    watched.session.start().unwrap();
    assert_eq!(watched.look().statuses, vec![Advancing]);
}

#[test]
fn controls_that_change_nothing_report_no_status() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.halt().unwrap();
    watched.session.reset().unwrap();
    assert_eq!(watched.look().statuses, vec![], "it was already halted");

    watched.session.start().unwrap();
    watched.look();
    watched.session.start().unwrap();
    watched.session.set_speed(2.0).unwrap();
    assert_eq!(watched.look().statuses, vec![], "it was already advancing");
}

#[test]
fn the_status_comes_after_what_start_reports() {
    let watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.session.sync().unwrap();
    let observations: Vec<Observation> = watched.session.observations().try_iter().collect();
    assert_eq!(observations.last(), Some(&Observation::Status(SessionStatus::Advancing)));
}

#[test]
fn reset_while_advancing_reports_halted() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    watched.look();
    watched.session.reset().unwrap();
    assert_eq!(watched.look().statuses, vec![SessionStatus::Halted]);
}

#[test]
fn a_run_error_reports_failed_and_only_reset_changes_that() {
    use SessionStatus::{Advancing, Failed, Halted};
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    assert_eq!(watched.look().statuses, vec![Advancing]);
    watched.at(30);
    assert_eq!(watched.look().statuses, vec![Failed]);

    watched.session.halt().unwrap();
    watched.session.start().unwrap();
    assert_eq!(watched.look().statuses, vec![], "still failed");

    watched.session.reset().unwrap();
    assert_eq!(watched.look().statuses, vec![Halted]);
    watched.session.start().unwrap();
    assert_eq!(watched.look().statuses, vec![Advancing]);
}

// ---------- Errors ----------

#[test]
fn a_zero_time_loop_arrives_as_an_error_rather_than_a_freeze() {
    let mut watched = Watched::new(zero_time_loop);
    watched.session.start().unwrap();
    let seen = watched.look();
    assert_eq!(seen.errors, vec![SessionError::Run(RunError::ZeroTimeLoop { time: SimTime::ZERO })]);
    assert!(seen.occurrences.len() as u64 > ZERO_TIME_EVENT_LIMIT, "what happened before it still arrives");
}

#[test]
fn a_run_error_halts_the_session_and_reports_what_came_before_it_first() {
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(30);
    watched.session.sync().unwrap();
    let observations: Vec<Observation> = watched.session.observations().try_iter().collect();

    let moved: Vec<_> = observations
        .iter()
        .filter_map(|o| if let Observation::Occurrences(occurrences) = o { Some(occurrences) } else { None })
        .flatten()
        .filter(|o| matches!(o.kind, OccurrenceKind::MuMoved { .. }))
        .map(|o| o.time)
        .collect();
    assert_eq!(moved, vec![secs(10), secs(20)]);
    assert_eq!(
        observations.last(),
        Some(&Observation::Error(SessionError::Run(RunError::ObjectFailed { object_id: 0, time: secs(25) })))
    );

    watched.at(100);
    let seen = watched.look();
    assert!(seen.occurrences.is_empty(), "a session stays halted after an error");
    assert_eq!(seen.shown_times, 0);
}

#[test]
fn start_after_a_run_error_is_an_error_until_reset() {
    let mut watched = Watched::new(zero_time_loop);
    watched.session.start().unwrap();
    watched.look();

    watched.session.start().unwrap();
    let seen = watched.look();
    assert_eq!(seen.errors, vec![SessionError::ResetRequired]);
    assert!(seen.occurrences.is_empty());
}

#[test]
fn reset_works_after_a_run_error_and_start_replays() {
    let mut watched = Watched::new(zero_time_loop);
    watched.session.start().unwrap();
    let first = watched.look();
    assert_eq!(first.errors.len(), 1);

    watched.session.reset().unwrap();
    let after_reset = watched.look();
    assert_eq!(after_reset.errors, vec![]);
    assert!(!watched.state.as_ref().unwrap().initialised);

    watched.session.start().unwrap();
    let second = watched.look();
    assert_eq!(second.occurrences, first.occurrences);
    assert_eq!(second.errors, first.errors);
}

#[test]
fn a_run_goes_on_past_where_it_failed_after_reset() {
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(30);
    assert_eq!(watched.look().errors.len(), 1);
    watched.session.reset().unwrap();
    watched.session.start().unwrap();
    watched.at(130);
    assert_eq!(watched.time(), secs(100));
}

// ---------- Staying responsive ----------

#[test]
fn halt_takes_effect_part_way_through_a_catch_up() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    watched.wait_for_catch_up_to_begin();
    watched.session.halt().unwrap();
    let reached = watched.time();
    assert!(reached < secs(HEAVY_CATCH_UP), "halted at {reached:?}, after all the work was done");

    watched.at(HEAVY_CATCH_UP * 2);
    assert_eq!(watched.look().shown_times, 0, "it stays halted");
    assert_eq!(watched.shown, reached);
}

#[test]
fn a_new_speed_drops_the_catch_up_rather_than_finishing_it() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    watched.wait_for_catch_up_to_begin();
    watched.session.set_speed(0.5).unwrap();
    let reached = watched.time();
    assert!(reached < secs(HEAVY_CATCH_UP), "reached {reached:?}, after all the work was done");

    watched.at(HEAVY_CATCH_UP + 10);
    assert_eq!(watched.time(), reached + secs(5), "and it carries on from there at the new speed");
}

#[test]
fn reset_takes_effect_part_way_through_a_catch_up() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    watched.wait_for_catch_up_to_begin();
    watched.session.reset().unwrap();
    assert_eq!(watched.time(), SimTime::ZERO);
}

#[test]
fn halting_before_shown_time_reaches_an_error_keeps_it_from_showing() {
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(10);
    watched.session.halt().unwrap();
    assert_eq!(watched.look().errors, vec![], "the run has computed past it, but nobody has seen that yet");
    watched.session.start().unwrap();
    watched.at(30);
    assert_eq!(watched.look().errors.len(), 1, "it still comes when shown time gets there");
}

// ---------- Backpressure ----------

/// The most observations a session can have reported once it refuses to go further: a full
/// channel, the rest of the slice that filled it, and the status from one command.
const MOST_UNREAD: usize = OBSERVATION_BACKLOG_LIMIT + 3;

/// Gives a session `ticks` separate advances, without reading anything, until it says it has
/// stopped. Fails if it never does.
fn tick_until_backlogged(watched: &Watched, ticks: u64) -> u64 {
    // Start reads the clock when the session gets to it, so it has to get there first.
    watched.session.sync().unwrap();
    for tick in 1..=ticks {
        watched.clock.set(wall_secs(tick));
        match watched.session.sync() {
            Ok(()) => {}
            Err(SessionError::Backlogged) => return tick,
            Err(error) => panic!("sync failed: {error}"),
        }
    }
    panic!("{ticks} advances without a reader did not stop the session");
}

#[test]
fn sync_returns_at_an_enormous_speed_with_a_recurring_source() {
    finishes("sync at an enormous speed", || {
        let session = session_on(busy_source, WallClock::new());
        session.set_speed(f64::MAX).unwrap();
        session.start().unwrap();
        // The goal saturates and is never reached, and nobody can be reading during a sync.
        assert_eq!(session.sync(), Err(SessionError::Backlogged));
        assert_eq!(session.sync(), Err(SessionError::Backlogged));

        session.halt().unwrap();
        let deadline = Instant::now() + PATIENCE;
        while session.sync() != Ok(()) {
            assert!(Instant::now() < deadline, "a halted session should settle once read");
            session.observations().try_iter().for_each(drop);
        }
    });
}

#[test]
fn a_session_nobody_reads_stops_advancing() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged), "and it stays that way");

    watched.session.halt().unwrap();
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));
    let observations: Vec<Observation> = watched.session.observations().try_iter().collect();
    assert!(observations.len() <= MOST_UNREAD, "{} observations were waiting", observations.len());
    let last_shown = observations
        .iter()
        .rev()
        .find_map(|o| if let Observation::ShownTime(shown) = o { Some(*shown) } else { None })
        .expect("it should have reported shown time");
    assert!(last_shown < secs(HEAVY_CATCH_UP), "it advanced to {last_shown:?}");
}

#[test]
fn an_idle_running_session_does_not_flood_a_reader_that_is_not_there() {
    let mut watched = Watched::new(empty_model);
    watched.session.start().unwrap();
    let limit = OBSERVATION_BACKLOG_LIMIT as u64;
    let stopped_at = tick_until_backlogged(&watched, 2 * limit);
    assert!(stopped_at <= limit, "an advance reports one shown time, so it stopped too late: {stopped_at}");

    // The clock moves on, but the session has stopped: neither this sync nor Halt advances it.
    watched.clock.set(wall_secs(5 * limit));
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));
    watched.session.halt().unwrap();
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));

    let seen = watched.look();
    assert!(seen.shown_times <= MOST_UNREAD, "{} shown times were waiting", seen.shown_times);
    assert_eq!(watched.shown, secs(stopped_at), "it went no further while unread");
}

#[test]
fn a_session_continues_towards_the_clock_once_read() {
    let mut watched = Watched::new(empty_model);
    watched.session.start().unwrap();
    let stopped_at = tick_until_backlogged(&watched, 2 * OBSERVATION_BACKLOG_LIMIT as u64);
    watched.clock.set(wall_secs(stopped_at + 100));
    assert_eq!(watched.time(), secs(stopped_at + 100), "time it spent stopped still counts");
}

#[test]
fn commands_are_handled_while_a_session_waits_for_a_reader() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));

    assert_eq!(watched.session.set_speed(-1.0), Err(SessionError::InvalidSpeed(-1.0)));
    watched.session.set_speed(2.0).unwrap();
    watched.session.reset().unwrap();
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged), "reset was handled, nothing got dropped");

    let seen = watched.look();
    assert_eq!(seen.statuses, vec![SessionStatus::Advancing, SessionStatus::Halted], "in order, after what came before");
    let state = watched.state.as_ref().unwrap();
    assert_eq!(state.time, SimTime::ZERO);
    assert!(!state.initialised);

    watched.at(HEAVY_CATCH_UP + 100);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP + 105);
    assert_eq!(watched.time(), secs(10), "the speed set during the wait applies");
}

#[test]
fn a_panic_with_every_slot_taken_is_still_reported_in_order_after_everything_else() {
    let watched = Watched::new(panics_at_1000);
    watched.session.start().unwrap();
    let stopped_at = tick_until_backlogged(&watched, 2 * OBSERVATION_BACKLOG_LIMIT as u64);
    assert!(stopped_at < 1000, "the panic should not have come yet");
    watched.clock.set(wall_secs(1000));

    // Read one at a time, so that the slice which panics finds the channel full again.
    let mut read = Vec::new();
    let deadline = Instant::now() + PATIENCE;
    while watched.session.sync() != Err(SessionError::Ended) {
        assert!(Instant::now() < deadline, "the session never reached its panic");
        read.push(watched.session.observations().recv_timeout(PATIENCE).unwrap());
    }
    read.extend(watched.session.observations().try_iter());
    let deadline = Instant::now() + PATIENCE;
    while !matches!(read.last(), Some(Observation::Error(SessionError::Panicked(_)))) {
        let left = deadline.saturating_duration_since(Instant::now());
        read.push(watched.session.observations().recv_timeout(left).expect("the panic was never reported"));
    }
    finishes("dropping a session after reporting its panic", move || drop(watched.session));
}

#[test]
fn dropping_a_session_whose_panic_nobody_read_does_not_hang() {
    let watched = Watched::new(panics_at_1000);
    watched.session.start().unwrap();
    tick_until_backlogged(&watched, 2 * OBSERVATION_BACKLOG_LIMIT as u64);
    watched.clock.set(wall_secs(1000));
    let deadline = Instant::now() + PATIENCE;
    while watched.session.sync() != Err(SessionError::Ended) {
        assert!(Instant::now() < deadline, "the session never reached its panic");
        watched.session.observations().recv_timeout(PATIENCE).unwrap();
    }
    finishes("dropping a session whose panic is unread", move || drop(watched.session));
}

#[test]
fn dropping_a_session_nobody_reads_does_not_hang() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    assert_eq!(watched.session.sync(), Err(SessionError::Backlogged));
    finishes("dropping a session waiting for a reader", move || drop(watched.session));
}

// ---------- The deadline on sync ----------

#[test]
fn sync_gives_up_at_its_deadline_on_a_session_that_is_not_getting_anywhere() {
    let (release, blocked) = channel::<()>();
    let session = session_on(
        move || {
            let _ = blocked.recv();
            empty_model()
        },
        FakeClock::default(),
    );
    assert_eq!(session.sync_within(Duration::from_millis(20)), Err(SessionError::SyncTimedOut));

    drop(release);
    assert_eq!(session.sync(), Ok(()), "the session can be used once it is going again");
}

// ---------- Same seed, same outcome ----------

const COMMON_TIME: u64 = 20_000;

fn expected_occurrences() -> Vec<Occurrence> {
    let mut run = line_with_random_failures();
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    occurrences.extend(run.advance_to(secs(COMMON_TIME)).unwrap());
    let stops = occurrences.iter().filter(|o| matches!(o.kind, OccurrenceKind::InterruptionBegan { .. })).count();
    assert!(stops > 10, "the model should depend on its random failures, had {stops} stops");
    assert!(occurrences.len() > 3_000, "the model should take more than one slice");
    occurrences
}

/// Plays a pattern of controls, then checks what the session reported up to the common time.
fn assert_watching_like_this_changes_nothing(pattern: impl FnOnce(&mut Watched, &mut Vec<Occurrence>)) {
    let mut watched = Watched::new(line_with_random_failures);
    let mut occurrences = Vec::new();
    pattern(&mut watched, &mut occurrences);
    let seen = watched.look();
    assert_eq!(seen.errors, vec![]);
    occurrences.extend(seen.occurrences);
    assert!(watched.shown >= secs(COMMON_TIME), "the pattern should reach the common time");

    occurrences.retain(|o| o.time <= secs(COMMON_TIME));
    let expected = expected_occurrences();
    assert_eq!(occurrences.len(), expected.len());
    assert_eq!(occurrences, expected);
}

#[test]
fn the_same_seed_gives_the_same_occurrences_at_any_speed() {
    for speed in [0.25, 1.0, 7.5, 1_000.0] {
        assert_watching_like_this_changes_nothing(|watched, _| {
            watched.session.set_speed(speed).unwrap();
            watched.session.start().unwrap();
            watched.at((COMMON_TIME as f64 / speed).ceil() as u64);
        });
    }
}

#[test]
fn the_same_seed_gives_the_same_occurrences_in_many_small_steps() {
    assert_watching_like_this_changes_nothing(|watched, occurrences| {
        watched.session.start().unwrap();
        for wall in (7..=COMMON_TIME).step_by(7).chain([COMMON_TIME]) {
            watched.at(wall);
            occurrences.extend(watched.look_without_errors());
        }
    });
}

#[test]
fn the_same_seed_gives_the_same_occurrences_however_it_is_halted_and_sped_up() {
    assert_watching_like_this_changes_nothing(|watched, occurrences| {
        let mut wall = 0;
        let speeds = [1.0, 40.0, 0.5, 3.0];
        for (round, speed) in speeds.iter().cycle().take(MAX_ROUNDS).enumerate() {
            watched.session.set_speed(*speed).unwrap();
            watched.session.start().unwrap();
            wall += 20 + round as u64;
            watched.at(wall);
            occurrences.extend(watched.look_without_errors());
            watched.session.halt().unwrap();
            wall += 1_000;
            watched.at(wall);
            if watched.shown >= secs(COMMON_TIME) {
                return;
            }
        }
        panic!("the pattern never reached the common time");
    });
}

#[test]
fn the_same_seed_gives_the_same_occurrences_when_read_only_after_a_long_wait() {
    assert_watching_like_this_changes_nothing(|watched, _| {
        watched.session.start().unwrap();
        // Small advances report often, so nobody reading holds the session up early on.
        let held_up_at = tick_until_backlogged(watched, 2 * OBSERVATION_BACKLOG_LIMIT as u64);
        assert!(held_up_at < COMMON_TIME);
        watched.collect();
        watched.clock.set(wall_secs(COMMON_TIME));
    });
}

#[test]
fn the_same_seed_gives_the_same_occurrences_when_halted_part_way_through_catching_up() {
    assert_watching_like_this_changes_nothing(|watched, occurrences| {
        let mut wall = 0;
        watched.session.set_speed(1_000.0).unwrap();
        for _ in 0..MAX_ROUNDS {
            watched.session.start().unwrap();
            watched.at(wall + 25);
            // No waiting here, so Halt lands wherever the session has got to.
            watched.session.halt().unwrap();
            wall += 1_000;
            occurrences.extend(watched.look_without_errors());
            watched.at(wall);
            if watched.shown >= secs(COMMON_TIME) {
                return;
            }
        }
        panic!("the pattern never reached the common time");
    });
}

// ---------- Shutting down ----------

#[test]
fn a_session_can_be_handed_to_another_thread_and_dropped_there() {
    fn must_be_send<T: Send>(_: &T) {}
    let session = session_on(source_into_drain, FakeClock::default());
    must_be_send(&session);
    finishes("dropping an idle session", move || drop(session));
}

#[test]
fn dropping_a_running_session_does_not_hang() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    finishes("dropping a running session", move || drop(watched.session));
}

#[test]
fn dropping_a_session_part_way_through_a_catch_up_does_not_hang() {
    let mut watched = Watched::new(busy_source);
    watched.session.start().unwrap();
    watched.at(HEAVY_CATCH_UP);
    watched.wait_for_catch_up_to_begin();
    finishes("dropping a session that is catching up", move || drop(watched.session));
}

#[test]
fn dropping_a_session_after_an_error_does_not_hang() {
    let mut watched = Watched::new(zero_time_loop);
    watched.session.start().unwrap();
    assert_eq!(watched.look().errors.len(), 1);
    finishes("dropping a session after an error", move || drop(watched.session));
}

fn wait_for_panic_report(session: &LiveSession) -> String {
    wait_for_observation(session, |observation| match observation {
        Observation::Error(SessionError::Panicked(message)) => Some(message),
        _ => None,
    })
}

#[test]
fn a_panic_while_building_the_run_is_reported_and_hangs_nothing() {
    let session = session_on(|| panic!("this model cannot be built"), FakeClock::default());
    assert_eq!(wait_for_panic_report(&session), "this model cannot be built");
    assert_eq!(session.sync(), Err(SessionError::Ended));
    assert_eq!(session.start(), Err(SessionError::Ended));
    finishes("dropping a session whose thread panicked", move || drop(session));
}

#[test]
fn a_panic_during_a_run_is_reported_and_hangs_nothing() {
    // An edge into a Source panics when the first MU tries to move.
    let mut watched = Watched::new(|| {
        let mut model = ModelDescription::new();
        let first = model.add_source(secs(1));
        let second = model.add_source(secs(1));
        model.connect(first, second);
        model.build(0)
    });
    watched.session.start().unwrap();
    watched.at(5);
    assert_eq!(wait_for_panic_report(&watched.session), "Source cannot receive blocked objects");
    assert_eq!(watched.session.sync(), Err(SessionError::Ended));
    finishes("dropping a session whose thread panicked", move || drop(watched.session));
}

// ---------- Reading ahead ----------

/// What a direct run gives at `t`: same seed, with `done` applied at its times.
enum Done {
    Pause(u32),
    Resume(u32),
    Clear,
}

fn direct_state(done: &[(u64, Done)], t: u64) -> RunState {
    direct(done, t).1
}

fn direct(done: &[(u64, Done)], t: u64) -> (Vec<Occurrence>, RunState) {
    let mut run = line_with_random_failures();
    run.set_recording(Recording::ALL);
    let mut occurrences = run.init().unwrap();
    for (at, what) in done {
        occurrences.extend(run.advance_to(secs(*at)).unwrap());
        match what {
            Done::Pause(id) => run.schedule_event(secs(*at), EventType::PauseOn, *id).unwrap(),
            Done::Resume(id) => run.schedule_event(secs(*at), EventType::PauseOff, *id).unwrap(),
            Done::Clear => run.clear_statistics(),
        }
    }
    occurrences.extend(run.advance_to(secs(t)).unwrap());
    (occurrences, run.state())
}

/// The Station in `line_with_random_failures`.
const STATION: u32 = 1;

#[test]
fn nothing_past_shown_time_is_shown() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(500);
    let seen = watched.look();
    assert!(!seen.occurrences.is_empty());
    assert!(seen.occurrences.iter().all(|o| o.time <= secs(500)), "the run has computed well past this");
}

#[test]
fn a_full_state_comes_only_at_init_after_reset_after_going_back_and_on_request() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    assert_eq!(watched.look().states, 1, "Init");
    watched.at(100);
    assert_eq!(watched.look().states, 0);
    watched.session.request_state().unwrap();
    assert_eq!(watched.look().states, 1, "on request");
    watched.session.go_back(secs(50)).unwrap();
    assert_eq!(watched.look().states, 1, "after going back");
    watched.session.reset().unwrap();
    assert_eq!(watched.look().states, 1, "after Reset");
}

#[test]
fn a_state_on_request_is_at_shown_time_rather_than_computed_time() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(3_000);
    watched.look();
    assert_eq!(watched.state_now(), direct_state(&[], 3_000), "statistics included");
}

#[test]
fn asking_for_a_state_changes_nothing() {
    assert_watching_like_this_changes_nothing(|watched, occurrences| {
        watched.session.start().unwrap();
        for wall in (500..=COMMON_TIME).step_by(500) {
            watched.at(wall);
            occurrences.extend(watched.look_without_errors());
            watched.session.request_state().unwrap();
        }
    });
}

// ---------- Going back ----------

#[test]
fn going_back_gives_the_state_there_and_halts() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(5_000);
    watched.look();

    watched.session.go_back(secs(1_234)).unwrap();
    let seen = watched.look();
    assert_eq!(seen.errors, vec![]);
    assert_eq!(seen.statuses, vec![SessionStatus::Halted]);
    assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[], 1_234));
    assert_eq!(watched.shown, secs(1_234));

    watched.at(6_000);
    assert_eq!(watched.look().shown_times, 0, "it stays there until Start");
}

#[test]
fn start_after_going_back_gives_the_same_future() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(COMMON_TIME);
    let all = watched.look().occurrences;

    watched.session.go_back(secs(7_000)).unwrap();
    watched.look();
    watched.session.start().unwrap();
    watched.at(COMMON_TIME + 13_000);
    let again = watched.look().occurrences;
    let expected: Vec<Occurrence> = all.into_iter().filter(|o| o.time > secs(7_000)).collect();
    assert_eq!(again, expected);
}

#[test]
fn going_back_works_however_far_with_any_checkpoint_settings() {
    for (spacing, budget) in [(1_000, 256_000_000), (100_000, 256_000_000), (500, 40_000), (500, 1)] {
        let settings = SessionSettings { checkpoint_spacing: spacing, checkpoint_budget_bytes: budget, ..SessionSettings::default() };
        let mut watched = Watched::with_settings(line_with_random_failures, settings);
        watched.session.start().unwrap();
        watched.at(COMMON_TIME);
        watched.look();
        for t in [15_000, 9_999, 3_000, 2_999, 0] {
            watched.session.go_back(secs(t)).unwrap();
            watched.look();
            assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[], t), "spacing {spacing}, budget {budget}, t={t}");
        }
    }
}

#[test]
fn going_back_to_a_time_after_shown_time_is_refused() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.at(25);
    watched.look();
    watched.session.go_back(secs(26)).unwrap();
    assert_eq!(watched.look().errors, vec![SessionError::NotInThePast { to: secs(26), shown: secs(25) }]);
    assert_eq!(watched.time(), secs(25), "and it carries on");
}

#[test]
fn going_back_before_init_is_an_error() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.go_back(SimTime::ZERO).unwrap();
    assert_eq!(watched.look().errors, vec![SessionError::Run(RunError::NotInitialised)]);
}

#[test]
fn reset_during_going_back_wins() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(COMMON_TIME);
    watched.look();
    watched.session.go_back(secs(100)).unwrap();
    watched.session.reset().unwrap();
    let seen = watched.look();
    assert!(!watched.state.as_ref().unwrap().initialised);
    assert_eq!(seen.errors, vec![]);
}

#[test]
fn commands_given_during_going_back_are_handled_in_order_after_it() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(COMMON_TIME);
    watched.look();
    watched.session.go_back(secs(100)).unwrap();
    watched.session.go_back(secs(50)).unwrap();
    watched.session.step_forward().unwrap();
    watched.look();
    assert!(watched.shown > secs(50), "it stepped on from the second one");
}

#[test]
fn going_back_gets_a_session_out_of_a_run_error() {
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(30);
    assert_eq!(watched.look().statuses, vec![SessionStatus::Advancing, SessionStatus::Failed]);

    watched.session.go_back(secs(15)).unwrap();
    let seen = watched.look();
    assert_eq!(seen.statuses, vec![SessionStatus::Halted]);
    assert_eq!(watched.shown, secs(15));

    // The error is part of this model's future, so it comes again.
    watched.session.start().unwrap();
    watched.at(50);
    assert_eq!(watched.look().errors, vec![SessionError::Run(RunError::ObjectFailed { object_id: 0, time: secs(25) })]);
}

#[test]
fn start_is_still_refused_after_a_run_error_without_going_back() {
    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(30);
    watched.look();
    watched.session.start().unwrap();
    assert_eq!(watched.look().errors, vec![SessionError::ResetRequired]);
}

// ---------- Stepping forward ----------

#[test]
fn stepping_forward_shows_one_instant_and_stays_halted() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.step_forward().unwrap();
    let seen = watched.look();
    assert_eq!(seen.occurrences[0].kind, OccurrenceKind::RunInitialised, "the first step is Init");
    assert_eq!(seen.statuses, vec![]);

    let mut times = Vec::new();
    for _ in 0..3 {
        watched.session.step_forward().unwrap();
        let seen = watched.look();
        assert!(seen.occurrences.iter().all(|o| o.time == seen.occurrences[0].time), "one instant");
        times.push(seen.occurrences[0].time);
        assert_eq!(watched.shown, seen.occurrences[0].time);
    }
    assert_eq!(times, vec![secs(10), secs(20), secs(30)]);
}

#[test]
fn stepping_gives_the_same_occurrences_as_running() {
    let (expected, _) = direct(&[], 500);
    let mut watched = Watched::new(line_with_random_failures);
    let mut occurrences = Vec::new();
    while watched.shown < secs(500) {
        watched.session.step_forward().unwrap();
        occurrences.extend(watched.look_without_errors());
    }
    occurrences.retain(|o| o.time <= secs(500));
    assert_eq!(occurrences, expected);
}

#[test]
fn stepping_back_and_forward_again_is_exact() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.halt().unwrap();
    watched.session.go_back(secs(400)).unwrap();
    watched.look();
    watched.session.step_forward().unwrap();
    let forward = watched.look().occurrences;
    let expected_time = forward[0].time;
    assert!(expected_time > secs(400));

    let (all, _) = direct(&[], expected_time.as_secs() + 1);
    let expected: Vec<Occurrence> = all.into_iter().filter(|o| o.time == expected_time).collect();
    assert_eq!(forward, expected);
}

#[test]
fn stepping_forward_does_nothing_while_advancing() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.start().unwrap();
    watched.look();
    watched.session.step_forward().unwrap();
    assert!(watched.look().occurrences.is_empty());
    assert_eq!(watched.shown, SimTime::ZERO);
}

#[test]
fn stepping_forward_with_no_next_instant_does_nothing() {
    let mut watched = Watched::new(empty_model);
    watched.session.step_forward().unwrap();
    watched.look();
    watched.session.step_forward().unwrap();
    let seen = watched.look();
    assert!(seen.occurrences.is_empty());
    assert_eq!(seen.errors, vec![]);
}

#[test]
fn stepping_forward_into_a_run_error_reports_it() {
    let mut watched = Watched::new(fails_at_25);
    for _ in 0..3 {
        watched.session.step_forward().unwrap();
    }
    assert_eq!(watched.look().errors, vec![SessionError::Run(RunError::ObjectFailed { object_id: 0, time: secs(25) })]);
    assert_eq!(watched.shown, secs(25));
}

// ---------- Interventions ----------

#[test]
fn a_pause_takes_effect_at_shown_time_without_a_halt() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();

    watched.session.pause(STATION).unwrap();
    let seen = watched.look();
    assert_eq!(seen.statuses, vec![], "still advancing");
    let paused = Occurrence { time: secs(1_000), kind: OccurrenceKind::InterruptionBegan { object_id: STATION, cause: StopCause::Pause } };
    assert!(seen.occurrences.contains(&paused), "{:?}", seen.occurrences);

    watched.at(1_100);
    watched.look();
    let state = watched.state_now();
    assert_eq!(state, direct_state(&[(1_000, Done::Pause(STATION))], 1_100));
}

#[test]
fn a_pause_while_halted_shows_at_once() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.session.halt().unwrap();
    watched.look();
    watched.session.pause(STATION).unwrap();
    let seen = watched.look();
    assert!(seen.occurrences.iter().any(|o| o.kind == OccurrenceKind::InterruptionBegan { object_id: STATION, cause: StopCause::Pause }));
    assert_eq!(watched.shown, secs(1_000));
}

#[test]
fn the_same_seed_and_the_same_interventions_give_the_same_result() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    let mut occurrences = Vec::new();
    for (wall, act) in [(1_000, true), (1_500, false), (4_000, true), (4_200, false)] {
        watched.at(wall);
        occurrences.extend(watched.look_without_errors());
        if act {
            watched.session.pause(STATION).unwrap();
        } else {
            watched.session.resume(STATION).unwrap();
        }
    }
    watched.at(COMMON_TIME);
    occurrences.extend(watched.look_without_errors());
    let done = [(1_000, Done::Pause(STATION)), (1_500, Done::Resume(STATION)), (4_000, Done::Pause(STATION)), (4_200, Done::Resume(STATION))];
    let (expected, state) = direct(&done, COMMON_TIME);
    assert_eq!(occurrences.len(), expected.len());
    assert!(occurrences == expected);
    assert_eq!(watched.state_now(), state);
}

#[test]
fn replay_applies_an_intervention_it_passes() {
    // With checkpoints this far apart, the one to restore comes from before the pause.
    let settings = SessionSettings { checkpoint_spacing: 1_000_000, ..SessionSettings::default() };
    let mut watched = Watched::with_settings(line_with_random_failures, settings);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.at(5_000);
    watched.look();
    watched.session.go_back(secs(3_000)).unwrap();
    watched.look();
    assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[(1_000, Done::Pause(STATION))], 3_000));
}

#[test]
fn going_back_behind_an_intervention_removes_it() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.at(2_000);
    watched.look();
    watched.session.go_back(secs(500)).unwrap();
    watched.look();
    watched.session.start().unwrap();
    watched.at(3_000);
    watched.look();
    // Start counts from its own wall time, so shown time went from 500 by 1,000 more.
    assert_eq!(watched.state_now(), direct_state(&[], 1_500), "as if it had never been paused");
}

#[test]
fn pausing_what_is_already_paused_does_nothing() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.at(1_100);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.session.resume(0 + STATION).unwrap();
    watched.session.resume(STATION).unwrap();
    assert_eq!(watched.look().errors, vec![]);
    let done = [(1_000, Done::Pause(STATION)), (1_100, Done::Resume(STATION))];
    assert_eq!(watched.state_now(), direct_state(&done, 1_100));
}

#[test]
fn only_a_station_can_be_paused_so_far() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.look();
    watched.session.pause(0).unwrap();
    watched.session.pause(2).unwrap();
    watched.session.resume(0).unwrap();
    watched.session.pause(99).unwrap();
    assert_eq!(
        watched.look().errors,
        vec![
            SessionError::CannotPause(0),
            SessionError::CannotPause(2),
            SessionError::CannotPause(0),
            SessionError::Run(RunError::UnknownObject(99)),
        ]
    );
}

#[test]
fn an_intervention_before_init_or_after_a_run_error_is_refused() {
    let mut watched = Watched::new(source_into_drain);
    watched.session.clear_statistics().unwrap();
    assert_eq!(watched.look().errors, vec![SessionError::Run(RunError::NotInitialised)]);

    let mut watched = Watched::new(fails_at_25);
    watched.session.start().unwrap();
    watched.at(30);
    watched.look();
    watched.session.clear_statistics().unwrap();
    assert_eq!(watched.look().errors, vec![SessionError::ResetRequired]);
}

#[test]
fn reset_removes_interventions() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.look();
    watched.session.reset().unwrap();
    watched.at(1_001);
    watched.session.start().unwrap();
    watched.at(1_501);
    watched.look();
    assert_eq!(watched.state_now(), direct_state(&[], 500));
}

#[test]
fn a_warm_up_is_remembered_like_an_intervention() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.clear_statistics().unwrap();
    watched.at(3_000);
    watched.look();
    assert_eq!(watched.state_now(), direct_state(&[(1_000, Done::Clear)], 3_000));
    assert_eq!(watched.state.as_ref().unwrap().statistics_since, secs(1_000));

    // Going back to after it keeps it, and going back behind it removes it.
    watched.session.go_back(secs(2_000)).unwrap();
    watched.look();
    assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[(1_000, Done::Clear)], 2_000));
    watched.session.go_back(secs(500)).unwrap();
    watched.look();
    assert_eq!(watched.state.as_ref().unwrap().statistics_since, SimTime::ZERO);
    assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[], 500));
}

// ---------- Time modes ----------

/// Source (one MU every 1,000s) -> Drain, for gaps long enough to skip.
fn slow_source() -> SimulationRun {
    source_every(secs(1_000)).build(0)
}

const SKIPPING: TimeMode = TimeMode::Skipping { longest_gap: Duration::from_secs(1) };

#[test]
fn skipping_keeps_a_long_gap_to_the_longest_allowed() {
    let settings = SessionSettings { time_mode: SKIPPING, ..SessionSettings::default() };
    let mut watched = Watched::with_settings(slow_source, settings);
    watched.session.start().unwrap();
    watched.look();
    assert_eq!(watched.shown, secs(999), "it has jumped to 1s before the next instant");
    // At speed 1 this gap would take 1,000s. It takes 1s, and shown time jumps over the rest.
    watched.at(1);
    let seen = watched.look();
    assert!(seen.occurrences.iter().any(|o| o.time == secs(1_000)), "{:?}", seen.occurrences);
    assert_eq!(watched.shown, secs(1_999), "and over the next gap as soon as that instant is out");
    watched.at(2);
    assert!(watched.look().occurrences.iter().any(|o| o.time == secs(2_000)));
}

#[test]
fn skipping_starts_with_a_longest_gap_of_a_quarter_of_a_second() {
    assert_eq!(TimeMode::SKIPPING, TimeMode::Skipping { longest_gap: Duration::from_millis(250) });
}

#[test]
fn skipping_leaves_short_gaps_at_the_speed() {
    let settings = SessionSettings { time_mode: SKIPPING, ..SessionSettings::default() };
    let mut watched = Watched::with_settings(source_into_drain, settings);
    watched.session.set_speed(100.0).unwrap();
    watched.session.start().unwrap();
    watched.look();
    // A 10s gap takes 0.1s at this speed, under the longest allowed, so it keeps its proportion.
    watched.clock.set(Duration::from_millis(50));
    watched.look();
    assert_eq!(watched.shown, secs(5));
    watched.clock.set(Duration::from_millis(100));
    let seen = watched.look();
    assert_eq!(watched.shown, secs(10));
    assert!(seen.occurrences.iter().any(|o| o.time == secs(10)));
}

#[test]
fn the_time_mode_can_change_at_any_time() {
    let mut watched = Watched::new(slow_source);
    watched.session.start().unwrap();
    watched.at(100);
    assert_eq!(watched.time(), secs(100));
    watched.session.set_time_mode(SKIPPING).unwrap();
    watched.at(101);
    assert_eq!(watched.time(), secs(1_999));
    watched.session.set_time_mode(TimeMode::Continuous).unwrap();
    watched.at(151);
    assert_eq!(watched.time(), secs(2_049), "back to the speed, with no more jumps");
}

#[test]
fn the_time_mode_changes_no_results() {
    let settings = SessionSettings { time_mode: TimeMode::Skipping { longest_gap: Duration::from_millis(1) }, ..SessionSettings::default() };
    let mut watched = Watched::with_settings(line_with_random_failures, settings);
    let mut occurrences = Vec::new();
    watched.session.start().unwrap();
    for wall in 1..=100_000 {
        if watched.shown >= secs(COMMON_TIME) {
            break;
        }
        watched.at(wall);
        occurrences.extend(watched.look_without_errors());
    }
    assert!(watched.shown >= secs(COMMON_TIME));
    occurrences.retain(|o| o.time <= secs(COMMON_TIME));
    assert_eq!(occurrences, expected_occurrences());
}

// ---------- What it takes to rebuild the picture ----------

/// What a UI needs to draw an object, built only from one full state and occurrences.
#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    contents: Vec<u32>,
    process: crabsim::ProcessState,
    stop: Option<StopCause>,
    remaining: Option<SimTime>,
}

/// One object, while a UI rebuilds it.
#[derive(Clone)]
struct Rebuilt {
    contents: Vec<u32>,
    process: crabsim::ProcessState,
    failed: bool,
    paused: bool,
    completion: Option<SimTime>,
    frozen: Option<SimTime>,
}

impl Rebuilt {
    fn from_state(state: &ObjectState, now: SimTime) -> Rebuilt {
        let stopped = state.stop.is_some();
        let remaining = state.remaining;
        Rebuilt {
            contents: state.contents.clone(),
            process: state.process.clone(),
            failed: state.stop == Some(StopCause::Failure),
            paused: state.stop == Some(StopCause::Pause),
            completion: remaining.filter(|_| !stopped).map(|left| now + left),
            frozen: remaining.filter(|_| stopped),
        }
    }

    fn drawn(&self, now: SimTime) -> Drawn {
        let stop = if self.failed {
            Some(StopCause::Failure)
        } else if self.paused {
            Some(StopCause::Pause)
        } else {
            None
        };
        let remaining = self.frozen.or_else(|| self.completion.map(|completion| completion.saturating_sub(now)));
        Drawn { contents: self.contents.clone(), process: self.process.clone(), stop, remaining }
    }

    fn stopped(&self) -> bool {
        self.failed || self.paused
    }
}

fn apply_occurrence(objects: &mut [Rebuilt], occurrence: &Occurrence) {
    use crabsim::ProcessState::{Blocked, Idle, Processing};
    let now = occurrence.time;
    match occurrence.kind {
        OccurrenceKind::RunInitialised => {}
        OccurrenceKind::MuCreated { mu_id, at, duration } => {
            let object = &mut objects[at as usize];
            object.contents = vec![mu_id];
            object.process = Processing;
            object.completion = Some(now + duration);
        }
        OccurrenceKind::MuMoved { mu_id, from, to } => {
            let object = &mut objects[from as usize];
            object.contents.retain(|&id| id != mu_id);
            object.process = Idle;
            object.completion = None;
            if !matches!(objects.get(to as usize), None) {
                objects[to as usize].contents.push(mu_id);
            }
        }
        OccurrenceKind::MuLeft { mu_id, at, .. } => objects[at as usize].contents.retain(|&id| id != mu_id),
        OccurrenceKind::WorkStarted { at, duration, .. } => {
            let object = &mut objects[at as usize];
            object.process = Processing;
            object.completion = Some(now + duration);
        }
        OccurrenceKind::WorkFinished { at, .. } => objects[at as usize].completion = None,
        OccurrenceKind::Blocked { object_id } => {
            objects[object_id as usize].process = Blocked;
            objects[object_id as usize].completion = None;
        }
        OccurrenceKind::Unblocked { object_id } => {
            // A Source unblocks by creating its next MU, which has already set what it is doing.
            let object = &mut objects[object_id as usize];
            if object.process == Blocked {
                object.process = Idle;
            }
        }
        OccurrenceKind::InterruptionBegan { object_id, cause } => {
            let object = &mut objects[object_id as usize];
            if !object.stopped() {
                object.frozen = object.completion.take().map(|completion| completion.saturating_sub(now));
            }
            match cause {
                StopCause::Failure => object.failed = true,
                StopCause::Pause => object.paused = true,
            }
        }
        OccurrenceKind::InterruptionEnded { object_id, cause } => {
            let object = &mut objects[object_id as usize];
            match cause {
                StopCause::Failure => object.failed = false,
                StopCause::Pause => object.paused = false,
            }
            if !object.stopped() {
                object.completion = object.frozen.take().map(|left| now + left);
            }
        }
    }
}

/// Plays a session through `steps`, rebuilding the picture only from its first state and its
/// occurrences, and compares it with the run's own state at each of them.
fn assert_picture_rebuilds(build: fn() -> SimulationRun, steps: &[u64], actions: &[(u64, u32)]) {
    let mut watched = Watched::new(build);
    watched.session.start().unwrap();
    let mut seen = watched.look();
    let first = watched.state.clone().expect("the state from Init");
    let mut objects: Vec<Rebuilt> = first.objects.iter().map(|o| Rebuilt::from_state(o, first.time)).collect();

    let mut compared = 0;
    let mut pending = actions.iter();
    let mut paused = false;
    let mut next_action = pending.next();
    for &step in steps {
        while let Some(&(at, id)) = next_action.filter(|(at, _)| *at <= step) {
            watched.at(at);
            seen.occurrences.extend(watched.look().occurrences);
            if paused {
                watched.session.resume(id).unwrap();
            } else {
                watched.session.pause(id).unwrap();
            }
            paused = !paused;
            next_action = pending.next();
        }
        watched.at(step);
        seen.occurrences.extend(watched.look().occurrences);
        // The Init state already holds what Init did.
        for occurrence in seen.occurrences.drain(..).filter(|o| o.time > SimTime::ZERO) {
            apply_occurrence(&mut objects, &occurrence);
        }
        let now = secs(step);
        let state = watched.state_now();
        assert_eq!(state.time, now);
        for (rebuilt, own) in objects.iter().zip(&state.objects) {
            let expected = Drawn { contents: own.contents.clone(), process: own.process.clone(), stop: own.stop, remaining: own.remaining };
            assert_eq!(rebuilt.drawn(now), expected, "object {} at t={step}", own.id);
            compared += 1;
        }
    }
    assert!(compared >= steps.len());
}

#[test]
fn the_picture_rebuilds_from_occurrences_with_random_failures() {
    assert_picture_rebuilds(line_with_random_failures, &[1, 9, 10, 18, 77, 500, 1_234, 5_000, 12_345, 20_000], &[]);
}

#[test]
fn the_picture_rebuilds_from_occurrences_with_pauses_overlapping_failures() {
    assert_picture_rebuilds(
        line_with_random_failures,
        &[100, 300, 700, 1_000, 1_001, 2_500, 4_000, 8_000],
        &[(200, STATION), (250, STATION), (600, STATION), (1_500, STATION), (3_000, STATION), (6_000, STATION)],
    );
}

/// A blocking model: Source -> slow Station -> slower Station -> Drain, all with failures.
fn blocking_line() -> SimulationRun {
    let failures = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    let src = model.add_source(secs(2));
    let a = model.add_station(secs(5), failures.clone());
    let b = model.add_station(secs(9), failures);
    let dr = model.add_drain();
    model.connect(src, a);
    model.connect(a, b);
    model.connect(b, dr);
    model.build(11)
}

#[test]
fn the_picture_rebuilds_from_occurrences_with_blocking() {
    assert_picture_rebuilds(blocking_line, &[1, 2, 5, 11, 40, 333, 1_000, 2_500, 7_777], &[]);
}

#[test]
fn the_picture_rebuilds_from_occurrences_on_the_demo_model() {
    // Branches, several predecessors and a Station that often blocks, paused along the way.
    assert_picture_rebuilds(
        || crabsim::demo::build_model(3),
        &[1, 7, 60, 301, 1_000, 3_600, 3_601, 10_000, 40_000],
        &[(500, 3), (900, 3), (3_000, 3), (20_000, 3)],
    );
}

#[test]
fn a_checkpoint_knows_which_interventions_it_already_holds() {
    // Checkpoints land well after the warm-up, so going back from one of those must not clear
    // statistics again. A slice is about 3,000 simulated seconds on this model.
    let settings = SessionSettings { checkpoint_spacing: 300, ..SessionSettings::default() };
    let mut watched = Watched::with_settings(line_with_random_failures, settings);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.clear_statistics().unwrap();
    watched.at(20_000);
    watched.look();
    watched.session.go_back(secs(15_000)).unwrap();
    watched.look();
    let state = watched.state.as_ref().unwrap();
    assert_eq!(state, &direct_state(&[(1_000, Done::Clear)], 15_000));
    assert_eq!(state.statistics_since, secs(1_000));
}

#[test]
fn going_back_to_the_very_time_of_an_intervention_keeps_it() {
    let mut watched = Watched::new(line_with_random_failures);
    watched.session.start().unwrap();
    watched.at(1_000);
    watched.look();
    watched.session.pause(STATION).unwrap();
    watched.at(1_500);
    watched.look();
    watched.session.go_back(secs(1_000)).unwrap();
    watched.look();
    assert_eq!(watched.state.as_ref().unwrap(), &direct_state(&[(1_000, Done::Pause(STATION))], 1_000));
    assert_eq!(object_stop(watched.state.as_ref().unwrap()), Some(StopCause::Pause));
}

fn object_stop(state: &RunState) -> Option<StopCause> {
    state.objects.iter().find(|o| o.id == STATION).unwrap().stop
}
