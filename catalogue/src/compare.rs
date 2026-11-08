//! Comparing two runs: each figure's difference with a 95% confidence interval, paired by
//! observation when both runs used the same seed and number of observations, and with Welch's
//! interval otherwise. See `docs/design/operation-catalogue.md` and ADR 0012.
//!
//! Pairing observation k with observation k is valid whatever the models, since different k are
//! independent. How much narrower it makes the interval depends on common random numbers: each
//! object's random numbers come from the seed and its stream (ADR 0012), so an object with the
//! same stream in both models drew the same numbers.

use crate::error::OperationError;
use crate::jobs::run_number;
use crate::results::{choose_objects, shown, t_quantile, STATISTICS};
use crate::{entry, Catalogue, Effect, Entry, Operation};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<CompareRuns>()]
}

/// What the store holds about one run, for comparing.
struct Run {
    id: String,
    model_path: String,
    model_text: String,
    seed: String,
    observations: u32,
    label: Option<String>,
    length_s: f64,
    warm_up_s: f64,
    /// Each observation that ended, by number: its error, if it failed.
    ended: BTreeMap<u32, Option<String>>,
    /// Each instance's stream, by name.
    streams: HashMap<String, u64>,
    /// The instance names in model order.
    names: Vec<String>,
}

impl Run {
    fn read(connection: &rusqlite::Connection, id: &str) -> Result<Run, OperationError> {
        let number = run_number(id)?;
        let (model_path, model_text, seed, observations, label, length_s, warm_up_s) = connection
            .query_row("SELECT model_path, model_text, seed, observations, label, length_s, warm_up_s FROM runs WHERE number = ?1", [number], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                ))
            })
            .optional()?
            .ok_or_else(|| OperationError::not_found(format!("there is no run `{id}` in the results store")))?;
        let mut ended = BTreeMap::new();
        let mut rows = connection.prepare("SELECT number, error FROM observations WHERE run = ?1")?;
        for row in rows.query_map([id], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Option<String>>(1)?)))? {
            let (number, error) = row?;
            ended.insert(number, error);
        }
        let model = crabsim_model::load(&model_text).map_err(|_| OperationError::run_error(format!("the model `{id}` ran can't be read back")))?;
        let names: Vec<String> = model.instances.iter().map(|instance| instance.name.clone()).collect();
        // A model saved before streams numbers its instances by place, as its run drew them.
        let streams = model.instances.iter().map(|instance| (instance.name.clone(), instance.stream.expect("loading gives every stream"))).collect();
        Ok(Run { id: id.to_string(), model_path, model_text, seed, observations, label, length_s, warm_up_s, ended, streams, names })
    }

    /// Why observation `k` gave no figures, if it didn't.
    fn missing(&self, k: u32) -> Option<String> {
        match self.ended.get(&k) {
            Some(None) => None,
            Some(Some(error)) => Some(format!("observation {k} failed in {}: {error}", self.id)),
            None => Some(format!("observation {k} didn't end in {}, which was cancelled or interrupted", self.id)),
        }
    }
}

/// Each observation's value of every figure of `object` in `run`, by statistic.
fn figures(connection: &rusqlite::Connection, run: &Run, object: &str) -> Result<HashMap<String, BTreeMap<u32, f64>>, OperationError> {
    let mut values: HashMap<String, BTreeMap<u32, f64>> = HashMap::new();
    let mut rows = connection.prepare("SELECT observation, statistic, value FROM statistics WHERE run = ?1 AND object = ?2")?;
    for row in rows.query_map(rusqlite::params![run.id, object], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?)))? {
        let (observation, statistic, value) = row?;
        values.entry(statistic).or_default().insert(observation, value);
    }
    Ok(values)
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn variance(values: &[f64]) -> f64 {
    let m = mean(values);
    values.iter().map(|value| (value - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Each observation of b minus the same observation of a, which drew the same random numbers.
    Paired,
    /// The difference of the means, with Welch's interval.
    Welch,
}

/// One figure of one object in both runs.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Difference {
    /// The mean in run a.
    pub a: f64,
    /// The mean in run b.
    pub b: f64,
    /// b minus a.
    pub difference: f64,
    /// The 95% confidence interval of the difference, with too few values none.
    pub ci95: Option<[f64; 2]>,
    /// The difference as a share of a's mean, none when that is 0.
    pub relative: Option<f64>,
    /// The interval doesn't include 0. Compared at 95% across many figures, a few are clear by
    /// chance alone.
    pub clear: bool,
    pub method: Method,
    /// How many observations of a and of b went in: for a paired figure, the pairs, twice.
    pub counted: [usize; 2],
}

impl Difference {
    fn paired(a: &BTreeMap<u32, f64>, b: &BTreeMap<u32, f64>) -> Option<Difference> {
        let pairs: Vec<(f64, f64)> = a.iter().filter_map(|(k, va)| b.get(k).map(|vb| (*va, *vb))).collect();
        if pairs.is_empty() {
            return None;
        }
        let differences: Vec<f64> = pairs.iter().map(|(va, vb)| vb - va).collect();
        let n = differences.len();
        let difference = mean(&differences);
        let ci95 = (n >= 2).then(|| {
            let half = t_quantile(n - 1) * (variance(&differences) / n as f64).sqrt();
            [difference - half, difference + half]
        });
        let a_mean = mean(&pairs.iter().map(|(va, _)| *va).collect::<Vec<_>>());
        let b_mean = mean(&pairs.iter().map(|(_, vb)| *vb).collect::<Vec<_>>());
        Some(Difference::new(a_mean, b_mean, difference, ci95, Method::Paired, [n, n]))
    }

    fn welch(a: &BTreeMap<u32, f64>, b: &BTreeMap<u32, f64>) -> Option<Difference> {
        let (va, vb): (Vec<f64>, Vec<f64>) = (a.values().copied().collect(), b.values().copied().collect());
        if va.is_empty() || vb.is_empty() {
            return None;
        }
        let difference = mean(&vb) - mean(&va);
        let ci95 = (va.len() >= 2 && vb.len() >= 2).then(|| {
            let (sa, sb) = (variance(&va) / va.len() as f64, variance(&vb) / vb.len() as f64);
            let spread = sa + sb;
            if spread == 0.0 {
                return [difference, difference];
            }
            // Welch–Satterthwaite, rounded down to whole degrees of freedom, which widens the
            // interval a little rather than narrowing it.
            let df = (spread.powi(2) / (sa.powi(2) / (va.len() - 1) as f64 + sb.powi(2) / (vb.len() - 1) as f64)).floor().max(1.0) as usize;
            let half = t_quantile(df) * spread.sqrt();
            [difference - half, difference + half]
        });
        Some(Difference::new(mean(&va), mean(&vb), difference, ci95, Method::Welch, [va.len(), vb.len()]))
    }

    fn new(a: f64, b: f64, difference: f64, ci95: Option<[f64; 2]>, method: Method, counted: [usize; 2]) -> Difference {
        let clear = ci95.is_some_and(|[low, high]| low > 0.0 || high < 0.0);
        Difference {
            a: shown(a),
            b: shown(b),
            difference: shown(difference),
            ci95: ci95.map(|[low, high]| [shown(low), shown(high)]),
            relative: (a != 0.0).then(|| shown(difference / a)),
            clear,
            method,
            counted,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ComparedObject {
    pub name: String,
    /// It drew the same random numbers in both runs, observation by observation: the runs are
    /// paired and it has the same stream in both models. Its intervals are then narrowest.
    pub common_random_numbers: bool,
    /// Why not, when the runs are paired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not_common: Option<String>,
    /// By statistic, named as in the store. Figures that are 0 in both runs are left out.
    pub figures: BTreeMap<String, Difference>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RunSide {
    pub run: String,
    pub model: String,
    pub seed: String,
    pub observations: u32,
    pub label: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Dropped {
    pub observation: u32,
    /// Such as "observation 3 failed in run-7: …".
    pub reason: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Pairing {
    Paired,
    Unpaired,
}

#[derive(Serialize, JsonSchema)]
pub struct CompareRunsOutput {
    pub a: RunSide,
    pub b: RunSide,
    /// Whether the runs used the same seed and number of observations, so that objects in the
    /// same place in both models are paired.
    pub pairing: Pairing,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_unpaired: Option<String>,
    /// What makes the comparison less meaningful, such as different lengths.
    pub warnings: Vec<String>,
    /// Observations that gave no figures, and why: for paired runs each dropped pair, otherwise
    /// each observation left out of either run.
    pub dropped: Vec<Dropped>,
    /// The objects with a figure that differs, leaving out figures identical in every pair.
    pub objects: Vec<ComparedObject>,
    /// Objects compared whose every figure was identical in every pair.
    pub unchanged: Vec<String>,
    /// Instances of a's model that b's lacks, by name.
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
}

pub struct CompareRuns;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareRunsInput {
    /// A run's id, such as `run-3`: the base.
    pub a: String,
    /// Another run's id, compared with a: differences are b minus a.
    pub b: String,
    /// Instance names to compare instead of the default.
    pub objects: Option<Vec<String>>,
    /// Classes whose instances to compare instead of the default, built-in classes such as Drain
    /// too.
    pub classes: Option<Vec<String>>,
}

impl Operation for CompareRuns {
    const NAME: &'static str = "compare_runs";
    const DESCRIPTION: &'static str = "Compares two runs, such as a model and a variant of it: per object and figure, a's mean, b's mean, the difference b minus a with its 95% confidence interval, the relative difference, and `clear` when the interval leaves out 0, to 4 significant digits. Runs with the same seed and number of observations are paired: every figure is compared observation by observation (method paired). An object with the same stream in both models also drew the same random numbers (`common_random_numbers`), which narrows its intervals most: every instance keeps its stream through edits, so variants made with edit_model share them. Other runs are compared with Welch's interval (method welch). `warnings` says what makes a comparison less meaningful, such as different lengths or warm-ups, `dropped` every observation that gave no figures and why, and `counted` how many went into each figure. Figures identical in every pair are left out, and objects with none left are listed in `unchanged`. Objects are matched by name, and those in only one model are listed apart. By default the same objects as run_results shows, from both runs: a small model whole, otherwise every Drain and the busiest, most blocked and clearly most failed. Give `objects` or `classes` for exactly those. Compared at 95%, a few of many figures are clear by chance alone.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = CompareRunsInput;
    type Output = CompareRunsOutput;

    fn run(catalogue: &Catalogue, input: CompareRunsInput) -> Result<CompareRunsOutput, OperationError> {
        let store = catalogue.store()?;
        let connection = store.connection();
        let (a, b) = (Run::read(connection, &input.a)?, Run::read(connection, &input.b)?);

        let why_unpaired = if a.seed != b.seed {
            Some(format!("the runs used different seeds ({} and {})", a.seed, b.seed))
        } else if a.observations != b.observations {
            Some(format!("the runs have different numbers of observations ({} and {})", a.observations, b.observations))
        } else {
            None
        };
        let paired_runs = why_unpaired.is_none();
        let mut warnings = Vec::new();
        let time = |secs: f64| crabsim::time::format_time_exact(crabsim::SimTime::from_secs_f64(secs));
        if a.length_s != b.length_s {
            warnings.push(format!(
                "the runs measured different lengths ({} and {}): counts such as mus_in aren't comparable, while rates and shares are. If their intervals agree, the shorter length was enough to capture the system's behaviour",
                time(a.length_s),
                time(b.length_s)
            ));
        }
        if a.warm_up_s != b.warm_up_s {
            warnings.push(format!("the runs left out different warm-ups ({} and {}), so they measured from different starting states", time(a.warm_up_s), time(b.warm_up_s)));
        }

        // Every observation that gives no figures, and why.
        let mut dropped = Vec::new();
        if paired_runs {
            for k in 1..=a.observations {
                let reasons: Vec<String> = [a.missing(k), b.missing(k)].into_iter().flatten().collect();
                if !reasons.is_empty() {
                    dropped.push(Dropped { observation: k, reason: reasons.join("; ") });
                }
            }
        } else {
            for run in [&a, &b] {
                for k in 1..=run.observations {
                    if let Some(reason) = run.missing(k) {
                        dropped.push(Dropped { observation: k, reason });
                    }
                }
            }
        }

        // Which objects: chosen ones from either model, or both runs' defaults put together.
        let mut names: Vec<String> = Vec::new();
        let mut add = |name: String| {
            if !names.contains(&name) {
                names.push(name);
            }
        };
        if input.objects.is_none() && input.classes.is_none() {
            for run in [&a, &b] {
                let mut chosen: Vec<String> = choose_objects(connection, &run.id, &run.model_text, None, None)?.into_iter().map(|(name, _)| name).collect();
                let order: HashMap<&String, usize> = run.names.iter().enumerate().map(|(place, name)| (name, place)).collect();
                chosen.sort_by_key(|name| order.get(name).copied().unwrap_or(usize::MAX));
                chosen.into_iter().for_each(&mut add);
            }
        } else {
            for name in input.objects.iter().flatten() {
                if !a.streams.contains_key(name) && !b.streams.contains_key(name) {
                    return Err(OperationError::not_found(format!("neither {} nor {} ran a model with an instance called `{name}`", a.id, b.id)));
                }
                add(name.clone());
            }
            for class in input.classes.iter().flatten() {
                let mut found = false;
                for run in [&a, &b] {
                    if let Ok(chosen) = choose_objects(connection, &run.id, &run.model_text, None, Some(std::slice::from_ref(class))) {
                        found = true;
                        chosen.into_iter().for_each(|(name, _)| add(name));
                    }
                }
                if !found {
                    return Err(OperationError::not_found(format!("neither {} nor {} ran a model with a class called `{class}`", a.id, b.id)));
                }
            }
        }

        let mut objects = Vec::new();
        let mut unchanged = Vec::new();
        for name in names.iter().filter(|name| a.streams.contains_key(*name) && b.streams.contains_key(*name)) {
            let (stream_a, stream_b) = (a.streams[name], b.streams[name]);
            let why_not_common = (paired_runs && stream_a != stream_b).then(|| {
                format!("it has stream {stream_a} in {}'s model and stream {stream_b} in {}'s, so it drew other random numbers", a.id, b.id)
            });
            let common = paired_runs && why_not_common.is_none();
            let (fa, fb) = (figures(connection, &a, name)?, figures(connection, &b, name)?);
            let empty = BTreeMap::new();
            let mut compared = BTreeMap::new();
            for statistic in STATISTICS {
                let (va, vb) = (fa.get(statistic).unwrap_or(&empty), fb.get(statistic).unwrap_or(&empty));
                if va.values().chain(vb.values()).all(|value| *value == 0.0) {
                    continue;
                }
                // Identical in every pair, such as a failure count with common random numbers.
                if paired_runs && va == vb {
                    continue;
                }
                let difference = if paired_runs { Difference::paired(va, vb) } else { Difference::welch(va, vb) };
                if let Some(difference) = difference {
                    compared.insert(statistic.to_string(), difference);
                }
            }
            if compared.is_empty() {
                unchanged.push(name.clone());
            } else {
                objects.push(ComparedObject { name: name.clone(), common_random_numbers: common, why_not_common, figures: compared });
            }
        }

        let side = |run: &Run| RunSide { run: run.id.clone(), model: run.model_path.clone(), seed: run.seed.clone(), observations: run.observations, label: run.label.clone() };
        Ok(CompareRunsOutput {
            only_in_a: a.names.iter().filter(|name| !b.streams.contains_key(*name)).cloned().collect(),
            only_in_b: b.names.iter().filter(|name| !a.streams.contains_key(*name)).cloned().collect(),
            a: side(&a),
            b: side(&b),
            pairing: if paired_runs { Pairing::Paired } else { Pairing::Unpaired },
            why_unpaired,
            warnings,
            dropped,
            objects,
            unchanged,
        })
    }
}
