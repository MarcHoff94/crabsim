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

/// How many of the most blocked and of the most failed objects a summary shows.
const SUMMARY_TOP: usize = 10;

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
fn t_quantile(df: usize) -> f64 {
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
fn shown(value: f64) -> f64 {
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

/// A number, or a time shown as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Amount {
    Number(f64),
    /// Simulated time, such as "00:03:12.500".
    Time(String),
}

/// One figure over a run's observations.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Figure {
    /// The mean over the observations that have this figure.
    pub mean: Amount,
    /// The 95% confidence interval of the mean (Student's t), with fewer than two values none.
    pub ci95: Option<[Amount; 2]>,
    pub min: Amount,
    pub max: Amount,
    /// Each observation's own value, in order from observation 1, null where it has none, such as
    /// a failed observation. Only when asked for with `values`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Option<Amount>>>,
}

impl Figure {
    /// The figure from each observation's value, with each one's own value if `each`, or `None`
    /// if no observation has one, or every one has 0.
    fn of(values: &[Option<f64>], each: bool, show: impl Fn(f64) -> Amount) -> Option<Figure> {
        let present: Vec<f64> = values.iter().flatten().copied().collect();
        if present.iter().all(|value| *value == 0.0) {
            return None;
        }
        let n = present.len() as f64;
        let mean = present.iter().sum::<f64>() / n;
        let ci95 = (present.len() >= 2).then(|| {
            let variance = present.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (n - 1.0);
            let half = t_quantile(present.len() - 1) * (variance / n).sqrt();
            [show(mean - half), show(mean + half)]
        });
        let min = present.iter().copied().fold(f64::INFINITY, f64::min);
        let max = present.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let values = each.then(|| values.iter().map(|value| value.map(&show)).collect());
        Some(Figure { mean: show(mean), ci95, min: show(min), max: show(max), values })
    }
}

/// Why an object is in a summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    Drain,
    MostBlocked,
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
    /// mean_lead_time (as simulated time). Shares and availability are from 0 to 1. A figure that
    /// is 0 in every observation is left out.
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
    const DESCRIPTION: &'static str = "Reads a run's results from the results store: its settings, its events and wall time, any observation that failed with its error, and per object every figure's mean over the observations, 95% confidence interval, minimum and maximum, to 4 significant digits. Figures that are 0 in every observation are left out. By default a short summary that fits even a 6,000-object model run 10,000 times: every Drain, the 10 most blocked and the 10 most failed objects. Give `objects` or `classes` to read exactly those instead, and `values` for each observation's own figures and seed. For full precision, or anything else such as comparing runs, use query_results.";
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
                "SELECT label, state, model_path, model_text, model_hash, seed, length_s, warm_up_s, observations, source, crabsim_version, started, finished FROM runs WHERE number = ?1",
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
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| OperationError::not_found(format!("there is no run `{id}` in the results store")))?;
        let (label, state, model_path, model_text, model_hash, seed, length, warm_up, observations, source, version, started, finished) = run;
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
        let mut chosen: Vec<(String, Why)> = Vec::new();
        if input.objects.is_none() && input.classes.is_none() {
            let drains = connection.prepare("SELECT DISTINCT object FROM statistics WHERE run = ?1 AND built_in_class = 'Drain'")?.query_map([&id], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
            chosen.extend(drains.into_iter().map(|name| (name, Why::Drain)));
            for (statistic, why) in [("blocked_share", Why::MostBlocked), ("failed_share", Why::MostFailed)] {
                let top = connection
                    .prepare(
                        "SELECT object FROM statistics WHERE run = ?1 AND statistic = ?2 GROUP BY object HAVING avg(value) > 0
                         ORDER BY avg(value) DESC, min(rowid) LIMIT ?3",
                    )?
                    .query_map(params![id, statistic, SUMMARY_TOP as i64], |row| row.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                chosen.extend(top.into_iter().map(|name| (name, why)));
            }
        } else {
            let model = crabsim_model::load(&model_text).map_err(|_| OperationError::run_error(format!("the model `{id}` ran can't be read back")))?;
            for name in input.objects.iter().flatten() {
                if !model.instances.iter().any(|instance| &instance.name == name) {
                    return Err(OperationError::not_found(format!("the model `{id}` ran has no instance called `{name}`")));
                }
                chosen.push((name.clone(), Why::Chosen));
            }
            for class in input.classes.iter().flatten() {
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
                let figure = if statistic == "mean_lead_time_s" {
                    Figure::of(values, input.values, |secs| Amount::Time(format_time_exact(SimTime::from_secs_f64(secs)))).map(|figure| ("mean_lead_time", figure))
                } else {
                    Figure::of(values, input.values, |value| Amount::Number(shown(value))).map(|figure| (statistic, figure))
                };
                if let Some((key, figure)) = figure {
                    figures.insert(key.to_string(), figure);
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

/// Each instance's place in the model, by name.
fn model_order(model_text: &str) -> HashMap<String, usize> {
    crabsim_model::load(model_text)
        .map(|model| model.instances.iter().enumerate().map(|(index, instance)| (instance.name.clone(), index)).collect())
        .unwrap_or_default()
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
    const DESCRIPTION: &'static str = "Deletes a run and all its results from the results store, for good. Refused while the run is still queued or running here: cancel it first.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = crate::jobs::RunInput;
    type Output = ForgetRunOutput;

    fn run(catalogue: &Catalogue, input: crate::jobs::RunInput) -> Result<ForgetRunOutput, OperationError> {
        let number = run_number(&input.run)?;
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
