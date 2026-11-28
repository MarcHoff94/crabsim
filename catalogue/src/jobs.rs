//! Runs as jobs: the observations of one experiment, run in the background on a pool with one
//! thread per core, each thread running one observation from start to end. Jobs take turns for
//! free threads, and every observation's results go into the results store when it ends.

use crate::error::OperationError;
use crate::model_files::read_file;
use crate::results::figures;
use crate::store::Store;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::time::{format_time_exact, parse_time};
use crabsim::{ModelDescription, SimTime, SimulationRun};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<StartRun>(), entry::<RunStatus>(), entry::<CancelRun>()]
}

/// How many events an observation runs between looking at its cancel flag and reporting how far
/// it has got. Small enough to cancel within milliseconds, large enough to cost nothing.
const CHUNK_EVENTS: usize = 100_000;

/// The most observations one run may have.
pub const MAX_OBSERVATIONS: u32 = 10_000;

/// The seed of observation `k`, counting from 1, of an experiment with seed `seed`:
/// `splitmix64(seed + k)`, with the sum wrapping. Fixed and documented, so the experiment's seed
/// alone reproduces every observation.
pub fn observation_seed(seed: u64, k: u32) -> u64 {
    let mut z = seed.wrapping_add(u64::from(k)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A run's state, as the store keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Waiting for a free thread.
    Queued,
    /// At least one observation has started.
    Running,
    /// Every observation ended, and at least one ran to its end.
    Done,
    /// Every observation failed with a run error.
    Failed,
    /// Cancelled before every observation ended.
    Cancelled,
    /// Its process ended before the run did, such as in a crash. The observations that ended
    /// keep their results.
    Interrupted,
}

impl RunState {
    pub fn name(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Interrupted => "interrupted",
        }
    }

    pub fn from_name(name: &str) -> RunState {
        match name {
            "queued" => RunState::Queued,
            "running" => RunState::Running,
            "done" => RunState::Done,
            "failed" => RunState::Failed,
            "cancelled" => RunState::Cancelled,
            _ => RunState::Interrupted,
        }
    }

    fn is_finished(self) -> bool {
        matches!(self, RunState::Done | RunState::Failed | RunState::Cancelled | RunState::Interrupted)
    }
}

/// One instance, as the results store names it.
pub(crate) struct ObjectInfo {
    pub name: String,
    pub class: String,
    pub built_in: &'static str,
}

/// Everything an observation needs, shared by all of a job's observations.
struct Plan {
    id: String,
    model: ModelDescription,
    objects: Vec<ObjectInfo>,
    seed: u64,
    length: SimTime,
    warm_up: SimTime,
    observations: u32,
    /// Without randomness: no object fails (ADR 0030).
    deterministic: bool,
    /// The model's expectations, which every observation reads its values for.
    expectations: Vec<crabsim_model::expect::Bound>,
    /// Every expectation's id, by its place in the model.
    expectation_ids: Vec<String>,
    /// What every observation watches for the expectations' limits on which MUs an object takes.
    watches: crabsim_model::expect::EntryWatches,
}

/// A job as this process knows it.
struct Job {
    plan: Arc<Plan>,
    /// The next observation to start, from 1.
    next: u32,
    /// The observations running now, with the simulated time each has reached.
    running: BTreeMap<u32, SimTime>,
    /// Observations that ended, failed or not.
    ended: u32,
    failed: u32,
    cancelled: bool,
    state: RunState,
}

impl Job {
    fn waiting(&self) -> bool {
        !self.cancelled && self.next <= self.plan.observations
    }
}

#[derive(Default)]
struct Pool {
    jobs: HashMap<String, Job>,
    /// Jobs with observations waiting, in the order they take turns.
    turns: Vec<String>,
    /// Whose turn is next.
    cursor: usize,
    shutdown: bool,
}

impl Pool {
    /// The next observation to run: from the next job in turn that has one waiting.
    fn take(&mut self) -> Option<(Arc<Plan>, u32)> {
        self.turns.retain(|id| self.jobs.get(id).is_some_and(Job::waiting));
        if self.turns.is_empty() {
            return None;
        }
        let at = self.cursor % self.turns.len();
        let job = self.jobs.get_mut(&self.turns[at]).expect("a job in turn exists");
        let k = job.next;
        job.next += 1;
        job.running.insert(k, SimTime::ZERO);
        job.state = RunState::Running;
        let plan = Arc::clone(&job.plan);
        // The next turn goes to the job after this one, or, if this one has nothing left
        // waiting, to the job that moves into its place.
        self.cursor = if job.waiting() { at + 1 } else { at };
        Some((plan, k))
    }
}

struct Shared {
    pool: Mutex<Pool>,
    /// Wakes threads when observations are waiting or the pool shuts down.
    work: Condvar,
    /// Wakes anyone waiting for a job to change, such as `cancel_run`.
    changed: Condvar,
    crabsim_folder: PathBuf,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pool> {
        self.pool.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The pool of threads running jobs' observations, started on the first run.
pub(crate) struct Jobs {
    shared: Arc<Shared>,
    threads: usize,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl Jobs {
    pub fn new(crabsim_folder: PathBuf, threads: usize) -> Jobs {
        let shared = Shared { pool: Mutex::default(), work: Condvar::new(), changed: Condvar::new(), crabsim_folder };
        Jobs { shared: Arc::new(shared), threads: threads.max(1), workers: Mutex::new(Vec::new()) }
    }

    fn submit(&self, plan: Plan) {
        {
            let mut workers = self.workers.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while workers.len() < self.threads {
                let shared = Arc::clone(&self.shared);
                workers.push(std::thread::Builder::new().name("crabsim-job".into()).spawn(move || work(&shared)).expect("a thread starts"));
            }
        }
        let mut pool = self.shared.lock();
        let id = plan.id.clone();
        let job = Job { plan: Arc::new(plan), next: 1, running: BTreeMap::new(), ended: 0, failed: 0, cancelled: false, state: RunState::Queued };
        pool.jobs.insert(id.clone(), job);
        pool.turns.push(id);
        drop(pool);
        self.shared.work.notify_all();
    }

    /// Waits until every job of this process has finished.
    pub fn wait_all(&self) {
        let mut pool = self.shared.lock();
        while pool.jobs.values().any(|job| !job.state.is_finished()) {
            pool = self.shared.changed.wait_timeout(pool, Duration::from_millis(100)).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
    }

    /// Whether any job of this process is still queued or running.
    pub fn any_going(&self) -> bool {
        self.shared.lock().jobs.values().any(|job| !job.state.is_finished())
    }

    /// The state of a job of this process.
    pub fn state(&self, id: &str) -> Option<RunState> {
        self.shared.lock().jobs.get(id).map(|job| job.state)
    }

    /// How a job of this process stands, or `None` if this process didn't run it.
    fn status(&self, id: &str) -> Option<Progress> {
        let pool = self.shared.lock();
        pool.jobs.get(id).map(Progress::of)
    }

    /// Cancels a job of this process and waits until none of its observations runs. `None` if
    /// this process didn't run it.
    fn cancel(&self, id: &str) -> Option<Progress> {
        let mut pool = self.shared.lock();
        let job = pool.jobs.get_mut(id)?;
        if !job.state.is_finished() {
            job.cancelled = true;
            if job.running.is_empty() {
                job.state = RunState::Cancelled;
                drop(pool);
                finish_in_store(&self.shared, id, RunState::Cancelled);
                pool = self.shared.lock();
            }
        }
        // The last observation still running sees the flag within one chunk of events, then
        // marks the job cancelled.
        let deadline = Instant::now() + Duration::from_secs(60);
        while pool.jobs.get(id).is_some_and(|job| !job.state.is_finished()) && Instant::now() < deadline {
            pool = self.shared.changed.wait_timeout(pool, Duration::from_millis(100)).unwrap_or_else(|poisoned| poisoned.into_inner()).0;
        }
        pool.jobs.get(id).map(Progress::of)
    }
}

impl Drop for Jobs {
    /// Cancels every job and waits for the threads, so nothing runs on after the catalogue.
    fn drop(&mut self) {
        {
            let mut pool = self.shared.lock();
            pool.shutdown = true;
            for job in pool.jobs.values_mut() {
                job.cancelled = true;
            }
        }
        self.shared.work.notify_all();
        let workers = std::mem::take(self.workers.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()));
        for worker in workers {
            let _ = worker.join();
        }
    }
}

/// A job's progress, as `run_status` reports it.
struct Progress {
    state: RunState,
    ended: u32,
    failed: u32,
    running: Vec<(u32, SimTime)>,
}

impl Progress {
    fn of(job: &Job) -> Progress {
        Progress { state: job.state, ended: job.ended, failed: job.failed, running: job.running.iter().map(|(&k, &t)| (k, t)).collect() }
    }
}

/// A pool thread: takes the next observation in turn and runs it, until the pool shuts down.
fn work(shared: &Shared) {
    let mut store: Option<Store> = None;
    loop {
        let (plan, k) = {
            let mut pool = shared.lock();
            loop {
                if pool.shutdown {
                    // Jobs cancelled by the shutdown, with nothing of them running now.
                    let ended: Vec<String> = pool
                        .jobs
                        .iter_mut()
                        .filter(|(_, job)| !job.state.is_finished() && job.running.is_empty())
                        .map(|(id, job)| {
                            job.state = RunState::Cancelled;
                            id.clone()
                        })
                        .collect();
                    drop(pool);
                    for id in ended {
                        finish_in_store(shared, &id, RunState::Cancelled);
                    }
                    return;
                }
                if let Some(next) = pool.take() {
                    break next;
                }
                pool = shared.work.wait(pool).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        if k == 1 {
            with_store(shared, &mut store, |store| {
                store.connection().execute("UPDATE runs SET state = 'running' WHERE id = ?1 AND state = 'queued'", [&plan.id])?;
                Ok(())
            });
        }
        let outcome = observe(shared, &plan, k);
        if let Some(outcome) = &outcome {
            with_store(shared, &mut store, |store| record(store, &plan, k, outcome));
        }
        let mut pool = shared.lock();
        let job = pool.jobs.get_mut(&plan.id).expect("a running job exists");
        job.running.remove(&k);
        if let Some(outcome) = &outcome {
            job.ended += 1;
            if outcome.error.is_some() {
                job.failed += 1;
            }
        }
        let last = job.running.is_empty() && !job.waiting();
        let state = if job.cancelled {
            RunState::Cancelled
        } else if job.failed == job.plan.observations {
            RunState::Failed
        } else {
            RunState::Done
        };
        if last {
            drop(pool);
            finish_in_store(shared, &plan.id, state);
            pool = shared.lock();
            pool.jobs.get_mut(&plan.id).expect("a running job exists").state = state;
        }
        drop(pool);
        shared.changed.notify_all();
    }
}

/// How an observation ended. `None` from `observe` means it was cancelled.
pub(crate) struct Outcome {
    pub seed: u64,
    pub events: u64,
    pub wall: Duration,
    /// The run error and the simulated time it happened.
    pub error: Option<(String, SimTime)>,
    pub state: crabsim::RunState,
    /// Each expectation's values, in the order of the plan's.
    pub observed: Vec<crabsim_model::expect::Observed>,
}

/// Runs observation `k` of `plan` from Init to its end, in chunks, reporting progress and
/// stopping early if the job is cancelled.
fn observe(shared: &Shared, plan: &Plan, k: u32) -> Option<Outcome> {
    let seed = observation_seed(plan.seed, k);
    let started = Instant::now();
    let mut run = if plan.deterministic { plan.model.build_deterministic(seed) } else { plan.model.build(seed) };
    if !plan.watches.list.is_empty() {
        run.watch(plan.watches.list.clone());
    }
    // The states a `last` stretch starts from, taken on the way: stopping changes nothing.
    let mut snapshots = std::collections::BTreeMap::new();
    let error = match run.init() {
        Err(error) => Some((error.to_string(), run.time())),
        Ok(_) => {
            let mut reached = |run: &SimulationRun| {
                let mut pool = shared.lock();
                let job = pool.jobs.get_mut(&plan.id).expect("a running job exists");
                job.running.insert(k, run.time());
                !job.cancelled
            };
            match advance(&mut run, plan.warm_up, &mut reached) {
                Ok(true) => {
                    if plan.warm_up > SimTime::ZERO {
                        run.clear_statistics();
                    }
                    let end = plan.warm_up + plan.length;
                    let mut stops = crabsim_model::expect::snapshot_times(&plan.expectations, plan.warm_up, plan.length);
                    stops.push(end);
                    let mut ended = None;
                    for stop in stops {
                        match advance(&mut run, stop, &mut reached) {
                            Ok(true) if stop < end => {
                                snapshots.insert(stop, run.state());
                            }
                            Ok(true) => {}
                            Ok(false) => return None,
                            Err(error) => {
                                ended = Some(error);
                                break;
                            }
                        }
                    }
                    ended
                }
                Ok(false) => return None,
                Err(error) => Some(error),
            }
        }
    };
    let state = run.state();
    let mut observed = Vec::new();
    if error.is_none() && !plan.expectations.is_empty() {
        observed = crabsim_model::expect::observe(&plan.expectations, &state, &snapshots, plan.warm_up, plan.length, &plan.watches);
        // Only an observation that broke a limit runs again, to find where.
        let broken = crabsim_model::expect::broken_limits(&plan.expectations, &observed);
        let limits: Vec<&crabsim_model::expect::Bound> = broken.iter().map(|&at| &plan.expectations[at]).collect();
        let found = crabsim_model::expect::first_violations(&plan.model, seed, plan.deterministic, plan.warm_up, plan.length, &limits);
        for (&at, violation) in broken.iter().zip(found) {
            observed[at].violation = violation;
        }
    }
    Some(Outcome { seed, events: run.events_processed(), wall: started.elapsed(), error, state, observed })
}

/// Advances `run` to `until` in chunks, asking `reached` after each whether to go on. `Ok(false)`
/// if told to stop, and the run error with its time if there was one.
fn advance(run: &mut SimulationRun, until: SimTime, reached: &mut impl FnMut(&SimulationRun) -> bool) -> Result<bool, (String, SimTime)> {
    loop {
        let advanced = run.advance_bounded(until, CHUNK_EVENTS).map_err(|failure| (failure.error.to_string(), run.time()))?;
        if !reached(run) {
            return Ok(false);
        }
        if advanced.reached {
            return Ok(true);
        }
    }
}

/// Writes an observation and its figures to the store, in one transaction.
fn record(store: &mut Store, plan: &Plan, k: u32, outcome: &Outcome) -> rusqlite::Result<()> {
    let transaction = store.connection_mut().transaction()?;
    let (error, error_time) = match &outcome.error {
        Some((error, time)) => (Some(error.as_str()), Some(time.as_secs_f64())),
        None => (None, None),
    };
    transaction.execute(
        "INSERT INTO observations (run, number, seed, events, wall_time_s, error, error_time_s) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![plan.id, k, outcome.seed.to_string(), outcome.events as i64, outcome.wall.as_secs_f64(), error, error_time],
    )?;
    if outcome.error.is_none() {
        let mut insert = transaction.prepare(
            "INSERT INTO statistics (run, observation, object, class, built_in_class, statistic, value) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut insert_bar = transaction.prepare("INSERT INTO histograms (run, observation, object, from_mus, to_mus, time_s) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        let mut insert_profile = transaction.prepare("INSERT INTO failure_profiles (run, observation, object, profile, failures, failed_s, time_basis, availability) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
        for (object, info) in outcome.state.objects.iter().zip(&plan.objects) {
            for profile in &object.failure_profiles {
                insert_profile.execute(params![
                    plan.id,
                    k,
                    info.name,
                    profile.name,
                    profile.failures as i64,
                    profile.time_failed.as_secs_f64(),
                    crabsim_model::time_basis_name(profile.time_basis),
                    profile.availability()
                ])?;
            }
            for (statistic, value) in figures(object, outcome.state.time) {
                insert.execute(params![plan.id, k, info.name, info.class, info.built_in, statistic, value])?;
            }
            if let Some(fill) = &object.fill {
                for (_, (from, to), time) in fill.histogram.used() {
                    insert_bar.execute(params![plan.id, k, info.name, from as i64, to.map(|to| to as i64), time.as_secs_f64()])?;
                }
            }
        }
        let mut insert_value = transaction.prepare("INSERT INTO expectation_values (run, observation, expectation, object, value) VALUES (?1, ?2, ?3, ?4, ?5)")?;
        let mut insert_violation = transaction
            .prepare("INSERT INTO violations (run, observation, expectation, time_s, object, mu, value, class, step) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)")?;
        for (bound, observed) in plan.expectations.iter().zip(&outcome.observed) {
            let id = &plan.expectation_ids[bound.index];
            for (subject, value) in &observed.values {
                insert_value.execute(params![plan.id, k, id, subject.map(|subject| plan.objects[subject as usize].name.as_str()), value])?;
            }
            if let Some(violation) = &observed.violation {
                insert_violation.execute(params![
                    plan.id,
                    k,
                    id,
                    violation.time.as_secs_f64(),
                    plan.objects[violation.object as usize].name,
                    violation.mu,
                    violation.value,
                    violation.class,
                    violation.step
                ])?;
            }
        }
    }
    transaction.commit()
}

fn finish_in_store(shared: &Shared, id: &str, state: RunState) {
    let mut store = None;
    with_store(shared, &mut store, |store| {
        store.connection().execute(
            "UPDATE runs SET state = ?2, finished = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![id, state.name()],
        )?;
        Ok(())
    });
}

/// Runs `write` on this thread's connection to the store, opening it if needed. A store that
/// can't be written loses the write and says so, rather than stopping the pool.
fn with_store(shared: &Shared, store: &mut Option<Store>, write: impl FnOnce(&mut Store) -> rusqlite::Result<()>) {
    if store.is_none() {
        match Store::open(&shared.crabsim_folder) {
            Ok(opened) => *store = Some(opened),
            Err(error) => {
                log::error!("the results store can't be opened: {error}");
                return;
            }
        }
    }
    if let Err(error) = write(store.as_mut().expect("opened above")) {
        log::error!("the results store can't be written: {error}");
    }
}

/// A time as an operation takes it, or why not.
fn time_input(name: &str, typed: &str) -> Result<SimTime, OperationError> {
    parse_time(typed).ok_or_else(|| OperationError::invalid_input(format!("`{name}` isn't a time: write it as 90, 1:30, 8:00:00 or 30d 12:00:00, not `{typed}`")))
}

/// The run `id` names, as `run-<n>`.
pub(crate) fn run_number(id: &str) -> Result<i64, OperationError> {
    id.strip_prefix("run-")
        .and_then(|number| number.parse::<i64>().ok())
        .ok_or_else(|| OperationError::not_found(format!("there is no run `{id}`: runs are called run-1, run-2 and so on")))
}

// --- start_run ---

pub struct StartRun;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRunInput {
    /// The model file to run, in the models folder, such as `line.json`.
    pub model: String,
    /// The experiment's seed. Observation k gets its own seed from it, so this seed alone
    /// reproduces every observation.
    #[serde(default)]
    pub seed: u64,
    /// How long each observation runs after the warm-up, in simulated time written as the UI
    /// shows it, such as "8:00:00", "30d" or "365d".
    pub length: String,
    /// Simulated time at the start of each observation that its statistics leave out, such as
    /// "1d". None if left out.
    pub warm_up: Option<String>,
    /// How many observations, from 1 to 10,000. They run in parallel, one per CPU core. Left
    /// out, 10, or 1 for a deterministic run, whose observations would all be the same.
    #[schemars(range(min = 1, max = 10_000), extend("default" = 10))]
    pub observations: Option<u32>,
    /// A run without randomness, for verification (ADR 0030): no object fails, and every random
    /// time is its mean. Its expectations are judged on one observation.
    #[serde(default)]
    pub deterministic: bool,
    /// A name for the run, such as "baseline" or "two more mills".
    pub label: Option<String>,
    /// Seconds to wait, from 1 to 60, for the run to end before answering, as run_status would.
    /// Left out, the answer comes at once.
    pub wait: Option<u32>,
}


#[derive(Serialize, JsonSchema)]
pub struct StartRunOutput {
    /// The run's id, such as `run-3`, for run_status, run_results and cancel_run.
    pub run: String,
    pub state: RunState,
    /// The model file was changed outside crabsim, or crabsim never saved it. It runs all the
    /// same, as it is now.
    pub changed_outside: bool,
    /// With `wait`, how the run stands when it ended or the wait was up, as run_status says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RunStatusOutput>,
    /// The runs verifying the model first, deterministically, since it has verification
    /// expectations and wasn't verified as it is now: get_status gives its stage once they end.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<String>,
}

/// A model file read and checked for running.
pub(crate) struct Prepared {
    pub model: crabsim_model::Model,
    pub description: ModelDescription,
    /// Its canonical text and that text's hash, as runs record them.
    pub canonical: String,
    pub hash: String,
    pub changed_outside: bool,
}

/// Reads and checks the model file at `path`, whose text is `text`, with its code.
pub(crate) fn prepare(catalogue: &Catalogue, path: &str, text: &str) -> Result<Prepared, OperationError> {
    let changed_outside = crate::editing::changed_outside(catalogue, path, text)?;
    let model = crabsim_model::load(text).map_err(|problems| OperationError::invalid_model(&format!("`{path}`"), &problems))?;
    // Its code, from the folder's `code/`, is loaded and checked now too (ADR 0029).
    let description = model
        .to_description_in(catalogue.folder().root(), crabsim_model::crabsim_code::Options::default())
        .map_err(|problems| OperationError::invalid_model(&format!("`{path}`"), &problems))?;
    let canonical = model.save();
    let hash: String = Sha256::digest(canonical.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(Prepared { model, description, canonical, hash, changed_outside })
}

/// A stage's check that a run is part of.
pub(crate) struct CheckPart {
    pub stage: crabsim_model::CheckStage,
    /// The number of the check's first run, `None` for that run itself.
    pub number: Option<i64>,
    /// The newest change in the file's history it checks.
    pub change: Option<i64>,
    /// The expectations it judges, by their place in the model.
    pub judged: Vec<usize>,
}

/// How to run a prepared model.
pub(crate) struct Launch {
    pub seed: u64,
    pub length: SimTime,
    pub warm_up: SimTime,
    pub observations: u32,
    pub deterministic: bool,
    pub label: Option<String>,
    pub check: Option<CheckPart>,
}

/// Records a run of a prepared model in the store and queues its observations, giving its id.
pub(crate) fn launch_run(catalogue: &Catalogue, path: &str, prepared: &Prepared, launch: Launch) -> Result<String, OperationError> {
    let model = &prepared.model;
    let objects = (0..model.instances.len())
        .map(|index| ObjectInfo {
            name: model.instances[index].name.clone(),
            class: model.instances[index].class.clone(),
            built_in: model.resolve(index).expect("a valid model resolves").built_in,
        })
        .collect();
    let who = catalogue.who()?;
    let store = catalogue.store()?;
    let (stage, check_number, change, judged) = match &launch.check {
        None => (None, None, None, None),
        Some(check) => {
            let ids: Vec<&str> = check.judged.iter().map(|&index| model.expectations[index].id.as_str()).collect();
            (Some(check.stage.name()), check.number, check.change, Some(serde_json::to_string(&ids).expect("ids serialise")))
        }
    };
    store.connection().execute(
        "INSERT INTO runs (model_path, model_text, model_hash, seed, length_s, warm_up_s, observations, label, source, crabsim_version, started, state, process, crabsim_build, agent, deterministic, stage, check_number, checked_change, expectations)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'queued', ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            path,
            prepared.canonical,
            prepared.hash,
            launch.seed.to_string(),
            launch.length.as_secs_f64(),
            launch.warm_up.as_secs_f64(),
            launch.observations,
            launch.label,
            catalogue.source().name(),
            env!("CARGO_PKG_VERSION"),
            catalogue.process_id(),
            env!("CRABSIM_BUILD"),
            who,
            launch.deterministic,
            stage,
            check_number,
            change,
            judged,
        ],
    )?;
    let number = store.connection().last_insert_rowid();
    // The check's first run gives the check its number.
    if launch.check.as_ref().is_some_and(|check| check.number.is_none()) {
        store.connection().execute("UPDATE runs SET check_number = ?1 WHERE number = ?1", [number])?;
    }
    drop(store);
    let id = format!("run-{number}");
    let mut expectations = model.bound_expectations();
    if let Some(check) = &launch.check {
        expectations.retain(|bound| check.judged.contains(&bound.index));
    }
    let plan = Plan {
        id: id.clone(),
        model: prepared.description.clone(),
        objects,
        seed: launch.seed,
        length: launch.length,
        warm_up: launch.warm_up,
        observations: launch.observations,
        deterministic: launch.deterministic,
        watches: model.entry_watches(&expectations),
        expectations,
        expectation_ids: model.expectations.iter().map(|expectation| expectation.id.clone()).collect(),
    };
    catalogue.jobs().submit(plan);
    Ok(id)
}

impl Operation for StartRun {
    const NAME: &'static str = "start_run";
    const DESCRIPTION: &'static str = "Starts an experiment on a model file in the models folder: its observations run in the background, in parallel, one per CPU core, and their results go into the results store. Returns the run's id at once, or with `wait` once it ended: poll run_status until it's done, then read run_results. The model is read and checked now, and an invalid one is refused with every problem. Later changes to the file don't reach the run. Observation k (from 1) gets the seed splitmix64(seed + k), reported with its results. Give `wait`, up to 60 seconds, to have a short run's end waited for, saving a call to run_status. Runs with the same seed and number of observations are paired, even of different models: each instance's random numbers come from the seed and its stream, which it keeps through every edit, so a variant gets common random numbers with its original, and their difference has a far narrower interval than either alone. 10 observations is a start: run more if an interval is too wide to decide.";
    const EFFECT: Effect = Effect::Additive;
    type Input = StartRunInput;
    type Output = StartRunOutput;

    fn run(catalogue: &Catalogue, input: StartRunInput) -> Result<StartRunOutput, OperationError> {
        let length = time_input("length", &input.length)?;
        if length == SimTime::ZERO {
            return Err(OperationError::invalid_input("`length` must be longer than 0"));
        }
        let warm_up = input.warm_up.as_deref().map(|typed| time_input("warm_up", typed)).transpose()?.unwrap_or(SimTime::ZERO);
        let observations = input.observations.unwrap_or(if input.deterministic { 1 } else { 10 });
        if !(1..=MAX_OBSERVATIONS).contains(&observations) {
            return Err(OperationError::invalid_input(format!("`observations` must be from 1 to {MAX_OBSERVATIONS}")));
        }
        if input.wait.is_some_and(|wait| !(1..=60).contains(&wait)) {
            return Err(OperationError::invalid_input("`wait` must be from 1 to 60 seconds"));
        }
        let (_, text) = read_file(catalogue, &input.model)?;
        let prepared = prepare(catalogue, &input.model, &text)?;
        // An experiment is verified first, unless its model already is as it is now (ADR 0030).
        let verification = if input.deterministic { Vec::new() } else { crate::stages::verify_if_unchecked(catalogue, &input.model, &prepared)? };
        let launch = Launch {
            seed: input.seed,
            length,
            warm_up,
            observations,
            deterministic: input.deterministic,
            label: input.label.clone(),
            check: None,
        };
        let id = launch_run(catalogue, &input.model, &prepared, launch)?;
        let changed_outside = prepared.changed_outside;
        let Some(wait) = input.wait else {
            return Ok(StartRunOutput { run: id, state: RunState::Queued, changed_outside, status: None, verification });
        };
        // Waits for a short run, saving the agent a call to run_status.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(u64::from(wait));
        loop {
            let status = RunStatus::run(catalogue, RunInput { run: id.clone() })?;
            if !matches!(status.state, RunState::Queued | RunState::Running) || std::time::Instant::now() >= deadline {
                return Ok(StartRunOutput { run: id, state: status.state, changed_outside, status: Some(status), verification });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

// --- run_status ---

pub struct RunStatus;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    /// The run's id, such as `run-3`.
    pub run: String,
}

/// An observation running now.
#[derive(Serialize, JsonSchema)]
pub struct RunningObservation {
    /// From 1.
    pub observation: u32,
    /// The simulated time it has reached, warm-up included, as the UI shows time.
    pub reached: String,
}

#[derive(Serialize, JsonSchema)]
pub struct RunStatusOutput {
    pub run: String,
    pub state: RunState,
    /// How many observations the run has.
    pub observations: u32,
    /// How many have ended, including failed ones.
    pub ended: u32,
    /// How many ended with a run error.
    pub failed: u32,
    /// The observations running now, and how far each has got.
    pub running: Vec<RunningObservation>,
    /// Whether this process is running it. A run started by another crabsim process shows only
    /// what the store holds.
    pub here: bool,
}

impl Operation for RunStatus {
    const NAME: &'static str = "run_status";
    const DESCRIPTION: &'static str = "Tells how a run stands: queued, running, done, failed, cancelled, or interrupted when the process running it ended first, how many of its observations have ended, and the simulated time each running observation has reached, so a slow run can be told from a stuck one. A run is done once every observation ended and at least one ran to its end, and failed only if every observation failed. `here` says whether this server is running it: a run started by another crabsim process, such as the CLI, shows only what the results store holds, and can't be cancelled from here.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = RunInput;
    type Output = RunStatusOutput;

    fn run(catalogue: &Catalogue, input: RunInput) -> Result<RunStatusOutput, OperationError> {
        status(catalogue, &input.run, catalogue.jobs().status(&input.run))
    }
}

/// A run's status, from this process's job if there is one, and otherwise from the store.
fn status(catalogue: &Catalogue, id: &str, progress: Option<Progress>) -> Result<RunStatusOutput, OperationError> {
    let number = run_number(id)?;
    let store = catalogue.swept_store()?;
    let stored = store
        .connection()
        .query_row(
            "SELECT state, observations, (SELECT count(*) FROM observations WHERE run = ?2), (SELECT count(*) FROM observations WHERE run = ?2 AND error IS NOT NULL)
             FROM runs WHERE number = ?1",
            params![number, id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?, row.get::<_, u32>(2)?, row.get::<_, u32>(3)?)),
        )
        .optional()?;
    let Some((state, observations, ended, failed)) = stored else {
        return Err(OperationError::not_found(format!("there is no run `{id}` in the results store")));
    };
    Ok(match progress {
        Some(progress) => RunStatusOutput {
            run: id.to_string(),
            state: progress.state,
            observations,
            ended: progress.ended,
            failed: progress.failed,
            running: progress.running.iter().map(|&(k, time)| RunningObservation { observation: k, reached: format_time_exact(time) }).collect(),
            here: true,
        },
        None => RunStatusOutput { run: id.to_string(), state: RunState::from_name(&state), observations, ended, failed, running: Vec::new(), here: false },
    })
}

// --- cancel_run ---

pub struct CancelRun;

impl Operation for CancelRun {
    const NAME: &'static str = "cancel_run";
    const DESCRIPTION: &'static str = "Cancels a run: observations not started yet never start, and running ones stop within moments. Returns once nothing of the run is running. Observations that already ended keep their results. Cancelling a run that has finished changes nothing.";
    const EFFECT: Effect = Effect::Additive;
    type Input = RunInput;
    type Output = RunStatusOutput;

    fn run(catalogue: &Catalogue, input: RunInput) -> Result<RunStatusOutput, OperationError> {
        run_number(&input.run)?;
        let progress = catalogue.jobs().cancel(&input.run);
        let status = status(catalogue, &input.run, progress)?;
        if !status.here && !status.state.is_finished() {
            return Err(OperationError::invalid_input(format!(
                "`{}` was started by another crabsim process, which is the only one that can cancel it",
                input.run
            )));
        }
        Ok(status)
    }
}
