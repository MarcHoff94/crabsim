//! A simulation run on its own thread, controlled live: commands go in, observations come
//! out. This knows nothing about any UI technology, and takes its clock from outside.

use crate::error::RunError;
use crate::observation::{Occurrence, RunState};
use crate::simulation_run::SimulationRun;
use std::{
    any::Any,
    fmt,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long a running session waits for a command before it looks at the clock again.
const TICK: Duration = Duration::from_millis(16);

/// How many events a session processes before it checks for commands again, so a long
/// catch-up can't keep a command waiting.
const EVENTS_PER_SLICE: usize = 1_000;

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
/// advancing the run directly would have returned. A state follows every advance.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    Occurrences(Vec<Occurrence>),
    State(RunState),
    Error(SessionError),
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
    Sync(Sender<()>),
    Shutdown,
}

/// The handle to a simulation run living on its own thread. Dropping it ends the thread.
pub struct LiveSession {
    commands: Sender<Command>,
    observations: Receiver<Observation>,
    thread: Option<JoinHandle<()>>,
}

impl LiveSession {
    /// Creates the session's thread and builds the whole run on it, because a run's objects
    /// can't leave the thread they were created on. The session starts out halted, at speed 1.
    pub fn new<B, C>(build: B, clock: C) -> LiveSession
    where
        B: FnOnce() -> SimulationRun + Send + 'static,
        C: Clock,
    {
        let (commands, command_inbox) = channel();
        let (observer, observations) = channel();
        let thread = thread::spawn(move || {
            let panic_observer = observer.clone();
            let outcome = catch_unwind(AssertUnwindSafe(move || {
                SessionThread::new(build(), clock, command_inbox, observer).run();
            }));
            if let Err(panic) = outcome {
                let _ = panic_observer.send(Observation::Error(SessionError::Panicked(panic_message(panic))));
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
    pub fn sync(&self) -> Result<(), SessionError> {
        let (done, wait) = channel();
        self.send(Command::Sync(done))?;
        wait.recv().map_err(|_| SessionError::Ended)
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
        let _ = self.commands.send(Command::Shutdown);
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
    simulation: Duration,
}

impl Pace {
    /// The simulation time this pace has reached at wall time `now`.
    fn target(&self, now: Duration, speed: f64) -> Duration {
        let passed = now.saturating_sub(self.wall).as_secs_f64() * speed;
        self.simulation.saturating_add(Duration::try_from_secs_f64(passed).unwrap_or(Duration::MAX))
    }
}

/// A `sync` waiting for the run to reach the simulation time the clock showed when it arrived.
struct Waiter {
    goal: Duration,
    done: Sender<()>,
}

struct SessionThread<C> {
    run: SimulationRun,
    clock: C,
    commands: Receiver<Command>,
    observer: Sender<Observation>,
    speed: f64,
    /// Present while the session is advancing, and absent while it is halted.
    pace: Option<Pace>,
    /// The last slice ran out of events before reaching its target.
    catching_up: bool,
    /// The run reported an error, so it can't continue until Reset.
    failed: bool,
    waiters: Vec<Waiter>,
}

impl<C: Clock> SessionThread<C> {
    fn new(run: SimulationRun, clock: C, commands: Receiver<Command>, observer: Sender<Observation>) -> Self {
        SessionThread {
            run,
            clock,
            commands,
            observer,
            speed: 1.0,
            pace: None,
            catching_up: false,
            failed: false,
            waiters: Vec::new(),
        }
    }

    fn run(mut self) {
        let mut busy = false;
        loop {
            match self.next_command(busy) {
                NextCommand::Shutdown => return,
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
        let received = if busy || self.catching_up || !self.waiters.is_empty() {
            match self.commands.try_recv() {
                Err(TryRecvError::Empty) => return NextCommand::None,
                other => other.ok(),
            }
        } else if self.pace.is_some() {
            match self.commands.recv_timeout(TICK) {
                Err(RecvTimeoutError::Timeout) => return NextCommand::None,
                other => other.ok(),
            }
        } else {
            self.commands.recv().ok()
        };
        match received {
            Some(Command::Shutdown) | None => NextCommand::Shutdown,
            Some(command) => NextCommand::Command(command),
        }
    }

    fn apply(&mut self, command: Command) {
        match command {
            Command::Start => self.start(),
            Command::Halt => self.halt(),
            Command::Reset => self.reset(),
            Command::SetSpeed(speed) => self.set_speed(speed),
            Command::Sync(done) => {
                let goal = self.target().unwrap_or(Duration::ZERO);
                self.waiters.push(Waiter { goal, done });
            }
            Command::Shutdown => {}
        }
    }

    fn start(&mut self) {
        if self.failed {
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
    }

    fn halt(&mut self) {
        // Wall time up to now still counts, but Halt won't wait for a catch-up to finish.
        if !self.catching_up {
            self.advance_one_slice();
        }
        self.pace = None;
        self.catching_up = false;
    }

    fn reset(&mut self) {
        self.run.reset();
        self.pace = None;
        self.catching_up = false;
        self.failed = false;
        self.observe(Observation::State(self.run.state()));
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
    fn target(&self) -> Option<Duration> {
        self.pace.map(|pace| pace.target(self.clock.elapsed(), self.speed))
    }

    /// Advances towards the clock's time, by at most `EVENTS_PER_SLICE` events.
    fn advance_one_slice(&mut self) {
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
        self.waiters.retain(|waiter| {
            let reached = time >= waiter.goal && next_event.is_none_or(|next| next > waiter.goal);
            if everyone || reached {
                let _ = waiter.done.send(());
            }
            !(everyone || reached)
        });
    }

    fn report(&self, occurrences: Vec<Occurrence>) {
        if !occurrences.is_empty() {
            self.observe(Observation::Occurrences(occurrences));
        }
        self.observe(Observation::State(self.run.state()));
    }

    /// Reports everything that happened before an error, then the error, and halts.
    fn report_failure(&mut self, occurrences: Vec<Occurrence>, error: RunError) {
        self.report(occurrences);
        self.observe(Observation::Error(SessionError::Run(error)));
        self.pace = None;
        self.catching_up = false;
        self.failed = true;
    }

    fn observe(&self, observation: Observation) {
        // Nobody is listening once the handle is gone, and the thread is about to end then.
        let _ = self.observer.send(observation);
    }
}

enum NextCommand {
    Command(Command),
    None,
    Shutdown,
}
