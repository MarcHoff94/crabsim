//! Where the open model stands, as the Status tab shows it (ADR 0030): its stage, each stage's
//! latest check, the user's requests, every expectation with its state and verdict, and the next
//! step, read with `get_status`. The user accepts what an agent proposed here, and checks a stage.
//! Plain Rust with no egui types, so it is tested without a window.

use serde_json::Value;

/// A stage's latest check.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StageCheck {
    pub passed: bool,
    /// The check was of the model as it is now.
    pub current: bool,
    /// Runs of a check still going.
    pub running: Vec<String>,
    /// Such as "met by run-4, not checked since change 12".
    pub summary: String,
}

/// A request as the tab shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RequestLine {
    pub id: String,
    pub words: String,
    pub restatement: Option<String>,
    pub expectations: Vec<String>,
    pub accepted: bool,
    /// open, built, met or broken.
    pub state: String,
}

/// An expectation as the tab shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExpectationLine {
    pub id: String,
    pub text: String,
    /// verification or validation.
    pub stage: String,
    /// proposed, accepted or questioned.
    pub state: String,
    pub question: Option<String>,
    /// met, not_met, undecided or error, from its stage's latest check.
    pub verdict: Option<String>,
    /// The verdict is of the model as it is now.
    pub current: bool,
    pub found: Option<String>,
    pub note: Option<String>,
}

impl ExpectationLine {
    /// Its verdict as the tab words it.
    pub fn verdict_text(&self) -> String {
        match (&self.verdict, self.current) {
            (None, _) => "unchecked".to_string(),
            (Some(verdict), true) => verdict.replace('_', " "),
            (Some(verdict), false) => format!("{} before a change", verdict.replace('_', " ")),
        }
    }

    /// Waiting for the user: proposed by an agent, or questioned.
    pub fn awaits_user(&self) -> bool {
        self.state != "accepted"
    }
}

/// Where the open model stands.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// draft, verified or validated.
    pub stage: String,
    pub verification: StageCheck,
    pub validation: StageCheck,
    pub requests: Vec<RequestLine>,
    pub expectations: Vec<ExpectationLine>,
    pub next: String,
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn texts(value: &Value) -> Vec<String> {
    value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn optional(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

impl Status {
    /// From `get_status`'s output.
    pub fn from_json(status: &Value) -> Status {
        let check = |check: &Value| StageCheck { passed: check["passed"] == true, current: check["current"] == true, running: texts(&check["running"]), summary: text(&check["summary"]) };
        Status {
            stage: text(&status["stage"]),
            verification: check(&status["verification"]),
            validation: check(&status["validation"]),
            requests: status["requests"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|request| RequestLine {
                    id: text(&request["id"]),
                    words: text(&request["words"]),
                    restatement: optional(&request["restatement"]),
                    expectations: texts(&request["expectations"]),
                    accepted: request["accepted"] == true,
                    state: text(&request["state"]),
                })
                .collect(),
            expectations: status["expectations"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|expectation| ExpectationLine {
                    id: text(&expectation["id"]),
                    text: text(&expectation["text"]),
                    stage: text(&expectation["stage"]),
                    state: text(&expectation["state"]),
                    question: optional(&expectation["question"]),
                    verdict: optional(&expectation["verdict"]),
                    current: expectation["current"] == true,
                    found: optional(&expectation["found"]),
                    note: optional(&expectation["note"]),
                })
                .collect(),
            next: text(&status["next"]),
        }
    }

    /// A check is going, so the tab reads the status again until it ends.
    pub fn checking(&self) -> bool {
        !self.verification.running.is_empty() || !self.validation.running.is_empty()
    }

    /// Everything waiting for the user's acceptance, by id: requests not accepted, and
    /// expectations proposed or questioned that no such request lists, since accepting the
    /// request accepts them.
    pub fn to_accept(&self) -> Vec<String> {
        let requests: Vec<&RequestLine> = self.requests.iter().filter(|request| !request.accepted).collect();
        let mut ids: Vec<String> = requests.iter().map(|request| request.id.clone()).collect();
        ids.extend(
            self.expectations
                .iter()
                .filter(|expectation| expectation.awaits_user() && !requests.iter().any(|request| request.expectations.contains(&expectation.id)))
                .map(|expectation| expectation.id.clone()),
        );
        ids
    }

    /// The expectations a request lists, as the tab shows them under it.
    pub fn expectations_of(&self, request: &RequestLine) -> Vec<&ExpectationLine> {
        request.expectations.iter().filter_map(|id| self.expectations.iter().find(|expectation| &expectation.id == id)).collect()
    }

    /// The expectations no request lists.
    pub fn unrequested(&self) -> Vec<&ExpectationLine> {
        self.expectations.iter().filter(|expectation| !self.requests.iter().any(|request| request.expectations.contains(&expectation.id))).collect()
    }

    /// Whether the model has expectations at `stage`, so it can be checked.
    pub fn has_stage(&self, stage: &str) -> bool {
        self.expectations.iter().any(|expectation| expectation.stage == stage)
    }
}
