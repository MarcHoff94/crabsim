//! A simulation run on its own thread, controlled live: commands go in, observations come
//! out. This knows nothing about any UI technology, and takes its clock from outside.

use crate::error::RunError;
use crate::observation::{Occurrence, Recording, RunState};
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;
use std::{
    any::Any,
    collections::VecDeque,
    fmt,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long a running session waits for a command before it looks at the clock again.
const TICK: Duration = Duration::from_millis(16);

/// How many events a session processes before it checks for commands again, so a long
/// catch-up can't keep a command waiting.
const EVENTS_PER_SLICE: usize = 1_000;

/// How many unread observations a session lets build up before it stops advancing until
/// they are read. It keeps handling commands meanwhile, and no observation is ever dropped.
pub const OBSERVATION_BACKLOG_LIMIT: usize = 256;

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

/// What a session reports. Occurrences arrive in order, and concatenated they are what
/// advancing the run directly would have returned. A state follows every advance, and a
/// status follows every change of status.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    Occurrences(Vec<Occurrence>),
    State(RunState),
    Status(SessionStatus),
    Error(SessionError),
}

/// Whether a session is advancing its run. A new session is `Halted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Advancing,
    Halted,
    /// Halted by a run error. Only Reset gets it going again.
    Failed,
}

/// Why a session refused a command or could not continue.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionError {
    /// The run reported an error. The session has halted, and only Reset gets it going again.
    Run(RunError),
    /// Start was refused, because the run has reported an error and hasn't been reset since.
    ResetRequired,
    /// A speed has to be positive and finite.
    InvalidSpeed(f64),
    /// The session thread panicked and has ended. Only a new session can continue.
    Panicked(String),
    /// `sync` stopped waiting because too many observations are unread, so the session won't
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
            SessionError::ResetRequired => write!(f, "the simulation run reported an error; reset it first"),
            SessionError::InvalidSpeed(speed) => write!(f, "{speed} is not a speed: it has to be positive and finite"),
            SessionError::Panicked(message) => write!(f, "the live session panicked: {message}"),
            SessionError::Backlogged => write!(f, "too many observations are unread; read them first"),
            SessionError::SyncTimedOut => write!(f, "the live session did not catch up in time"),
            SessionError::Ended => write!(f, "the live session has ended"),
        }
    }
}

impl std::error::Error for SessionError {}

enum Command {
    Start,
    Halt,
    Reset,
    SetSpeed(f64),
    Sync(Sender<Result<(), SessionError>>),
}

/// The handle to a simulation run living on its own thread. Dropping it ends the thread.
pub struct LiveSession {
    commands: Sender<Command>,
    observations: Receiver<Observation>,
    thread: Option<JoinHandle<()>>,
}

impl LiveSession {
    /// Creates the session's thread and builds the whole run on it. The session starts out
    /// halted, at speed 1, and records everything, because its observers rebuild what they show
    /// from occurrences.
    pub fn new<B, C>(build: B, clock: C) -> LiveSession
    where
        B: FnOnce() -> SimulationRun + Send + 'static,
        C: Clock,
    {
        let (commands, command_inbox) = channel();
        let (observer, observations) = sync_channel(OBSERVATION_BACKLOG_LIMIT);
        let thread = thread::spawn(move || {
            let mut outbox = Outbox { observer, unsent: VecDeque::new() };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                SessionThread::new(build(), clock, command_inbox, &mut outbox).run();
            }));
            if let Err(panic) = outcome {
                // By now the command channel is closed, so commands find the session has ended.
                outbox.send(Observation::Error(SessionError::Panicked(panic_message(panic))));
                while outbox.is_backlogged() {
                    thread::sleep(TICK);
                    outbox.flush();
                }
            }
        });
        LiveSession { commands, observations, thread: Some(thread) }
    }

    /// Starts or continues advancing, doing Init first if the run needs it. Does nothing if
    /// the session is already advancing.
    pub fn start(&self) -> Result<(), SessionError> {
        self.send(Command::Start)
    }

    /// Stops time advancing and keeps everything exactly as it is. Wall time doesn't count
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

    /// Blocks until the session has handled every earlier command and advanced to the
    /// clock's current time, with everything that produced waiting in `observations`.
    ///
    /// It always returns. `Backlogged` means earlier commands are handled but the session
    /// won't go further until observations are read, so read them and sync again. `SyncTimedOut`
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

    pub fn observations(&self) -> &Receiver<Observation> {
        &self.observations
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
        while self.observations.recv().is_ok() {}
        if let Some(thread) = self.thread.take() {
            // A panic has already been reported as an observation.
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

/// A `sync` waiting for the run to reach the simulation time the clock showed when it arrived.
struct Waiter {
    goal: SimTime,
    done: Sender<Result<(), SessionError>>,
}

/// Observations on their way out. Sending never blocks the session thread, so it can always
/// handle commands. What the channel has no room for waits here, in order, and a session
/// doesn't advance while anything is waiting, so only commands can add more.
struct Outbox {
    observer: SyncSender<Observation>,
    unsent: VecDeque<Observation>,
}

impl Outbox {
    fn send(&mut self, observation: Observation) {
        self.unsent.push_back(observation);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(observation) = self.unsent.pop_front() {
            match self.observer.try_send(observation) {
                Ok(()) => {}
                Err(TrySendError::Full(observation)) => {
                    self.unsent.push_front(observation);
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

struct SessionThread<'a, C> {
    run: SimulationRun,
    clock: C,
    commands: Receiver<Command>,
    outbox: &'a mut Outbox,
    speed: f64,
    /// Present while the session is advancing, and absent while it is halted.
    pace: Option<Pace>,
    /// The last slice ran out of events before reaching its target.
    catching_up: bool,
    status: SessionStatus,
    waiters: Vec<Waiter>,
}

impl<'a, C: Clock> SessionThread<'a, C> {
    fn new(mut run: SimulationRun, clock: C, commands: Receiver<Command>, outbox: &'a mut Outbox) -> Self {
        run.set_recording(Recording::ALL);
        SessionThread {
            run,
            clock,
            commands,
            outbox,
            speed: 1.0,
            pace: None,
            catching_up: false,
            status: SessionStatus::Halted,
            waiters: Vec::new(),
        }
    }

    fn run(mut self) {
        let mut busy = false;
        loop {
            self.outbox.flush();
            match self.next_command(busy) {
                NextCommand::HandleGone => return,
                NextCommand::Command(command) => {
                    self.apply(command);
                    busy = true;
                }
                NextCommand::None => {
                    self.advance_one_slice();
                    self.release_waiters();
                    busy = false;
                }
            }
        }
    }

    /// Waits for a command only as long as there is nothing else to do.
    fn next_command(&self, busy: bool) -> NextCommand {
        let backlogged = self.outbox.is_backlogged();
        let received = if busy || (self.catching_up && !backlogged) || !self.waiters.is_empty() {
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

    fn apply(&mut self, command: Command) {
        match command {
            Command::Start => self.start(),
            Command::Halt => self.halt(),
            Command::Reset => self.reset(),
            Command::SetSpeed(speed) => self.set_speed(speed),
            Command::Sync(done) => {
                let goal = self.target().unwrap_or(SimTime::ZERO);
                self.waiters.push(Waiter { goal, done });
            }
        }
    }

    fn start(&mut self) {
        if self.status == SessionStatus::Failed {
            self.observe(Observation::Error(SessionError::ResetRequired));
            return;
        }
        if self.pace.is_some() {
            return;
        }
        if !self.run.is_initialised() {
            match self.run.init() {
                Ok(occurrences) => self.report(occurrences),
                Err(error) => {
                    self.report_failure(Vec::new(), error);
                    return;
                }
            }
        }
        self.pace = Some(Pace { wall: self.clock.elapsed(), simulation: self.run.time() });
        self.set_status(SessionStatus::Advancing);
    }

    fn halt(&mut self) {
        // Wall time up to now still counts, but Halt won't wait for a catch-up to finish.
        if !self.catching_up {
            self.advance_one_slice();
        }
        self.pace = None;
        self.catching_up = false;
        // That last slice may have ended in a run error, and Halt can't clear a failure.
        if self.status == SessionStatus::Advancing {
            self.set_status(SessionStatus::Halted);
        }
    }

    fn reset(&mut self) {
        self.run.reset();
        self.pace = None;
        self.catching_up = false;
        self.observe(Observation::State(self.run.state()));
        self.set_status(SessionStatus::Halted);
    }

    fn set_speed(&mut self, speed: f64) {
        if let Some(target) = self.target() {
            // Wall time up to now counts at the old speed. A catch-up in progress is dropped,
            // or slowing down to watch closely would first mean waiting for it to finish.
            let simulation = if self.catching_up { self.run.time() } else { target };
            self.pace = Some(Pace { wall: self.clock.elapsed(), simulation });
            self.catching_up = false;
        }
        self.speed = speed;
    }

    /// The simulation time the clock calls for, if the session is advancing.
    fn target(&self) -> Option<SimTime> {
        self.pace.map(|pace| pace.target(self.clock.elapsed(), self.speed))
    }

    /// Advances towards the clock's time, by at most `EVENTS_PER_SLICE` events, unless too many
    /// observations are unread. The pace stays as it is, so the time spent waiting for a reader
    /// counts and is caught up on afterwards. The run's outcome doesn't depend on where the
    /// slices fall.
    fn advance_one_slice(&mut self) {
        if self.outbox.is_backlogged() {
            return;
        }
        let Some(target) = self.target() else { return };
        let time_before = self.run.time();
        match self.run.advance_bounded(target, EVENTS_PER_SLICE) {
            Ok(advance) => {
                self.catching_up = !advance.reached;
                if !advance.occurrences.is_empty() || self.run.time() != time_before {
                    self.report(advance.occurrences);
                }
            }
            Err(failure) => self.report_failure(failure.occurrences, failure.error),
        }
    }

    fn release_waiters(&mut self) {
        // Not catching up means the run has reached the clock's time, which is at or past
        // every waiter's goal, and a halted run isn't going any further.
        let everyone = !self.catching_up || self.pace.is_none();
        let time = self.run.time();
        let next_event = self.run.next_event_time();
        // A backlogged session can't promise everything is waiting in `observations`, and
        // nobody can read while a sync is waiting, so no waiter would ever be released.
        let backlogged = self.outbox.is_backlogged();
        self.waiters.retain(|waiter| {
            let reached = time >= waiter.goal && next_event.is_none_or(|next| next > waiter.goal);
            if backlogged {
                let _ = waiter.done.send(Err(SessionError::Backlogged));
            } else if everyone || reached {
                let _ = waiter.done.send(Ok(()));
            }
            !(backlogged || everyone || reached)
        });
    }

    fn report(&mut self, occurrences: Vec<Occurrence>) {
        if !occurrences.is_empty() {
            self.observe(Observation::Occurrences(occurrences));
        }
        self.observe(Observation::State(self.run.state()));
    }

    /// Reports everything that happened before an error, then the error, and halts.
    fn report_failure(&mut self, occurrences: Vec<Occurrence>, error: RunError) {
        self.report(occurrences);
        self.pace = None;
        self.catching_up = false;
        self.set_status(SessionStatus::Failed);
        self.observe(Observation::Error(SessionError::Run(error)));
    }

    fn set_status(&mut self, status: SessionStatus) {
        if self.status != status {
            self.status = status;
            self.observe(Observation::Status(status));
        }
    }

    fn observe(&mut self, observation: Observation) {
        self.outbox.send(observation);
    }
}

enum NextCommand {
    Command(Command),
    None,
    HandleGone,
}
