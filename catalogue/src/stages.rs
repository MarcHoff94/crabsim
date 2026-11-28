//! Stages (ADR 0030): how far a model is checked, draft, verified or validated, which crabsim
//! works out from the checks it keeps in the results store, never from the model file, so a check
//! never changes the model. A check of a stage is one run for each set of settings its
//! expectations are checked with: deterministic for verification, with randomness for validation.
//! Verification runs on request, when an agent releases its claim on a model, when the window
//! opens one and before an experiment, each time only if the model wasn't checked as it is now.
//! Validation runs on request. `get_status` gives an agent or the user everything in one place:
//! the stage, the requests, each expectation's verdict, what's questioned and the next step.

use crate::error::OperationError;
use crate::expectations::ExpectationResults;
use crate::jobs::{launch_run, prepare, CheckPart, Launch, Prepared};
use crate::model_files::read_file;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::SimTime;
use crabsim_model::{Agreement, CheckStage, Model, RequestState, Settings};
use crabsim_model::expect::Verdict;
use rusqlite::params;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<CheckStageOperation>(), entry::<GetStatus>()]
}

/// How agents and the user work towards a model that does what was asked, as the MCP server's
/// instructions and the CLI's help describe it.
pub const PROCESS: &str = "Work in five steps (ADR 0030). 1. Agree: turn what the user asks for into a request with the user's own words, your restatement in precise terms and expectations crabsim can judge (edit_model add_request and add_expectation, list_figures for the figures), and ask the user to accept them, which only the user does; skip this when the model already has accepted expectations for the work. get_status's `cannot_be_met` says which crabsim works out can't be met, without running anything: change those before the user accepts them. 2. Build the model without randomness first, no failures, and mark the request built (set_request built). 3. Verify: check_stage verification runs it deterministically and judges the verification expectations; crabsim also verifies when you release your claim and before an experiment. 4. Add randomness, such as failures. 5. Validate against reference figures from the real line, validation expectations with a tolerance and their `source`, such as `Drain.throughput_per_hour since start = 4100/d ± 3%`: check_stage validation runs an experiment and judges each by its 95% interval, met once it lies within the tolerance. get_status says at any time where the model stands and what to do next. You can't change what the user accepted: question an expectation with its reason when the model or the data says it can't be met, and tell your user.";

/// How far a model is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Not verified: unchecked, or a verification expectation not met.
    Draft,
    /// Its latest verification met every accepted verification expectation.
    Verified,
    /// Verified, and its latest validation met every accepted validation expectation.
    Validated,
}

/// A stage's latest finished check, from the store.
struct Finished {
    runs: Vec<String>,
    model_hash: String,
    /// The model the check ran, as the store keeps it.
    model: Option<Model>,
    change: Option<i64>,
    /// Each expectation's verdict, by id.
    verdicts: HashMap<String, ExpectationResults>,
}

/// A stage's checks of a model file: the latest finished one, and the runs still going.
struct Checks {
    finished: Option<Finished>,
    running: Vec<String>,
}

/// The checks of `stage` of the model file whose key is `key`.
fn checks(catalogue: &Catalogue, key: &str, stage: CheckStage) -> Result<Checks, OperationError> {
    let store = catalogue.swept_store()?;
    let connection = store.connection();
    let mut statement = connection.prepare(
        "SELECT id, check_number, state, model_text, model_hash, deterministic, length_s, checked_change FROM runs WHERE model_path = ?1 AND stage = ?2 AND check_number IS NOT NULL ORDER BY check_number DESC, number",
    )?;
    type Row = (String, i64, String, String, String, bool, f64, Option<i64>);
    let rows: Vec<Row> = statement
        .query_map(params![key, stage.name()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?)))?
        .collect::<Result<_, _>>()?;
    let mut running = Vec::new();
    let mut at = 0;
    while at < rows.len() {
        let number = rows[at].1;
        let check: Vec<&Row> = rows[at..].iter().take_while(|row| row.1 == number).collect();
        at += check.len();
        let states: Vec<&str> = check.iter().map(|row| row.2.as_str()).collect();
        if states.iter().any(|state| matches!(*state, "queued" | "running")) {
            running.extend(check.iter().map(|row| row.0.clone()));
            continue;
        }
        // Cancelled or interrupted, it checked nothing.
        if states.iter().any(|state| !matches!(*state, "done" | "failed")) {
            continue;
        }
        let mut verdicts = HashMap::new();
        for row in &check {
            for result in crate::expectations::verdicts(connection, &row.0, &row.3, row.5, SimTime::from_secs_f64(row.6))? {
                verdicts.insert(result.id.clone(), result);
            }
            // A run that failed judged nothing, which is an error for each of its expectations.
        }
        let first = check[0];
        let finished = Finished {
            runs: check.iter().map(|row| row.0.clone()).collect(),
            model_hash: first.4.clone(),
            model: crabsim_model::load(&first.3).ok(),
            change: first.7,
            verdicts,
        };
        return Ok(Checks { finished: Some(finished), running });
    }
    Ok(Checks { finished: None, running })
}

impl Finished {
    /// Whether every expectation of `stage` the user agreed to in the model it checked is met,
    /// and there is one at least.
    fn passed(&self, stage: CheckStage) -> bool {
        let Some(model) = &self.model else { return false };
        let agreed: Vec<&str> = model.expectations.iter().filter(|expectation| expectation.stage == stage && expectation.state.in_force()).map(|expectation| expectation.id.as_str()).collect();
        !agreed.is_empty() && agreed.iter().all(|id| self.verdicts.get(*id).is_some_and(|result| result.verdict == Verdict::Met))
    }
}

/// Starts a check of `stage` of the prepared model at `path`, one run for each of its settings,
/// giving the runs. Refused if the stage has no expectations.
pub(crate) fn start_check(catalogue: &Catalogue, path: &str, prepared: &Prepared, stage: CheckStage) -> Result<Vec<String>, OperationError> {
    let key = crate::editing::key_of(catalogue, path)?;
    let groups = prepared.model.checks_of(stage);
    if groups.is_empty() {
        return Err(OperationError::invalid_input(format!(
            "`{path}` has no {} expectations to check: add them with edit_model add_expectation{}",
            stage.name(),
            if stage == CheckStage::Validation { ", with `stage` validation" } else { "" }
        )));
    }
    let change = crate::history::newest(catalogue, &key);
    let mut runs = Vec::new();
    let mut number = None;
    for (settings, judged) in groups {
        let launch = Launch {
            seed: 0,
            length: settings.length,
            warm_up: settings.warm_up,
            observations: settings.observations,
            deterministic: stage == CheckStage::Verification,
            label: Some(stage.name().to_string()),
            check: Some(CheckPart { stage, number, change, judged }),
        };
        let id = launch_run(catalogue, &key, prepared, launch)?;
        number = number.or_else(|| crate::jobs::run_number(&id).ok());
        runs.push(id);
    }
    Ok(runs)
}

/// Starts verifying the prepared model at `path` if it has verification expectations and isn't
/// verified, or being verified, as it is now, giving the runs started.
pub(crate) fn verify_if_unchecked(catalogue: &Catalogue, path: &str, prepared: &Prepared) -> Result<Vec<String>, OperationError> {
    if prepared.model.checks_of(CheckStage::Verification).is_empty() {
        return Ok(Vec::new());
    }
    let key = crate::editing::key_of(catalogue, path)?;
    let checks = checks(catalogue, &key, CheckStage::Verification)?;
    if !checks.running.is_empty() || checks.finished.as_ref().is_some_and(|finished| finished.model_hash == prepared.hash) {
        return Ok(Vec::new());
    }
    start_check(catalogue, path, prepared, CheckStage::Verification)
}

/// As `verify_if_unchecked`, for a model file that may not be valid, which it leaves alone. A
/// milestone such as releasing a claim never fails for it.
pub(crate) fn verify_at_milestone(catalogue: &Catalogue, path: &str) -> Vec<String> {
    let Ok((_, text)) = read_file(catalogue, path) else { return Vec::new() };
    let Ok(prepared) = prepare(catalogue, path, &text) else { return Vec::new() };
    verify_if_unchecked(catalogue, path, &prepared).unwrap_or_else(|error| {
        log::warn!("`{path}` wasn't verified: {error}");
        Vec::new()
    })
}

// ---------- The status ----------

/// A stage's check as the status gives it.
#[derive(Serialize, JsonSchema)]
pub struct StageCheck {
    /// Its latest check met every expectation of the stage the user accepted.
    pub passed: bool,
    /// The runs of its latest finished check, one for each of its settings.
    pub runs: Vec<String>,
    /// The runs of a check still going.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<String>,
    /// The latest check was of the model file as it is now.
    pub current: bool,
    /// The number of the change in the file's history the latest check saw, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_change: Option<i64>,
    /// Not checked since this change, by its number in the history, the oldest made since.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_checked_since: Option<i64>,
    /// How many changes were made since the latest check.
    pub changes_since: u32,
    /// How the stage's checks run, where its expectations don't say otherwise.
    pub settings: Settings,
    /// In a few words, such as "met by run-4, not checked since change 12".
    pub summary: String,
}

/// An expectation as the status gives it.
#[derive(Serialize, JsonSchema)]
pub struct ExpectationStatus {
    pub id: String,
    pub text: String,
    pub stage: CheckStage,
    /// proposed by an agent, accepted by the user, or questioned by an agent, still in force.
    pub state: Agreement,
    /// Why an agent questions it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Its verdict in its stage's latest check, if that judged it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// The verdict is of the model as it is now, the line unchanged.
    pub current: bool,
    /// What the check found, in the line's unit: the mean, for a limit the extreme, or with
    /// `every` how many instances met it, such as "2 of 12 met".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<String>,
    /// Why it couldn't be judged, or what to do next, such as run more observations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Why crabsim works out from the model, without running it, that it can't be met, or that
    /// it contradicts another expectation, such as a throughput above what a Station can pass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cannot_be_met: Option<String>,
    /// Where it comes from; for a reference figure, where it was measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The requests it checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<String>,
}

/// A request as the status gives it.
#[derive(Serialize, JsonSchema)]
pub struct RequestStatus {
    pub id: String,
    /// The user's own words.
    pub words: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restatement: Option<String>,
    pub expectations: Vec<String>,
    /// The user accepted it, with its expectations.
    pub accepted: bool,
    /// open until built; once built, met when every expectation of it is met in its check of the
    /// model as it is now, broken when one isn't, built until then.
    pub state: RequestState,
}

#[derive(Serialize, JsonSchema)]
pub struct GetStatusOutput {
    pub path: String,
    /// draft, verified or validated, from the latest checks, which may be of an earlier version:
    /// see `current` in each.
    pub stage: Stage,
    /// The model has no problems, so it can be checked.
    pub valid: bool,
    /// How many problems it has: validate_model lists them.
    pub problems: usize,
    pub verification: StageCheck,
    pub validation: StageCheck,
    pub requests: Vec<RequestStatus>,
    pub expectations: Vec<ExpectationStatus>,
    /// What to do next, in a sentence.
    pub next: String,
}

/// The status of the model file at `path`.
pub(crate) fn status(catalogue: &Catalogue, path: &str) -> Result<GetStatusOutput, OperationError> {
    let (_, text) = read_file(catalogue, path)?;
    let key = crate::editing::key_of(catalogue, path)?;
    let (model, problems) = crabsim_model::load_what_can_be_read(&text);
    let hash = crate::edit::model_hash(&text);
    let mut stages = Vec::new();
    for stage in [CheckStage::Verification, CheckStage::Validation] {
        let checks = checks(catalogue, &key, stage)?;
        let current = checks.finished.as_ref().is_some_and(|finished| Some(&finished.model_hash) == hash.as_ref());
        let checked_change = checks.finished.as_ref().and_then(|finished| finished.change);
        let (not_checked_since, changes_since) = match &checks.finished {
            Some(_) if !current => crate::history::since(catalogue, &key, checked_change),
            _ => (None, 0),
        };
        let passed = checks.finished.as_ref().is_some_and(|finished| finished.passed(stage));
        let summary = match &checks.finished {
            None if checks.running.is_empty() => "never checked".to_string(),
            None => format!("being checked by {}", checks.running.join(", ")),
            Some(finished) => {
                let outcome = if passed { "met" } else { "not met" };
                let mut summary = format!("{outcome} by {}", finished.runs.join(", "));
                match (current, not_checked_since) {
                    (true, _) => {}
                    (false, Some(change)) => summary += &format!(", not checked since change {change}"),
                    (false, None) => summary += ", not checked since the file changed",
                }
                if !checks.running.is_empty() {
                    summary += &format!("; being checked again by {}", checks.running.join(", "));
                }
                summary
            }
        };
        let check = StageCheck {
            passed,
            runs: checks.finished.as_ref().map(|finished| finished.runs.clone()).unwrap_or_default(),
            running: checks.running.clone(),
            current,
            checked_change,
            not_checked_since,
            changes_since,
            settings: model.stages.settings(stage),
            summary,
        };
        stages.push((check, checks.finished));
    }
    let (validation, validated) = stages.pop().expect("two stages");
    let (verification, verified) = stages.pop().expect("two stages");
    let stage = match (verification.passed, validation.passed) {
        (true, true) => Stage::Validated,
        (true, false) => Stage::Verified,
        _ => Stage::Draft,
    };
    // What crabsim works out without a run, for a model that can run.
    let bounds: HashMap<String, String> = match &hash {
        Some(hash) if problems.is_empty() => bounds_of(hash, &model),
        _ => HashMap::new(),
    };
    let finished_of = |stage: CheckStage| if stage == CheckStage::Verification { (verified.as_ref(), verification.current) } else { (validated.as_ref(), validation.current) };
    let expectations: Vec<ExpectationStatus> = model
        .expectations
        .iter()
        .map(|expectation| {
            let (finished, check_current) = finished_of(expectation.stage);
            let result = finished.and_then(|finished| finished.verdicts.get(&expectation.id));
            let same_line = result.is_some_and(|result| result.text == expectation.text);
            ExpectationStatus {
                id: expectation.id.clone(),
                text: expectation.text.clone(),
                stage: expectation.stage,
                state: expectation.state,
                question: expectation.question.clone(),
                verdict: result.map(|result| result.verdict),
                current: check_current && same_line,
                found: result.and_then(|result| result.extreme.clone().or_else(|| result.mean.clone()).or_else(|| result.met.as_ref().map(|met| format!("{met} met")))),
                note: result.and_then(|result| result.why.clone().or_else(|| result.advice.clone())),
                cannot_be_met: bounds.get(&expectation.id).cloned(),
                source: expectation.source.clone(),
                requests: model.requests.iter().filter(|request| request.expectations.contains(&expectation.id)).map(|request| request.id.clone()).collect(),
            }
        })
        .collect();
    let current_verdict = |id: &str| expectations.iter().find(|expectation| expectation.id == id && expectation.current).and_then(|expectation| expectation.verdict);
    let requests: Vec<RequestStatus> = model
        .requests
        .iter()
        .map(|request| RequestStatus {
            id: request.id.clone(),
            words: request.words.clone(),
            restatement: request.restatement.clone(),
            expectations: request.expectations.clone(),
            accepted: request.accepted,
            state: request.state(current_verdict),
        })
        .collect();
    let next = next_step(catalogue, path, &model, problems.len(), &verification, &validation, &requests, &expectations);
    Ok(GetStatusOutput { path: path.to_string(), stage, valid: problems.is_empty(), problems: problems.len(), verification, validation, requests, expectations, next })
}

/// Each expectation's bound by id, as `Model::expectation_bounds` warns of it, kept for the last
/// few models by their hash: the window reads the status every few seconds, and a bound changes
/// only with the model, while working it out on a large one takes a good part of a second.
fn bounds_of(hash: &str, model: &Model) -> HashMap<String, String> {
    const KEPT: usize = 8;
    static KNOWN: std::sync::Mutex<Vec<(String, HashMap<String, String>)>> = std::sync::Mutex::new(Vec::new());
    if let Some((_, bounds)) = KNOWN.lock().expect("never poisoned").iter().find(|(known, _)| known == hash) {
        return bounds.clone();
    }
    let mut bounds: HashMap<String, String> = HashMap::new();
    for warning in model.expectation_bounds() {
        if let Some(id) = warning.name {
            bounds.entry(id).or_insert(warning.message);
        }
    }
    let mut known = KNOWN.lock().expect("never poisoned");
    if known.len() >= KEPT {
        known.remove(0);
    }
    known.push((hash.to_string(), bounds.clone()));
    bounds
}

/// What to do next, the first thing that holds the model back.
#[allow(clippy::too_many_arguments)]
fn next_step(
    catalogue: &Catalogue,
    path: &str,
    model: &Model,
    problems: usize,
    verification: &StageCheck,
    validation: &StageCheck,
    requests: &[RequestStatus],
    expectations: &[ExpectationStatus],
) -> String {
    let agent = !catalogue.source().is_user();
    let ids = |found: Vec<&str>| found.join(", ");
    if problems > 0 {
        return format!("fix the model's {problems} problem{}: validate_model lists them", if problems == 1 { "" } else { "s" });
    }
    if expectations.is_empty() {
        return "agree what the model must achieve: a request with the user's own words, a restatement and expectations crabsim can judge (edit_model add_request, add_expectation; list_figures gives the figures)".to_string();
    }
    // What can't be met is better changed before anyone accepts it, and questioned once accepted.
    let cannot = |proposed: bool| -> Vec<&str> {
        expectations
            .iter()
            .filter(|expectation| expectation.cannot_be_met.is_some() && (expectation.state == Agreement::Proposed) == proposed && expectation.state != Agreement::Questioned)
            .map(|expectation| expectation.id.as_str())
            .collect()
    };
    let (proposed_cannot, accepted_cannot) = (cannot(true), cannot(false));
    let them = |found: &[&str]| if found.len() == 1 { "it" } else { "them" };
    if !proposed_cannot.is_empty() {
        let (these, it) = (ids(proposed_cannot.clone()), them(&proposed_cannot));
        return if agent {
            format!("crabsim works out that {these} can't be met as the model is, as `cannot_be_met` says: change the model or {it} before asking the user to accept {it}")
        } else {
            format!("crabsim works out that {these}, which an agent proposed, can't be met as the model is, as `cannot_be_met` says: change the model, or change or remove {it}")
        };
    }
    if !accepted_cannot.is_empty() {
        let (these, it) = (ids(accepted_cannot.clone()), them(&accepted_cannot));
        return if agent {
            format!("crabsim works out that {these} can't be met as the model is, as `cannot_be_met` says: change the model, or question {it} with edit_model question_expectation, giving why, and tell your user")
        } else {
            format!("crabsim works out that {these} can't be met as the model is, as `cannot_be_met` says: change the model, or change or remove {it}")
        };
    }
    let unaccepted_requests: Vec<&str> = requests.iter().filter(|request| !request.accepted).map(|request| request.id.as_str()).collect();
    let proposed: Vec<&str> = expectations.iter().filter(|expectation| expectation.state == Agreement::Proposed).map(|expectation| expectation.id.as_str()).collect();
    if !unaccepted_requests.is_empty() || !proposed.is_empty() {
        let what = if unaccepted_requests.is_empty() { format!("the expectations {}", ids(proposed)) } else { format!("the requests {}", ids(unaccepted_requests)) };
        return if agent {
            format!("ask the user to accept {what}: only the user accepts, in the window's Status tab or with `crabsim accept`. Building may go on meanwhile")
        } else {
            format!("accept {what} if they say what you want, or change them")
        };
    }
    let questioned: Vec<&str> = expectations.iter().filter(|expectation| expectation.state == Agreement::Questioned).map(|expectation| expectation.id.as_str()).collect();
    if !questioned.is_empty() {
        return if agent {
            format!("the user answers the question on {}: tell your user why you question it", ids(questioned))
        } else {
            format!("answer the question on {}: accept it again, change it or remove it", ids(questioned))
        };
    }
    let open: Vec<&str> = requests.iter().filter(|request| request.state == RequestState::Open).map(|request| request.id.as_str()).collect();
    if !open.is_empty() && agent {
        return format!("build {}, without randomness first, then mark it built (edit_model set_request with built true)", ids(open));
    }
    let has = |stage: CheckStage| model.expectations.iter().any(|expectation| expectation.stage == stage);
    let check = |stage: CheckStage, check: &StageCheck| -> Option<String> {
        if !check.running.is_empty() {
            return Some(format!("wait for the {} runs {} to end (run_status), then read get_status again", stage.name(), check.running.join(", ")));
        }
        if !check.current {
            return Some(format!("check the {} stage: check_stage with `stage` {}", stage.name(), stage.name()));
        }
        if !check.passed {
            let failing: Vec<&str> = expectations
                .iter()
                .filter(|expectation| expectation.stage == stage && expectation.state.in_force() && expectation.verdict != Some(Verdict::Met))
                .map(|expectation| expectation.id.as_str())
                .collect();
            if expectations.iter().any(|expectation| expectation.stage == stage && expectation.verdict == Some(Verdict::Undecided)) {
                return Some(format!("{} is undecided: run more observations, with set_stage_settings or the expectation's own `observations`", ids(failing)));
            }
            return Some(format!("change the model until {} {} met, or question {} if it can't be", ids(failing.clone()), if failing.len() == 1 { "is" } else { "are" }, if failing.len() == 1 { "it" } else { "them" }));
        }
        None
    };
    if let Some(next) = check(CheckStage::Verification, verification) {
        return next;
    }
    if !has(CheckStage::Validation) {
        return format!("verified. Next: add randomness, such as failures, and reference figures from the real line to validate `{path}` against: expectations at the validation stage with a tolerance and their `source`, such as `Drain.throughput_per_hour since start = 4100/d ± 3%`");
    }
    if let Some(next) = check(CheckStage::Validation, validation) {
        return next;
    }
    "validated: every expectation the user accepted is met".to_string()
}

// --- check_stage ---

pub struct CheckStageOperation;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckStageInput {
    /// The model file, in the models folder, such as `line.json`.
    pub path: String,
    /// `verification`, the default, or `validation`.
    #[serde(default)]
    pub stage: Option<CheckStage>,
    /// Seconds to wait, from 1 to 60, for the check to end before answering. Left out, the
    /// answer comes at once.
    pub wait: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
pub struct CheckStageOutput {
    /// The check's runs, one for each of the stage's settings: run_results gives each one's
    /// verdicts.
    pub runs: Vec<String>,
    /// The check was started now, rather than one of the model as it is already done or going.
    pub started: bool,
    /// Where the model stands, as get_status says, once the check ended or the wait was up.
    pub status: GetStatusOutput,
}

impl Operation for CheckStageOperation {
    const NAME: &'static str = "check_stage";
    const DESCRIPTION: &'static str = "Checks a model file's stage (ADR 0030): `verification`, the default, runs it deterministically, without failures, and judges its verification expectations; `validation` runs an experiment with randomness and judges its validation expectations by their 95% intervals. Each check is one run for each of the stage's settings (set_stage_settings, or an expectation's own length, warm-up and observations), recorded in the results store, so get_status works out the stage from it: verified when every accepted verification expectation is met, validated when every accepted validation expectation is met too. A check of the model as it is now that is already done or going is given rather than started again. crabsim also verifies by itself when an agent releases its claim on the file and before an experiment. Give `wait`, up to 60 seconds, to have the check's end waited for.";
    const EFFECT: Effect = Effect::Additive;
    type Input = CheckStageInput;
    type Output = CheckStageOutput;

    fn run(catalogue: &Catalogue, input: CheckStageInput) -> Result<CheckStageOutput, OperationError> {
        if input.wait.is_some_and(|wait| !(1..=60).contains(&wait)) {
            return Err(OperationError::invalid_input("`wait` must be from 1 to 60 seconds"));
        }
        let stage = input.stage.unwrap_or_default();
        let (_, text) = read_file(catalogue, &input.path)?;
        let prepared = prepare(catalogue, &input.path, &text)?;
        let key = crate::editing::key_of(catalogue, &input.path)?;
        let checks = checks(catalogue, &key, stage)?;
        let (runs, started) = if !checks.running.is_empty() {
            (checks.running, false)
        } else if let Some(finished) = checks.finished.filter(|finished| finished.model_hash == prepared.hash) {
            (finished.runs, false)
        } else {
            (start_check(catalogue, &input.path, &prepared, stage)?, true)
        };
        if let Some(wait) = input.wait {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(u64::from(wait));
            while runs.iter().any(|run| catalogue.jobs_status(run).is_some_and(|state| matches!(state, crate::RunState::Queued | crate::RunState::Running)))
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        Ok(CheckStageOutput { runs, started, status: status(catalogue, &input.path)? })
    }
}

// --- get_status ---

pub struct GetStatus;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetStatusInput {
    /// The model file, in the models folder, such as `line.json`.
    pub path: String,
}

impl Operation for GetStatus {
    const NAME: &'static str = "get_status";
    const DESCRIPTION: &'static str = "Says where a model file stands (ADR 0030), in one call to start a session with: its stage (draft, verified or validated), each stage's latest check with whether it was of the model as it is now or `not_checked_since` which change, every request with the user's words, whether the user accepted it and its state (open, built, met or broken), every expectation with its state (proposed, accepted or questioned with its question), its verdict and, where crabsim works out without running it that it can't be met or contradicts another, `cannot_be_met` with why, and `next`, what to do next. Stages and verdicts come from the checks crabsim keeps, never from the model file, which a check never changes.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = GetStatusInput;
    type Output = GetStatusOutput;

    fn run(catalogue: &Catalogue, input: GetStatusInput) -> Result<GetStatusOutput, OperationError> {
        status(catalogue, &input.path)
    }
}
