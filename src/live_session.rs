//! A simulation run on its own thread, controlled live: commands go in, outputs come
//! out. This knows nothing about any UI technology, and takes its clock from outside.
//!
//! The run computes ahead as fast as it can, into a bounded buffer, and the clock releases what
//! it computed. So what the user experiences follows shown time, while the run is at a later
//! computed time. To reach an earlier instant, the session restores the nearest earlier
//! checkpoint and replays. See ADR 0002.

use crate::error::RunError;
use crate::event::EventType;
use crate::materialflow_objects::ModelObject;
use crate::report::{Occurrence, Recording, RunState};
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;
use std::{
    any::Any,
    collections::VecDeque,
    fmt,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long a running session waits for a command before it looks at the clock again.
const TICK: Duration = Duration::from_millis(16);

/// How many events a session computes before it checks for commands again.
const EVENTS_PER_SLICE: usize = 1_000;

/// How many instants a session releases before it checks for commands again.
const INSTANTS_PER_RELEASE: usize = 1_000;

/// How many events a replay processes before it checks for commands again. A replay is faster
/// than computing, because it records nothing.
const EVENTS_PER_REPLAY_SLICE: usize = 10_000;

/// How many unread outputs a session lets build up before it stops releasing until they are
/// read. It keeps handling commands meanwhile, and no output is ever dropped.
pub const OUTPUT_BACKLOG_LIMIT: usize = 256;

/// How long `sync` waits before it gives up. Far longer than a session needs to reach the
/// clock or a backlog, so only a session stuck on something else runs into this.
const SYNC_DEADLINE: Duration = Duration::from_secs(30);

/// Where a session's wall time comes from. Only differences between readings matter.
pub trait Clock: Send + 'static {
    fn elapsed(&self) -> Duration;
}

/// Real wall time, counted from when the clock was created.
pub struct WallClock {
    created: Instant,
}

impl WallClock {
    pub fn new() -> WallClock {
        WallClock { created: Instant::now() }
    }
}

impl Default for WallClock {
    fn default() -> WallClock {
        WallClock::new()
    }
}

impl Clock for WallClock {
    fn elapsed(&self) -> Duration {
        self.created.elapsed()
    }
}

/// How shown time passes between instants. Neither changes what happens in the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeMode {
    /// Shown time passes at the session's speed throughout.
    Continuous,
    /// As Continuous, but no gap between instants takes longer than `longest_gap` of wall time.
    /// Shown time jumps over the rest of a longer gap.
    Skipping { longest_gap: Duration },
}

impl TimeMode {
    /// Skipping with the longest gap to start with.
    pub const SKIPPING: TimeMode = TimeMode::Skipping { longest_gap: Duration::from_millis(250) };
}

/// A session's settings, fixed when it is created except for the time mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionSettings {
    /// How many instants the run may compute ahead of what has been released. It can pass this
    /// by up to one slice of events, because it checks between slices.
    pub buffer_instants: usize,
    /// How many events apart checkpoints are taken.
    pub checkpoint_spacing: u64,
    /// How much memory checkpoints may take. Past this, every other checkpoint in the older half
    /// goes, so the whole run stays reachable and only long jumps take longer.
    pub checkpoint_budget_bytes: u64,
    pub time_mode: TimeMode,
    /// Whether Reset keeps settings changed during the run, such as a process time, as the new
    /// default state. Momentary interventions, such as a Pause, go either way.
    pub reset_keeps_changes: bool,
}

impl Default for SessionSettings {
    fn default() -> SessionSettings {
        SessionSettings {
            buffer_instants: 100_000,
            checkpoint_spacing: 100_000,
            checkpoint_budget_bytes: 256_000_000,
            time_mode: TimeMode::Continuous,
            reset_keeps_changes: false,
        }
    }
}

/// What a session reports. Occurrences arrive in order, and concatenated they are what
/// advancing the run directly would have returned, up to any going back. A full state comes only
/// at Init, after Reset, after going back and on request, and a status follows every change of
/// status.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionOutput {
    Occurrences(Vec<Occurrence>),
    /// Long stretches can pass with no occurrences, and shown time still moves.
    ShownTime(SimTime),
    State(RunState),
    Status(SessionStatus),
    Error(SessionError),
    /// The answer to `locate_step_back`: from shown time `shown`, stepping back would go to the
    /// instant `to`, or nowhere from the first instant.
    StepBackTarget { shown: SimTime, to: Option<SimTime> },
}

/// Whether a session is advancing shown time. A new session is `Halted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Advancing,
    Halted,
    /// Halted by itself at the end time, with everything at it done. Start and stepping forward
    /// are refused until the end time moves later or is cleared.
    ReachedEnd,
    /// Halted by a run error. Reset or going back gets it going again.
    Failed,
}

/// Why a session refused a command or could not continue.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// The run reported an error, at shown time. The session has halted, and Reset or going back
    /// gets it going again.
    Run(RunError),
    /// Refused, because the run has reported an error at shown time. Reset or go back first.
    ResetRequired,
    /// A speed has to be positive and finite.
    InvalidSpeed(f64),
    /// Only a Station can be paused, forced to fail or have its process time changed, so far.
    NotAStation(u32),
    /// Going back goes to shown time or earlier.
    NotInThePast { to: SimTime, shown: SimTime },
    /// Refused, because shown time is at the end time. Move the end time later or clear it first.
    AtEndTime(SimTime),
    /// An end time has to be later than shown time. Go back first for an earlier one.
    EndNotAhead { end: SimTime, shown: SimTime },
    /// The session thread panicked and has ended. Only a new session can continue.
    Panicked(String),
    /// `sync` stopped waiting because too many outputs are unread, so the session won't
    /// go further until they are read. Earlier commands have been handled.
    Backlogged,
    /// `sync` reached its deadline before the session got there.
    SyncTimedOut,
    /// The session thread is gone.
    Ended,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Run(error) => write!(f, "{error}"),
            SessionError::ResetRequired => write!(f, "the simulation run reported an error; reset it or go back first"),
            SessionError::InvalidSpeed(speed) => write!(f, "{speed} is not a speed: it has to be positive and finite"),
            SessionError::NotAStation(id) => write!(f, "object {id} is not a Station, and only a Station can do this so far"),
            SessionError::NotInThePast { to, shown } => write!(f, "can't go back to {to}s: shown time is {shown}s"),
            SessionError::AtEndTime(end) => {
                write!(f, "the run has reached its end time, {}: move or clear it to go on", crate::time::format_time_exact(*end))
            }
            SessionError::EndNotAhead { end, shown } => write!(
                f,
                "the end time has to be later than shown time, {}, not {}: go back first for an earlier one",
                crate::time::format_time_exact(*shown),
                crate::time::format_time_exact(*end)
            ),
            SessionError::Panicked(message) => write!(f, "the live session panicked: {message}"),
            SessionError::Backlogged => write!(f, "too many outputs are unread; read them first"),
            SessionError::SyncTimedOut => write!(f, "the live session did not catch up in time"),
            SessionError::Ended => write!(f, "the live session has ended"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Something done to a run from outside, at shown time, which replay has to do again.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Pause(u32),
    Resume(u32),
    ClearStatistics,
    SetProcessTime(u32, SimTime),
    ForceFailure(u32),
    EndForcedFailure(u32),
}

/// An interruption the user switches on and off.
#[derive(Clone, Copy)]
enum Switch {
    Pause,
    ForcedFailure,
}

enum Command {
    Start,
    Halt,
    Reset,
    SetSpeed(f64),
    SetTimeMode(TimeMode),
    GoBack(SimTime),
    StepForward,
    StepBack,
    LocateStepBack,
    SetResetKeepsChanges(bool),
    SetEndTime(Option<SimTime>),
    RequestState,
    Intervene(Action),
    Sync(Sender<Result<(), SessionError>>),
}

/// The handle to a simulation run living on its own thread. Dropping it ends the thread.
pub struct LiveSession {
    commands: Sender<Command>,
    outputs: Receiver<SessionOutput>,
    thread: Option<JoinHandle<()>>,
}

impl LiveSession {
    /// A session with the default settings. See `with_settings`.
    pub fn new<B, C>(build: B, clock: C) -> LiveSession
    where
        B: FnOnce() -> SimulationRun + Send + 'static,
        C: Clock,
    {
        LiveSession::with_settings(build, clock, SessionSettings::default())
    }

    /// Creates the session's thread and builds the whole run on it. The session starts out
    /// halted, at speed 1, and records everything, because its observers rebuild what they show
    /// from occurrences.
    pub fn with_settings<B, C>(build: B, clock: C, settings: SessionSettings) -> LiveSession
    where
        B: FnOnce() -> SimulationRun + Send + 'static,
        C: Clock,
    {
        let (commands, command_inbox) = channel();
        let (observer, outputs) = sync_channel(OUTPUT_BACKLOG_LIMIT);
        let thread = thread::spawn(move || {
            let mut outbox = Outbox { observer, unsent: VecDeque::new() };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                SessionThread::new(build(), clock, command_inbox, &mut outbox, settings).run();
            }));
            if let Err(panic) = outcome {
                // By now the command channel is closed, so commands find the session has ended.
                outbox.send(SessionOutput::Error(SessionError::Panicked(panic_message(panic))));
                while outbox.is_backlogged() {
                    thread::sleep(TICK);
                    outbox.flush();
                }
            }
        });
        LiveSession { commands, outputs, thread: Some(thread) }
    }

    /// Starts or continues advancing shown time, doing Init first if the run needs it. Does
    /// nothing if the session is already advancing.
    pub fn start(&self) -> Result<(), SessionError> {
        self.send(Command::Start)
    }

    /// Stops shown time advancing and keeps everything exactly as it is. Wall time doesn't count
    /// until Start. Does nothing if the session is already halted.
    pub fn halt(&self) -> Result<(), SessionError> {
        self.send(Command::Halt)
    }

    /// Returns the run to its default state, at any time. It stays there until Start. The
    /// speed is a setting of the session rather than the run, so it stays too.
    pub fn reset(&self) -> Result<(), SessionError> {
        self.send(Command::Reset)
    }

    /// Sets how much simulation time passes per unit of wall time, from now on.
    pub fn set_speed(&self, speed: f64) -> Result<(), SessionError> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(SessionError::InvalidSpeed(speed));
        }
        self.send(Command::SetSpeed(speed))
    }

    /// Sets how shown time passes between instants, from now on. Results don't depend on it.
    pub fn set_time_mode(&self, mode: TimeMode) -> Result<(), SessionError> {
        self.send(Command::SetTimeMode(mode))
    }

    /// Goes to simulation time `to`, at shown time or earlier, after everything at that instant.
    /// The session halts there and reports a full state, also out of a run error. The computed
    /// future is gone, and so is anything done from outside after `to`.
    pub fn go_back(&self, to: SimTime) -> Result<(), SessionError> {
        self.send(Command::GoBack(to))
    }

    /// Sets the end time, at which the session halts by itself with everything at it done, or
    /// clears it with `None`. It must be later than shown time. It isn't an intervention: it
    /// changes when a run stops, not what happens in it, so it isn't logged, and Reset and going
    /// back keep it, like the speed. Computing never goes past it, so it costs nothing per event.
    pub fn set_end_time(&self, end: Option<SimTime>) -> Result<(), SessionError> {
        self.send(Command::SetEndTime(end))
    }

    /// While halted, shows the next instant and stays halted. Does nothing while advancing, or if
    /// there is no next instant.
    pub fn step_forward(&self) -> Result<(), SessionError> {
        self.send(Command::StepForward)
    }

    /// Goes back to the instant before the one on screen, which is the last instant at or before
    /// shown time, and leaves the session halted there, as `go_back` does. It finds that instant
    /// by replaying from the nearest checkpoint, so it costs about as much as going back, and
    /// nothing has to remember past instants. From the first instant it does nothing.
    pub fn step_back(&self) -> Result<(), SessionError> {
        self.send(Command::StepBack)
    }

    /// Says where `step_back` would go from shown time, as a `StepBackTarget`, and changes
    /// nothing. A UI that warns before dropping interventions needs to know, and costs the same
    /// replay as stepping back, so a UI asks only when it has interventions to warn about.
    pub fn locate_step_back(&self) -> Result<(), SessionError> {
        self.send(Command::LocateStepBack)
    }

    /// Whether Reset keeps changed settings, such as a process time, as the new default state.
    /// It is read when Reset is handled, and is off to start with.
    pub fn set_reset_keeps_changes(&self, keep: bool) -> Result<(), SessionError> {
        self.send(Command::SetResetKeepsChanges(keep))
    }

    /// Asks for a full state at shown time, statistics included. It comes from a copy of the run,
    /// which takes one replay, so ask a few times a second rather than every frame.
    pub fn request_state(&self) -> Result<(), SessionError> {
        self.send(Command::RequestState)
    }

    /// Pauses a Station at shown time, without a Halt. The computed future is gone, and Reset or
    /// going back behind it undoes it. Pausing what is already paused does nothing.
    pub fn pause(&self, object_id: u32) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::Pause(object_id)))
    }

    /// Ends a pause at shown time, on the same terms as `pause`.
    pub fn resume(&self, object_id: u32) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::Resume(object_id)))
    }

    /// Forces a Station to fail at shown time, without a Halt, until `end_forced_failure`. It
    /// counts as a failure, and random failures carry on underneath it. The computed future is
    /// gone, and Reset or going back behind it undoes it. Forcing what is already forced does
    /// nothing.
    pub fn force_failure(&self, object_id: u32) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::ForceFailure(object_id)))
    }

    /// Ends a forced failure at shown time, on the same terms as `force_failure`.
    pub fn end_forced_failure(&self, object_id: u32) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::EndForcedFailure(object_id)))
    }

    /// Changes a Station's process time at shown time, without a Halt. The MU in work keeps its
    /// time and the next MU uses the new one. The computed future is gone, and Reset or going
    /// back behind it undoes it.
    pub fn set_process_time(&self, object_id: u32, process_time: SimTime) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::SetProcessTime(object_id, process_time)))
    }

    /// Starts statistics afresh at shown time, such as at the end of a warm-up. Going back behind
    /// it undoes it.
    pub fn clear_statistics(&self) -> Result<(), SessionError> {
        self.send(Command::Intervene(Action::ClearStatistics))
    }

    /// Blocks until the session has handled every earlier command and advanced to the
    /// clock's current time, with everything that produced waiting in `outputs`.
    ///
    /// It always returns. `Backlogged` means earlier commands are handled but the session
    /// won't go further until outputs are read, so read them and sync again. `SyncTimedOut`
    /// means neither happened by a generous deadline.
    pub fn sync(&self) -> Result<(), SessionError> {
        self.sync_within(SYNC_DEADLINE)
    }

    /// `sync` with a deadline of your choosing.
    pub fn sync_within(&self, deadline: Duration) -> Result<(), SessionError> {
        let (done, wait) = channel();
        self.send(Command::Sync(done))?;
        match wait.recv_timeout(deadline) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => Err(SessionError::SyncTimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(SessionError::Ended),
        }
    }

    pub fn outputs(&self) -> &Receiver<SessionOutput> {
        &self.outputs
    }

    fn send(&self, command: Command) -> Result<(), SessionError> {
        self.commands.send(command).map_err(|_| SessionError::Ended)
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        // The thread ends when it finds its command channel closed.
        (self.commands, _) = channel();
        // A thread that panicked stays until its report is out, and nothing else will read it.
        while self.outputs.recv().is_ok() {}
        if let Some(thread) = self.thread.take() {
            // A panic has already been reported as an output.
            let _ = thread.join();
        }
    }
}

fn panic_message(panic: Box<dyn Any + Send>) -> String {
    match panic.downcast::<String>() {
        Ok(message) => *message,
        Err(panic) => panic.downcast_ref::<&str>().map_or_else(|| "unknown panic".to_string(), |m| m.to_string()),
    }
}

/// The wall time and simulation time a running session measures from.
#[derive(Clone, Copy)]
struct Pace {
    wall: Duration,
    simulation: SimTime,
}

impl Pace {
    /// The simulation time this pace has reached at wall time `now`.
    fn target(&self, now: Duration, speed: f64) -> SimTime {
        let passed = now.saturating_sub(self.wall).as_secs_f64() * speed;
        self.simulation.saturating_add(SimTime::from_secs_f64(passed))
    }
}

/// A `sync` waiting for shown time to reach what the clock showed when it arrived.
struct Waiter {
    goal: SimTime,
    done: Sender<Result<(), SessionError>>,
}

/// Outputs on their way out. Sending never blocks the session thread, so it can always
/// handle commands. What the channel has no room for waits here, in order, and a session
/// doesn't release while anything is waiting, so only commands can add more.
struct Outbox {
    observer: SyncSender<SessionOutput>,
    unsent: VecDeque<SessionOutput>,
}

impl Outbox {
    fn send(&mut self, output: SessionOutput) {
        self.unsent.push_back(output);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(output) = self.unsent.pop_front() {
            match self.observer.try_send(output) {
                Ok(()) => {}
                Err(TrySendError::Full(output)) => {
                    self.unsent.push_front(output);
                    return;
                }
                // Nobody is listening once the handle is gone.
                Err(TrySendError::Disconnected(_)) => self.unsent.clear(),
            }
        }
    }

    fn is_backlogged(&self) -> bool {
        !self.unsent.is_empty()
    }
}

/// Everything computed at one instant, waiting for shown time to reach it.
struct BufferedInstant {
    time: SimTime,
    occurrences: Vec<Occurrence>,
}

/// How computing ended. Nothing is computed after this, and it waits for shown time to reach it.
enum Stopped {
    RunError { at: SimTime, error: RunError },
    Panic { at: SimTime, panic: Box<dyn Any + Send> },
    /// Shown time got there and the error has been reported.
    Reported,
}

impl Stopped {
    /// When this is due, if it still is.
    fn due_at(&self) -> Option<SimTime> {
        match self {
            Stopped::RunError { at, .. } | Stopped::Panic { at, .. } => Some(*at),
            Stopped::Reported => None,
        }
    }
}

/// A clone of the run, plus how much of the log is in it. Time alone can't say, because a
/// checkpoint can land at the very time of an action, on either side of it.
struct Checkpoint {
    run: SimulationRun,
    log_len: usize,
}

/// A replay in progress, in slices so that commands still get through.
struct Replay {
    run: SimulationRun,
    target: SimTime,
    /// The next entry of the log to apply.
    log_index: usize,
    then: AfterReplay,
}

impl Replay {
    fn from(checkpoint: &Checkpoint, target: SimTime, then: AfterReplay) -> Replay {
        let mut run = checkpoint.run.clone();
        run.set_recording(Recording::NOTHING);
        Replay { run, target, log_index: checkpoint.log_len, then }
    }
}

enum AfterReplay {
    /// The session halts at the target and reports the state there.
    GoBack,
    /// Does this at the target, and the session carries on from there.
    Intervene(Action),
    /// Reports the copy's state and throws it away. The computed run carries on.
    ReportState,
}

fn apply_action(run: &mut SimulationRun, action: Action) {
    let at = run.time();
    match action {
        Action::Pause(id) => run.schedule_event(at, EventType::PauseOn, id).expect("the run has been initialised"),
        Action::Resume(id) => run.schedule_event(at, EventType::PauseOff, id).expect("the run has been initialised"),
        Action::ClearStatistics => run.clear_statistics(),
        Action::SetProcessTime(id, time) => run.set_process_time(id, time).expect("checked to be a Station"),
        Action::ForceFailure(id) => run.schedule_event(at, EventType::ForceFailureOn, id).expect("the run has been initialised"),
        Action::EndForcedFailure(id) => run.schedule_event(at, EventType::ForceFailureOff, id).expect("the run has been initialised"),
    }
}

struct SessionThread<'a, C> {
    /// Ahead of shown time, by what is in the buffer.
    run: SimulationRun,
    clock: C,
    commands: Receiver<Command>,
    outbox: &'a mut Outbox,
    settings: SessionSettings,
    speed: f64,
    /// Present while the session is advancing, and absent while it is halted.
    pace: Option<Pace>,
    /// When shown time began waiting for computing. That wait doesn't count once it is over.
    waiting_since: Option<Duration>,
    status: SessionStatus,
    waiters: Vec<Waiter>,
    shown: SimTime,
    shown_reported: SimTime,
    buffer: VecDeque<BufferedInstant>,
    stopped: Option<Stopped>,
    /// The first is from Init and always stays.
    checkpoints: Vec<Checkpoint>,
    events_at_last_checkpoint: u64,
    log: Vec<(SimTime, Action)>,
    replay: Option<Replay>,
    /// Commands that arrived during a replay, in order.
    deferred: VecDeque<Command>,
    steps_wanted: u32,
    /// Where the session halts by itself. Nothing past it is computed or released.
    end: Option<SimTime>,
}

impl<'a, C: Clock> SessionThread<'a, C> {
    fn new(
        mut run: SimulationRun,
        clock: C,
        commands: Receiver<Command>,
        outbox: &'a mut Outbox,
        settings: SessionSettings,
    ) -> Self {
        run.set_recording(Recording::ALL);
        SessionThread {
            shown: run.time(),
            shown_reported: run.time(),
            run,
            clock,
            commands,
            outbox,
            settings,
            speed: 1.0,
            pace: None,
            waiting_since: None,
            status: SessionStatus::Halted,
            waiters: Vec::new(),
            buffer: VecDeque::new(),
            stopped: None,
            checkpoints: Vec::new(),
            events_at_last_checkpoint: 0,
            log: Vec::new(),
            replay: None,
            deferred: VecDeque::new(),
            steps_wanted: 0,
            end: None,
        }
    }

    fn run(mut self) {
        let mut busy = false;
        loop {
            self.outbox.flush();
            match self.next_command(busy) {
                NextCommand::HandleGone => return,
                NextCommand::Command(command) => {
                    self.accept(command);
                    busy = true;
                }
                NextCommand::None => {
                    self.do_some_work();
                    self.release_waiters();
                    busy = false;
                }
            }
        }
    }

    /// Waits for a command only as long as there is nothing else to do.
    fn next_command(&self, busy: bool) -> NextCommand {
        let backlogged = self.outbox.is_backlogged();
        let received = if busy || self.has_work() || !self.waiters.is_empty() {
            match self.commands.try_recv() {
                Err(TryRecvError::Empty) => return NextCommand::None,
                other => other.ok(),
            }
        } else if self.pace.is_some() || backlogged {
            match self.commands.recv_timeout(TICK) {
                Err(RecvTimeoutError::Timeout) => return NextCommand::None,
                other => other.ok(),
            }
        } else {
            self.commands.recv().ok()
        };
        received.map_or(NextCommand::HandleGone, NextCommand::Command)
    }

    fn has_work(&self) -> bool {
        self.replay.is_some() || !self.deferred.is_empty() || self.can_compute() || self.steps_wanted > 0
    }

    fn do_some_work(&mut self) {
        if self.replay.is_some() {
            self.replay_some();
        } else if let Some(command) = self.deferred.pop_front() {
            self.apply(command);
        } else {
            self.compute_some();
            self.release();
            self.take_steps();
        }
    }

    /// A replay holds other commands back until it is done, except Reset, which makes it pointless.
    fn accept(&mut self, command: Command) {
        let replaying = self.replay.is_some() || !self.deferred.is_empty();
        match command {
            Command::Reset if replaying => {
                let held_back = std::mem::take(&mut self.deferred);
                self.reset();
                for command in held_back {
                    if let Command::Sync(_) = command {
                        self.apply(command);
                    }
                }
            }
            command if replaying => self.deferred.push_back(command),
            command => self.apply(command),
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Start => self.start(),
            Command::Halt => self.halt(),
            Command::Reset => self.reset(),
            Command::SetSpeed(speed) => self.set_speed(speed),
            Command::SetTimeMode(mode) => self.settings.time_mode = mode,
            Command::GoBack(to) => self.go_back(to),
            Command::StepBack => self.step_back(),
            Command::LocateStepBack => self.locate_step_back(),
            Command::SetResetKeepsChanges(keep) => self.settings.reset_keeps_changes = keep,
            Command::SetEndTime(end) => self.set_end_time(end),
            Command::StepForward => {
                if let Some(end) = self.at_end() {
                    self.observe(SessionOutput::Error(SessionError::AtEndTime(end)));
                } else if self.pace.is_none() && self.status != SessionStatus::Failed {
                    self.steps_wanted += 1;
                }
            }
            Command::RequestState => self.request_state(),
            Command::Intervene(action) => self.intervene(action),
            Command::Sync(done) => {
                let goal = self.target().unwrap_or(self.shown);
                self.waiters.push(Waiter { goal, done });
            }
        }
    }

    fn start(&mut self) {
        if self.status == SessionStatus::Failed {
            self.observe(SessionOutput::Error(SessionError::ResetRequired));
            return;
        }
        if self.pace.is_some() {
            return;
        }
        if let Some(end) = self.at_end() {
            self.observe(SessionOutput::Error(SessionError::AtEndTime(end)));
            return;
        }
        if !self.ensure_initialised() {
            return;
        }
        self.pace = Some(Pace { wall: self.clock.elapsed(), simulation: self.shown });
        self.waiting_since = None;
        self.set_status(SessionStatus::Advancing);
        self.compute_some();
    }

    /// Does Init if the run needs it, reporting it, and takes the first checkpoint.
    fn ensure_initialised(&mut self) -> bool {
        if !self.run.is_initialised() {
            match self.run.init() {
                Ok(occurrences) => {
                    self.observe(SessionOutput::Occurrences(occurrences));
                    self.observe(SessionOutput::State(self.run.state()));
                }
                Err(error) => {
                    self.fail(error);
                    return false;
                }
            }
        }
        if self.checkpoints.is_empty() {
            self.take_checkpoint();
        }
        true
    }

    fn halt(&mut self) {
        // Wall time up to now still counts, as far as the run has got.
        self.release();
        self.pace = None;
        self.waiting_since = None;
        // That may have ended in a run error, and Halt can't clear a failure.
        if self.status == SessionStatus::Advancing {
            self.set_status(SessionStatus::Halted);
        }
    }

    fn reset(&mut self) {
        // What to keep comes from the log, which holds what is true at shown time, rather than
        // from the run, which may be ahead or on its way back.
        let mut kept: Vec<(u32, SimTime)> = Vec::new();
        if self.settings.reset_keeps_changes {
            for &(_, action) in &self.log {
                if let Action::SetProcessTime(id, time) = action {
                    kept.retain(|&(other, _)| other != id);
                    kept.push((id, time));
                }
            }
        }
        self.run.reset();
        if self.settings.reset_keeps_changes {
            for (id, time) in kept {
                self.run.set_process_time(id, time).expect("only logged for a Station");
            }
            self.run.keep_changes_as_default();
        }
        self.pace = None;
        self.waiting_since = None;
        self.shown = SimTime::ZERO;
        self.shown_reported = SimTime::ZERO;
        self.buffer.clear();
        self.stopped = None;
        self.checkpoints.clear();
        self.events_at_last_checkpoint = 0;
        self.log.clear();
        self.replay = None;
        self.steps_wanted = 0;
        self.observe(SessionOutput::State(self.run.state()));
        self.set_status(SessionStatus::Halted);
    }

    fn set_end_time(&mut self, end: Option<SimTime>) {
        if let Some(end) = end.filter(|&end| end <= self.shown) {
            self.observe(SessionOutput::Error(SessionError::EndNotAhead { end, shown: self.shown }));
            return;
        }
        self.end = end;
        if self.status == SessionStatus::ReachedEnd {
            self.set_status(SessionStatus::Halted);
        }
    }

    /// The end time, if shown time is at it.
    fn at_end(&self) -> Option<SimTime> {
        self.end.filter(|&end| self.shown >= end)
    }

    /// Whether everything at or before the end time has been released, so the session can stop.
    fn nothing_left_before_the_end(&self, end: SimTime) -> bool {
        self.shown >= end && self.next_unreleased().is_none_or(|next| next > end)
    }

    /// `time`, or the end time if that comes first. Never earlier than shown time.
    fn within_end(&self, time: SimTime) -> SimTime {
        self.end.map_or(time, |end| time.min(end.max(self.shown)))
    }

    /// Halts at the end time, which shown time has reached.
    fn reach_end(&mut self, end: SimTime) {
        self.shown = self.shown.max(end);
        self.report_shown_time();
        self.pace = None;
        self.waiting_since = None;
        self.set_status(SessionStatus::ReachedEnd);
    }

    fn set_speed(&mut self, speed: f64) {
        if self.pace.is_some() {
            // Wall time up to now counts at the old speed, as far as the run has got. Anything
            // beyond that is dropped, or slowing down would first mean waiting for it to pass.
            self.release();
            if self.pace.is_some() {
                self.pace = Some(Pace { wall: self.clock.elapsed(), simulation: self.shown });
                self.waiting_since = None;
            }
        }
        self.speed = speed;
    }

    fn step_back(&mut self) {
        if !self.run.is_initialised() {
            self.observe(SessionOutput::Error(SessionError::Run(RunError::NotInitialised)));
        } else if let Some(to) = self.instant_before_shown() {
            self.go_back(to);
        }
    }

    fn locate_step_back(&mut self) {
        if !self.run.is_initialised() {
            self.observe(SessionOutput::Error(SessionError::Run(RunError::NotInitialised)));
        } else {
            let to = self.instant_before_shown();
            self.observe(SessionOutput::StepBackTarget { shown: self.shown, to });
        }
    }

    /// The instant before the last instant at or before shown time, with Init's t=0 the
    /// earliest. An instant is a time with occurrences, as in the buffer and for stepping
    /// forward, so an event that records nothing, such as a stale exit, makes none. A copy of the
    /// nearest checkpoint replays up to shown time, applying logged interventions as a replay
    /// does, and notes each instant it passes. A checkpoint's own time may or may not be one, so
    /// if the copy sees fewer than two, an earlier checkpoint tries again.
    fn instant_before_shown(&self) -> Option<SimTime> {
        let shown = self.shown;
        for checkpoint in self.checkpoints.iter().rev().filter(|c| c.run.time() <= shown) {
            let mut run = checkpoint.run.clone();
            run.set_recording(Recording::ALL);
            let mut actions = self.log[checkpoint.log_len..].iter().filter(|(at, _)| *at <= shown).peekable();
            // Init's checkpoint is the start, and t=0 the earliest instant.
            let mut last = (run.events_processed() == 0).then_some(SimTime::ZERO);
            let mut before = None;
            'replay: loop {
                let stop = actions.peek().map_or(shown, |&&(at, _)| at);
                while let Some(time) = run.next_event_time().filter(|&time| time <= stop) {
                    match run.step() {
                        Ok(occurrences) if occurrences.is_empty() => {}
                        Ok(_) if last != Some(time) => {
                            before = last;
                            last = Some(time);
                        }
                        Ok(_) => {}
                        Err(_) => break 'replay,
                    }
                }
                match actions.next() {
                    Some(&(at, action)) => {
                        // Logged at a time the run has reached, with every event before it done.
                        let _ = run.advance_to(at);
                        apply_action(&mut run, action);
                    }
                    None => break,
                }
            }
            if before.is_some() {
                return before;
            }
        }
        None
    }

    fn go_back(&mut self, to: SimTime) {
        if !self.run.is_initialised() {
            self.observe(SessionOutput::Error(SessionError::Run(RunError::NotInitialised)));
        } else if to > self.shown {
            self.observe(SessionOutput::Error(SessionError::NotInThePast { to, shown: self.shown }));
        } else {
            self.abandon_future_and_replay_to(to, AfterReplay::GoBack);
        }
    }

    fn intervene(&mut self, action: Action) {
        if !self.run.is_initialised() {
            self.observe(SessionOutput::Error(SessionError::Run(RunError::NotInitialised)));
            return;
        }
        if self.status == SessionStatus::Failed {
            self.observe(SessionOutput::Error(SessionError::ResetRequired));
            return;
        }
        if let Action::Pause(id)
        | Action::Resume(id)
        | Action::SetProcessTime(id, _)
        | Action::ForceFailure(id)
        | Action::EndForcedFailure(id) = action
        {
            match self.run.object(id) {
                Some(ModelObject::Station(_)) => {}
                Some(_) => {
                    self.observe(SessionOutput::Error(SessionError::NotAStation(id)));
                    return;
                }
                None => {
                    self.observe(SessionOutput::Error(SessionError::Run(RunError::UnknownObject(id))));
                    return;
                }
            }
        }
        // Switching on what is already on, or off what is off, changes nothing.
        let already = match action {
            Action::Pause(id) => self.is_on_in_log(id, Switch::Pause),
            Action::Resume(id) => !self.is_on_in_log(id, Switch::Pause),
            Action::ForceFailure(id) => self.is_on_in_log(id, Switch::ForcedFailure),
            Action::EndForcedFailure(id) => !self.is_on_in_log(id, Switch::ForcedFailure),
            Action::ClearStatistics | Action::SetProcessTime(..) => false,
        };
        if already {
            return;
        }
        self.abandon_future_and_replay_to(self.shown, AfterReplay::Intervene(action));
    }

    /// Whether an interruption the user switches is on for an object. The log holds nothing after
    /// shown time, so its last word on an object is the current one.
    fn is_on_in_log(&self, id: u32, switch: Switch) -> bool {
        self.log.iter().rev().find_map(|&(_, action)| match (switch, action) {
            (Switch::Pause, Action::Pause(on)) | (Switch::ForcedFailure, Action::ForceFailure(on)) if on == id => Some(true),
            (Switch::Pause, Action::Resume(off)) | (Switch::ForcedFailure, Action::EndForcedFailure(off)) if off == id => {
                Some(false)
            }
            _ => None,
        }) == Some(true)
    }

    /// Drops what going back abandons, and starts replaying from the nearest earlier checkpoint.
    fn abandon_future_and_replay_to(&mut self, to: SimTime, then: AfterReplay) {
        self.log.retain(|&(at, _)| at <= to);
        let log_len = self.log.len();
        self.checkpoints.retain(|c| c.run.time() <= to && c.log_len <= log_len);
        self.buffer.clear();
        self.stopped = None;
        self.steps_wanted = 0;
        self.waiting_since = None;
        let nearest = self.checkpoints.last().expect("the checkpoint from Init always stays");
        self.events_at_last_checkpoint = nearest.run.events_processed();
        self.replay = Some(Replay::from(nearest, to, then));
    }

    fn request_state(&mut self) {
        // The run stands at shown time with nothing left to do there, so it can report itself.
        // An intervention leaves an event due at shown time until the run handles it.
        let done_at_shown = self.run.time() == self.shown && self.run.next_event_time().is_none_or(|next| next > self.shown);
        if !self.run.is_initialised() || (self.buffer.is_empty() && self.stopped.is_none() && done_at_shown) {
            self.observe(SessionOutput::State(self.run.state()));
            return;
        }
        let shown = self.shown;
        let nearest = self.checkpoints.iter().rev().find(|c| c.run.time() <= shown).expect("the checkpoint from Init always stays");
        self.replay = Some(Replay::from(nearest, shown, AfterReplay::ReportState));
    }

    fn replay_some(&mut self) {
        let mut replay = self.replay.take().expect("only called during a replay");
        let next_action = self.log.get(replay.log_index).copied();
        let stop = next_action.map_or(replay.target, |(at, _)| at.min(replay.target));
        match replay.run.advance_bounded(stop, EVENTS_PER_REPLAY_SLICE) {
            Ok(advance) if !advance.reached => self.replay = Some(replay),
            Ok(_) => match next_action {
                Some((at, action)) if at <= replay.target => {
                    apply_action(&mut replay.run, action);
                    replay.log_index += 1;
                    self.replay = Some(replay);
                }
                _ => self.finish_replay(replay, None),
            },
            Err(failure) => self.finish_replay(replay, Some(failure.error)),
        }
    }

    fn finish_replay(&mut self, replay: Replay, error: Option<RunError>) {
        let Replay { mut run, target, then, .. } = replay;
        if let AfterReplay::ReportState = then {
            self.observe(SessionOutput::State(run.state()));
            return;
        }
        run.set_recording(Recording::ALL);
        self.run = run;
        self.shown = target;
        self.shown_reported = target;
        if let Some(error) = error {
            self.stopped = Some(Stopped::RunError { at: self.run.time(), error });
        }
        match then {
            AfterReplay::GoBack => {
                self.pace = None;
                self.observe(SessionOutput::State(self.run.state()));
                self.set_status(SessionStatus::Halted);
            }
            AfterReplay::Intervene(action) => {
                apply_action(&mut self.run, action);
                self.log.push((target, action));
                if self.pace.is_some() {
                    self.pace = Some(Pace { wall: self.clock.elapsed(), simulation: target });
                }
            }
            AfterReplay::ReportState => unreachable!("handled above"),
        }
    }

    /// How many instants at the front of the buffer are finished. The last may still be growing.
    fn complete_instants(&self) -> usize {
        let growing = self.stopped.is_none()
            && self.buffer.back().is_some_and(|last| self.run.next_event_time() == Some(last.time));
        self.buffer.len() - growing as usize
    }

    fn can_compute(&self) -> bool {
        self.run.is_initialised()
            && self.stopped.is_none()
            && self.run.next_event_time().is_some_and(|next| self.end.is_none_or(|end| next <= end))
            && self.complete_instants() < self.settings.buffer_instants
    }

    /// Computes one slice ahead into the buffer, if there is room. A panic waits for shown time
    /// to reach it, like an error, so that everything before it still arrives.
    fn compute_some(&mut self) {
        if !self.can_compute() {
            return;
        }
        // With an end time, a slice stops there, so nothing past it is ever computed.
        let end = self.end;
        let slice = |run: &mut SimulationRun| match end {
            Some(end) => run.advance_bounded(end, EVENTS_PER_SLICE).map(|advance| advance.occurrences),
            None => run.advance_events(EVENTS_PER_SLICE),
        };
        match catch_unwind(AssertUnwindSafe(|| slice(&mut self.run))) {
            Ok(Ok(occurrences)) => self.buffer_occurrences(occurrences),
            Ok(Err(failure)) => {
                self.buffer_occurrences(failure.occurrences);
                self.stopped = Some(Stopped::RunError { at: self.run.time(), error: failure.error });
            }
            Err(panic) => {
                let occurrences = self.run.take_recorded();
                self.buffer_occurrences(occurrences);
                self.stopped = Some(Stopped::Panic { at: self.run.time(), panic });
            }
        }
        if self.stopped.is_none() && self.run.events_processed() - self.events_at_last_checkpoint >= self.settings.checkpoint_spacing {
            self.take_checkpoint();
        }
    }

    fn buffer_occurrences(&mut self, occurrences: Vec<Occurrence>) {
        for occurrence in occurrences {
            match self.buffer.back_mut() {
                Some(last) if last.time == occurrence.time => last.occurrences.push(occurrence),
                _ => self.buffer.push_back(BufferedInstant { time: occurrence.time, occurrences: vec![occurrence] }),
            }
        }
    }

    fn take_checkpoint(&mut self) {
        self.events_at_last_checkpoint = self.run.events_processed();
        self.checkpoints.push(Checkpoint { run: self.run.clone(), log_len: self.log.len() });
        let each = self.run.checkpoint_bytes();
        while self.checkpoints.len() > 2 && self.checkpoints.len() as u64 * each > self.settings.checkpoint_budget_bytes {
            self.thin_out_older_checkpoints();
        }
    }

    /// Removes every other checkpoint from the older half. The one from Init stays, so the whole
    /// run stays reachable, and recent history stays dense for stepping back through.
    fn thin_out_older_checkpoints(&mut self) {
        let older_half = self.checkpoints.len().div_ceil(2);
        let mut index = 0;
        self.checkpoints.retain(|_| {
            let keep = index >= older_half || index % 2 == 0;
            index += 1;
            keep
        });
    }

    /// The earliest thing not yet released, if anything is known to be coming.
    fn next_unreleased(&self) -> Option<SimTime> {
        if let Some(front) = self.buffer.front() {
            return Some(front.time);
        }
        match &self.stopped {
            Some(stopped) => stopped.due_at(),
            None => self.run.next_event_time(),
        }
    }

    /// The simulation time the clock calls for, if the session is advancing.
    fn target(&self) -> Option<SimTime> {
        self.target_at(self.clock.elapsed())
    }

    /// The simulation time a reading of the clock calls for, if the session is advancing.
    fn target_at(&self, now: Duration) -> Option<SimTime> {
        self.pace.map(|pace| pace.target(now, self.speed))
    }

    /// Takes away the rest of a long gap, in the mode that does, so shown time jumps over it. It
    /// looks only at the gap ahead, so asking again changes nothing.
    fn skip_long_gap(&mut self, now: Duration) {
        let TimeMode::Skipping { longest_gap } = self.settings.time_mode else { return };
        let Some(next) = self.next_unreleased() else { return };
        let Some(target) = self.target_at(now) else { return };
        let allowed = SimTime::from_secs_f64(longest_gap.as_secs_f64() * self.speed);
        let gap = next.saturating_sub(target);
        if gap > allowed {
            if let Some(pace) = &mut self.pace {
                pace.simulation = pace.simulation.saturating_add(gap.saturating_sub(allowed));
            }
        }
    }

    /// Releases what the clock has reached, unless too many outputs are unread. Time spent
    /// waiting for a reader counts and is caught up on afterwards. Time spent waiting for the run
    /// to compute doesn't, so shown time carries on from where it waited. Results depend on none
    /// of this.
    ///
    /// It reads the clock once. A second reading would find instants that fell due meanwhile,
    /// already computed but not yet released, and take them for computing being behind. Shown
    /// time would then lose the time until the next release, which with a full buffer includes a
    /// whole wait for commands.
    fn release(&mut self) {
        if self.outbox.is_backlogged() {
            return;
        }
        let now = self.clock.elapsed();
        let complete = self.complete_instants();
        if complete > 0 || self.stopped.is_some() || self.run.next_event_time().is_none() {
            if let (Some(since), Some(pace)) = (self.waiting_since.take(), &mut self.pace) {
                pace.wall += now.saturating_sub(since);
            }
        }
        self.skip_long_gap(now);
        // While halted, only what was done at shown time from outside can be due. Nothing past
        // the end time is released.
        let limit = self.within_end(self.target_at(now).unwrap_or(self.shown));

        let mut released = Vec::new();
        let mut instants = 0;
        while instants < INSTANTS_PER_RELEASE && instants < complete && self.buffer.front().is_some_and(|i| i.time <= limit) {
            let instant = self.buffer.pop_front().expect("checked above");
            self.shown = self.shown.max(instant.time);
            released.extend(instant.occurrences);
            instants += 1;
        }
        if self.pace.is_some() && instants < INSTANTS_PER_RELEASE {
            // Releasing opened the next gap, so shown time jumps over it now rather than later.
            self.skip_long_gap(now);
            let limit = self.target_at(now).map_or(limit, |target| self.within_end(target));
            let reached = self.next_unreleased().map_or(limit, |next| next.min(limit));
            self.shown = self.shown.max(reached);
            if reached < limit && self.stopped.is_none() {
                self.waiting_since.get_or_insert(now);
            }
        }
        if !released.is_empty() {
            self.observe(SessionOutput::Occurrences(released));
        }
        self.report_shown_time();
        let stop_is_due = self.stopped.as_ref().and_then(Stopped::due_at).is_some_and(|at| at <= limit);
        if self.buffer.is_empty() && stop_is_due {
            self.stop_here();
        }
        if let Some(end) = self.end.filter(|&end| self.pace.is_some() && self.nothing_left_before_the_end(end)) {
            self.reach_end(end);
        }
    }

    fn report_shown_time(&mut self) {
        if self.shown != self.shown_reported {
            self.shown_reported = self.shown;
            self.observe(SessionOutput::ShownTime(self.shown));
        }
    }

    /// Shown time has reached how computing ended.
    fn stop_here(&mut self) {
        match self.stopped.take() {
            Some(Stopped::RunError { at, error }) => {
                self.shown = self.shown.max(at);
                self.report_shown_time();
                self.stopped = Some(Stopped::Reported);
                self.fail(error);
            }
            // The thread ends here, reporting it, as any other panic on this thread does.
            Some(Stopped::Panic { panic, .. }) => resume_unwind(panic),
            other => self.stopped = other,
        }
    }

    fn take_steps(&mut self) {
        while self.steps_wanted > 0 && !self.outbox.is_backlogged() {
            if self.pace.is_some() || self.status == SessionStatus::Failed {
                self.steps_wanted = 0;
            } else if !self.run.is_initialised() {
                self.steps_wanted -= 1;
                self.ensure_initialised();
            } else if let Some(end) = self.end.filter(|&end| self.buffer.front().is_some_and(|next| next.time > end)) {
                // The next instant is past the end time, so a step goes to the end time.
                self.steps_wanted = 0;
                self.reach_end(end);
            } else if self.complete_instants() > 0 {
                self.steps_wanted -= 1;
                let instant = self.buffer.pop_front().expect("checked above");
                self.shown = instant.time;
                self.observe(SessionOutput::Occurrences(instant.occurrences));
                self.report_shown_time();
            } else if self.stopped.is_some() {
                self.steps_wanted = 0;
                self.stop_here();
            } else if self.can_compute() {
                return;
            } else if let Some(end) = self.end.filter(|&end| self.run.next_event_time().is_some_and(|next| next > end)) {
                // The next event is past the end time, so a step goes to the end time.
                self.steps_wanted = 0;
                self.reach_end(end);
            } else {
                // No instant is coming.
                self.steps_wanted = 0;
            }
        }
    }

    fn release_waiters(&mut self) {
        let settled = self.replay.is_none() && self.deferred.is_empty() && self.steps_wanted == 0;
        let halted = self.pace.is_none();
        let shown = self.shown;
        let next = self.next_unreleased();
        // A backlogged session can't promise everything is waiting in `outputs`, and
        // nobody can read while a sync is waiting, so no waiter would ever be released.
        let backlogged = self.outbox.is_backlogged();
        self.waiters.retain(|waiter| {
            // A halted session goes no further than shown time, once what is due there is out.
            let goal = if halted { shown } else { waiter.goal };
            let reached = settled && shown >= goal && next.is_none_or(|next| next > goal);
            if backlogged {
                let _ = waiter.done.send(Err(SessionError::Backlogged));
            } else if reached {
                let _ = waiter.done.send(Ok(()));
            }
            !(backlogged || reached)
        });
    }

    /// Reports a run error at shown time, and halts.
    fn fail(&mut self, error: RunError) {
        self.pace = None;
        self.waiting_since = None;
        self.set_status(SessionStatus::Failed);
        self.observe(SessionOutput::Error(SessionError::Run(error)));
    }

    fn set_status(&mut self, status: SessionStatus) {
        if self.status != status {
            self.status = status;
            self.observe(SessionOutput::Status(status));
        }
    }

    fn observe(&mut self, output: SessionOutput) {
        self.outbox.send(output);
    }
}

enum NextCommand {
    Command(Command),
    None,
    HandleGone,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::FailureProfile;
    use crate::model::ModelDescription;
    use crate::random_numbers::Distribution;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct FakeClock(Arc<Mutex<Duration>>);

    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            *self.0.lock().unwrap()
        }
    }

    fn secs(s: u64) -> SimTime {
        SimTime::from_secs(s)
    }

    /// Source (one MU every second) -> Station with failures -> Drain.
    fn busy_line() -> SimulationRun {
        let failures = FailureProfile::new(0.8, Distribution::ERLANG { mean: 20_000.0, k: 2 }, 0);
        let mut model = ModelDescription::new();
        let src = model.add_source(secs(1));
        let st = model.add_station(secs(1), failures);
        let dr = model.add_drain();
        model.connect(src, st);
        model.connect(st, dr);
        model.build(3)
    }

    /// Drives a session thread by hand, for what can't be seen from outside.
    fn with_session(settings: SessionSettings, check: impl FnOnce(&mut SessionThread<'_, FakeClock>, &FakeClock)) {
        with_session_on(busy_line(), settings, check);
    }

    fn with_session_on(run: SimulationRun, settings: SessionSettings, check: impl FnOnce(&mut SessionThread<'_, FakeClock>, &FakeClock)) {
        let (_commands, inbox) = channel();
        let (observer, outputs) = sync_channel(1_000_000);
        let mut outbox = Outbox { observer, unsent: VecDeque::new() };
        let clock = FakeClock::default();
        let mut session = SessionThread::new(run, clock.clone(), inbox, &mut outbox, settings);
        session.start();
        check(&mut session, &clock);
        drop(outputs);
    }

    fn finish_replay_by_hand(session: &mut SessionThread<'_, FakeClock>) {
        while session.replay.is_some() {
            session.replay_some();
        }
    }

    #[test]
    fn pausing_what_is_already_paused_is_not_logged() {
        with_session(SessionSettings::default(), |session, _| {
            session.intervene(Action::Pause(1));
            finish_replay_by_hand(session);
            assert_eq!(session.log.len(), 1);
            session.intervene(Action::Pause(1));
            assert!(session.replay.is_none(), "and the computed future stays");
            assert_eq!(session.log.len(), 1);
            session.intervene(Action::Resume(1));
            finish_replay_by_hand(session);
            session.intervene(Action::Resume(1));
            assert_eq!(session.log.len(), 2);
        });
    }

    #[test]
    fn shown_time_jumps_over_the_next_gap_as_soon_as_an_instant_is_out() {
        let mut model = ModelDescription::new();
        let src = model.add_source(secs(1_000));
        let dr = model.add_drain();
        model.connect(src, dr);
        let settings = SessionSettings { time_mode: TimeMode::Skipping { longest_gap: Duration::from_secs(1) }, ..SessionSettings::default() };
        with_session_on(model.build(0), settings, |session, clock| {
            session.release();
            assert_eq!(session.shown, secs(999));
            *clock.0.lock().unwrap() = Duration::from_secs(1);
            session.release();
            assert_eq!(session.shown, secs(1_999), "in that one pass");
        });
    }

    #[test]
    fn computing_stops_at_the_buffer_limit() {
        let settings = SessionSettings { buffer_instants: 5_000, ..SessionSettings::default() };
        with_session(settings, |session, _| {
            for _ in 0..100 {
                session.compute_some();
            }
            // It checks between slices, so it can pass the limit by up to one slice.
            let held = session.buffer.len();
            assert!((5_000..=5_000 + EVENTS_PER_SLICE + 1).contains(&held), "had {held}");
            assert!(!session.can_compute());
            assert_eq!(session.shown, SimTime::ZERO, "computing ahead shows nothing");
        });
    }

    #[test]
    fn computing_never_goes_past_the_end_time() {
        with_session(SessionSettings::default(), |session, _| {
            session.set_end_time(Some(secs(500)));
            for _ in 0..100 {
                session.compute_some();
            }
            assert!(!session.can_compute(), "everything up to the end time is computed");
            assert!(session.run.next_event_time().is_some_and(|next| next > secs(500)), "the events at the end time are done");
            assert!(session.buffer.back().is_some_and(|last| last.time <= secs(500)), "nothing past the end time is computed");
        });
    }

    #[test]
    fn checkpoints_come_at_the_spacing_and_the_first_is_from_init() {
        let settings = SessionSettings { checkpoint_spacing: 2_000, ..SessionSettings::default() };
        with_session(settings, |session, _| {
            assert_eq!(session.checkpoints.len(), 1);
            assert_eq!(session.checkpoints[0].run.time(), SimTime::ZERO);
            for _ in 0..10 {
                session.compute_some();
            }
            assert_eq!(session.checkpoints.len(), 6, "after 11 slices of 1,000 events");
        });
    }

    #[test]
    fn past_the_budget_the_older_half_thins_out_and_init_stays() {
        let each = busy_line().checkpoint_bytes();
        let settings = SessionSettings {
            checkpoint_spacing: 1_000,
            checkpoint_budget_bytes: 16 * each,
            buffer_instants: usize::MAX,
            ..SessionSettings::default()
        };
        with_session(settings, |session, _| {
            for _ in 0..200 {
                session.compute_some();
            }
            assert!(session.checkpoints.len() <= 16, "had {}", session.checkpoints.len());
            assert_eq!(session.checkpoints[0].run.time(), SimTime::ZERO, "Init always stays");

            let events: Vec<u64> = session.checkpoints.iter().map(|c| c.run.events_processed()).collect();
            assert!(events.windows(2).all(|pair| pair[0] < pair[1]));
            let (first, last) = (events[1] - events[0], events[events.len() - 1] - events[events.len() - 2]);
            assert_eq!(last, 1_000, "the most recent are still dense");
            assert!(first > last, "the older ones are sparser: {events:?}");
        });
    }

    #[test]
    fn a_budget_too_small_for_anything_keeps_init_and_the_latest() {
        let settings = SessionSettings { checkpoint_spacing: 1_000, checkpoint_budget_bytes: 1, ..SessionSettings::default() };
        with_session(settings, |session, _| {
            for _ in 0..50 {
                session.compute_some();
            }
            assert_eq!(session.checkpoints.len(), 2);
            assert_eq!(session.checkpoints[0].run.time(), SimTime::ZERO);
        });
    }

    #[test]
    fn time_spent_waiting_for_computing_does_not_count() {
        let settings = SessionSettings { buffer_instants: 2, ..SessionSettings::default() };
        with_session(settings, |session, clock| {
            // Empty the buffer, then let the clock run with nothing computed.
            session.buffer.clear();
            while session.run.next_event_time().is_none() {
                session.compute_some();
            }
            let next = session.run.next_event_time().unwrap();
            *clock.0.lock().unwrap() = Duration::from_secs(next.as_secs() + 10);
            session.release();
            assert_eq!(session.shown, next, "shown time waits at what is known");
            assert!(session.waiting_since.is_some());

            // The clock moves on by 100s while it waits, and then computing delivers.
            *clock.0.lock().unwrap() = Duration::from_secs(next.as_secs() + 110);
            session.compute_some();
            session.release();
            assert!(session.waiting_since.is_none());
            let target = session.target().unwrap();
            assert!(
                target <= next + secs(11),
                "the 100s of waiting must not count: target {target:?}, waited at {next:?}"
            );
        });
    }
}
