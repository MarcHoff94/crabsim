//! Expectations (ADR 0030): checks of a model that crabsim judges on its own, each one line of
//! text in a fixed form, such as `every Press.working_share since start >= 60%` or
//! `Buffer X.mus always <= 20`, kept in the model file's top-level `expectations`.
//!
//! A line names its subject (an instance, a class with `every` or `mean`, or `model`), a figure of
//! the vocabulary (`crabsim::figures`), the time scope it is read over, a comparison and a value
//! with an explicit unit. This module parses and writes lines, checks them against a model, reads
//! their values from a run's states and judges them over the observations of an experiment.

use crate::{Model, Problem, State};
use crabsim::figures::{state_name, Figure, Scope, Subject as FigureSubject, Unit};
use crabsim::time::{format_time, format_time_exact};
use crabsim::{ModelDescription, ObjectKind, OccurrenceKind, OccurrenceType, Recording, RunState, SimTime};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

/// One expectation, as a model file holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Expectation {
    /// Unique among the model's expectations, such as `presses-busy`: how requests, verdicts
    /// and people refer to it.
    pub id: String,
    /// The check as one line, such as `every Press.working_share since start >= 60%`. Saved in its
    /// canonical form once it parses.
    pub text: String,
    /// Which stage it belongs to: verification, the default, or validation.
    pub stage: CheckStage,
    /// What it takes for granted, such as "two shifts a day".
    pub assumptions: Option<String>,
    /// Where it comes from: a SysML element, a document or a person.
    pub source: Option<String>,
    /// Where it stands between an agent and the user: proposed, the default, accepted or
    /// questioned.
    pub state: crate::Agreement,
    /// Why an agent questions it, while it is questioned.
    pub question: Option<String>,
    /// How its check runs, where it differs from its stage's settings.
    pub run: crate::RunSettings,
}

impl Expectation {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Expectation {
        Expectation {
            id: id.into(),
            text: text.into(),
            stage: CheckStage::Verification,
            assumptions: None,
            source: None,
            state: crate::Agreement::Proposed,
            question: None,
            run: crate::RunSettings::default(),
        }
    }
}

/// The stage an expectation belongs to (ADR 0030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStage {
    /// What was specified, checked in a deterministic run.
    #[default]
    Verification,
    /// How the real line behaves, checked in runs with randomness.
    Validation,
}

impl CheckStage {
    pub fn name(self) -> &'static str {
        match self {
            CheckStage::Verification => "verification",
            CheckStage::Validation => "validation",
        }
    }

    pub fn named(name: &str) -> Option<CheckStage> {
        [CheckStage::Verification, CheckStage::Validation].into_iter().find(|stage| stage.name() == name)
    }
}

/// How a line takes its subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    /// One instance, or the model.
    One,
    /// Each instance of a class, every one of which must meet it.
    Every,
    /// The mean over a class's instances.
    Mean,
}

/// What a line is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckSubject {
    /// An instance, by its name; with `every` or `mean`, a class.
    Named(String),
    /// The model as a whole.
    Model,
}

/// How a line compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub enum Comparison {
    #[serde(rename = ">=")]
    AtLeast,
    #[serde(rename = "<=")]
    AtMost,
    #[serde(rename = ">")]
    Above,
    #[serde(rename = "<")]
    Below,
    #[serde(rename = "=")]
    Equal,
    #[serde(rename = "!=")]
    NotEqual,
}

impl Comparison {
    const ALL: [Comparison; 6] = [Comparison::AtLeast, Comparison::AtMost, Comparison::NotEqual, Comparison::Above, Comparison::Below, Comparison::Equal];

    pub fn written(self) -> &'static str {
        match self {
            Comparison::AtLeast => ">=",
            Comparison::AtMost => "<=",
            Comparison::Above => ">",
            Comparison::Below => "<",
            Comparison::Equal => "=",
            Comparison::NotEqual => "!=",
        }
    }

    /// Whether `value` meets it against `limit`.
    pub fn holds(self, value: f64, limit: f64) -> bool {
        match self {
            Comparison::AtLeast => value >= limit,
            Comparison::AtMost => value <= limit,
            Comparison::Above => value > limit,
            Comparison::Below => value < limit,
            Comparison::Equal => value == limit,
            Comparison::NotEqual => value != limit,
        }
    }

    /// An upper limit, which the most a figure reached is compared with.
    fn upper(self) -> bool {
        matches!(self, Comparison::AtMost | Comparison::Below)
    }
}

/// The value a line compares with.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A number in the figure's own unit (a share from 0 to 1, MUs an hour, seconds, a count), and
    /// how the line writes it, such as `60%`.
    Number { value: f64, written: String },
    /// A state's name.
    State(State),
}

impl Target {
    fn written(&self) -> &str {
        match self {
            Target::Number { written, .. } => written,
            Target::State(state) => state.name(),
        }
    }
}

/// How far from an `=` line's value a figure may lie, such as a reference figure's `± 3%`: in
/// the figure's own unit, or for a figure not counted in shares, a percentage of the value.
#[derive(Debug, Clone, PartialEq)]
pub struct Tolerance {
    /// In the figure's own unit, or as a share of the value when `relative`.
    pub amount: f64,
    /// A share of the value, such as `± 3%` on a rate, rather than an amount in its unit.
    pub relative: bool,
    /// How the line writes it, such as `3%` or `30s`.
    written: String,
}

impl Tolerance {
    /// How the line writes it, such as `3%`, without the `±`.
    pub fn written(&self) -> &str {
        &self.written
    }
}

/// A line, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub aggregate: Aggregate,
    pub subject: CheckSubject,
    pub figure: Figure,
    pub scope: Scope,
    /// How a `last` scope's length is written, such as `1h`.
    last_written: Option<String>,
    pub comparison: Comparison,
    pub target: Target,
    /// With `=`, how far from the value it may lie: a reference figure's tolerance (ADR 0030).
    pub tolerance: Option<Tolerance>,
}

/// Why a line doesn't parse, and where, counting characters from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineError {
    pub at: usize,
    pub message: String,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at character {}: {}", self.at, self.message)
    }
}

const FORM: &str = "write it as `[every|mean] <instance, class or model>.<figure> <now|since start|last <time>|always> <comparison> <value>`, such as `every Press.working_share since start >= 60%`, or with `=` and a tolerance, such as `Drain.throughput_per_hour since start = 4100/d ± 3%`";

/// How a tolerance may be typed after an `=` line's value; the first is how it is written.
const PLUS_MINUS: [&str; 3] = ["±", "+/-", "+-"];

impl Check {
    /// Reads a line. Names aren't looked up: that's `Model::expectation_problems`.
    pub fn parse(text: &str) -> Result<Check, LineError> {
        let error = |at: usize, message: String| Err(LineError { at: at + 1, message });
        // Characters, so a position counts as people do, whatever the letters.
        let chars: Vec<char> = text.chars().collect();
        let mut at = skip_spaces(&chars, 0);
        let word_end = |from: usize| (from..chars.len()).find(|&i| chars[i].is_whitespace()).unwrap_or(chars.len());
        let first: String = chars[at..word_end(at)].iter().collect();
        let aggregate = match first.as_str() {
            "every" => Aggregate::Every,
            "mean" => Aggregate::Mean,
            _ => Aggregate::One,
        };
        if aggregate != Aggregate::One {
            at = skip_spaces(&chars, word_end(at));
        }
        // The address runs to the first space after the last `.` that a figure's name follows.
        let mut found = None;
        for dot in (at..chars.len()).filter(|&i| chars[i] == '.') {
            let name: String = chars[dot + 1..word_end(dot + 1)].iter().collect();
            if let Some(figure) = Figure::named(&name) {
                found = Some((dot, figure));
            }
        }
        let Some((dot, figure)) = found else {
            let names: Vec<&str> = Figure::ALL.iter().filter(|figure| figure.subject() == FigureSubject::Object).map(|figure| figure.name()).collect();
            return error(at, format!("no figure after a `.`: {FORM}. The figures are {}", names.join(", ")));
        };
        let name: String = chars[at..dot].iter().collect();
        if name.trim().is_empty() {
            return error(at, format!("the line names no instance, class or model before `.{}`", figure.name()));
        }
        if name.trim() != name {
            return error(at, "a name can't start or end with spaces".into());
        }
        let subject = if name == "model" { CheckSubject::Model } else { CheckSubject::Named(name) };
        if figure.subject() == FigureSubject::Mu {
            return error(dot + 1, format!("`{}` is an MU's figure, for rules and code: an expectation names an object, a class or the model", figure.name()));
        }
        match (&subject, aggregate) {
            (CheckSubject::Model, Aggregate::Every | Aggregate::Mean) => return error(0, "`every` and `mean` take a class, not the model".into()),
            (CheckSubject::Model, _) if !figure.of_model() => {
                let names: Vec<&str> = Figure::ALL.iter().filter(|figure| figure.of_model()).map(|figure| figure.name()).collect();
                return error(dot + 1, format!("the model has no `{}`: its figures are {}", figure.name(), names.join(", ")));
            }
            _ => {}
        }
        at = skip_spaces(&chars, word_end(dot + 1));
        let word = |from: usize| -> String { chars[from..word_end(from)].iter().collect() };
        let scope_at = at;
        let (scope, last_written) = match word(at).as_str() {
            "now" => {
                at = word_end(at);
                (Scope::Now, None)
            }
            "always" => {
                at = word_end(at);
                (Scope::Always, None)
            }
            "since" => {
                let next = skip_spaces(&chars, word_end(at));
                if word(next) != "start" {
                    return error(next, "`since` goes on with `start`: statistics begin at Init, or at the end of the warm-up".into());
                }
                at = word_end(next);
                (Scope::SinceStart, None)
            }
            "last" => {
                let next = skip_spaces(&chars, word_end(at));
                let typed = word(next);
                let Some((length, written)) = time_value(&typed) else {
                    return error(next, format!("`last` takes a length of time, such as `1h`, `30min` or `8:00:00`, not `{typed}`"));
                };
                if length == SimTime::ZERO {
                    return error(next, "`last` takes a length longer than 0".into());
                }
                at = word_end(next);
                (Scope::Last(length), Some(written))
            }
            other => return error(at, format!("a time scope comes after the figure: `now`, `since start`, `last <time>` or `always`, not `{other}`")),
        };
        if !figure.has_scope(scope.kind()) {
            let scopes: Vec<&str> = figure.scopes().iter().map(|scope| scope.written()).collect();
            return error(scope_at, format!("`{}` is read {}, not {}", figure.name(), scopes.join(" or "), scope.kind().written()));
        }
        if scope == Scope::Always && aggregate == Aggregate::Mean {
            return error(scope_at, "a limit with `always` holds for each instance: write `every`, not `mean`".into());
        }
        at = skip_spaces(&chars, at);
        let rest: String = chars[at..].iter().collect();
        let Some(comparison) = Comparison::ALL.into_iter().find(|comparison| rest.starts_with(comparison.written())) else {
            return error(at, format!("a comparison comes after the scope: >=, <=, >, <, = or !=, not `{}`", rest.trim()));
        };
        let value_at = skip_spaces(&chars, at + comparison.written().len());
        let rest: String = chars[value_at..].iter().collect();
        // A tolerance follows the value, after `±`.
        let marked = PLUS_MINUS.iter().filter_map(|mark| rest.find(mark).map(|byte| (byte, mark.len()))).min();
        let (value, tolerance_at) = match marked {
            Some((byte, length)) => {
                let before = rest[..byte].chars().count();
                let after = rest[byte + length..].chars().count();
                (rest[..byte].trim_end().to_string(), Some((value_at + before, chars.len() - after)))
            }
            None => (rest.trim_end().to_string(), None),
        };
        if value.is_empty() {
            return error(value_at, "the line ends before its value".into());
        }
        let unit = figure.unit();
        if unit == Unit::State && !matches!(comparison, Comparison::Equal | Comparison::NotEqual) {
            return error(at, "a state is compared with = or !=".into());
        }
        if scope == Scope::Always && !comparison.upper() && !(figure == Figure::LeadTime && matches!(comparison, Comparison::AtLeast | Comparison::Above)) {
            return error(at, format!("a limit on `{}` with `always` is an upper one: <= or <", figure.name()));
        }
        let target = match target(unit, &value) {
            Ok(target) => target,
            Err(message) => return error(value_at, message),
        };
        let tolerance = match tolerance_at {
            None => None,
            Some((mark_at, _)) if comparison != Comparison::Equal => {
                return error(mark_at, "a tolerance goes with `=`, such as `= 4100/d ± 3%`: other comparisons say on which side the value must lie".into());
            }
            Some((mark_at, _)) if scope == Scope::Always => return error(mark_at, "a limit with `always` takes no tolerance: it is an upper one, <= or <".into()),
            Some((mark_at, _)) if unit == Unit::State => return error(mark_at, "a state takes no tolerance".into()),
            Some((_, from)) => {
                let from = skip_spaces(&chars, from);
                let typed: String = chars[from..].iter().collect::<String>().trim_end().to_string();
                if typed.is_empty() {
                    return error(from, "the line ends before its tolerance, such as `3%`".into());
                }
                match tolerance(unit, &typed) {
                    Ok(tolerance) => Some(tolerance),
                    Err(message) => return error(from, message),
                }
            }
        };
        Ok(Check { aggregate, subject, figure, scope, last_written, comparison, target, tolerance })
    }

    /// The line in its canonical form.
    pub fn text(&self) -> String {
        let aggregate = match self.aggregate {
            Aggregate::One => "",
            Aggregate::Every => "every ",
            Aggregate::Mean => "mean ",
        };
        let subject = match &self.subject {
            CheckSubject::Named(name) => name.as_str(),
            CheckSubject::Model => "model",
        };
        let tolerance = self.tolerance.as_ref().map_or_else(String::new, |tolerance| format!(" {} {}", PLUS_MINUS[0], tolerance.written));
        format!("{aggregate}{subject}.{} {} {} {}{tolerance}", self.figure.name(), self.scope_text(), self.comparison.written(), self.value_text())
    }

    /// For `=` with a tolerance, the lowest and highest values that meet it, in the figure's unit.
    pub fn band(&self) -> Option<(f64, f64)> {
        let tolerance = self.tolerance.as_ref()?;
        let Target::Number { value, .. } = self.target else { return None };
        let amount = if tolerance.relative { value.abs() * tolerance.amount } else { tolerance.amount };
        Some((value - amount, value + amount))
    }

    /// The values that meet it, as the lowest and highest with whether each is included, `None`
    /// for no limit on that side; for `!=` and a state, every value but one, which this can't
    /// say, so `None`.
    pub fn accepted(&self) -> Option<(Option<(f64, bool)>, Option<(f64, bool)>)> {
        if let Some((low, high)) = self.band() {
            return Some((Some((low, true)), Some((high, true))));
        }
        let Target::Number { value, .. } = self.target else { return None };
        Some(match self.comparison {
            Comparison::AtLeast => (Some((value, true)), None),
            Comparison::Above => (Some((value, false)), None),
            Comparison::AtMost => (None, Some((value, true))),
            Comparison::Below => (None, Some((value, false))),
            Comparison::Equal => (Some((value, true)), Some((value, true))),
            Comparison::NotEqual => return None,
        })
    }

    /// Its scope as the line writes it, such as `last 1h`.
    pub fn scope_text(&self) -> String {
        match self.scope {
            Scope::Now => "now".to_string(),
            Scope::SinceStart => "since start".to_string(),
            Scope::Last(_) => format!("last {}", self.last_written.as_deref().unwrap_or_default()),
            Scope::Always => "always".to_string(),
        }
    }

    /// The value it compares with as the line writes it, such as `60%`.
    pub fn value_text(&self) -> &str {
        self.target.written()
    }

    /// The limit it compares with, as a number: a state's is 1, for "in that state".
    fn limit(&self) -> f64 {
        match &self.target {
            Target::Number { value, .. } => *value,
            Target::State(_) => 1.0,
        }
    }

    /// The comparison a value is judged by: a state's value is 1 in it and 0 otherwise.
    fn judged_by(&self) -> Comparison {
        match (&self.target, self.comparison) {
            (Target::State(_), Comparison::NotEqual) => Comparison::NotEqual,
            (Target::State(_), _) => Comparison::Equal,
            (_, comparison) => comparison,
        }
    }

    /// A limit during the run, whose first violation a replay can find.
    pub fn is_limit(&self) -> bool {
        self.scope == Scope::Always
    }

    /// How its value is written in the line's own unit, such as `62.5%`.
    pub fn written_value(&self, value: f64) -> String {
        written(self.figure.unit(), value)
    }

    /// The subject's name, if it names one.
    pub fn named(&self) -> Option<&str> {
        match &self.subject {
            CheckSubject::Named(name) => Some(name),
            CheckSubject::Model => None,
        }
    }

    /// The same line about another subject.
    pub fn with_subject(&self, subject: CheckSubject) -> Check {
        Check { subject, ..self.clone() }
    }

    /// The same line naming `to` where it named `from`, for a rename.
    pub fn renamed(&self, from: &str, to: &str) -> Option<Check> {
        (self.named() == Some(from)).then(|| Check { subject: CheckSubject::Named(to.to_string()), ..self.clone() })
    }
}

fn skip_spaces(chars: &[char], from: usize) -> usize {
    (from..chars.len()).find(|&i| !chars[i].is_whitespace()).unwrap_or(chars.len())
}

/// A number as a line writes it: plain digits with an optional point, so `1.5`, never `1,5`.
fn number(typed: &str) -> Option<f64> {
    (!typed.is_empty() && typed.chars().all(|c| c.is_ascii_digit() || c == '.') && typed.chars().any(|c| c.is_ascii_digit()))
        .then(|| typed.parse::<f64>().ok())
        .flatten()
}

/// `value` as `{}` writes a number, without a trailing `.0`.
pub(crate) fn plain(value: f64) -> String {
    let rounded = (value * 1e9).round() / 1e9;
    format!("{rounded}")
}

/// A length of time with its unit: `90s`, `30min`, `2h`, `1d`, or as the UI writes time, such as
/// `1:30` or `2d 06:00:00`. A bare number isn't one: its unit wouldn't be explicit.
pub(crate) fn time_value(typed: &str) -> Option<(SimTime, String)> {
    for (suffix, seconds) in [("min", 60.0), ("s", 1.0), ("h", 3_600.0)] {
        if let Some(amount) = typed.strip_suffix(suffix).and_then(number) {
            return Some((SimTime::from_secs_f64(amount * seconds), format!("{}{suffix}", plain(amount))));
        }
    }
    if typed.contains(':') || typed.contains('d') {
        let time = crabsim::time::parse_time(typed)?;
        return Some((time, format_time_exact(time)));
    }
    None
}

/// The value a line compares with, read in the figure's unit.
pub(crate) fn target(unit: Unit, typed: &str) -> Result<Target, String> {
    let number_target = |value: f64, written: String| Ok(Target::Number { value, written });
    match unit {
        Unit::Share => match typed.strip_suffix('%').and_then(number) {
            Some(percent) => number_target(percent / 100.0, format!("{}%", plain(percent))),
            None => Err(format!("a share is written as a percentage, such as `60%`, not `{typed}`")),
        },
        Unit::PerHour => {
            for (suffix, per_hour) in [("/h", 1.0), ("/min", 60.0), ("/d", 1.0 / 24.0)] {
                if let Some(rate) = typed.strip_suffix(suffix).and_then(number) {
                    return number_target(rate * per_hour, format!("{}{suffix}", plain(rate)));
                }
            }
            Err(format!("a rate is written with its unit, such as `4000/h`, `70/min` or `96000/d`, not `{typed}`"))
        }
        Unit::Time => match time_value(typed) {
            Some((time, written)) => number_target(time.as_secs_f64(), written),
            None => Err(format!("a time is written with its unit, such as `90s`, `30min`, `2h` or `1:30`, not `{typed}`")),
        },
        Unit::Count | Unit::Mus => match number(typed) {
            Some(value) => number_target(value, plain(value)),
            None => Err(format!("a number of MUs is written plainly, such as `20`, not `{typed}`")),
        },
        Unit::State => match State::from_name(typed) {
            Some(state) => Ok(Target::State(state)),
            None => Err(format!("`{typed}` isn't a state: busy, holding, blocked, idle, receiving, failed, stopped or paused")),
        },
        Unit::Name => Err("an expectation can't compare a name".into()),
    }
}

/// A tolerance as a line writes it after `±`: in the figure's own unit, such as `100/d`, `30s` or,
/// for a share, `5%` (percentage points), or for any other figure a percentage of the value, such
/// as `3%`.
pub(crate) fn tolerance(unit: Unit, typed: &str) -> Result<Tolerance, String> {
    let relative = unit != Unit::Share && typed.ends_with('%');
    let amount = if relative {
        match typed.strip_suffix('%').and_then(number) {
            Some(percent) => Tolerance { amount: percent / 100.0, relative: true, written: format!("{}%", plain(percent)) },
            None => return Err(format!("a tolerance is written in the figure's unit or as a percentage of the value, such as `3%`, not `{typed}`")),
        }
    } else {
        match target(unit, typed) {
            Ok(Target::Number { value, written }) => Tolerance { amount: value, relative: false, written },
            Ok(Target::State(_)) => return Err("a state takes no tolerance".into()),
            Err(message) => {
                let also = if unit == Unit::Share { "; for a share, in percentage points" } else { ", or as a percentage of the value, such as `3%`" };
                return Err(format!("{message}{also}"));
            }
        }
    };
    if amount.amount <= 0.0 {
        return Err("a tolerance is more than 0: leave it out for an exact value".into());
    }
    Ok(amount)
}

/// `value`, in a figure's own unit, as a line would write it.
pub fn written(unit: Unit, value: f64) -> String {
    match unit {
        Unit::Share => format!("{}%", plain(crate_round(value * 100.0))),
        Unit::PerHour => format!("{}/h", plain(crate_round(value))),
        Unit::Time => format_time_exact(SimTime::from_secs_f64(value.max(0.0))),
        _ => plain(crate_round(value)),
    }
}

/// To 4 significant digits, as results show figures.
fn crate_round(value: f64) -> f64 {
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

/// The kind of object a built-in class makes.
pub fn kind_of(built_in: &str) -> Option<ObjectKind> {
    Some(match built_in {
        "Source" => ObjectKind::Source,
        "Station" => ObjectKind::Station,
        "Buffer" => ObjectKind::Buffer,
        "Drain" => ObjectKind::Drain,
        "Conveyor" => ObjectKind::Conveyor,
        "CrossConveyor" => ObjectKind::CrossConveyor,
        _ => return None,
    })
}

// ---------- Against a model ----------

/// An expectation bound to a valid model: its line and the instances it is about, by id.
#[derive(Debug, Clone)]
pub struct Bound {
    /// Its place in the model's expectations.
    pub index: usize,
    pub check: Check,
    /// The instances it reads, in id order: one for an instance, each of a class's, none for the
    /// model.
    pub subjects: Vec<u32>,
}

impl Model {
    /// The instances of a class, by id: those whose chain of classes holds it, a built-in class
    /// counting as the root of every chain that ends in it.
    pub fn instances_of(&self, class: &str) -> Vec<u32> {
        (0..self.instances.len())
            .filter(|&index| {
                let instance = &self.instances[index];
                instance.class == class || self.chain(&instance.class).is_some_and(|(chain, built_in)| built_in == class || chain.iter().any(|link| link.name == class))
            })
            .map(|index| index as u32)
            .collect()
    }

    /// The problems in the model's expectations: what can't be read, and names, figures and
    /// states that don't fit the model.
    pub(crate) fn expectation_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, expectation) in self.expectations.iter().enumerate() {
            let mut problem = |at: &str, message: String| problems.push(Problem { pointer: format!("/expectations/{index}{at}"), name: Some(expectation.id.clone()).filter(|id| !id.is_empty()), message });
            if expectation.id.trim().is_empty() {
                problem("/id", "an expectation needs an id, such as `presses-busy`".into());
            } else if expectation.id.trim() != expectation.id || expectation.id.contains(char::is_whitespace) {
                problem("/id", "an expectation's id has no spaces, such as `presses-busy`".into());
            } else if let Some(first) = seen.get(expectation.id.as_str()) {
                problem("/id", format!("expectation {first} already has this id"));
            } else {
                seen.insert(&expectation.id, index);
            }
            if expectation.text.trim().is_empty() {
                problem("/text", format!("an expectation needs its `text`: {FORM}"));
                continue;
            }
            let check = match Check::parse(&expectation.text) {
                Ok(check) => check,
                Err(error) => {
                    problem("/text", error.to_string());
                    continue;
                }
            };
            if let Err(message) = self.fits(&check) {
                problem("/text", message);
            }
        }
        problems
    }

    /// Whether a parsed line fits this model: its subject exists and has its figure and state.
    fn fits(&self, check: &Check) -> Result<(), String> {
        let CheckSubject::Named(name) = &check.subject else {
            return Ok(());
        };
        let built_in = match check.aggregate {
            Aggregate::One => {
                let Some(index) = self.instances.iter().position(|instance| &instance.name == name) else {
                    let also = if self.class(name).is_some() || crate::BUILT_IN_OBJECT_CLASSES.contains(&name.as_str()) {
                        format!(": `{name}` is a class, which takes `every` or `mean` before it")
                    } else {
                        String::new()
                    };
                    return Err(format!("there is no instance `{name}`{also}"));
                };
                self.chain(&self.instances[index].class).map(|(_, built_in)| built_in)
            }
            Aggregate::Every | Aggregate::Mean => {
                if !crate::BUILT_IN_OBJECT_CLASSES.contains(&name.as_str()) && self.class(name).is_none() {
                    let also = if self.instances.iter().any(|instance| &instance.name == name) { ": it's an instance, which takes no `every` or `mean`" } else { "" };
                    return Err(format!("there is no class `{name}`{also}"));
                }
                self.chain(name).map(|(_, built_in)| built_in)
            }
        };
        // A broken chain is a problem of its own.
        let Some(built_in) = built_in else {
            return Ok(());
        };
        let Some(kind) = kind_of(built_in) else {
            return Err(format!("`{name}` is an MU class: an expectation names an object, a class of objects or the model"));
        };
        if !check.figure.applies_to(kind) {
            let names: Vec<&str> = Figure::ALL.iter().filter(|figure| figure.applies_to(kind)).map(|figure| figure.name()).collect();
            return Err(format!("{} {built_in} has no `{}`: its figures are {}", crate::article(built_in), check.figure.name(), names.join(", ")));
        }
        if check.is_limit() && !check.figure.limit_kept(kind, check.comparison.upper()) {
            return Err(format!("nothing keeps {} {built_in}'s `{}` at every moment", crate::article(built_in), check.figure.name()));
        }
        if let Target::State(state) = check.target {
            if !State::of(built_in).contains(&state) {
                return Err(format!("{} {built_in} is never {}", crate::article(built_in), state.never()));
            }
        }
        Ok(())
    }

    /// Every expectation bound to the model, which must be valid.
    pub fn bound_expectations(&self) -> Vec<Bound> {
        self.expectations
            .iter()
            .enumerate()
            .filter_map(|(index, expectation)| {
                let check = Check::parse(&expectation.text).ok()?;
                let subjects = match (&check.subject, check.aggregate) {
                    (CheckSubject::Model, _) => Vec::new(),
                    (CheckSubject::Named(name), Aggregate::One) => self.instances.iter().position(|instance| &instance.name == name).map(|index| index as u32).into_iter().collect(),
                    (CheckSubject::Named(name), _) => self.instances_of(name),
                };
                Some(Bound { index, check, subjects })
            })
            .collect()
    }
}

// ---------- Reading values from a run ----------

/// What a run must stop at for its expectations: the start of each `last` stretch, as a time
/// since Init, given the warm-up and the length after it. A stretch longer than the length is
/// left out, and its expectation judged an error.
pub fn snapshot_times(bound: &[Bound], warm_up: SimTime, length: SimTime) -> Vec<SimTime> {
    let mut times: Vec<SimTime> = bound
        .iter()
        .filter_map(|bound| match bound.check.scope {
            Scope::Last(stretch) if stretch <= length => Some((warm_up + length).saturating_sub(stretch)),
            _ => None,
        })
        .collect();
    times.sort();
    times.dedup();
    times
}

/// One expectation's values in one observation: for each subject, by id, or `None` for the model
/// or a class's mean, its value, `None` where it couldn't be read, such as a mean lead time with no
/// MU. A state's value is 1 in the state and 0 otherwise.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Observed {
    pub values: Vec<(Option<u32>, Option<f64>)>,
    /// For a limit, its first violation, once a replay has found it.
    pub violation: Option<Violation>,
}

/// When and by which MU a limit was first broken.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Violation {
    /// The instance's id.
    #[serde(skip)]
    pub object: u32,
    /// Simulated time since Init.
    #[serde(skip)]
    pub time: SimTime,
    /// The MU that broke it, entering the object or leaving into the Drain.
    pub mu: u32,
    /// The value it reached, in the figure's own unit.
    pub value: f64,
}

/// Each expectation's values at the end of an observation, from its last state and the states at
/// its `snapshot_times`, by time.
pub fn observe(bound: &[Bound], end: &RunState, snapshots: &BTreeMap<SimTime, RunState>, warm_up: SimTime, length: SimTime) -> Vec<Observed> {
    bound
        .iter()
        .map(|bound| {
            let check = &bound.check;
            let start = match check.scope {
                Scope::Last(stretch) if stretch <= length => snapshots.get(&((warm_up + length).saturating_sub(stretch))),
                _ => None,
            };
            let read = |id: u32| -> Option<f64> {
                let object = &end.objects[id as usize];
                match check.scope {
                    Scope::SinceStart => check.figure.value(object, end.time),
                    Scope::Last(_) => check.figure.value_over(&start?.objects[id as usize], object),
                    Scope::Always => check.figure.extreme(object, check.comparison.upper()),
                    Scope::Now => match &check.target {
                        Target::State(state) => Some(if state_name(object) == state.name() { 1.0 } else { 0.0 }),
                        Target::Number { .. } => check.figure.value(object, end.time),
                    },
                }
            };
            let values = match (&check.subject, check.aggregate) {
                (CheckSubject::Model, _) => {
                    let value = match check.scope {
                        Scope::SinceStart => check.figure.model_value(&end.objects, end.time),
                        Scope::Last(_) => start.and_then(|start| check.figure.model_value_over(&start.objects, &end.objects)),
                        Scope::Always => check.figure.model_extreme(&end.objects, check.comparison.upper()),
                        Scope::Now => None,
                    };
                    vec![(None, value)]
                }
                (_, Aggregate::Mean) => {
                    let values: Vec<f64> = bound.subjects.iter().filter_map(|&id| read(id)).collect();
                    vec![(None, (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64))]
                }
                _ => bound.subjects.iter().map(|&id| (Some(id), read(id))).collect(),
            };
            Observed { values, violation: None }
        })
        .collect()
}

/// The limits an observation broke, by their place in `bound`: those whose value fails.
pub fn broken_limits(bound: &[Bound], observed: &[Observed]) -> Vec<usize> {
    (0..bound.len())
        .filter(|&at| bound[at].check.is_limit() && observed[at].values.iter().any(|(_, value)| value.is_some_and(|value| !bound[at].check.comparison.holds(value, bound[at].check.limit()))))
        .collect()
}

/// Runs an observation again to find where each broken limit was first broken: the same model,
/// seed and warm-up, recording only MUs coming and going, stopping once every one is found or the
/// observation's end is reached. Only an observation that broke a limit pays for this.
pub fn first_violations(description: &ModelDescription, seed: u64, deterministic: bool, warm_up: SimTime, length: SimTime, limits: &[&Bound]) -> Vec<Option<Violation>> {
    let mut found = vec![None; limits.len()];
    if limits.is_empty() {
        return found;
    }
    let mut run = if deterministic { description.build_deterministic(seed) } else { description.build(seed) };
    if run.init().is_err() || run.advance_to(warm_up).is_err() {
        return found;
    }
    if warm_up > SimTime::ZERO {
        run.clear_statistics();
    }
    let state = run.state();
    let mut held: Vec<i64> = state.objects.iter().map(|object| object.contents.len() as i64).collect();
    run.set_recording(Recording::only(&[OccurrenceType::MuCreated, OccurrenceType::MuMoved, OccurrenceType::MuLeft]));
    let end = warm_up + length;
    loop {
        let Ok(advance) = run.advance_bounded(end, 100_000) else {
            return found;
        };
        for occurrence in &advance.occurrences {
            let (mu, entered, lead_time) = match occurrence.kind {
                OccurrenceKind::MuCreated { mu_id, at, .. } => {
                    held[at as usize] += 1;
                    (mu_id, at, None)
                }
                OccurrenceKind::MuMoved { mu_id, from, to } => {
                    held[from as usize] -= 1;
                    held[to as usize] += 1;
                    (mu_id, to, None)
                }
                OccurrenceKind::MuLeft { mu_id, at, lead_time } => {
                    held[at as usize] -= 1;
                    (mu_id, at, Some(lead_time))
                }
                _ => continue,
            };
            for (place, limit) in limits.iter().enumerate() {
                if found[place].is_some() {
                    continue;
                }
                let watched = matches!(limit.check.subject, CheckSubject::Model) || limit.subjects.contains(&entered);
                let value = match (limit.check.figure, lead_time) {
                    (Figure::Mus, None) => held[entered as usize] as f64,
                    (Figure::LeadTime, Some(lead_time)) => lead_time.as_secs_f64(),
                    _ => continue,
                };
                if watched && !limit.check.comparison.holds(value, limit.check.limit()) {
                    found[place] = Some(Violation { object: entered, time: occurrence.time, mu, value });
                }
            }
        }
        if advance.reached || found.iter().all(Option::is_some) {
            return found;
        }
    }
}

// ---------- Verdicts ----------

/// What crabsim says of an expectation (ADR 0030), as SysML's verdicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Met,
    /// An experiment's 95% interval straddles the limit: more observations would tell.
    Undecided,
    /// It couldn't be judged, such as a figure with no value.
    Error,
    NotMet,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Met => "met",
            Verdict::Undecided => "undecided",
            Verdict::Error => "error",
            Verdict::NotMet => "not met",
        }
    }
}

/// One subject's judgement over an experiment's observations.
#[derive(Debug, Clone, PartialEq)]
pub struct SubjectJudgement {
    /// The instance's id, or `None` for the model or a class's mean.
    pub subject: Option<u32>,
    pub verdict: Verdict,
    /// The mean over the observations with a value.
    pub mean: Option<f64>,
    /// The mean's 95% interval, from two observations on.
    pub ci95: Option<[f64; 2]>,
    /// For a limit, the extreme it reached in any observation.
    pub extreme: Option<f64>,
    /// Why it is an error.
    pub why: Option<String>,
    /// About how many observations in all would decide an undecided one.
    pub observations_needed: Option<u32>,
}

/// An expectation's judgement.
#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    pub verdict: Verdict,
    pub subjects: Vec<SubjectJudgement>,
    /// For a limit, the first violation of the first observation that broke it, with that
    /// observation's number from 1.
    pub violation: Option<(u32, Violation)>,
    /// What to do next, such as running more observations.
    pub advice: Option<String>,
}

/// Judges an expectation over the observations of a run, each with its values, `deterministic`
/// for a run without randomness, where one observation decides. With one observation of a run
/// with randomness the verdict holds for that observation: met so far.
pub fn judge(bound: &Bound, observations: &[Observed], deterministic: bool, length: SimTime) -> Judgement {
    let check = &bound.check;
    if let Scope::Last(stretch) = check.scope {
        if stretch > length {
            let why = format!("the run measured {} after its warm-up, shorter than the last {}", format_time(length), format_time(stretch));
            return Judgement { verdict: Verdict::Error, subjects: vec![error_judgement(None, why.clone())], violation: None, advice: Some("run it longer, or shorten the stretch".into()) };
        }
    }
    if observations.is_empty() {
        return Judgement { verdict: Verdict::Error, subjects: Vec::new(), violation: None, advice: Some("no observation ran to its end".into()) };
    }
    let subjects: Vec<Option<u32>> = observations[0].values.iter().map(|(subject, _)| *subject).collect();
    if subjects.is_empty() {
        let why = "the class has no instances".to_string();
        return Judgement { verdict: Verdict::Error, subjects: vec![error_judgement(None, why)], violation: None, advice: None };
    }
    let judged: Vec<SubjectJudgement> = subjects
        .iter()
        .enumerate()
        .map(|(place, &subject)| {
            let values: Vec<Option<f64>> = observations.iter().map(|observed| observed.values.get(place).and_then(|(_, value)| *value)).collect();
            judge_subject(check, subject, &values, deterministic)
        })
        .collect();
    // Not met outranks an error, an error being undecided, and that being met.
    let verdict = judged.iter().map(|subject| subject.verdict).max().unwrap_or(Verdict::Error);
    let violation = observations.iter().enumerate().find_map(|(k, observed)| observed.violation.map(|violation| (k as u32 + 1, violation)));
    let advice = match verdict {
        Verdict::Undecided => {
            let needed = judged.iter().filter_map(|subject| subject.observations_needed).max();
            Some(match needed {
                Some(needed) => format!("run more observations: about {needed} in all would decide it"),
                None => "run more observations".into(),
            })
        }
        Verdict::Met | Verdict::NotMet if observations.len() == 1 && !deterministic && !check.is_limit() => {
            Some("one observation of a run with randomness: so far. Run an experiment of 10 or more to judge it".into())
        }
        _ => None,
    };
    Judgement { verdict, subjects: judged, violation, advice }
}

fn error_judgement(subject: Option<u32>, why: String) -> SubjectJudgement {
    SubjectJudgement { subject, verdict: Verdict::Error, mean: None, ci95: None, extreme: None, why: Some(why), observations_needed: None }
}

fn judge_subject(check: &Check, subject: Option<u32>, values: &[Option<f64>], deterministic: bool) -> SubjectJudgement {
    let missing = values.iter().filter(|value| value.is_none()).count();
    if missing > 0 {
        let why = if missing == values.len() {
            format!("`{}` has no value: nothing it counts happened", check.figure.name())
        } else {
            format!("`{}` has no value in {missing} of {} observations", check.figure.name(), values.len())
        };
        return error_judgement(subject, why);
    }
    let values: Vec<f64> = values.iter().map(|value| value.expect("none missing")).collect();
    let (comparison, limit) = (check.judged_by(), check.limit());
    let n = values.len();
    let mean = values.iter().sum::<f64>() / n as f64;
    let judged = |verdict, ci95, extreme, observations_needed| SubjectJudgement { subject, verdict, mean: Some(mean), ci95, extreme, why: None, observations_needed };
    // A limit is broken if any observation broke it, and a state is judged by observation.
    if check.is_limit() {
        let upper = check.comparison.upper();
        let extreme = values.iter().copied().reduce(if upper { f64::max } else { f64::min }).expect("one value at least");
        let verdict = if comparison.holds(extreme, limit) { Verdict::Met } else { Verdict::NotMet };
        return judged(verdict, None, Some(extreme), None);
    }
    // A tolerance's band is met by a value inside it, and over an experiment by an interval inside
    // it: the model's mean is within it, whatever the spread.
    if let Some((low_edge, high_edge)) = check.band() {
        let inside = |value: f64| (low_edge..=high_edge).contains(&value);
        if n == 1 || deterministic {
            let verdict = if values.iter().all(|&value| inside(value)) { Verdict::Met } else { Verdict::NotMet };
            return judged(verdict, None, None, None);
        }
        let half = half_width(&values, mean);
        let (low, high) = (mean - half, mean + half);
        let verdict = if inside(low) && inside(high) {
            Verdict::Met
        } else if high < low_edge || low > high_edge {
            Verdict::NotMet
        } else {
            Verdict::Undecided
        };
        // The nearer edge decides it once the interval no longer reaches across it.
        let distance = if inside(mean) { (mean - low_edge).min(high_edge - mean) } else { (low_edge - mean).max(mean - high_edge) };
        return judged(verdict, Some([low, high]), None, observations_needed(verdict, n, half, distance));
    }
    // Equality is exact, so it is judged observation by observation, as a state is.
    if matches!(check.target, Target::State(_)) || matches!(comparison, Comparison::Equal | Comparison::NotEqual) {
        let holding = values.iter().filter(|&&value| comparison.holds(value, limit)).count();
        let verdict = if holding == n { Verdict::Met } else if holding == 0 { Verdict::NotMet } else { Verdict::Undecided };
        return judged(verdict, None, None, None);
    }
    if n == 1 || deterministic {
        let verdict = if values.iter().all(|&value| comparison.holds(value, limit)) { Verdict::Met } else { Verdict::NotMet };
        return judged(verdict, None, None, None);
    }
    let half = half_width(&values, mean);
    let (low, high) = (mean - half, mean + half);
    let verdict = match comparison {
        Comparison::AtLeast | Comparison::Above if comparison.holds(low, limit) => Verdict::Met,
        Comparison::AtLeast | Comparison::Above if !comparison.holds(high, limit) => Verdict::NotMet,
        Comparison::AtMost | Comparison::Below if comparison.holds(high, limit) => Verdict::Met,
        Comparison::AtMost | Comparison::Below if !comparison.holds(low, limit) => Verdict::NotMet,
        _ => Verdict::Undecided,
    };
    judged(verdict, Some([low, high]), None, observations_needed(verdict, n, half, (mean - limit).abs()))
}

/// Half the width of the 95% interval of the mean of two or more values.
fn half_width(values: &[f64], mean: f64) -> f64 {
    let n = values.len();
    let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    crabsim::statistics::t_quantile(n - 1) * (variance / n as f64).sqrt()
}

/// About how many observations in all would decide an undecided verdict: the interval narrows as
/// one over the root of the observations, so enough of them to bring its half-width under the
/// mean's distance from what decides it.
fn observations_needed(verdict: Verdict, n: usize, half: f64, distance: f64) -> Option<u32> {
    (verdict == Verdict::Undecided && distance > 0.0).then(|| {
        let ratio = half / distance;
        ((n as f64 * ratio * ratio).ceil().min(f64::from(u32::MAX)) as u32).max(n as u32 + 1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_and_writes_back_in_its_canonical_form() {
        for (typed, canonical) in [
            ("every Press.working_share since start >= 60%", "every Press.working_share since start >= 60%"),
            ("  mean   Press.throughput_per_hour   since  start >=4000/h", "mean Press.throughput_per_hour since start >= 4000/h"),
            ("Buffer X.mus always <= 20", "Buffer X.mus always <= 20"),
            ("Line 1.2 Mill.idle_share last 1h < 12.50%", "Line 1.2 Mill.idle_share last 1h < 12.5%"),
            ("model.mean_lead_time_s since start <= 1:30:00", "model.mean_lead_time_s since start <= 01:30:00"),
            ("Drain.lead_time_s always <= 90s", "Drain.lead_time_s always <= 90s"),
            ("Mill.state now != failed", "Mill.state now != failed"),
        ] {
            let check = Check::parse(typed).unwrap_or_else(|error| panic!("{typed}: {error}"));
            assert_eq!(check.text(), canonical);
            assert_eq!(Check::parse(canonical).unwrap(), check);
        }
        let check = Check::parse("Line 1.2 Mill.idle_share last 1h < 12.5%").unwrap();
        assert_eq!(check.subject, CheckSubject::Named("Line 1.2 Mill".into()));
        assert_eq!(check.scope, Scope::Last(SimTime::from_secs(3600)));
        assert_eq!(check.target, Target::Number { value: 0.125, written: "12.5%".into() });
    }

    #[test]
    fn a_line_that_doesnt_read_says_where() {
        for (typed, at, part) in [
            ("Press.working_share since start >= 60", 36, "percentage"),
            ("Press.busy since start >= 60%", 1, "no figure"),
            ("Press.working_share lately >= 60%", 21, "time scope"),
            ("Press.max_mus last 1h <= 3", 15, "read since start"),
            ("Press.mean_lead_time_s since start <= 90", 39, "unit"),
            ("mean Buffer.mus always <= 3", 17, "every"),
            ("Mill.state now >= busy", 16, "= or !="),
            ("model.working_share since start >= 50%", 7, "the model has no"),
            ("Mill.working_share since start >= ", 35, "ends before"),
        ] {
            let error = Check::parse(typed).expect_err(typed);
            assert_eq!(error.at, at, "{typed}: {error}");
            assert!(error.message.contains(part), "{typed}: {error}");
        }
    }

    #[test]
    fn a_tolerance_follows_an_equals_value_in_the_figures_unit_or_as_a_share_of_it() {
        for (typed, canonical, band) in [
            ("Drain.throughput_per_hour since start = 4100/d ± 3%", "Drain.throughput_per_hour since start = 4100/d ± 3%", (4100.0 / 24.0 * 0.97, 4100.0 / 24.0 * 1.03)),
            ("Drain.throughput_per_hour since start = 170/h +- 10/h", "Drain.throughput_per_hour since start = 170/h ± 10/h", (160.0, 180.0)),
            ("Press.working_share since start = 70% +/- 5%", "Press.working_share since start = 70% ± 5%", (0.65, 0.75)),
            ("model.mean_lead_time_s last 8h = 5min±30s", "model.mean_lead_time_s last 8h = 5min ± 30s", (270.0, 330.0)),
            ("Drain.mus_in since start = 2000 ± 2.5%", "Drain.mus_in since start = 2000 ± 2.5%", (1950.0, 2050.0)),
            ("Buffer.mean_mus since start = 4 ± 0.5", "Buffer.mean_mus since start = 4 ± 0.5", (3.5, 4.5)),
        ] {
            let check = Check::parse(typed).unwrap_or_else(|error| panic!("{typed}: {error}"));
            assert_eq!(check.text(), canonical);
            assert_eq!(Check::parse(canonical).unwrap(), check);
            let (low, high) = check.band().unwrap();
            assert!((low - band.0).abs() < 1e-9 && (high - band.1).abs() < 1e-9, "{typed}: {low} to {high}");
        }
        assert_eq!(Check::parse("Drain.mus_in since start = 2000").unwrap().band(), None);
        for (typed, at, part) in [
            ("Drain.mus_in since start >= 2000 ± 3%", 34, "goes with `=`"),
            ("Drain.mus_in since start != 2000 ± 3%", 34, "goes with `=`"),
            ("Mill.state now = busy ± 1", 23, "no tolerance"),
            ("Drain.mus_in since start = 2000 ± ", 35, "before its tolerance"),
            ("Drain.mus_in since start = 2000 ± 0", 35, "more than 0"),
            ("Drain.throughput_per_hour since start = 170/h ± 10", 49, "percentage of the value"),
            ("Press.working_share since start = 70% ± 5", 41, "percentage points"),
        ] {
            let error = Check::parse(typed).expect_err(typed);
            assert_eq!(error.at, at, "{typed}: {error}");
            assert!(error.message.contains(part), "{typed}: {error}");
        }
    }

    #[test]
    fn a_reference_figure_is_met_once_its_interval_lies_within_its_tolerance() {
        let day = SimTime::from_secs(86_400);
        let reference = bound("Drain.throughput_per_hour since start = 100/h ± 5%");
        // One observation, or a deterministic run, decides by its value.
        assert_eq!(judge(&reference, &observed(&[104.0]), true, day).verdict, Verdict::Met);
        assert_eq!(judge(&reference, &observed(&[106.0]), true, day).verdict, Verdict::NotMet);
        // The mean inside, its interval too.
        assert_eq!(judge(&reference, &observed(&[99.0, 101.0, 100.5, 99.5, 100.0]), false, day).verdict, Verdict::Met);
        // Wholly outside.
        assert_eq!(judge(&reference, &observed(&[110.0, 112.0, 111.0, 109.0, 110.5]), false, day).verdict, Verdict::NotMet);
        assert_eq!(judge(&reference, &observed(&[88.0, 90.0, 89.0, 91.0, 90.5]), false, day).verdict, Verdict::NotMet);
        // The mean inside but the interval reaching past an edge: more observations would tell.
        let straddling = judge(&reference, &observed(&[96.0, 106.0, 101.0, 98.0, 104.0]), false, day);
        assert_eq!(straddling.verdict, Verdict::Undecided);
        assert!(straddling.subjects[0].observations_needed.unwrap() > 5);
        assert!(straddling.advice.unwrap().contains("more observations"));
        // Equal without a tolerance is still judged observation by observation.
        let exact = bound("Drain.mus_in since start = 3");
        assert_eq!(judge(&exact, &observed(&[3.0, 3.0, 4.0]), false, day).verdict, Verdict::Undecided);
    }

    fn bound(text: &str) -> Bound {
        Bound { index: 0, check: Check::parse(text).unwrap(), subjects: vec![0] }
    }

    fn observed(values: &[f64]) -> Vec<Observed> {
        values.iter().map(|&value| Observed { values: vec![(Some(0), Some(value))], violation: None }).collect()
    }

    #[test]
    fn an_experiment_is_undecided_while_its_interval_straddles_the_limit() {
        let day = SimTime::from_secs(86_400);
        let expectation = bound("Mill.working_share since start >= 60%");
        let straddling = judge(&expectation, &observed(&[0.55, 0.62, 0.66, 0.58, 0.64]), false, day);
        assert_eq!(straddling.verdict, Verdict::Undecided);
        assert!(straddling.advice.unwrap().contains("more observations"));
        let needed = straddling.subjects[0].observations_needed.unwrap();
        assert!(needed > 5, "{needed}");
        // The same spread around a mean further above the limit decides it.
        assert_eq!(judge(&expectation, &observed(&[0.65, 0.72, 0.76, 0.68, 0.74]), false, day).verdict, Verdict::Met);
        assert_eq!(judge(&expectation, &observed(&[0.45, 0.52, 0.56, 0.48, 0.54]), false, day).verdict, Verdict::NotMet);
        // One observation decides a deterministic run, and says "so far" for a random one.
        assert_eq!(judge(&expectation, &observed(&[0.61]), true, day), Judgement { advice: None, ..judge(&expectation, &observed(&[0.61]), true, day) });
        assert!(judge(&expectation, &observed(&[0.61]), false, day).advice.unwrap().contains("so far"));
    }

    #[test]
    fn a_limit_is_broken_if_any_observation_broke_it() {
        let day = SimTime::from_secs(86_400);
        let limit = bound("Buffer.mus always <= 20");
        assert_eq!(judge(&limit, &observed(&[12.0, 20.0, 15.0]), false, day).verdict, Verdict::Met);
        let broken = judge(&limit, &observed(&[12.0, 21.0, 15.0]), false, day);
        assert_eq!(broken.verdict, Verdict::NotMet);
        assert_eq!(broken.subjects[0].extreme, Some(21.0));
    }

    #[test]
    fn a_missing_value_or_too_long_a_stretch_is_an_error() {
        let hour = SimTime::from_secs(3600);
        let lead = bound("Drain.mean_lead_time_s since start <= 2h");
        let none = vec![Observed { values: vec![(Some(0), None)], violation: None }];
        assert_eq!(judge(&lead, &none, false, hour).verdict, Verdict::Error);
        let stretch = bound("Drain.mus_in last 2h >= 3");
        assert_eq!(judge(&stretch, &observed(&[4.0]), false, hour).verdict, Verdict::Error);
    }
}
