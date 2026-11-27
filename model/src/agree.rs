//! Agreeing (ADR 0030): where each expectation stands between an agent and the user, the user's
//! requests that expectations check, and the run settings each stage is checked with.
//!
//! An agent proposes an expectation, the user accepts it, and only the user changes an accepted
//! one; an agent may question it with its reason, which leaves it in force until the user answers.
//! A request holds the user's own words, an agent's restatement and the expectations that check
//! it, and is accepted with them in one step. What a check found, such as a request being met,
//! never goes into the model file: crabsim works it out from the checks it keeps, so checking a
//! model never changes it.

use crate::expect::{CheckStage, Verdict};
use crate::{Model, Problem};
use crabsim::time::format_time_exact;
use crabsim::SimTime;
use serde::Serialize;
use std::collections::BTreeMap;

/// Where an expectation stands between an agent and the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Agreement {
    /// Written by an agent and not accepted yet: it is judged, but no stage rests on it.
    #[default]
    Proposed,
    /// Accepted by the user, or written by the user: only the user changes it.
    Accepted,
    /// Accepted, and questioned by an agent with its reason. Still in force until the user
    /// accepts it again, changes it or removes it.
    Questioned,
}

impl Agreement {
    pub const ALL: [Agreement; 3] = [Agreement::Proposed, Agreement::Accepted, Agreement::Questioned];

    pub fn name(self) -> &'static str {
        match self {
            Agreement::Proposed => "proposed",
            Agreement::Accepted => "accepted",
            Agreement::Questioned => "questioned",
        }
    }

    pub fn named(name: &str) -> Option<Agreement> {
        Agreement::ALL.into_iter().find(|agreement| agreement.name() == name)
    }

    /// Whether the user has agreed to it, so a stage rests on it and only the user changes it.
    pub fn in_force(self) -> bool {
        self != Agreement::Proposed
    }
}

/// What a user wants from the model, with the expectations that check it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Request {
    /// Unique among the model's requests, with no spaces, such as `r1`.
    pub id: String,
    /// The user's own words, exactly.
    pub words: String,
    /// An agent's restatement in precise terms.
    pub restatement: Option<String>,
    /// The expectations that check it, by id.
    pub expectations: Vec<String>,
    /// The user accepted its restatement and expectations.
    pub accepted: bool,
    /// Whoever builds it says it is built, which is when crabsim starts saying whether it is met.
    pub built: bool,
}

/// Where a request stands, as crabsim reports it: what the file says, and once built, what its
/// expectations' latest checks found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    /// Not built yet.
    Open,
    /// Built, and not every expectation of it checked since, or one undecided.
    Built,
    /// Built, and every expectation of it met in its latest check.
    Met,
    /// Built, and an expectation of it not met, or not judged, in its latest check.
    Broken,
}

impl Request {
    /// Its state, given each expectation's verdict in its latest check of the model as it is,
    /// `None` where it hasn't been checked since the model last changed.
    pub fn state(&self, verdict: impl Fn(&str) -> Option<Verdict>) -> RequestState {
        if !self.built {
            return RequestState::Open;
        }
        let verdicts: Vec<Option<Verdict>> = self.expectations.iter().map(|id| verdict(id)).collect();
        if verdicts.iter().any(|verdict| matches!(verdict, Some(Verdict::NotMet | Verdict::Error))) {
            RequestState::Broken
        } else if !verdicts.is_empty() && verdicts.iter().all(|verdict| *verdict == Some(Verdict::Met)) {
            RequestState::Met
        } else {
            RequestState::Built
        }
    }
}

/// How long a check runs and how often, as a file gives it: each part left out takes the
/// stage's, and the stage's its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunSettings {
    /// How long each observation runs after the warm-up.
    pub length: Option<SimTime>,
    /// Simulated time at the start of each observation that statistics leave out.
    pub warm_up: Option<SimTime>,
    /// How many observations.
    pub observations: Option<u32>,
}

impl RunSettings {
    pub fn is_empty(&self) -> bool {
        *self == RunSettings::default()
    }

    /// These settings, each part left out taken from `base`.
    pub fn over(&self, base: Settings) -> Settings {
        Settings {
            length: self.length.unwrap_or(base.length),
            warm_up: self.warm_up.unwrap_or(base.warm_up),
            observations: self.observations.unwrap_or(base.observations),
        }
    }
}

/// How a check runs, every part known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, schemars::JsonSchema)]
pub struct Settings {
    #[serde(serialize_with = "written_time")]
    #[schemars(with = "String")]
    pub length: SimTime,
    #[serde(serialize_with = "written_time")]
    #[schemars(with = "String")]
    pub warm_up: SimTime,
    pub observations: u32,
}

fn written_time<S: serde::Serializer>(time: &SimTime, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format_time_exact(*time))
}

/// The most observations a check may have, as for any run.
pub const MAX_OBSERVATIONS: u32 = 10_000;

/// Each stage's run settings, as the model gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stages {
    pub verification: RunSettings,
    pub validation: RunSettings,
}

impl Stages {
    pub fn is_empty(&self) -> bool {
        self.verification.is_empty() && self.validation.is_empty()
    }

    pub fn of(&self, stage: CheckStage) -> &RunSettings {
        match stage {
            CheckStage::Verification => &self.verification,
            CheckStage::Validation => &self.validation,
        }
    }

    pub fn of_mut(&mut self, stage: CheckStage) -> &mut RunSettings {
        match stage {
            CheckStage::Verification => &mut self.verification,
            CheckStage::Validation => &mut self.validation,
        }
    }

    /// A stage's settings: a day after no warm-up, one observation for verification, which is
    /// deterministic, and ten for validation.
    pub fn settings(&self, stage: CheckStage) -> Settings {
        let default = Settings { length: SimTime::from_secs(86_400), warm_up: SimTime::ZERO, observations: if stage == CheckStage::Verification { 1 } else { 10 } };
        self.of(stage).over(default)
    }
}

impl Model {
    /// The settings the expectation at `index` is checked with: its own, over its stage's.
    pub fn settings_of(&self, index: usize) -> Settings {
        let expectation = &self.expectations[index];
        expectation.run.over(self.stages.settings(expectation.stage))
    }

    /// The expectations of a stage, by their place in the model, grouped by the settings they are
    /// checked with: one run for each group.
    pub fn checks_of(&self, stage: CheckStage) -> BTreeMap<Settings, Vec<usize>> {
        let mut groups: BTreeMap<Settings, Vec<usize>> = BTreeMap::new();
        for (index, expectation) in self.expectations.iter().enumerate() {
            if expectation.stage == stage {
                groups.entry(self.settings_of(index)).or_default().push(index);
            }
        }
        groups
    }

    /// The request with this id.
    pub fn request(&self, id: &str) -> Option<&Request> {
        self.requests.iter().find(|request| request.id == id)
    }

    /// The problems in the model's requests, agreement and run settings.
    pub(crate) fn agreement_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (index, expectation) in self.expectations.iter().enumerate() {
            let mut problem = |at: &str, message: String| problems.push(Problem { pointer: format!("/expectations/{index}{at}"), name: Some(expectation.id.clone()).filter(|id| !id.is_empty()), message });
            match (&expectation.question, expectation.state) {
                (None, Agreement::Questioned) => problem("/question", "a questioned expectation says why in its `question`".into()),
                (Some(question), Agreement::Questioned) if question.trim().is_empty() => problem("/question", "a questioned expectation says why in its `question`".into()),
                (Some(_), state) if state != Agreement::Questioned => problem("/question", format!("only a questioned expectation has a `question`, and this one is {}", state.name())),
                _ => {}
            }
            run_problems(&expectation.run, &mut |at, message| problem(at, message));
        }
        for stage in [CheckStage::Verification, CheckStage::Validation] {
            run_problems(self.stages.of(stage), &mut |at, message| problems.push(Problem { pointer: format!("/stages/{}{at}", stage.name()), name: None, message }));
        }
        let ids: Vec<&str> = self.expectations.iter().map(|expectation| expectation.id.as_str()).collect();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, request) in self.requests.iter().enumerate() {
            let mut problem = |at: &str, message: String| problems.push(Problem { pointer: format!("/requests/{index}{at}"), name: Some(request.id.clone()).filter(|id| !id.is_empty()), message });
            if request.id.trim().is_empty() {
                problem("/id", "a request needs an id, such as `r1`".into());
            } else if request.id.contains(char::is_whitespace) {
                problem("/id", "a request's id has no spaces, such as `r1`".into());
            } else if let Some(first) = seen.get(request.id.as_str()) {
                problem("/id", format!("request {first} already has this id"));
            } else {
                seen.insert(&request.id, index);
            }
            if request.words.trim().is_empty() {
                problem("/words", "a request needs the user's own `words`".into());
            }
            for (place, id) in request.expectations.iter().enumerate() {
                if !ids.contains(&id.as_str()) {
                    let known = if ids.is_empty() { "the model has none".to_string() } else { format!("there are {}", ids.join(", ")) };
                    problem(&format!("/expectations/{place}"), format!("there is no expectation `{id}`: {known}"));
                } else if request.expectations[..place].contains(id) {
                    problem(&format!("/expectations/{place}"), format!("`{id}` is listed twice"));
                }
            }
        }
        problems
    }
}

fn run_problems(run: &RunSettings, problem: &mut dyn FnMut(&str, String)) {
    if run.length == Some(SimTime::ZERO) {
        problem("/length", "a check runs for longer than 0".into());
    }
    if let Some(observations) = run.observations
        && !(1..=MAX_OBSERVATIONS).contains(&observations)
    {
        problem("/observations", format!("a check has from 1 to {MAX_OBSERVATIONS} observations"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_met_once_built_and_every_expectation_of_it_is_met() {
        let mut request = Request { id: "r1".into(), words: "faster".into(), expectations: vec!["e1".into(), "e2".into()], ..Request::default() };
        let all = |verdict: Option<Verdict>| move |_: &str| verdict;
        assert_eq!(request.state(all(Some(Verdict::Met))), RequestState::Open, "not built yet");
        request.built = true;
        assert_eq!(request.state(all(Some(Verdict::Met))), RequestState::Met);
        assert_eq!(request.state(all(None)), RequestState::Built, "not checked since");
        assert_eq!(request.state(|id| if id == "e1" { Some(Verdict::Met) } else { Some(Verdict::Undecided) }), RequestState::Built);
        assert_eq!(request.state(|id| if id == "e1" { Some(Verdict::Met) } else { Some(Verdict::NotMet) }), RequestState::Broken);
        assert_eq!(request.state(|id| if id == "e1" { None } else { Some(Verdict::Error) }), RequestState::Broken);
    }

    #[test]
    fn settings_come_from_the_expectation_then_its_stage_then_the_default() {
        let mut stages = Stages::default();
        assert_eq!(stages.settings(CheckStage::Verification), Settings { length: SimTime::from_secs(86_400), warm_up: SimTime::ZERO, observations: 1 });
        assert_eq!(stages.settings(CheckStage::Validation).observations, 10);
        stages.validation.length = Some(SimTime::from_secs(3600));
        let own = RunSettings { observations: Some(30), ..RunSettings::default() };
        assert_eq!(own.over(stages.settings(CheckStage::Validation)), Settings { length: SimTime::from_secs(3600), warm_up: SimTime::ZERO, observations: 30 });
    }
}
