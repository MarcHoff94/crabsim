//! Reading a run's results from the store: the figures each observation keeps, and the mean,
//! 95% confidence interval, minimum and maximum over the observations. Also labelling and
//! forgetting runs.

use crate::error::OperationError;
use crate::jobs::{run_number, RunState};
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::time::format_time_exact;
use crabsim::{ObjectStatistics, SimTime};
use rusqlite::{params, OptionalExtension};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<RunResults>(), entry::<LabelRun>(), entry::<ForgetRun>()]
}

/// Every statistic the store keeps, in the order results show them.
pub const STATISTICS: [&str; 11] = [
    "mus_in",
    "mus_out",
    "working_share",
    "blocked_share",
    "idle_share",
    "failed_share",
    "paused_share",
    "failures",
    "availability",
    "throughput_per_hour",
    "mean_lead_time_s",
];

/// How many of the busiest, the most blocked and the most failed objects a summary shows.
const SUMMARY_TOP: usize = 10;

/// A model with at most this many instances is shown whole in a summary, since picking the most
/// of anything from so few would pick nearly all of them anyway.
const SMALL_MODEL: usize = 20;

/// The blocked share below which an object isn't worth showing as most blocked.
const BLOCKED_FLOOR: f64 = 0.01;

/// An object's figures at the end of an observation, by their names in the store. A figure that
/// doesn't exist, such as a share before any time was measured, is left out.
pub(crate) fn figures(statistics: &ObjectStatistics, now: SimTime) -> Vec<(&'static str, f64)> {
    let mut figures = vec![("mus_in", statistics.mus_in as f64), ("mus_out", statistics.mus_out as f64)];
    let measured = statistics.measured_time().as_millis() as f64;
    if measured > 0.0 {
        for (name, time) in [
            ("working_share", statistics.time_busy),
            ("blocked_share", statistics.time_blocked),
            ("idle_share", statistics.time_idle),
            ("failed_share", statistics.time_failed),
            ("paused_share", statistics.time_paused),
        ] {
            figures.push((name, time.as_millis() as f64 / measured));
        }
    }
    figures.push(("failures", statistics.failures as f64));
    if let Some(availability) = statistics.achieved_availability() {
        figures.push(("availability", availability));
    }
    if let Some(per_hour) = statistics.mus_per_hour(now) {
        figures.push(("throughput_per_hour", per_hour));
    }
    if let Some(lead_time) = statistics.lead_time_mean() {
        figures.push(("mean_lead_time_s", lead_time.as_secs_f64()));
    }
    figures
}

/// The 97.5% quantile of Student's t distribution with `df` degrees of freedom, for a two-sided
/// 95% interval.
pub(crate) fn t_quantile(df: usize) -> f64 {
    const TABLE: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160, 2.145, 2.131, 2.120, 2.110, 2.101,
        2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056, 2.052, 2.048, 2.045, 2.042,
    ];
    if (1..=30).contains(&df) {
        return TABLE[df - 1];
    }
    // Beyond the table, the Cornish-Fisher expansion around the normal quantile, accurate to
    // better than 0.001 from 30 degrees of freedom on.
    let z: f64 = 1.959_964;
    let n = df as f64;
    z + (z.powi(3) + z) / (4.0 * n) + (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / (96.0 * n * n)
        + (3.0 * z.powi(7) + 19.0 * z.powi(5) + 17.0 * z.powi(3) - 15.0 * z) / (384.0 * n.powi(3))
}

/// `value` as results show it: to 4 significant digits, but never fewer than its whole number, so
/// 0.74604 is 0.746, 182.85 is 182.9 and 131650.75 is 131651. The store keeps every digit.
pub(crate) fn shown(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let decimals = 3 - value.abs().log10().floor() as i32;
    if decimals <= 0 {
        return value.round();
    }
    let scale = 10f64.powi(decimals);
    (value * scale).round() / scale
}

/// The mean of `values` and its 95% confidence interval (Student's t), the interval `None` for
/// fewer than two values.
fn mean_and_interval(values: &[f64]) -> (f64, Option<[f64; 2]>) {
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let interval = (values.len() >= 2).then(|| {
        let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let half = t_quantile(values.len() - 1) * (variance / n).sqrt();
        [mean - half, mean + half]
    });
    (mean, interval)
}

/// One figure over a run's observations, to 4 significant digits.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Figure {
    /// The mean over the observations that have this figure.
    pub mean: f64,
    /// The 95% confidence interval of the mean (Student's t), with fewer than two values none.
    pub ci95: Option<[f64; 2]>,
    pub min: f64,
    pub max: f64,
    /// Each observation's own value, in order from observation 1, null where it has none, such as
    /// a failed observation. Only when asked for with `values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Option<f64>>>,
}

impl Figure {
    /// The figure from each observation's value, with each one's own value if `each`, or `None`
    /// if no observation has one, or every one has 0.
    fn of(values: &[Option<f64>], each: bool) -> Option<Figure> {
        let present: Vec<f64> = values.iter().flatten().copied().collect();
        if present.iter().all(|value| *value == 0.0) {
            return None;
        }
        let (mean, interval) = mean_and_interval(&present);
        let min = present.iter().copied().fold(f64::INFINITY, f64::min);
        let max = present.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let values = each.then(|| values.iter().map(|value| value.map(shown)).collect());
        Some(Figure { mean: shown(mean), ci95: interval.map(|[low, high]| [shown(low), shown(high)]), min: shown(min), max: shown(max), values })
    }
}

/// Why an object is in a summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The model is small enough to show whole.
    All,
    Drain,
    /// Among the 10 with the highest working share: where a bottleneck is.
    Busiest,
    /// Among the 10 with the highest blocked share, above 1%.
    MostBlocked,
    /// Among the 10 whose failed share's 95% interval lies above that of the median object.
    MostFailed,
    Chosen,
}

/// One object's figures over a run's observations.
#[derive(Serialize, JsonSchema)]
pub struct ObjectResults {
    pub name: String,
    pub class: String,
    pub built_in_class: String,
    /// Why it's in these results.
    pub why: Vec<Why>,
    /// By statistic: mus_in, mus_out, working_share, blocked_share, idle_share, failed_share,
    /// paused_share, failures, availability, and for Drains throughput_per_hour and
    /// mean_lead_time_s, named as in the store. Shares and availability are from 0 to 1, times in
    /// seconds. A figure that is 0 in every observation is left out.
    pub figures: BTreeMap<String, Figure>,
}

/// One observation of a run.
#[derive(Clone, Serialize, JsonSchema)]
pub struct ObservationResults {
    /// From 1.
    pub observation: u32,
    /// Its own seed: `crabsim --model <file> --seed <seed>` reruns it alone.
    pub seed: String,
    pub events: u64,
    pub wall_time_s: f64,
    /// The run error that ended it, if one did.
    pub error: Option<String>,
    /// The simulated time of the error.
    pub error_time: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunResultsOutput {
    pub run: String,
    pub label: Option<String>,
    pub state: RunState,
    /// The model file it ran, as it was then.
    pub model: String,
    /// SHA-256 of the canonical model it ran: equal hashes mean the same model.
    pub model_hash: String,
    pub seed: String,
    pub length: String,
    pub warm_up: String,
    pub observations: u32,
    /// Who started it: mcp or cli.
    pub source: String,
    pub crabsim_version: String,
    /// The git commit crabsim was built from, `-dirty` with uncommitted changes, or unknown.
    pub crabsim_build: String,
    /// Who started it: such as `agent-3 (claude-code)`, or the user. Unknown for older runs.
    pub agent: String,
    pub started: String,
    pub finished: Option<String>,
    /// The events of every observation that ended, added up.
    pub events: u64,
    /// The wall time of every observation that ended, added up, in seconds. They run in
    /// parallel, so the run took less.
    pub wall_time_s: f64,
    /// Every observation that ended with a run error, in order, usually none.
    pub failed_observations: Vec<ObservationResults>,
    /// Every observation that ended, in order. Only when asked for with `values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_runs: Option<Vec<ObservationResults>>,
    /// The objects asked for, or by default every Drain and the most blocked and most failed
    /// objects, in model order.
    pub objects: Vec<ObjectResults>,
}

// --- run_results ---

pub struct RunResults;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunResultsInput {
    /// The run's id, such as `run-3`.
    pub run: String,
    /// Instance names to show instead of the summary.
    pub objects: Option<Vec<String>>,
    /// Classes whose instances to show instead of the summary: every instance whose chain of
    /// classes includes one of them, built-in classes such as Drain too.
    pub classes: Option<Vec<String>>,
    /// Also each observation's own value of every figure, and each observation's seed, events
    /// and wall time. Left out, only the mean, interval, minimum and maximum, so the results
    /// stay the same size whatever the number of observations.
    #[serde(default)]
    pub values: bool,
}

impl Operation for RunResults {
    const NAME: &'static str = "run_results";
    const DESCRIPTION: &'static str = "Reads a run's results from the results store: its settings, `events` and `wall_time_s` (every observation's, added up: they run in parallel, so the run took less), any observation that failed with its error, and per object every figure's mean over the observations, 95% confidence interval, minimum and maximum, to 4 significant digits. Figures are named as in the store, such as blocked_share (0 to 1) and mean_lead_time_s (seconds), and those that are 0 in every observation are left out. By default a short summary whatever the model's size or number of observations: a model of 20 or fewer instances whole, and otherwise every Drain, the 10 busiest (highest working share, where a bottleneck is), the 10 most blocked above 1%, and the objects whose failed share is clearly above the rest. Each object's `why` says which of these put it there: all, drain, busiest, most_blocked, most_failed, or chosen. Give `objects` or `classes` to read exactly those instead, and `values` for each observation's own figures and seed. For full precision, comparing runs, or anything else, use query_results.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = RunResultsInput;
    type Output = RunResultsOutput;

    fn run(catalogue: &Catalogue, input: RunResultsInput) -> Result<RunResultsOutput, OperationError> {
        let number = run_number(&input.run)?;
        let id = input.run;
        let store = catalogue.store()?;
        let connection = store.connection();
        let run = connection
            .query_row(
                "SELECT label, state, model_path, model_text, model_hash, seed, length_s, warm_up_s, observations, source, crabsim_version, started, finished, coalesce(crabsim_build, 'unknown'), coalesce(agent, 'unknown') FROM runs WHERE number = ?1",
                [number],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, f64>(6)?,
                        row.get::<_, f64>(7)?,
                        row.get::<_, u32>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, String>(14)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| OperationError::not_found(format!("there is no run `{id}` in the results store")))?;
        let (label, state, model_path, model_text, model_hash, seed, length, warm_up, observations, source, version, started, finished, build, agent) = run;
        let state = catalogue.jobs_status(&id).unwrap_or(RunState::from_name(&state));

        let mut statement =
            connection.prepare("SELECT number, seed, events, wall_time_s, error, error_time_s FROM observations WHERE run = ?1 ORDER BY number")?;
        let observation_runs = statement
            .query_map([&id], |row| {
                Ok(ObservationResults {
                    observation: row.get(0)?,
                    seed: row.get(1)?,
                    events: row.get::<_, i64>(2)? as u64,
                    wall_time_s: row.get(3)?,
                    error: row.get(4)?,
                    error_time: row.get::<_, Option<f64>>(5)?.map(|secs| format_time_exact(SimTime::from_secs_f64(secs))),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let events = observation_runs.iter().map(|observation| observation.events).sum();
        let wall_time_s = shown(observation_runs.iter().map(|observation| observation.wall_time_s).sum());
        let failed_observations: Vec<ObservationResults> = observation_runs.iter().filter(|observation| observation.error.is_some()).cloned().collect();
        let observation_runs = input.values.then_some(observation_runs);

        // Which objects, and why.
        let chosen = choose_objects(connection, &id, &model_text, input.objects.as_deref(), input.classes.as_deref())?;
        let mut why: HashMap<String, Vec<Why>> = HashMap::new();
        for (name, reason) in chosen {
            let reasons = why.entry(name).or_default();
            if !reasons.contains(&reason) {
                reasons.push(reason);
            }
        }

        // Their figures, in model order: the order they were written in.
        let mut rows = connection.prepare(
            "SELECT object, class, built_in_class, observation, statistic, value FROM statistics WHERE run = ?1 AND object = ?2 ORDER BY rowid",
        )?;
        let order = model_order(&model_text);
        let mut names: Vec<&String> = why.keys().collect();
        names.sort_by_key(|name| order.get(name.as_str()).copied().unwrap_or(usize::MAX));
        let mut objects = Vec::new();
        for name in names {
            let mut class = String::new();
            let mut built_in = String::new();
            let mut values: HashMap<String, Vec<Option<f64>>> = HashMap::new();
            let found = rows.query_map(params![id, name], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, u32>(3)?, row.get::<_, String>(4)?, row.get::<_, f64>(5)?))
            })?;
            for row in found {
                let (row_class, row_built_in, observation, statistic, value) = row?;
                class = row_class;
                built_in = row_built_in;
                let slot = values.entry(statistic).or_insert_with(|| vec![None; observations as usize]);
                if let Some(slot) = slot.get_mut(observation as usize - 1) {
                    *slot = Some(value);
                }
            }
            if class.is_empty() {
                // A chosen object with no figures, as when every observation failed.
                let model = crabsim_model::load(&model_text).ok();
                if let Some(index) = model.as_ref().and_then(|model| model.instances.iter().position(|instance| &instance.name == name)) {
                    let model = model.as_ref().expect("loaded above");
                    class = model.instances[index].class.clone();
                    built_in = model.resolve(index).map_or("", |resolved| resolved.built_in).to_string();
                }
            }
            let mut figures = BTreeMap::new();
            for statistic in STATISTICS {
                let Some(values) = values.get(statistic) else { continue };
                if let Some(figure) = Figure::of(values, input.values) {
                    figures.insert(statistic.to_string(), figure);
                }
            }
            objects.push(ObjectResults { name: name.clone(), class, built_in_class: built_in, why: why[name].clone(), figures });
        }

        Ok(RunResultsOutput {
            run: id,
            label,
            state,
            model: model_path,
            model_hash,
            seed,
            length: format_time_exact(SimTime::from_secs_f64(length)),
            warm_up: format_time_exact(SimTime::from_secs_f64(warm_up)),
            observations,
            source,
            crabsim_version: version,
            crabsim_build: build,
            agent,
            started,
            finished,
            events,
            wall_time_s,
            failed_observations,
            observation_runs,
            objects,
        })
    }
}

/// The objects a run's results show, and why: with `objects` or `classes` exactly those, and
/// otherwise a small model whole, or every Drain, the busiest, the most blocked and the clearly
/// most failed. An object or class the model doesn't have is `not_found`.
pub(crate) fn choose_objects(
    connection: &rusqlite::Connection,
    id: &str,
    model_text: &str,
    objects: Option<&[String]>,
    classes: Option<&[String]>,
) -> Result<Vec<(String, Why)>, OperationError> {
    let mut chosen: Vec<(String, Why)> = Vec::new();
    let read_back = || crabsim_model::load(model_text).map_err(|_| OperationError::run_error(format!("the model `{id}` ran can't be read back")));
    let instances = crabsim_model::load(model_text).map_or(0, |model| model.instances.len());
    if objects.is_none() && classes.is_none() && instances <= SMALL_MODEL {
        chosen.extend(read_back()?.instances.iter().map(|instance| (instance.name.clone(), Why::All)));
    } else if objects.is_none() && classes.is_none() {
        let drains = connection.prepare("SELECT DISTINCT object FROM statistics WHERE run = ?1 AND built_in_class = 'Drain'")?.query_map([id], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
        chosen.extend(drains.into_iter().map(|name| (name, Why::Drain)));
        for (statistic, floor, why) in [("working_share", 0.0, Why::Busiest), ("blocked_share", BLOCKED_FLOOR, Why::MostBlocked)] {
            let top = connection
                .prepare(
                    "SELECT object FROM statistics WHERE run = ?1 AND statistic = ?2 GROUP BY object HAVING avg(value) > ?3
                     ORDER BY avg(value) DESC, min(rowid) LIMIT ?4",
                )?
                .query_map(params![id, statistic, floor, SUMMARY_TOP as i64], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            chosen.extend(top.into_iter().map(|name| (name, why)));
        }
        chosen.extend(clearly_most_failed(connection, id)?.into_iter().map(|name| (name, Why::MostFailed)));
    } else {
        let model = read_back()?;
        for name in objects.into_iter().flatten() {
            if !model.instances.iter().any(|instance| &instance.name == name) {
                return Err(OperationError::not_found(format!("the model `{id}` ran has no instance called `{name}`")));
            }
            chosen.push((name.clone(), Why::Chosen));
        }
        for class in classes.into_iter().flatten() {
            if model.class(class).is_none() && !crabsim_model::BUILT_IN_CLASSES.contains(&class.as_str()) {
                return Err(OperationError::not_found(format!("the model `{id}` ran has no class called `{class}`")));
            }
            for instance in &model.instances {
                let Some((chain, built_in)) = model.chain(&instance.class) else { continue };
                if built_in == class || chain.iter().any(|link| &link.name == class) {
                    chosen.push((instance.name.clone(), Why::Chosen));
                }
            }
        }
    }
    Ok(chosen)
}

/// The objects, at most 10, whose failed share's 95% interval lies wholly above that of the median
/// object, by mean, so that objects failing alike, such as instances of one class, aren't ranked
/// by chance. With fewer than two observations there are no intervals, and so none.
fn clearly_most_failed(connection: &rusqlite::Connection, run: &str) -> Result<Vec<String>, OperationError> {
    let mut values: Vec<(String, Vec<f64>)> = Vec::new();
    let mut rows = connection.prepare("SELECT object, value FROM statistics WHERE run = ?1 AND statistic = 'failed_share' ORDER BY rowid")?;
    for row in rows.query_map([run], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))? {
        let (object, value) = row?;
        match values.iter_mut().find(|(name, _)| *name == object) {
            Some((_, found)) => found.push(value),
            None => values.push((object, vec![value])),
        }
    }
    let mut intervals: Vec<(String, f64, [f64; 2])> = values
        .into_iter()
        .filter_map(|(name, values)| {
            let (mean, interval) = mean_and_interval(&values);
            interval.map(|interval| (name, mean, interval))
        })
        .collect();
    if intervals.is_empty() {
        return Ok(Vec::new());
    }
    intervals.sort_by(|a, b| a.1.total_cmp(&b.1));
    let median_high = intervals[intervals.len() / 2].2[1];
    let mut above: Vec<(String, f64)> = intervals.into_iter().filter(|(_, _, [low, _])| *low > median_high).map(|(name, mean, _)| (name, mean)).collect();
    above.sort_by(|a, b| b.1.total_cmp(&a.1));
    Ok(above.into_iter().take(SUMMARY_TOP).map(|(name, _)| name).collect())
}

/// Each instance's place in the model, by name.
fn model_order(model_text: &str) -> HashMap<String, usize> {
    crabsim_model::load(model_text)
        .map(|model| model.instances.iter().enumerate().map(|(index, instance)| (instance.name.clone(), index)).collect())
        .unwrap_or_default()
}

/// Refuses an agent a change to a run the user started: agents may read the user's runs, but
/// only the user may relabel or forget them (ADR 0011).
fn refuse_users_run(catalogue: &Catalogue, number: i64, run: &str, what: &str) -> Result<(), OperationError> {
    if catalogue.source() != crate::Source::Mcp {
        return Ok(());
    }
    let source: Option<String> = catalogue.store()?.connection().query_row("SELECT source FROM runs WHERE number = ?1", [number], |row| row.get(0)).optional()?;
    if source.as_deref().is_some_and(|source| source != crate::Source::Mcp.name()) {
        return Err(OperationError::invalid_input(format!("`{run}` was started by the user, so only the user can {what} it. Agents may read it")));
    }
    Ok(())
}

// --- label_run ---

pub struct LabelRun;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LabelRunInput {
    /// The run's id, such as `run-3`.
    pub run: String,
    /// The new label, such as "baseline". Left out, the label is removed.
    pub label: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct LabelRunOutput {
    pub run: String,
    pub label: Option<String>,
}

impl Operation for LabelRun {
    const NAME: &'static str = "label_run";
    const DESCRIPTION: &'static str = "Sets a run's label, such as \"baseline\" or \"two more mills\", to tell runs apart, or removes it.";
    const EFFECT: Effect = Effect::Additive;
    type Input = LabelRunInput;
    type Output = LabelRunOutput;

    fn run(catalogue: &Catalogue, input: LabelRunInput) -> Result<LabelRunOutput, OperationError> {
        let number = run_number(&input.run)?;
        refuse_users_run(catalogue, number, &input.run, "relabel")?;
        let changed = catalogue.store()?.connection().execute("UPDATE runs SET label = ?2 WHERE number = ?1", params![number, input.label])?;
        if changed == 0 {
            return Err(OperationError::not_found(format!("there is no run `{}` in the results store", input.run)));
        }
        Ok(LabelRunOutput { run: input.run, label: input.label })
    }
}

// --- forget_run ---

pub struct ForgetRun;

#[derive(Serialize, JsonSchema)]
pub struct ForgetRunOutput {
    /// The run whose results were deleted.
    pub forgotten: String,
}

impl Operation for ForgetRun {
    const NAME: &'static str = "forget_run";
    const DESCRIPTION: &'static str = "Deletes a run and all its results from the results store, for good. An agent may forget only the runs its own session started, and never the user's. Refused while the run is still queued or running here: cancel it first.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = crate::jobs::RunInput;
    type Output = ForgetRunOutput;

    fn run(catalogue: &Catalogue, input: crate::jobs::RunInput) -> Result<ForgetRunOutput, OperationError> {
        let number = run_number(&input.run)?;
        refuse_users_run(catalogue, number, &input.run, "forget")?;
        if catalogue.source() == crate::Source::Mcp {
            let process: Option<Option<String>> = catalogue.store()?.connection().query_row("SELECT process FROM runs WHERE number = ?1", [number], |row| row.get(0)).optional()?;
            if let Some(process) = process
                && process.as_deref() != Some(catalogue.process_id())
            {
                return Err(OperationError::invalid_input(format!(
                    "`{}` was started by another agent session, so this one can't forget it: an agent forgets only its own runs. Label it instead, or ask the user",
                    input.run
                )));
            }
        }
        if catalogue.jobs_status(&input.run).is_some_and(|state| matches!(state, RunState::Queued | RunState::Running)) {
            return Err(OperationError::invalid_input(format!("`{}` is still running: cancel it first", input.run)));
        }
        let mut store = catalogue.swept_store()?;
        let state: Option<String> = store.connection().query_row("SELECT state FROM runs WHERE number = ?1", [number], |row| row.get(0)).optional()?;
        if state.as_deref().is_some_and(|state| state == "queued" || state == "running") {
            return Err(OperationError::invalid_input(format!(
                "`{}` is still running in another crabsim process: cancel it there first",
                input.run
            )));
        }
        let transaction = store.connection_mut().transaction()?;
        let deleted = transaction.execute("DELETE FROM runs WHERE number = ?1", [number])?;
        if deleted == 0 {
            return Err(OperationError::not_found(format!("there is no run `{}` in the results store", input.run)));
        }
        transaction.execute("DELETE FROM observations WHERE run = ?1", [&input.run])?;
        transaction.execute("DELETE FROM statistics WHERE run = ?1", [&input.run])?;
        transaction.commit()?;
        Ok(ForgetRunOutput { forgotten: input.run })
    }
}
