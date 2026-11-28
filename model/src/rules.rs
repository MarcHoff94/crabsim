//! Rules in a model file (ADR 0030): an object's distribution as when/then lines, read in order,
//! the first that holds deciding where the MU goes, its `default` strategy deciding when none
//! does or a line says so:
//!
//! ```json
//! "distribution": { "rules": [
//!     "when class = Frame then smallest working_share last 30min of Line A, Line B among those free",
//!     "when time of day >= 22:00 or time of day < 06:00 then to Night shift",
//!     "otherwise default"
//! ], "default": "round_robin" }
//! ```
//!
//! A line is `when <conditions> then <action>`, or `otherwise <action>` for one that always holds.
//! Conditions join with `and`, which binds first, and `or`. A condition is `class = <MU class>`
//! (or `!=`, a comma-separated list for any of them), `destination = <destination>` (or `!=`),
//! `time of day <comparison> <hh:mm>`, `<instance>.state now = <state>` (or `!=`), or a figure
//! `<instance>.<figure> <now|since start|last <time>>` compared with a value with its unit or
//! another figure. An action is `to <successor>`, `smallest` or `largest <figure> <scope>`, `of`
//! some successors or all, `among those free` (the default: the best free one takes it) or
//! `and wait` (only the best, waited for), `round robin` among some or all, `weighted among`
//! some (by the object's own weighted ranking), or `default`.
//!
//! This module reads and writes lines, checks them against the model and builds them into the
//! engine's `crabsim::rules`, which never sees names.

use crate::expect::{target, time_value, Comparison, Target};
use crate::{DistributionStrategy, Model, Origin, Problem, State};
use crabsim::figures::{Figure, Scope, Subject};
use crabsim::rules::{Action, Compare, Condition, Operand, Reading, RuleLine, RuleScope, Rules, StateName, TimeRules};
use crabsim::{DestinationId, MuKindId, SimTime};
use std::collections::HashMap;

/// An object's rules and the strategy they hand the choice back to.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleDistribution {
    /// Each line in its canonical form once it reads.
    pub lines: Vec<String>,
    /// Round robin, weighted or by destination: what `default` means, and what decides when no
    /// line holds.
    pub default: DistributionStrategy,
}

/// A figure of an instance, as a rule names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Named {
    pub name: String,
    pub figure: Figure,
    pub scope: Scope,
    /// How a `last` scope's length is written, such as `30min`.
    last_written: Option<String>,
}

impl Named {
    fn text(&self) -> String {
        format!("{}.{} {}", self.name, self.figure.name(), scope_text(self.scope, self.last_written.as_deref()))
    }
}

/// What a figure is compared with.
#[derive(Debug, Clone, PartialEq)]
pub enum Against {
    Value(Target),
    Figure(Named),
}

/// One condition, by name.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Class { names: Vec<String>, equal: bool },
    Destination { name: String, equal: bool },
    /// The MU's current step names one of these classes or instances (ADR 0030).
    Step { names: Vec<String>, equal: bool },
    TimeOfDay { comparison: Comparison, time: SimTime },
    Compare { left: Named, comparison: Comparison, right: Against },
}

/// One action, by name.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    To(String),
    Best { figure: Figure, scope: Scope, last_written: Option<String>, largest: bool, of: Vec<String>, wait: bool },
    RoundRobin { among: Vec<String> },
    Weighted { among: Vec<String> },
    Default,
}

/// A line, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Groups joined by `or`, conditions within one by `and`. Empty for `otherwise`.
    pub when: Vec<Vec<Cond>>,
    pub then: Act,
}

const COMPARISONS: [(&str, Comparison); 6] = [
    (">=", Comparison::AtLeast),
    ("<=", Comparison::AtMost),
    ("!=", Comparison::NotEqual),
    (">", Comparison::Above),
    ("<", Comparison::Below),
    ("=", Comparison::Equal),
];

const FORM: &str = "write a rule as `when <conditions> then <action>` or `otherwise <action>`, such as `when class = Frame then smallest working_share last 30min of Line A, Line B among those free`";

fn scope_text(scope: Scope, last: Option<&str>) -> String {
    match scope {
        Scope::Now => "now".into(),
        Scope::SinceStart => "since start".into(),
        Scope::Last(_) => format!("last {}", last.unwrap_or_default()),
        Scope::Always => "always".into(),
    }
}

fn list(text: &str) -> Vec<String> {
    text.split(',').map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect()
}

/// Splits `text` at every ` <word> ` standing alone, such as ` or `.
fn split_word<'t>(text: &'t str, word: &str) -> Vec<&'t str> {
    text.split(&format!(" {word} ")).map(str::trim).collect()
}

/// A time of day as `h:mm` or `h:mm:ss`, from 0:00 to 24:00.
fn time_of_day(typed: &str) -> Option<SimTime> {
    let parts: Vec<&str> = typed.split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let numbers: Vec<u64> = parts.iter().map(|part| part.parse().ok()).collect::<Option<_>>()?;
    let (hours, minutes, seconds) = (numbers[0], numbers[1], numbers.get(2).copied().unwrap_or(0));
    let total = (hours * 3600 + minutes * 60 + seconds) * 1000;
    (minutes < 60 && seconds < 60 && total <= 86_400_000).then(|| SimTime::from_millis(total))
}

fn time_of_day_text(time: SimTime) -> String {
    let seconds = time.as_millis() / 1000;
    if seconds % 60 == 0 {
        format!("{:02}:{:02}", seconds / 3600, seconds / 60 % 60)
    } else {
        format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    }
}

/// `<name>.<figure> <scope>` at the start of `text`, and what follows it.
fn reading(text: &str) -> Result<(Named, &str), String> {
    let text = text.trim_start();
    // The name runs to the last `.` that a figure's name follows.
    let mut found = None;
    for (dot, _) in text.match_indices('.') {
        let rest = &text[dot + 1..];
        let word = rest.split_whitespace().next().unwrap_or_default();
        if let Some(figure) = Figure::named(word) {
            found = Some((dot, figure, dot + 1 + word.len()));
        }
    }
    let Some((dot, figure, after)) = found else {
        return Err(format!("no figure after a `.` in `{}`: a figure is read as `<instance>.<figure> <now|since start|last <time>>`, such as `Line A.working_share last 30min`", text.trim()));
    };
    let name = text[..dot].trim();
    if name.is_empty() {
        return Err(format!("`.{}` names no instance before it", figure.name()));
    }
    if figure.subject() == Subject::Mu {
        return Err(format!("`{}` is an MU's figure: a rule reads the MU's class and destination as `class = …` and `destination = …`", figure.name()));
    }
    let rest = text[after..].trim_start();
    let (scope, last_written, rest) = if let Some(rest) = rest.strip_prefix("now") {
        (Scope::Now, None, rest)
    } else if let Some(rest) = rest.strip_prefix("since start") {
        (Scope::SinceStart, None, rest)
    } else if let Some(rest) = rest.strip_prefix("last ") {
        let typed = rest.split_whitespace().next().unwrap_or_default();
        let Some((length, written)) = time_value(typed) else {
            return Err(format!("`last` takes a length of time, such as `30min`, `1h` or `8:00:00`, not `{typed}`"));
        };
        if length == SimTime::ZERO {
            return Err("`last` takes a length longer than 0".into());
        }
        (Scope::Last(length), Some(written), &rest.trim_start()[typed.len()..])
    } else if rest.starts_with("always") {
        return Err(format!("a rule reads `{}` now, since start or over the last stretch: `always` is for expectations", figure.name()));
    } else {
        return Err(format!("a scope comes after `{}`: `now`, `since start` or `last <time>`", figure.name()));
    };
    if !crabsim::rules::readable(figure) {
        return Err(format!("a rule can't read `{}`: it reads figures the statistics keep as the run goes, such as working_share, mus or throughput_per_hour", figure.name()));
    }
    if !figure.has_scope(scope.kind()) {
        let scopes: Vec<&str> = figure.scopes().iter().filter(|kind| **kind != crabsim::figures::ScopeKind::Always).map(|kind| kind.written()).collect();
        return Err(format!("`{}` is read {}, not {}", figure.name(), scopes.join(" or "), scope.kind().written()));
    }
    Ok((Named { name: name.to_string(), figure, scope, last_written }, rest))
}

fn comparison(text: &str) -> Option<(Comparison, &str)> {
    let text = text.trim_start();
    COMPARISONS.iter().find(|(written, _)| text.starts_with(written)).map(|(written, comparison)| (*comparison, text[written.len()..].trim()))
}

fn written(comparison: Comparison) -> &'static str {
    COMPARISONS.iter().find(|(_, found)| *found == comparison).map(|(written, _)| *written).expect("every comparison is written")
}

fn condition(text: &str) -> Result<Cond, String> {
    let text = text.trim();
    for (word, example) in [("class", "Frame"), ("destination", "Shipping"), ("step", "Welding")] {
        if let Some(rest) = text.strip_prefix(word).filter(|rest| rest.starts_with([' ', '=', '!'])) {
            let Some((comparison, value)) = comparison(rest).filter(|(comparison, _)| matches!(comparison, Comparison::Equal | Comparison::NotEqual)) else {
                return Err(format!("`{word}` is compared with = or !=, such as `{word} = {example}`"));
            };
            let equal = comparison == Comparison::Equal;
            if value.is_empty() {
                return Err(format!("`{word} {}` names nothing", written(comparison)));
            }
            return Ok(match word {
                "class" => Cond::Class { names: list(value), equal },
                "step" => Cond::Step { names: list(value), equal },
                _ => Cond::Destination { name: value.to_string(), equal },
            });
        }
    }
    if let Some(rest) = text.strip_prefix("time of day") {
        let Some((comparison, value)) = comparison(rest) else {
            return Err("`time of day` is compared with >=, <=, >, <, = or !=, such as `time of day >= 06:00`".into());
        };
        let Some(time) = time_of_day(value) else {
            return Err(format!("a time of day is written as hours and minutes, such as `06:00` or `22:30`, not `{value}`"));
        };
        return Ok(Cond::TimeOfDay { comparison, time });
    }
    // The figure read runs to the first comparison.
    let Some(at) = text.find(['<', '>', '=', '!']) else {
        let (left, _) = reading(text)?;
        return Err(format!("a comparison comes after `{}`: >=, <=, >, <, = or !=", left.text()));
    };
    let (left, rest) = reading(&text[..at])?;
    if !rest.trim().is_empty() {
        return Err(format!("`{}` is followed by `{}` before its comparison", left.text(), rest.trim()));
    }
    let Some((comparison, right)) = comparison(&text[at..]) else {
        return Err(format!("a comparison comes after `{}`: >=, <=, >, <, = or !=", left.text()));
    };
    if right.is_empty() {
        return Err(format!("`{} {}` compares with nothing", left.text(), written(comparison)));
    }
    let unit = left.figure.unit();
    let right = if right.contains('.') && right.split('.').next_back().and_then(|word| word.split_whitespace().next()).is_some_and(|word| Figure::named(word).is_some()) {
        let (named, rest) = reading(right)?;
        if !rest.trim().is_empty() {
            return Err(format!("`{}` is followed by `{}`", named.text(), rest.trim()));
        }
        if named.figure.unit() != unit || unit == crabsim::figures::Unit::State {
            return Err(format!("`{}` and `{}` aren't in the same unit", left.figure.name(), named.figure.name()));
        }
        Against::Figure(named)
    } else {
        Against::Value(target(unit, right)?)
    };
    if unit == crabsim::figures::Unit::State && !matches!(comparison, Comparison::Equal | Comparison::NotEqual) {
        return Err("a state is compared with = or !=".into());
    }
    Ok(Cond::Compare { left, comparison, right })
}

fn action(text: &str) -> Result<Act, String> {
    let text = text.trim();
    if text == "default" {
        return Ok(Act::Default);
    }
    if let Some(name) = text.strip_prefix("to ") {
        return Ok(Act::To(name.trim().to_string()));
    }
    if let Some(rest) = text.strip_prefix("round robin") {
        let rest = rest.trim();
        if rest.is_empty() {
            return Ok(Act::RoundRobin { among: Vec::new() });
        }
        let Some(among) = rest.strip_prefix("among ") else {
            return Err(format!("`round robin` goes on with `among <successors>` or nothing, not `{rest}`"));
        };
        return Ok(Act::RoundRobin { among: list(among) });
    }
    if let Some(among) = text.strip_prefix("weighted among ") {
        return Ok(Act::Weighted { among: list(among) });
    }
    for (word, largest) in [("smallest ", false), ("largest ", true)] {
        let Some(rest) = text.strip_prefix(word) else { continue };
        // The figure is each successor's: read as if of a successor named `_`.
        let as_read = format!("_.{rest}");
        let (named, rest) = reading(&as_read)?;
        let mut rest = rest.trim();
        let wait = if let Some(before) = rest.strip_suffix("and wait") {
            rest = before.trim();
            true
        } else {
            if let Some(before) = rest.strip_suffix("among those free") {
                rest = before.trim();
            }
            false
        };
        let of = if rest.is_empty() {
            Vec::new()
        } else if let Some(names) = rest.strip_prefix("of ") {
            list(names)
        } else {
            return Err(format!("after the scope come `of <successors>`, then `among those free` or `and wait`, not `{rest}`"));
        };
        if named.figure == Figure::State {
            return Err("a state has no smallest or largest: choose by a figure, such as working_share or mus".into());
        }
        return Ok(Act::Best { figure: named.figure, scope: named.scope, last_written: named.last_written, largest, of, wait });
    }
    Err(format!("`{text}` isn't an action: `to <successor>`, `smallest` or `largest <figure> <scope> [of <successors>] [among those free|and wait]`, `round robin [among <successors>]`, `weighted among <successors>` or `default`"))
}

/// A line's conditions and its action's text: `when <conditions> then <action>`, or
/// `otherwise <action>` with no conditions. `form` says how to write one.
fn when_then<'t>(text: &'t str, form: &str) -> Result<(Vec<Vec<Cond>>, &'t str), String> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("otherwise ") {
        return Ok((Vec::new(), rest));
    }
    let Some(rest) = text.strip_prefix("when ") else {
        return Err(form.to_string());
    };
    let Some((conditions, then)) = rest.split_once(" then ") else {
        return Err(format!("a rule's conditions are followed by ` then <action>`: {form}"));
    };
    let when = split_word(conditions, "or").into_iter().map(|group| split_word(group, "and").into_iter().map(condition).collect::<Result<Vec<_>, _>>()).collect::<Result<Vec<_>, _>>()?;
    Ok((when, then))
}

/// A line as written from its conditions and its action's text.
fn when_then_text(when: &[Vec<Cond>], then: &str) -> String {
    if when.is_empty() {
        return format!("otherwise {then}");
    }
    let condition = |condition: &Cond| match condition {
        Cond::Class { names, equal } => format!("class {} {}", if *equal { "=" } else { "!=" }, names.join(", ")),
        Cond::Destination { name, equal } => format!("destination {} {name}", if *equal { "=" } else { "!=" }),
        Cond::Step { names, equal } => format!("step {} {}", if *equal { "=" } else { "!=" }, names.join(", ")),
        Cond::TimeOfDay { comparison, time } => format!("time of day {} {}", written(*comparison), time_of_day_text(*time)),
        Cond::Compare { left, comparison, right } => {
            let right = match right {
                Against::Value(Target::Number { written, .. }) => written.clone(),
                Against::Value(Target::State(state)) => state.name().to_string(),
                Against::Value(Target::Names(names)) => names.join(", "),
                Against::Figure(named) => named.text(),
            };
            format!("{} {} {right}", left.text(), written(*comparison))
        }
    };
    let when: Vec<String> = when.iter().map(|group| group.iter().map(condition).collect::<Vec<_>>().join(" and ")).collect();
    format!("when {} then {then}", when.join(" or "))
}

/// Renames the instance `from`, or with `class` the class or MU class `from`, in conditions,
/// saying whether any named it.
fn rename_conditions(when: &mut [Vec<Cond>], from: &str, to: &str, class: bool) -> bool {
    let mut changed = false;
    let mut rename = |name: &mut String| {
        if name == from {
            *name = to.to_string();
            changed = true;
        }
    };
    for condition in when.iter_mut().flatten() {
        match condition {
            Cond::Class { names, .. } if class => names.iter_mut().for_each(&mut rename),
            // A destination or a step names an instance or a class.
            Cond::Destination { name, .. } => rename(name),
            Cond::Step { names, .. } => names.iter_mut().for_each(&mut rename),
            Cond::Compare { left, right, .. } if !class => {
                rename(&mut left.name);
                if let Against::Figure(named) = right {
                    rename(&mut named.name);
                }
            }
            _ => {}
        }
    }
    changed
}

impl Line {
    /// Reads a line. Names aren't looked up: that's `Model::rule_problems`.
    pub fn parse(text: &str) -> Result<Line, String> {
        let (when, then) = when_then(text, FORM)?;
        Ok(Line { when, then: action(then)? })
    }

    /// The line in its canonical form.
    pub fn text(&self) -> String {
        let then = match &self.then {
            Act::To(name) => format!("to {name}"),
            Act::Best { figure, scope, last_written, largest, of, wait } => {
                let of = if of.is_empty() { String::new() } else { format!(" of {}", of.join(", ")) };
                let how = if *wait { "and wait" } else { "among those free" };
                format!("{} {} {}{of} {how}", if *largest { "largest" } else { "smallest" }, figure.name(), scope_text(*scope, last_written.as_deref()))
            }
            Act::RoundRobin { among } if among.is_empty() => "round robin".into(),
            Act::RoundRobin { among } => format!("round robin among {}", among.join(", ")),
            Act::Weighted { among } => format!("weighted among {}", among.join(", ")),
            Act::Default => "default".into(),
        };
        when_then_text(&self.when, &then)
    }

    /// The same line naming `to` where it named the instance `from`, or with `class` the class or
    /// MU class `from`, for a rename. `None` if it doesn't name it.
    pub fn renamed(&self, from: &str, to: &str, class: bool) -> Option<Line> {
        let mut line = self.clone();
        let mut changed = rename_conditions(&mut line.when, from, to, class);
        let mut rename = |name: &mut String| {
            if name == from {
                *name = to.to_string();
                changed = true;
            }
        };
        if !class {
            match &mut line.then {
                Act::To(name) => rename(name),
                Act::Best { of: names, .. } | Act::RoundRobin { among: names } | Act::Weighted { among: names } => names.iter_mut().for_each(&mut rename),
                Act::Default => {}
            }
        }
        changed.then_some(line)
    }
}

// ---------- A Station's process time by MU ----------

const TIME_FORM: &str = "write a line of a process time as `when <conditions> then <time>` or `otherwise <time>`, a time with its unit or `default` for the Station's own, such as `when class = Frame then 30s` or `when step = Deburr then 1:20`";

/// A line of a Station's process time by MU (ADR 0030), read: the conditions a distribution
/// rule has, without the next destination, and a time.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeLine {
    /// Groups joined by `or`, conditions within one by `and`. Empty for `otherwise`.
    pub when: Vec<Vec<Cond>>,
    /// The time, and how the line writes it, such as `30s`; `None` for `default`, the Station's own.
    pub then: Option<(SimTime, String)>,
}

impl TimeLine {
    /// Reads a line. Names aren't looked up: that's `Model::rule_problems`.
    pub fn parse(text: &str) -> Result<TimeLine, String> {
        let (when, then) = when_then(text, TIME_FORM)?;
        let then = then.trim();
        if then == "default" {
            return Ok(TimeLine { when, then: None });
        }
        match time_value(then) {
            Some(time) => Ok(TimeLine { when, then: Some(time) }),
            None if then.parse::<f64>().is_ok() => Err(format!("a line's time is written with its unit, such as `{then}s`, `1:30` or `2min`, not `{then}`")),
            None => Err(format!("`{then}` isn't a time: {TIME_FORM}")),
        }
    }

    /// The line in its canonical form.
    pub fn text(&self) -> String {
        when_then_text(&self.when, self.then.as_ref().map_or("default", |(_, written)| written.as_str()))
    }

    /// The same line naming `to` where it named `from`, as `Line::renamed` does.
    pub fn renamed(&self, from: &str, to: &str, class: bool) -> Option<TimeLine> {
        let mut line = self.clone();
        rename_conditions(&mut line.when, from, to, class).then_some(line)
    }

    /// The time it gives, `None` for the Station's own.
    pub fn time(&self) -> Option<SimTime> {
        self.then.as_ref().map(|(time, _)| *time)
    }
}

// ---------- Against the model ----------

/// What an instance's rules are checked and built against.
struct Context<'m> {
    model: &'m Model,
    /// The instance deciding, by its place.
    index: usize,
    /// Its successors' names, in exit order.
    successors: Vec<&'m str>,
}

impl Context<'_> {
    fn instance(&self, name: &str) -> Result<usize, String> {
        let index = self.model.instances.iter().position(|instance| instance.name == name).ok_or_else(|| format!("there is no instance `{name}`"))?;
        if index == self.index {
            return Err(format!("`{name}` is the object deciding: a rule reads other objects' figures"));
        }
        Ok(index)
    }

    fn reading(&self, named: &Named) -> Result<Reading, String> {
        let index = self.instance(&named.name)?;
        let built_in = self.model.resolve(index).map_or("Station", |resolved| resolved.built_in);
        let kind = crate::expect::kind_of(built_in).expect("an instance is an object");
        if !named.figure.applies_to(kind) {
            return Err(format!("{} {built_in} has no `{}`", crate::article(built_in), named.figure.name()));
        }
        Ok(Reading { object: index as u32, figure: named.figure, scope: rule_scope(named.scope) })
    }

    fn exit(&self, name: &str) -> Result<usize, String> {
        self.successors.iter().position(|successor| *successor == name).ok_or_else(|| {
            let its = if self.successors.is_empty() { "it has none".to_string() } else { format!("they are {}", self.successors.join(", ")) };
            format!("`{name}` isn't one of the successors: {its}")
        })
    }

    fn exits(&self, names: &[String]) -> Result<Vec<usize>, String> {
        names.iter().map(|name| self.exit(name)).collect()
    }
}

fn rule_scope(scope: Scope) -> RuleScope {
    match scope {
        Scope::Now | Scope::Always => RuleScope::Now,
        Scope::SinceStart => RuleScope::SinceStart,
        Scope::Last(length) => RuleScope::Last(length),
    }
}

fn compare(comparison: Comparison) -> Compare {
    match comparison {
        Comparison::AtLeast => Compare::AtLeast,
        Comparison::AtMost => Compare::AtMost,
        Comparison::Above => Compare::Above,
        Comparison::Below => Compare::Below,
        Comparison::Equal => Compare::Equal,
        Comparison::NotEqual => Compare::NotEqual,
        Comparison::In | Comparison::NotIn => unreachable!("a rule's line never compares with `in`"),
    }
}

fn state_name(state: State) -> Option<StateName> {
    Some(match state {
        State::Busy | State::Holding => StateName::Busy,
        State::Blocked => StateName::Blocked,
        State::Idle => StateName::Idle,
        State::Receiving => StateName::Receiving,
        State::Failed | State::Stopped => StateName::Failed,
        State::Paused => StateName::Paused,
    })
}

/// What a line needs from the model as it is built: MU classes' kinds and destinations' ids.
pub(crate) struct Ids<'a> {
    pub kinds: &'a HashMap<String, Vec<MuKindId>>,
    pub destinations: &'a HashMap<&'a str, DestinationId>,
}

/// A line's conditions for the instance `context` names, by id, or why they don't fit the model.
/// With `ids`, ready to run; without, only checked. `times` for a process time's line, which
/// reads the MU's step but not its destination.
fn resolve_when(when: &[Vec<Cond>], context: &Context<'_>, ids: Option<&Ids<'_>>, times: bool) -> Result<Vec<Vec<Condition>>, String> {
    let mut resolved = Vec::new();
    for group in when {
        let mut conditions = Vec::new();
        for condition in group {
            conditions.push(match condition {
                Cond::Class { names, equal } => {
                    for name in names {
                        if !context.model.is_mu_class(name) {
                            return Err(format!("`{name}` isn't an MU class"));
                        }
                    }
                    let kinds = ids.map_or_else(Vec::new, |ids| names.iter().flat_map(|name| ids.kinds.get(name).cloned().unwrap_or_default()).collect());
                    Condition::Class { kinds, equal: *equal }
                }
                Cond::Destination { .. } if times => {
                    return Err("a process time reads where the MU is in its work plan as `step = …`, not its destination".into());
                }
                Cond::Destination { name, equal } => {
                    if context.model.destination_members(name).is_none() || context.model.is_mu_class(name) {
                        return Err(format!("`{name}` isn't a destination: an instance or a class of objects"));
                    }
                    // A destination no plan, detour or exit names is no MU's next one.
                    match ids.and_then(|ids| ids.destinations.get(name.as_str()).copied()) {
                        Some(destination) => Condition::Destination { destination, equal: *equal },
                        None => Condition::Class { kinds: Vec::new(), equal: *equal },
                    }
                }
                Cond::Step { names, equal } => {
                    for name in names {
                        context.model.step_name(name)?;
                    }
                    let destinations = ids.map_or_else(Vec::new, |ids| context.model.step_destinations(names, ids.destinations));
                    Condition::Step { destinations, equal: *equal }
                }
                Cond::TimeOfDay { comparison, time } => Condition::TimeOfDay { compare: compare(*comparison), time: *time },
                Cond::Compare { left, comparison, right: Against::Value(Target::State(state)) } => {
                    let reading = context.reading(left)?;
                    let built_in = context.model.resolve(reading.object as usize).map_or("Station", |resolved| resolved.built_in);
                    if !State::of(built_in).contains(state) {
                        return Err(format!("{} {built_in} is never {}", crate::article(built_in), state.never()));
                    }
                    Condition::State { object: reading.object, state: state_name(*state).expect("every state has a name"), equal: *comparison == Comparison::Equal }
                }
                Cond::Compare { left, comparison, right } => Condition::Compare {
                    left: context.reading(left)?,
                    compare: compare(*comparison),
                    right: match right {
                        Against::Value(Target::Number { value, .. }) => Operand::Number(*value),
                        Against::Value(Target::State(_) | Target::Names(_)) => unreachable!("a state is handled above, and a rule compares no names"),
                        Against::Figure(named) => Operand::Figure(context.reading(named)?),
                    },
                },
            });
        }
        resolved.push(conditions);
    }
    Ok(resolved)
}

impl Line {
    /// The line for the instance `context` names, by id, or why it doesn't fit the model. With
    /// `ids`, ready to run; without, only checked.
    fn resolve(&self, context: &Context<'_>, default: &DistributionStrategy, ids: Option<&Ids<'_>>) -> Result<RuleLine, String> {
        let when = resolve_when(&self.when, context, ids, false)?;
        let then = match &self.then {
            Act::To(name) => Action::To(context.exit(name)?),
            Act::Best { figure, scope, of, largest, wait, .. } => {
                let among = context.exits(of)?;
                let exits: Vec<usize> = if among.is_empty() { (0..context.successors.len()).collect() } else { among.clone() };
                for exit in exits {
                    let index = context.model.instances.iter().position(|instance| instance.name == context.successors[exit]).expect("a successor is an instance");
                    let built_in = context.model.resolve(index).map_or("Station", |resolved| resolved.built_in);
                    let kind = crate::expect::kind_of(built_in).expect("an instance is an object");
                    if !figure.applies_to(kind) {
                        return Err(format!("the successor `{}` is {} {built_in}, which has no `{}`", context.successors[exit], crate::article(built_in), figure.name()));
                    }
                }
                Action::Best { figure: *figure, scope: rule_scope(*scope), largest: *largest, among, wait: *wait }
            }
            Act::RoundRobin { among } => Action::RoundRobin { among: context.exits(among)? },
            Act::Weighted { among } => {
                if !matches!(default, DistributionStrategy::Weighted(_)) {
                    return Err("`weighted among` ranks by the object's own weighted strategy: make its `default` {\"weighted\": …}".into());
                }
                Action::Weighted { among: context.exits(among)? }
            }
            Act::Default => Action::Default,
        };
        Ok(RuleLine { when, then })
    }
}

impl Model {
    /// Every instance's rules that don't fit the model, pointing at the line where it is set: on
    /// the instance, or in the class it inherits it from, naming the instance.
    pub(crate) fn rule_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for index in 0..self.instances.len() {
            let Some(resolved) = self.resolve(index) else { continue };
            let Some(DistributionStrategy::Rules(rules)) = &resolved.attributes.distribution else { continue };
            let name = &self.instances[index].name;
            let at = match resolved.origin(crate::Attribute::Distribution) {
                Origin::Class(class) => match self.classes.iter().position(|found| &found.name == class) {
                    Some(place) => format!("/classes/{place}/attributes/distribution/rules"),
                    None => continue,
                },
                _ => format!("/instances/{index}/attributes/distribution/rules"),
            };
            let context = Context { model: self, index, successors: self.successors(name) };
            for (place, text) in rules.lines.iter().enumerate() {
                // A line that doesn't read is a problem where it is read.
                let Ok(line) = Line::parse(text) else { continue };
                if let Err(message) = line.resolve(&context, &rules.default, None) {
                    let message = if at.starts_with("/classes") { format!("for its instance `{name}`: {message}") } else { message };
                    problems.push(Problem { pointer: format!("{at}/{place}"), name: Some(name.clone()), message });
                }
            }
        }
        problems
    }

    /// Every Station's process time by MU that doesn't fit the model, pointing at the line where it
    /// is set, as `rule_problems` does.
    pub(crate) fn time_rule_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for index in 0..self.instances.len() {
            let Some(resolved) = self.resolve(index) else { continue };
            let Some(lines) = &resolved.attributes.process_time_rules else { continue };
            let name = &self.instances[index].name;
            let at = match resolved.origin(crate::Attribute::ProcessTime) {
                Origin::Class(class) => match self.classes.iter().position(|found| &found.name == class) {
                    Some(place) => format!("/classes/{place}/attributes/process_time/rules"),
                    None => continue,
                },
                _ => format!("/instances/{index}/attributes/process_time/rules"),
            };
            let context = Context { model: self, index, successors: Vec::new() };
            for (place, text) in lines.iter().enumerate() {
                let Ok(line) = TimeLine::parse(text) else { continue };
                if let Err(message) = resolve_when(&line.when, &context, None, true) {
                    let message = if at.starts_with("/classes") { format!("for its instance `{name}`: {message}") } else { message };
                    problems.push(Problem { pointer: format!("{at}/{place}"), name: Some(name.clone()), message });
                }
            }
        }
        problems
    }

    /// A Station's process time by MU ready to run, for a valid model.
    pub(crate) fn time_rules_of(&self, index: usize, lines: &[String], ids: &Ids<'_>) -> TimeRules {
        let context = Context { model: self, index, successors: Vec::new() };
        let lines = lines
            .iter()
            .filter_map(|text| TimeLine::parse(text).ok())
            .filter_map(|line| Some(crabsim::rules::TimeLine { when: resolve_when(&line.when, &context, Some(ids), true).ok()?, then: line.time() }))
            .collect();
        TimeRules { lines }
    }

    /// Whether `name` is something a step can name: an instance or a class of objects.
    pub(crate) fn step_name(&self, name: &str) -> Result<(), String> {
        if self.destination_members(name).is_none() || self.is_mu_class(name) {
            return Err(format!("`{name}` isn't a class of objects or an instance, which a step names"));
        }
        Ok(())
    }

    /// Every destination a step can be on that one of `names` stands for: the destination named
    /// so, and for a class, every class derived from it and every instance of it or of them. A
    /// listed instance stands for itself alone (ADR 0026).
    pub(crate) fn step_destinations(&self, names: &[String], destinations: &HashMap<&str, DestinationId>) -> Vec<DestinationId> {
        let mut found: Vec<DestinationId> = destinations.iter().filter(|(destination, _)| names.iter().any(|name| self.step_covers(name, destination))).map(|(_, id)| *id).collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Whether a step naming `destination` is one `name` stands for.
    pub fn step_covers(&self, name: &str, destination: &str) -> bool {
        if name == destination {
            return true;
        }
        if self.instances.iter().any(|instance| instance.name == name) {
            return false;
        }
        match self.instances.iter().find(|instance| instance.name == destination) {
            Some(instance) => instance.class == name || self.is_of_class(&instance.class, name),
            None => self.is_of_class(destination, name),
        }
    }

    /// An instance's rules ready to run, for a valid model.
    pub(crate) fn rules_of(&self, index: usize, rules: &RuleDistribution, ids: &Ids<'_>) -> Rules {
        let name = &self.instances[index].name;
        let context = Context { model: self, index, successors: self.successors(name) };
        let lines = rules.lines.iter().filter_map(|text| Line::parse(text).ok()).filter_map(|line| line.resolve(&context, &rules.default, Some(ids)).ok()).collect();
        Rules { lines }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_reads_and_writes_back_in_its_canonical_form() {
        for (typed, canonical) in [
            (
                "when class = Frame then smallest working_share last 30min of Line A, Line B",
                "when class = Frame then smallest working_share last 30min of Line A, Line B among those free",
            ),
            ("when time of day >= 22:00 or time of day < 6:00 then to Night shift", "when time of day >= 22:00 or time of day < 06:00 then to Night shift"),
            ("when Buffer 1.mus now >= 10 and destination != Rework then largest idle_share since start and wait", "when Buffer 1.mus now >= 10 and destination != Rework then largest idle_share since start and wait"),
            ("when Line A.working_share last 1h < Line B.working_share last 1h then to Line A", "when Line A.working_share last 1h < Line B.working_share last 1h then to Line A"),
            ("when Mill.state now != failed then round robin among Mill, Saw", "when Mill.state now != failed then round robin among Mill, Saw"),
            ("when class != Frame, Panel then weighted among A, B", "when class != Frame, Panel then weighted among A, B"),
            ("otherwise default", "otherwise default"),
            ("otherwise round robin", "otherwise round robin"),
        ] {
            let line = Line::parse(typed).unwrap_or_else(|error| panic!("{typed}: {error}"));
            assert_eq!(line.text(), canonical);
            assert_eq!(Line::parse(canonical).unwrap(), line);
        }
        let line = Line::parse("when class = Frame and time of day < 12:00 or destination = Paint then to A").unwrap();
        assert_eq!(line.when.len(), 2, "and binds first");
        assert_eq!(line.when[0].len(), 2);
    }

    #[test]
    fn a_line_that_doesnt_read_says_why() {
        for (typed, part) in [
            ("class = Frame then to A", "write a rule as"),
            ("when class = Frame to A", "followed by ` then"),
            ("when class > Frame then to A", "= or !="),
            ("when time of day >= 25:00 then to A", "hours and minutes"),
            ("when Mill.working_share always >= 50% then to A", "`always` is for expectations"),
            ("when Mill.working_share since start >= 50 then to A", "percentage"),
            ("when Mill.median_mus since start >= 3 then to A", "can't read"),
            ("when Mill.working_share since start >= Mill.mus now then to A", "same unit"),
            ("otherwise smallest state now", "no smallest"),
            ("otherwise to", "isn't an action"),
            ("otherwise shortest mus now", "isn't an action"),
        ] {
            let error = Line::parse(typed).expect_err(typed);
            assert!(error.contains(part), "{typed}: {error}");
        }
    }

    #[test]
    fn a_rename_reaches_every_name_of_its_kind() {
        let line = Line::parse("when class = Frame and Line A.working_share last 30min < 80% and destination = Line A then smallest mus now of Line A, Line B among those free").unwrap();
        let renamed = line.renamed("Line A", "Line 1", false).unwrap().text();
        assert_eq!(renamed, "when class = Frame and Line 1.working_share last 30min < 80% and destination = Line 1 then smallest mus now of Line 1, Line B among those free");
        assert_eq!(line.renamed("Frame", "Big frame", true).unwrap().text().split(" and ").next().unwrap(), "when class = Big frame");
        assert!(line.renamed("Saw", "Saw 2", false).is_none());
    }
}
