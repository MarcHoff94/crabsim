//! The vocabulary of figures as an operation, and expectations' verdicts on a stored run
//! (ADR 0030).

use crate::error::OperationError;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::figures::{Figure, Subject, Unit};
use crabsim::time::format_time_exact;
use crabsim::{ObjectKind, SimTime};
use crabsim_model::expect::{self, Aggregate, CheckSubject, Observed, Verdict, Violation};
use crabsim_model::{CheckStage, Model};
use rusqlite::{params, Connection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<ListFigures>()]
}

// --- list_figures ---

pub struct ListFigures;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListFiguresInput {
    /// Only the figures of this built-in class: Source, Station, Buffer, Drain, Conveyor or
    /// CrossConveyor, or `model` for the model's own, or `MU` for an MU's. Left out, every figure.
    pub of: Option<String>,
}

/// One figure of the vocabulary.
#[derive(Serialize, JsonSchema)]
pub struct FigureInfo {
    /// Its name, the same in expectations, the results store and code, such as `working_share`.
    pub name: String,
    pub meaning: String,
    /// What its value is counted in, and how an expectation writes it: count and MUs plainly
    /// (`20`), share as a percentage (`60%`), per hour with its rate (`4000/h`), time with its
    /// unit (`90s`, `1:30`), state by its name (`busy`), name for an MU's class or destination.
    pub unit: String,
    /// The built-in classes it is a figure of, `model` when the model as a whole has it, or `MU`.
    pub of: Vec<String>,
    /// The time scopes it can be read over: `since start` (after the warm-up), `now` (when read,
    /// at a run's end), `last <time>` (the stretch before) and `always` (at every moment: a limit).
    pub scopes: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListFiguresOutput {
    pub figures: Vec<FigureInfo>,
}

fn class_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Source => "Source",
        ObjectKind::Station => "Station",
        ObjectKind::Buffer => "Buffer",
        ObjectKind::Drain => "Drain",
        ObjectKind::Conveyor => "Conveyor",
        ObjectKind::CrossConveyor => "CrossConveyor",
    }
}

impl Operation for ListFigures {
    const NAME: &'static str = "list_figures";
    const DESCRIPTION: &'static str = "Lists the vocabulary of figures (ADR 0030): everything crabsim measures about an object, the model or an MU, each with its meaning, unit, the built-in classes it is a figure of and the time scopes it can be read over. The same names are the results store's statistics and what an expectation names, such as `every Press.working_share since start >= 60%`. Give `of` for one built-in class's, `model` for the model's, or `MU` for an MU's, which rules and code read.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ListFiguresInput;
    type Output = ListFiguresOutput;

    fn run(_: &Catalogue, input: ListFiguresInput) -> Result<ListFiguresOutput, OperationError> {
        let of = |figure: Figure| -> Vec<String> {
            let mut of: Vec<String> = figure.kinds().iter().map(|&kind| class_name(kind).to_string()).collect();
            if figure.of_model() {
                of.push("model".into());
            }
            if figure.subject() == Subject::Mu {
                of.push("MU".into());
            }
            of
        };
        if let Some(wanted) = &input.of {
            let known = ["Source", "Station", "Buffer", "Drain", "Conveyor", "CrossConveyor", "model", "MU"];
            if !known.contains(&wanted.as_str()) {
                return Err(OperationError::invalid_input(format!("`of` is one of {}, not `{wanted}`", known.join(", "))));
            }
        }
        let figures = Figure::ALL
            .into_iter()
            .filter(|&figure| input.of.as_ref().is_none_or(|wanted| of(figure).contains(wanted)))
            .map(|figure| FigureInfo {
                name: figure.name().to_string(),
                meaning: figure.meaning().to_string(),
                unit: figure.unit().name().to_string(),
                of: of(figure),
                scopes: figure.scopes().iter().map(|scope| scope.written().to_string()).collect(),
            })
            .collect();
        Ok(ListFiguresOutput { figures })
    }
}

// --- Verdicts on a stored run ---

/// One subject's verdict, for an expectation on each of a class's instances.
#[derive(Serialize, JsonSchema)]
pub struct SubjectVerdict {
    pub name: String,
    pub verdict: Verdict,
    /// The mean over the observations, in the line's unit, such as `62.5%`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci95: Option<[String; 2]>,
    /// For a limit, the most (or fewest) it reached in any observation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extreme: Option<String>,
    /// Why it couldn't be judged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// A limit's first violation.
#[derive(Serialize, JsonSchema)]
pub struct ViolationResult {
    /// The observation it happened in, from 1.
    pub observation: u32,
    /// The simulated time since Init, as the UI shows time.
    pub time: String,
    pub object: String,
    /// The MU that broke it: entering the object, or for a lead time leaving into the Drain.
    pub mu: u32,
    /// The value it reached, in the line's unit.
    pub value: String,
}

/// An expectation's verdict on a run.
#[derive(Serialize, JsonSchema)]
pub struct ExpectationResults {
    pub id: String,
    /// The line, as the model the run used holds it.
    pub text: String,
    pub stage: CheckStage,
    /// Its subject: an instance's or class's name, or `model`.
    pub subject: String,
    pub aggregate: Aggregate,
    pub figure: String,
    /// `now`, `since start`, `last <time>` or `always`.
    pub scope: String,
    pub comparison: expect::Comparison,
    /// The value it compares with, as the line writes it.
    pub value: String,
    /// met, not_met, undecided when an experiment's 95% interval straddles the value, or error
    /// when it couldn't be judged. On one observation of a run with randomness, met so far.
    pub verdict: Verdict,
    /// For one instance, the model or a class's mean: the mean over the observations, in the
    /// line's unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ci95: Option<[String; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extreme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// With `every`: how many of the class's instances met it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub met: Option<String>,
    /// With `every`: each instance that didn't meet it, or every one if there are few.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<SubjectVerdict>,
    /// For a limit with `always`, where it was first broken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation: Option<ViolationResult>,
    /// What to do next, such as running more observations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advice: Option<String>,
}

/// With `every`, a class of at most this many instances shows each of them.
const FEW: usize = 10;

/// Every expectation of the model a run used, judged on the observations that ran to their end.
pub(crate) fn verdicts(connection: &Connection, run: &str, model_text: &str, deterministic: bool, length: SimTime) -> Result<Vec<ExpectationResults>, OperationError> {
    let Ok(model) = crabsim_model::load(model_text) else {
        return Ok(Vec::new());
    };
    if model.expectations.is_empty() {
        return Ok(Vec::new());
    }
    let names: HashMap<&str, u32> = model.instances.iter().enumerate().map(|(index, instance)| (instance.name.as_str(), index as u32)).collect();
    let ended: Vec<u32> = connection
        .prepare("SELECT number FROM observations WHERE run = ?1 AND error IS NULL ORDER BY number")?
        .query_map([run], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut values = connection.prepare("SELECT observation, object, value FROM expectation_values WHERE run = ?1 AND expectation = ?2 ORDER BY observation, rowid")?;
    let mut violations = connection.prepare("SELECT observation, time_s, object, mu, value FROM violations WHERE run = ?1 AND expectation = ?2")?;
    let mut results = Vec::new();
    for bound in model.bound_expectations() {
        let expectation = &model.expectations[bound.index];
        let mut by_observation: HashMap<u32, Observed> = HashMap::new();
        for row in values.query_map(params![run, expectation.id], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<f64>>(2)?)))? {
            let (observation, object, value) = row?;
            let subject = object.and_then(|object| names.get(object.as_str()).copied());
            by_observation.entry(observation).or_default().values.push((subject, value));
        }
        for row in violations.query_map(params![run, expectation.id], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, f64>(1)?, row.get::<_, String>(2)?, row.get::<_, u32>(3)?, row.get::<_, f64>(4)?)))? {
            let (observation, time, object, mu, value) = row?;
            if let Some(observed) = by_observation.get_mut(&observation) {
                observed.violation = Some(Violation { object: names.get(object.as_str()).copied().unwrap_or_default(), time: SimTime::from_secs_f64(time), mu, value });
            }
        }
        let observed: Vec<Observed> = ended.iter().filter_map(|k| by_observation.remove(k)).collect();
        let judgement = expect::judge(&bound, &observed, deterministic, length);
        results.push(expectation_results(&model, &bound, &judgement));
    }
    Ok(results)
}

/// An expectation's judgement as results give it, values in the line's own unit and subjects by
/// name.
pub fn expectation_results(model: &Model, bound: &expect::Bound, judgement: &expect::Judgement) -> ExpectationResults {
    let expectation = &model.expectations[bound.index];
    let check = &bound.check;
    let unit = check.figure.unit();
    let show = |value: f64| if unit == Unit::State { format!("{value}") } else { check.written_value(value) };
    let name_of = |subject: Option<u32>| subject.map_or_else(String::new, |id| model.instances[id as usize].name.clone());
    let subject_verdict = |judged: &expect::SubjectJudgement| SubjectVerdict {
        name: name_of(judged.subject),
        verdict: judged.verdict,
        // A limit's extreme says it all.
        mean: judged.mean.filter(|_| unit != Unit::State && judged.extreme.is_none()).map(show),
        ci95: judged.ci95.map(|[low, high]| [show(low), show(high)]),
        extreme: judged.extreme.map(show),
        why: judged.why.clone(),
    };
    let mut result = ExpectationResults {
        id: expectation.id.clone(),
        text: expectation.text.clone(),
        stage: expectation.stage,
        subject: match &check.subject {
            CheckSubject::Named(name) => name.clone(),
            CheckSubject::Model => "model".into(),
        },
        aggregate: check.aggregate,
        figure: check.figure.name().into(),
        scope: check.scope_text(),
        comparison: check.comparison,
        value: check.value_text().to_string(),
        verdict: judgement.verdict,
        mean: None,
        ci95: None,
        extreme: None,
        why: None,
        met: None,
        subjects: Vec::new(),
        violation: judgement.violation.map(|(observation, violation)| ViolationResult {
            observation,
            time: format_time_exact(violation.time),
            object: name_of(Some(violation.object)),
            mu: violation.mu,
            value: show(violation.value),
        }),
        advice: judgement.advice.clone(),
    };
    let judged = judgement.subjects.as_slice();
    match (check.aggregate, judged) {
        // A class without instances, or a run with no observation to judge.
        (_, []) => result.why = Some("no observation ran to its end".into()),
        (Aggregate::Every, judged) if judged.iter().all(|subject| subject.subject.is_none()) => result.why = judged[0].why.clone(),
        (Aggregate::Every, judged) => {
            let met = judged.iter().filter(|subject| subject.verdict == Verdict::Met).count();
            result.met = Some(format!("{met} of {}", judged.len()));
            result.subjects = judged.iter().filter(|subject| judged.len() <= FEW || subject.verdict != Verdict::Met).map(subject_verdict).collect();
        }
        (_, [one, ..]) => {
            let one = subject_verdict(one);
            (result.mean, result.ci95, result.extreme, result.why) = (one.mean, one.ci95, one.extreme, one.why);
        }
    }
    result
}
