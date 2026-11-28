//! The inspector's fields for an instance's attributes: a model file's value as text to edit, and
//! the text back as the value a model file writes, checked as far as the field can say. Whatever
//! the field lets through, the model file's own rules check when `edit_model` saves it. Plain Rust
//! with no egui types, so it is tested without a window. See spec issue 63.

use crabsim::time::parse_time;
use crabsim::Side;
use crabsim_model::{Bend, Segment, State};
use serde_json::{json, Map, Value};

/// A model file's value as one short line, such as "2 × 1.5 × 1.5 m" or "95 % available, repair
/// 00:01:00", for showing a class's values without fields.
pub fn summary(value: &Value) -> String {
    let number = |value: &Value| value.as_f64().map_or_else(|| value.to_string(), |n| format!("{n}"));
    let text = |value: &Value| value.as_str().map_or_else(|| value.to_string(), str::to_string);
    if value == "round_robin" {
        return "round robin".to_string();
    }
    if let Some(by_destination) = value.get("by_destination") {
        let then = by_destination.get("then").map_or_else(|| "round robin".to_string(), summary);
        return format!("by destination, then {then}");
    }
    if let Some(lines) = value.get("rules").and_then(Value::as_array) {
        let then = value.get("default").map_or_else(|| "round robin".to_string(), |default| if default.is_string() { format!("its own {}", text(default)) } else { summary(default) });
        let first = lines.first().map(&text).unwrap_or_default();
        let more = if lines.len() > 1 { format!(" and {} more", lines.len() - 1) } else { String::new() };
        return format!("rules: {first}{more}; else {then}");
    }
    if let Some(weighted) = value.get("weighted") {
        let by = WEIGHABLE.iter().find(|(name, _)| weighted["by"] == *name).map_or("?", |(_, label)| label).to_lowercase();
        let order = if weighted["order"] == "largest" { "largest" } else { "smallest" };
        let what = if weighted["rank"] == "successor" { "successor" } else { "route" };
        let drains = weighted["drains"].as_array().map_or(String::new(), |names| {
            format!(", to {}", names.iter().map(&text).collect::<Vec<_>>().join(", "))
        });
        return format!("weighted: {what} by {by}, {order} first{drains}");
    }
    if let Some((_, label)) = EXIT_STRATEGIES.iter().chain(&STARTS).chain(&DIRECTIONS).find(|(name, _)| value == *name) {
        return label.to_lowercase();
    }
    if let Value::Bool(freezes) = value {
        return if *freezes { "freezes" } else { "runs on" }.to_string();
    }
    // A conveyor's path, in its short form, such as "4, arc 1.5 90".
    if let Some(path) = crabsim_model::path::from_json(value).filter(|path| !path.is_empty()) {
        return crabsim_model::path::short_form(&path);
    }
    // A work plan, each step with its time, such as "Saw 0:40, Mill, Out".
    if let Value::Array(steps) = value
        && !steps.is_empty()
        && steps.iter().all(|step| step.get("at").is_some())
    {
        return work_plan_summary(steps);
    }
    // Failure profiles, each by name.
    if let Value::Array(profiles) = value
        && profiles.iter().all(|profile| profile.get("availability").is_some())
    {
        if profiles.is_empty() {
            return "none".to_string();
        }
        return profiles
            .iter()
            .map(|profile| {
                let line = summary(&json!({ "availability": profile["availability"], "repair_time": profile["repair_time"] }));
                match profile["name"].as_str() {
                    Some(name) => format!("{name}: {line}"),
                    None => line,
                }
            })
            .collect::<Vec<_>>()
            .join("; ");
    }
    let Value::Object(fields) = value else {
        return text(value);
    };
    if let (Some(length), Some(width), Some(height)) = (fields.get("length"), fields.get("width"), fields.get("height")) {
        let mut line = format!("{} × {} × {} m", number(length), number(width), number(height));
        if let Some(turned) = fields.get("rotation_degrees").and_then(Value::as_f64).filter(|&turned| turned != 0.0) {
            line += &format!(", turned {turned}°");
        }
        return line;
    }
    if let (Some(availability), Some(repair)) = (fields.get("availability").and_then(Value::as_f64), fields.get("repair_time")) {
        let repair = if let Some(time) = repair.get("constant") {
            format!("repair {}", text(time))
        } else if let Some(spread) = repair.get("uniform") {
            format!("repair {} to {}", text(&spread["min"]), text(&spread["max"]))
        } else if let Some(spread) = repair.get("normal") {
            format!("repair mean {} (normal, sd {})", text(&spread["mean"]), text(&spread["std"]))
        } else if let Some(spread) = repair.get("erlang") {
            format!("repair mean {} (Erlang, k {})", text(&spread["mean"]), number(&spread["k"]))
        } else {
            repair.to_string()
        };
        return format!("{} % available, {repair}", (availability * 1000.0).round() / 10.0);
    }
    // A Source's own MU values.
    if !fields.is_empty() && fields.keys().all(|key| ["length", "width", "booking_point", "work_plan"].contains(&key.as_str())) {
        let mut parts = Vec::new();
        for (key, label, unit) in [("length", "length", " m"), ("width", "width", " m"), ("booking_point", "booking point", "")] {
            if let Some(value) = fields.get(key) {
                parts.push(format!("{label} {}{unit}", number(value)));
            }
        }
        if let Some(steps) = fields.get("work_plan").and_then(Value::as_array) {
            parts.push(if steps.is_empty() { "no work plan".to_string() } else { format!("work plan {}", work_plan_summary(steps)) });
        }
        return parts.join(", ");
    }
    // A cross conveyor's exits, by side, such as "left: Mill; right: Rework, Out".
    if fields.keys().all(|key| Side::from_name(key).is_some()) {
        if fields.is_empty() {
            return "none".to_string();
        }
        return Side::ALL
            .into_iter()
            .filter_map(|side| Some(format!("{}: {}", side.name(), fields.get(side.name())?.as_array()?.iter().map(&text).collect::<Vec<_>>().join(", "))))
            .collect::<Vec<_>>()
            .join("; ");
    }
    // State notes, by state.
    format!("notes on {}", fields.keys().cloned().collect::<Vec<_>>().join(", "))
}

/// A time as short as it reads back the same, so a step's time fits its narrow field: under an
/// hour as minutes and seconds, such as "0:20" for a file's "00:00:20". Anything else, or a
/// fraction of a second, stays as written.
fn short_time(text: &str) -> String {
    match parse_time(text) {
        Some(time) if time.as_millis() % 1_000 == 0 && time.as_secs() < 3_600 => format!("{}:{:02}", time.as_secs() / 60, time.as_secs() % 60),
        _ => text.to_string(),
    }
}

/// A work plan's steps on one line, each with its time where it has one.
fn work_plan_summary(steps: &[Value]) -> String {
    steps
        .iter()
        .map(|step| {
            let at = step["at"].as_str().unwrap_or("?");
            match step.get("process_time").and_then(Value::as_str) {
                Some(time) => format!("{at} {time}"),
                None => at.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// How a repair time is spread, as a model file names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairKind {
    Constant,
    Uniform,
    Normal,
    Erlang,
}

impl RepairKind {
    pub const ALL: [RepairKind; 4] = [RepairKind::Constant, RepairKind::Uniform, RepairKind::Normal, RepairKind::Erlang];

    /// Its name in a model file.
    pub fn name(self) -> &'static str {
        match self {
            RepairKind::Constant => "constant",
            RepairKind::Uniform => "uniform",
            RepairKind::Normal => "normal",
            RepairKind::Erlang => "erlang",
        }
    }

    /// The labels of its fields after the availability.
    fn labels(self) -> [&'static str; 2] {
        match self {
            RepairKind::Constant => ["repair time", ""],
            RepairKind::Uniform => ["repair min", "repair max"],
            RepairKind::Normal => ["repair mean", "repair std"],
            RepairKind::Erlang => ["repair mean", "k"],
        }
    }
}

/// One text field of a form.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub label: &'static str,
    pub text: String,
}

/// The fields for one attribute, as the inspector shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub attribute: String,
    pub parts: Vec<Part>,
    /// For failures: how the repair time is spread.
    pub repair: Option<RepairKind>,
    /// For a distribution strategy: its choices, which aren't text.
    pub distribution: Option<DistributionChoice>,
    /// For a value that is one of a few: each as a model file writes it, with its label. The
    /// one chosen is the first part's text.
    pub options: &'static [(&'static str, &'static str)],
    /// For failures: each named profile, in the object's order.
    pub profiles: Option<Vec<ProfileForm>>,
    /// For a value that is one of the model's names, such as a Source's MU class: each name. The
    /// one chosen is the first part's text.
    pub choices: Vec<String>,
    /// For a conveyor's path: its segments as rows of fields.
    pub path: Option<PathRows>,
    /// For a work plan, or a Source's own MU values: its steps as rows of fields.
    pub plan: Option<PlanRows>,
    /// For a cross conveyor's exits: each side's destinations.
    pub exits: Option<ExitLists>,
    /// For a Station's process time by MU (ADR 0030): its when/then lines, one a line, the first
    /// part being its own time. `None` for one time for every MU.
    pub time_rules: Option<String>,
}

/// The names of the open model that forms offer to choose from.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Names {
    /// Its Drains, which a weighted distribution counts.
    pub drains: Vec<String>,
    /// Its MU classes, which a Source makes.
    pub mu_classes: Vec<String>,
    /// Every name a destination can be (ADR 0025): its object classes, then its instances.
    pub destinations: Vec<String>,
}

/// One step of a work plan: where, and its process time as typed, empty for the Station's own.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanRow {
    pub at: String,
    pub time: String,
}

/// A work plan as rows of fields, one per step (spec issue 148).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanRows {
    pub rows: Vec<PlanRow>,
}

impl PlanRows {
    /// The rows of a work plan as a model file writes it: none without one.
    pub fn from_value(value: Option<&Value>) -> PlanRows {
        let rows = value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|step| PlanRow { at: text_of(step.get("at")), time: short_time(&text_of(step.get("process_time"))) })
            .collect();
        PlanRows { rows }
    }

    /// The steps as a model file writes them, or what is wrong with the first that can't be read.
    /// Whether each place can be a destination is the model file's check.
    pub fn value(&self) -> Result<Value, String> {
        let mut steps = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            let at = row.at.trim();
            if at.is_empty() {
                return Err(format!("step {} needs a place: choose one", index + 1));
            }
            let mut step = Map::new();
            step.insert("at".into(), json!(at));
            let time = row.time.trim();
            if !time.is_empty() {
                if parse_time(time).is_none() {
                    return Err(format!("step {}: `{time}` isn't a time: type one such as 45 or 1:30, or leave it empty for the Station's own", index + 1));
                }
                step.insert("process_time".into(), json!(time));
            }
            steps.push(Value::Object(step));
        }
        Ok(Value::Array(steps))
    }

    /// Adds a step at `at`, with the Station's own time, after the last.
    pub fn add(&mut self, at: &str) {
        self.rows.push(PlanRow { at: at.to_string(), time: String::new() });
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
    }

    /// Swaps the step at `index` with the one after it.
    pub fn swap_with_next(&mut self, index: usize) {
        if index + 1 < self.rows.len() {
            self.rows.swap(index, index + 1);
        }
    }
}

/// A cross conveyor's exits (spec issue 148): for each side, the destinations it lists, in order,
/// and a name being typed to add to it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExitLists {
    /// In `Side::ALL`'s order.
    pub sides: Vec<(Side, Vec<String>)>,
    /// For each side, in the same order, what has been typed to add.
    pub typed: Vec<String>,
}

impl ExitLists {
    /// The lists for `value`, each side's as a model file writes them.
    pub fn from_value(value: Option<&Value>) -> ExitLists {
        let sides: Vec<(Side, Vec<String>)> = Side::ALL
            .into_iter()
            .map(|side| {
                let listed = value
                    .and_then(|exits| exits.get(side.name()))
                    .and_then(Value::as_array)
                    .map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                (side, listed)
            })
            .collect();
        ExitLists { typed: vec![String::new(); sides.len()], sides }
    }

    /// Each side listing anything, with its names in order.
    pub fn value(&self) -> Value {
        let mut exits = Map::new();
        for (side, names) in &self.sides {
            if !names.is_empty() {
                exits.insert(side.name().into(), json!(names));
            }
        }
        Value::Object(exits)
    }

    /// Adds what was typed for the side at `place` to its list, unless it is empty or already
    /// there. Returns whether the list changed.
    pub fn add_typed(&mut self, place: usize) -> bool {
        let name = std::mem::take(&mut self.typed[place]).trim().to_string();
        let names = &mut self.sides[place].1;
        if name.is_empty() || names.contains(&name) {
            return false;
        }
        names.push(name);
        true
    }

    /// Takes the name at `index` off the side at `place`.
    pub fn remove(&mut self, place: usize, index: usize) {
        if index < self.sides[place].1.len() {
            self.sides[place].1.remove(index);
        }
    }

    /// What a side lists, such as "Mill, Rework", or "none".
    pub fn listed(&self, side: Side) -> String {
        let names: Vec<&str> = self.sides.iter().filter(|(listed, _)| *listed == side).flat_map(|(_, names)| names.iter().map(String::as_str)).collect();
        if names.is_empty() { "none".to_string() } else { names.join(", ") }
    }
}

/// The names that complete what was typed, at most `limit`, for a field that takes one of the
/// model's names: those beginning with it first, then those containing it, each in the order
/// given, ignoring case. Nothing typed offers the first names.
pub fn completions<'a>(names: &'a [String], typed: &str, limit: usize) -> Vec<&'a str> {
    let typed = typed.trim().to_lowercase();
    let lower: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let starting = names.iter().zip(&lower).filter(|(_, low)| low.starts_with(&typed));
    let containing = names.iter().zip(&lower).filter(|(_, low)| !low.starts_with(&typed) && low.contains(&typed));
    starting.chain(containing).map(|(name, _)| name.as_str()).take(limit).collect()
}

/// The kinds of segment, as the path editor's menu offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Straight,
    Arc,
    Transition,
    Helix,
}

impl SegmentKind {
    pub const ALL: [SegmentKind; 4] = [SegmentKind::Straight, SegmentKind::Arc, SegmentKind::Transition, SegmentKind::Helix];

    pub fn label(self) -> &'static str {
        match self {
            SegmentKind::Straight => "Straight",
            SegmentKind::Arc => "Arc",
            SegmentKind::Transition => "Transition",
            SegmentKind::Helix => "Helix",
        }
    }

    /// Its two fields' labels, the second empty for a straight.
    pub fn fields(self) -> [&'static str; 2] {
        match self {
            SegmentKind::Straight => ["length m", ""],
            SegmentKind::Arc => ["radius m", "angle °"],
            SegmentKind::Transition => ["length m", "to radius m"],
            SegmentKind::Helix => ["radius m", "turns"],
        }
    }

    /// What it is, for hover text.
    pub fn hint(self) -> &'static str {
        match self {
            SegmentKind::Straight => "Straight on, its length in plan",
            SegmentKind::Arc => "A circular bend: its radius, and the angle it turns through, positive to the left, negative to the right",
            SegmentKind::Transition => "A bend that tightens or eases evenly, from the bend before it to the radius given, positive to the left, or to straight",
            SegmentKind::Helix => "A spiral that climbs or falls: its radius, its turns, positive to the left, and its rise, which can't be 0",
        }
    }
}

/// One segment's fields: its kind, its two values as typed and its rise.
#[derive(Debug, Clone, PartialEq)]
pub struct PathRow {
    pub kind: SegmentKind,
    pub first: String,
    pub second: String,
    pub rise: String,
}

/// A conveyor's path as rows of fields, one per segment, and the short form for typing a whole
/// path at once (spec issue 141).
#[derive(Debug, Clone, PartialEq)]
pub struct PathRows {
    pub rows: Vec<PathRow>,
    /// The path in its short form, such as "4, arc 1.5 90", which the rows follow when it is read.
    pub text: String,
    /// What is wrong with the short form last read, if anything.
    pub text_problem: Option<String>,
}

/// What a path's worked-out geometry says, for the editor to show beneath its rows.
#[derive(Debug, Clone, PartialEq)]
pub struct PathFigures {
    pub length: f64,
    pub plan_length: f64,
    /// Where its last point is from its first, in its own frame: ahead along its rotation, to the
    /// left of it, and up, in metres, and how far it has turned, in degrees.
    pub ahead: f64,
    pub left: f64,
    pub up: f64,
    pub turned: f64,
    pub tightest_radius: Option<f64>,
    pub steepest_incline: f64,
}

impl PathFigures {
    /// In words, such as "9.14 m long (8.71 m in plan). Ends 6 m ahead, 2 m to the left, 1 m up,
    /// turned 90°. Tightest bend 2 m, steepest 5.7°."
    pub fn describe(&self) -> String {
        let mut line = format!("{} m long ({} m in plan). Ends {} m ahead", number2(self.length), number2(self.plan_length), number2(self.ahead));
        if number2(self.left) != "0" {
            line += &format!(", {} m {}", number2(self.left.abs()), if self.left > 0.0 { "to the left" } else { "to the right" });
        }
        if number2(self.up) != "0" {
            line += &format!(", {} m {}", number2(self.up.abs()), if self.up > 0.0 { "up" } else { "down" });
        }
        line += &format!(", turned {}°.", number2(self.turned));
        if let Some(radius) = self.tightest_radius {
            line += &format!(" Tightest bend {} m.", number2(radius));
        }
        if self.steepest_incline > 0.0 {
            line += &format!(" Steepest {:.1}°.", self.steepest_incline);
        }
        line
    }
}

/// A number to the centimetre, without trailing zeros.
fn number2(x: f64) -> String {
    let rounded = (x * 100.0).round() / 100.0;
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

impl PathRows {
    /// The rows of a path as a model file writes it, or of the default 5 m straight without one.
    pub fn from_value(value: Option<&Value>) -> PathRows {
        let path = value.and_then(crabsim_model::path::from_json).unwrap_or_else(|| crabsim_model::DEFAULT_PATH.to_vec());
        let mut rows = PathRows { rows: path.iter().map(|&segment| PathRows::row_of(segment)).collect(), text: String::new(), text_problem: None };
        rows.refresh_text();
        rows
    }

    fn row_of(segment: Segment) -> PathRow {
        let rise = if segment.rise() == 0.0 { String::new() } else { number(segment.rise()) };
        let (kind, first, second) = match segment {
            Segment::Straight { length, .. } => (SegmentKind::Straight, number(length), String::new()),
            Segment::Arc { radius, angle, .. } => (SegmentKind::Arc, number(radius), number(angle)),
            Segment::Transition { length, to, .. } => {
                let to = match to {
                    Bend::Straight => "straight".to_string(),
                    Bend::Radius(radius) => number(radius),
                };
                (SegmentKind::Transition, number(length), to)
            }
            Segment::Helix { radius, turns, .. } => (SegmentKind::Helix, number(radius), number(turns)),
        };
        PathRow { kind, first, second, rise }
    }

    /// The segment the row at `index` says, or what is wrong with what was typed.
    pub fn segment(&self, index: usize) -> Result<Segment, String> {
        let row = &self.rows[index];
        let [first_label, second_label] = row.kind.fields();
        let read = |text: &str, label: &str| -> Result<f64, String> {
            text.trim().parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("segment {}: `{}` isn't a number, the {label}", index + 1, text.trim()))
        };
        let rise = if row.rise.trim().is_empty() { 0.0 } else { read(&row.rise, "rise m")? };
        let first = read(&row.first, first_label)?;
        Ok(match row.kind {
            SegmentKind::Straight => Segment::Straight { length: first, rise },
            SegmentKind::Arc => Segment::Arc { radius: first, angle: read(&row.second, second_label)?, rise },
            SegmentKind::Transition if row.second.trim().eq_ignore_ascii_case("straight") => Segment::Transition { length: first, to: Bend::Straight, rise },
            SegmentKind::Transition => Segment::Transition { length: first, to: Bend::Radius(read(&row.second, "to radius m, or straight")?), rise },
            SegmentKind::Helix => Segment::Helix { radius: first, turns: read(&row.second, second_label)?, rise },
        })
    }

    /// Every row's segment, or what is wrong with the first that can't be read.
    pub fn segments(&self) -> Result<Vec<Segment>, String> {
        if self.rows.is_empty() {
            return Err("a path needs at least one segment: Add segment".to_string());
        }
        (0..self.rows.len()).map(|index| self.segment(index)).collect()
    }

    /// Makes the row at `index` another kind, keeping its first value and its rise, with a value
    /// to start from for the second: a quarter turn, straight, one turn. A new helix rises 1 m.
    pub fn set_kind(&mut self, index: usize, kind: SegmentKind) {
        let row = &mut self.rows[index];
        if row.kind == kind {
            return;
        }
        row.kind = kind;
        row.second = match kind {
            SegmentKind::Straight => String::new(),
            SegmentKind::Arc => "90".to_string(),
            SegmentKind::Transition => "straight".to_string(),
            SegmentKind::Helix => "1".to_string(),
        };
        if kind == SegmentKind::Helix && row.rise.trim().is_empty() {
            row.rise = "1".to_string();
        }
        self.refresh_text();
    }

    /// Adds a straight of 1 m after the last row.
    pub fn add(&mut self) {
        self.rows.push(PathRow { kind: SegmentKind::Straight, first: "1".into(), second: String::new(), rise: String::new() });
        self.refresh_text();
    }

    pub fn remove(&mut self, index: usize) {
        self.rows.remove(index);
        self.refresh_text();
    }

    /// Swaps the row at `index` with the one after it.
    pub fn swap_with_next(&mut self, index: usize) {
        if index + 1 < self.rows.len() {
            self.rows.swap(index, index + 1);
            self.refresh_text();
        }
    }

    /// Writes the short form from the rows, when they can all be read.
    pub fn refresh_text(&mut self) {
        if let Ok(path) = self.segments() {
            self.text = crabsim_model::path::short_form(&path);
            self.text_problem = None;
        }
    }

    /// Reads the short form typed into the rows, or keeps the rows and says what's wrong with it.
    pub fn read_text(&mut self) {
        match crabsim_model::path::read_short_form(&self.text) {
            Ok(path) => {
                self.rows = path.iter().map(|&segment| PathRows::row_of(segment)).collect();
                self.text_problem = None;
            }
            Err(problem) => self.text_problem = Some(problem),
        }
    }

    /// What is wrong with each row, given the conveyor's width and limits: its own values, then
    /// the limits it breaks, each in words.
    pub fn problems(&self, width: f64, min_radius: Option<f64>, max_incline: f64) -> Vec<Vec<String>> {
        let mut found = vec![Vec::new(); self.rows.len()];
        let segments: Vec<Option<Segment>> = (0..self.rows.len()).map(|index| self.segment(index).ok()).collect();
        let Some(path) = segments.iter().copied().collect::<Option<Vec<Segment>>>() else { return found };
        for problem in crabsim_model::path::value_problems(&path).into_iter().chain(crabsim_model::path::limit_problems(&path, width, min_radius, max_incline)) {
            found[problem.index].push(problem.message);
        }
        found
    }

    /// The path's worked-out figures, when every row can be read and is sound.
    pub fn figures(&self) -> Option<PathFigures> {
        let path = self.segments().ok()?;
        if !path.iter().all(|&segment| crabsim_model::path::sound(segment)) {
            return None;
        }
        let origin = crabsim_model::path::Pose { x: 0.0, y: 0.0, z: 0.0, heading_degrees: 0.0 };
        let laid = crabsim_model::path::Laid { origin, width: 0.0, direction: crabsim_model::Direction::Forward };
        let geometry = crabsim_model::path::geometry(&path, laid);
        Some(PathFigures {
            length: geometry.length,
            plan_length: geometry.plan_length,
            ahead: geometry.end.x,
            left: geometry.end.y,
            up: geometry.end.z,
            turned: geometry.end.heading_degrees,
            tightest_radius: geometry.tightest_radius,
            steepest_incline: geometry.steepest_incline_degrees,
        })
    }
}

/// One failure profile's fields: its name, the stream it keeps, and its availability and repair
/// time as a form of its own (`failure`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileForm {
    pub name: String,
    /// Its stream within the object: kept when it has one, its place in the list when it came
    /// without one, and new for one added in the window.
    pub stream: Option<u64>,
    /// What counts down to its next failure, as a model file names it.
    pub time_basis: String,
    pub form: Form,
}

/// The time bases, as a model file names them, with their labels in the window.
pub const TIME_BASES: [(&str, &str); 3] = [("simulation", "Simulation time"), ("working", "Working time"), ("processing", "Processing time")];

impl Form {
    /// Adds a failure profile, 95 % available with a constant repair of a minute, named so no
    /// other has its name, with a stream after every other's.
    pub fn add_profile(&mut self) {
        let Some(profiles) = self.profiles.as_mut() else { return };
        let taken = |name: &str| profiles.iter().any(|profile| profile.name == name);
        let name = if !taken("Failures") {
            "Failures".to_string()
        } else {
            (2..).map(|n| format!("Failures {n}")).find(|name| !taken(name)).expect("some number is free")
        };
        let stream = profiles.iter().enumerate().map(|(place, profile)| profile.stream.unwrap_or(place as u64) + 1).max().unwrap_or(0);
        let form = Form::new("failure", Some(&json!({ "availability": 0.95, "repair_time": { "constant": "00:01:00" } })));
        profiles.push(ProfileForm { name, stream: Some(stream), time_basis: "simulation".into(), form });
    }

    /// Takes the failure profile at `place` out.
    pub fn remove_profile(&mut self, place: usize) {
        if let Some(profiles) = self.profiles.as_mut().filter(|profiles| place < profiles.len()) {
            profiles.remove(place);
        }
    }

    fn failures(value: Option<&Value>) -> Form {
        // An older file's single object reads as one profile named "Failures".
        let list = match value {
            Some(Value::Array(list)) => list.clone(),
            Some(Value::Object(_)) => vec![value.cloned().expect("an object")],
            _ => Vec::new(),
        };
        let profiles = list
            .iter()
            .enumerate()
            .map(|(place, profile)| ProfileForm {
                name: profile["name"].as_str().unwrap_or("Failures").to_string(),
                stream: Some(profile["stream"].as_u64().unwrap_or(place as u64)),
                time_basis: profile["time_basis"].as_str().unwrap_or("simulation").to_string(),
                form: Form::new("failure", Some(profile)),
            })
            .collect();
        Form { attribute: "failures".into(), parts: Vec::new(), repair: None, distribution: None, options: &[], profiles: Some(profiles), choices: Vec::new(), path: None, plan: None, exits: None, time_rules: None }
    }

    /// The profiles as a model file writes them, each with its stream. An empty list is none at
    /// all, even where a class gives some.
    fn failures_value(profiles: &[ProfileForm]) -> Result<Value, String> {
        let mut list = Vec::new();
        for (place, profile) in profiles.iter().enumerate() {
            let name = profile.name.trim();
            if name.is_empty() {
                return Err(format!("the {} failure profile needs a name", crate::attributes::ordinal(place + 1)));
            }
            if profiles[..place].iter().any(|earlier| earlier.name.trim() == name) {
                return Err(format!("two failure profiles are named \"{name}\": each needs its own name"));
            }
            let mut value = match profile.form.value()? {
                Value::Null => return Err(format!("\"{name}\" at 100 % never fails: remove it instead")),
                value => value,
            };
            let mut fields = Map::new();
            fields.insert("name".into(), json!(name));
            fields.insert("stream".into(), json!(profile.stream.unwrap_or(place as u64)));
            if profile.time_basis != "simulation" {
                fields.insert("time_basis".into(), json!(profile.time_basis));
            }
            for (key, field) in value.as_object_mut().into_iter().flat_map(std::mem::take) {
                fields.insert(key, field);
            }
            list.push(Value::Object(fields));
        }
        Ok(Value::Array(list))
    }
}

/// A booking point typed as a share of an MU's length from its front, or what is wrong with it.
fn booking_point(typed: &str) -> Result<f64, String> {
    typed
        .parse::<f64>()
        .ok()
        .filter(|share| (0.0..=1.0).contains(share))
        .ok_or_else(|| format!("`{typed}` isn't a booking point: type a share of the MU's length from its front, 0 at the front to 1 at the rear"))
}

/// "1st", "2nd", "3rd", "4th" and so on.
pub fn ordinal(place: usize) -> String {
    let suffix = match (place % 10, place % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{place}{suffix}")
}

/// A Buffer's exit strategies, by name in a model file, with their labels in the window.
pub const EXIT_STRATEGIES: [(&str, &str); 2] = [("first_in_first_out", "First in, first out"), ("last_in_first_out", "Last in, first out")];

/// When a Station with capacity begins work, by name in a model file, with its label in the window.
pub const STARTS: [(&str, &str); 2] = [("individual", "Each MU on entry"), ("batch", "Batch, when full")];

/// What an interruption does to a Buffer's dwelling time, as a model file writes it, with its
/// label in the window.
pub const DWELLING_IN_A_STOP: [(&str, &str); 2] = [("false", "Runs on"), ("true", "Freezes")];

/// What a Conveyor does with the MUs behind one that stops: close up and stop while the rest move
/// on, or stop as a whole.
pub const ACCUMULATION: [(&str, &str); 2] = [("true", "Accumulating"), ("false", "Stops as a whole")];

/// Which way a Conveyor's MUs travel along its path, by name in a model file, with its label in
/// the window.
pub const DIRECTIONS: [(&str, &str); 2] = [("forward", "Forward"), ("backward", "Backward")];

/// What can be weighed, by name in a model file, with its label in the window.
pub const WEIGHABLE: [(&str, &str); 4] = [("process_time", "Process time"), ("interval", "Interval"), ("starting_mus", "Starting MUs"), ("capacity", "Capacity")];

/// A distribution strategy's choices (ADR 0016).
#[derive(Debug, Clone, PartialEq)]
pub struct DistributionChoice {
    /// When/then rules (ADR 0030), one a line, read first, the rest being their default. `None`
    /// for no rules.
    pub rules: Option<String>,
    /// Only the successors that reach the MU's destination (ADR 0025), chosen among by the rest.
    pub by_destination: bool,
    /// Weighted rather than round robin.
    pub weighted: bool,
    /// One of `WEIGHABLE`'s names.
    pub by: String,
    pub largest: bool,
    /// Ranked by the successor's own weight rather than its route onward.
    pub successor: bool,
    /// The model's Drains, and any other names the value counts, each ticked if it counts. None
    /// ticked counts every Drain.
    pub drains: Vec<(String, bool)>,
}

fn part(label: &'static str, text: impl Into<String>) -> Part {
    Part { label, text: text.into() }
}

/// A number as a person types it, without a trailing `.0`.
fn number(x: f64) -> String {
    let rounded = (x * 1e6).round() / 1e6;
    format!("{rounded}")
}

fn text_of(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

impl Form {
    /// The fields for `attribute`, filled from its value as a model file writes it, if it has one.
    /// Failures without a value start at 100 % available with a constant repair time: one that
    /// never fails, until the availability is lowered.
    pub fn new(attribute: &str, value: Option<&Value>) -> Form {
        if attribute == "failures" {
            return Form::failures(value);
        }
        let (parts, repair) = match attribute {
            "failure" => {
                let failures = value.cloned().unwrap_or_else(|| json!({ "availability": 1.0, "repair_time": { "constant": "00:01:00" } }));
                let availability = failures["availability"].as_f64().map_or(String::new(), |a| number(a * 100.0));
                let (kind, first, second) = match failures["repair_time"].as_object().and_then(|repair| repair.iter().next()) {
                    Some((name, spread)) => match name.as_str() {
                        "uniform" => (RepairKind::Uniform, text_of(spread.get("min")), text_of(spread.get("max"))),
                        "normal" => (RepairKind::Normal, text_of(spread.get("mean")), text_of(spread.get("std"))),
                        "erlang" => (RepairKind::Erlang, text_of(spread.get("mean")), text_of(spread.get("k"))),
                        _ => (RepairKind::Constant, text_of(Some(spread)), String::new()),
                    },
                    None => (RepairKind::Constant, String::new(), String::new()),
                };
                let [first_label, second_label] = kind.labels();
                (vec![part("available %", availability), part(first_label, first), part(second_label, second)], Some(kind))
            }
            "dimensions" => {
                let size = value.cloned().unwrap_or(Value::Null);
                let field = |key: &str| size.get(key).and_then(Value::as_f64).map_or(String::new(), number);
                let rotation = size.get("rotation_degrees").and_then(Value::as_f64).map_or_else(|| "0".to_string(), number);
                (vec![part("length", field("length")), part("width", field("width")), part("height", field("height")), part("rotation °", rotation)], None)
            }
            "exit_strategy" => (vec![part("", value.and_then(Value::as_str).unwrap_or(EXIT_STRATEGIES[0].0))], None),
            "start" => (vec![part("", value.and_then(Value::as_str).unwrap_or(STARTS[0].0))], None),
            "stop_freezes_dwelling" => (vec![part("", if value.and_then(Value::as_bool) == Some(true) { "true" } else { "false" })], None),
            "accumulating" => (vec![part("", if value.and_then(Value::as_bool) == Some(false) { "false" } else { "true" })], None),
            // Each may be left empty, taking the MU class's, and its own work plan has rows.
            "mu_values" => {
                let field = |key: &str| value.and_then(|values| values.get(key)).and_then(Value::as_f64).map_or(String::new(), number);
                (vec![part("length m", field("length")), part("width m", field("width")), part("booking point", field("booking_point"))], None)
            }
            // Rows of steps, or ticks per side, instead of parts.
            "work_plan" | "exits" => (Vec::new(), None),
            "direction" => (vec![part("", value.and_then(Value::as_str).unwrap_or(DIRECTIONS[0].0))], None),
            "length" | "width" | "booking_point" | "speed" | "gap" | "min_radius" | "max_incline" => {
                (vec![part("", value.and_then(Value::as_f64).map_or(String::new(), number))], None)
            }
            // Rows of its segments instead of parts.
            "path" => (Vec::new(), None),
            _ => (vec![part("", text_of(value))], None),
        };
        let options: &[(&str, &str)] = match attribute {
            "exit_strategy" => &EXIT_STRATEGIES,
            "start" => &STARTS,
            "stop_freezes_dwelling" => &DWELLING_IN_A_STOP,
            "accumulating" => &ACCUMULATION,
            "direction" => &DIRECTIONS,
            _ => &[],
        };
        let path = (attribute == "path").then(|| PathRows::from_value(value));
        let plan = match attribute {
            "work_plan" => Some(PlanRows::from_value(value)),
            "mu_values" => Some(PlanRows::from_value(value.and_then(|values| values.get("work_plan")))),
            _ => None,
        };
        let exits = (attribute == "exits").then(|| ExitLists::from_value(value));
        // A process time by MU: its lines, and its own time as the field.
        let time_rules = (attribute == "process_time")
            .then(|| value.and_then(|value| value.get("rules")).and_then(Value::as_array).map(|lines| lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")))
            .flatten();
        let parts = if time_rules.is_some() { vec![part("", text_of(value.and_then(|value| value.get("default"))))] } else { parts };
        Form { attribute: attribute.to_string(), parts, repair, distribution: None, options, profiles: None, choices: Vec::new(), path, plan, exits, time_rules }
    }

    /// The fields for `attribute`, whatever it is: state notes need the `states` the object can be
    /// in, and the rest the model's `names`: a distribution its Drains, an MU class its MU classes,
    /// a detour, a work plan's steps and exits the names a destination can be, to complete what is
    /// typed.
    pub fn for_attribute(attribute: &str, value: Option<&Value>, states: &[State], names: &Names) -> Form {
        match attribute {
            "state_notes" => Form::state_notes(states, value),
            "distribution" => Form::distribution(value, &names.drains),
            "mu_class" => Form { choices: names.mu_classes.to_vec(), ..Form::new(attribute, value) },
            // Each step's place is chosen from them.
            "work_plan" | "mu_values" => Form { choices: names.destinations.to_vec(), ..Form::new(attribute, value) },
            // Typed, completed from the names a destination can be.
            "detour" | "exits" => Form { choices: names.destinations.to_vec(), ..Form::new(attribute, value) },
            attribute => Form::new(attribute, value),
        }
    }

    /// The choices of a distribution strategy, from its value as a model file writes it: round
    /// robin without one. The Drains to tick are the model's `drains`, then any other name the
    /// value counts, so saving again never drops one silently.
    pub fn distribution(value: Option<&Value>, drains: &[String]) -> Form {
        // Rules come first, and what follows is their default.
        let rules = value.and_then(|value| value.get("rules")).and_then(Value::as_array).map(|lines| lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"));
        let value = if rules.is_some() { value.and_then(|value| value.get("default")) } else { value };
        let by_destination = value.and_then(|value| value.get("by_destination"));
        // By destination, what then chooses is read as a strategy of its own.
        let value = by_destination.map_or(value, |by| by.get("then"));
        let weighted = value.and_then(|value| value.get("weighted"));
        let named: Vec<String> = weighted
            .and_then(|weighted| weighted.get("drains"))
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let mut ticks: Vec<(String, bool)> = drains.iter().map(|drain| (drain.clone(), named.contains(drain))).collect();
        ticks.extend(named.iter().filter(|name| !drains.contains(name)).map(|name| (name.clone(), true)));
        let field = |key: &str| weighted.and_then(|weighted| weighted.get(key)).and_then(Value::as_str);
        let choice = DistributionChoice {
            rules,
            by_destination: by_destination.is_some(),
            weighted: weighted.is_some(),
            by: field("by").unwrap_or("process_time").to_string(),
            largest: field("order") == Some("largest"),
            successor: field("rank") == Some("successor"),
            drains: ticks,
        };
        Form { attribute: "distribution".into(), parts: Vec::new(), repair: None, distribution: Some(choice), options: &[], profiles: None, choices: Vec::new(), path: None, plan: None, exits: None, time_rules: None }
    }

    /// The fields for state notes: one per state the object can be in, labelled with the state and
    /// filled from `value`, the notes as a model file writes them.
    pub fn state_notes(states: &[State], value: Option<&Value>) -> Form {
        let parts = states
            .iter()
            .map(|state| part(state.label(), value.and_then(|notes| notes.get(state.name())).and_then(Value::as_str).unwrap_or_default()))
            .collect();
        Form { attribute: "state_notes".into(), parts, repair: None, distribution: None, options: &[], profiles: None, choices: Vec::new(), path: None, plan: None, exits: None, time_rules: None }
    }

    /// The states a state notes form has a field for.
    fn states(&self) -> Vec<State> {
        self.parts.iter().filter_map(|part| State::ALL.into_iter().find(|state| state.label() == part.label)).collect()
    }

    /// Chooses how the repair time is spread, keeping what was typed.
    pub fn set_repair(&mut self, kind: RepairKind) {
        self.repair = Some(kind);
        let [first, second] = kind.labels();
        self.parts[1].label = first;
        self.parts[2].label = second;
    }

    /// The value as a model file writes it, or what is wrong with what was typed. `null` for
    /// failures at 100 % available: none at all.
    pub fn value(&self) -> Result<Value, String> {
        let text = |index: usize| self.parts[index].text.trim();
        let time = |index: usize| -> Result<Value, String> {
            parse_time(text(index))
                .map(|_| json!(text(index)))
                .ok_or_else(|| format!("`{}` isn't a time: type one such as 45, 0:45 or 1:30:00", text(index)))
        };
        let float = |index: usize| -> Result<f64, String> {
            text(index).parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("`{}` isn't a number", text(index)))
        };
        let whole = |index: usize| -> Result<u64, String> { text(index).parse::<u64>().map_err(|_| format!("`{}` isn't a whole number of 0 or more", text(index))) };
        match self.attribute.as_str() {
            "process_time" if self.time_rules.is_some() => {
                let lines: Vec<String> = self.time_rules.as_deref().unwrap_or_default().lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect();
                if lines.is_empty() {
                    return Err("write a line a row, such as `when class = Frame then 30s`, or untick Rules".to_string());
                }
                Ok(json!({ "rules": lines, "default": time(0)? }))
            }
            "interval" | "process_time" | "dwelling_time" | "switch_time" | "release_time" => time(0),
            "starting_mus" => whole(0).map(|n| json!(n)),
            // Automatic is no value at all, which a model file says by leaving it out.
            "histogram_bar_width" if text(0).is_empty() || text(0).eq_ignore_ascii_case("automatic") => Ok(Value::Null),
            "histogram_bar_width" => match text(0).parse::<u64>() {
                Ok(width) if width >= 1 => Ok(json!(width)),
                _ => Err(format!("`{}` isn't a bar width: type a whole number of MUs from 1, or leave it empty for automatic", text(0))),
            },
            // No limit is -1 or the word, and a model file writes the word.
            "capacity" if text(0) == "-1" || text(0).eq_ignore_ascii_case("unlimited") => Ok(json!("unlimited")),
            "capacity" => match text(0).parse::<u64>() {
                Ok(limit) if limit >= 1 => Ok(json!(limit)),
                _ => Err(format!("`{}` isn't a capacity: type a whole number from 1, or -1 for no limit", text(0))),
            },
            "failures" => Form::failures_value(self.profiles.as_deref().unwrap_or_default()),
            "failure" => {
                let percent = float(0)?;
                // 100 % available never fails, which a model file says by leaving failures out.
                if percent == 100.0 {
                    return Ok(Value::Null);
                }
                if percent <= 0.0 || percent > 100.0 {
                    return Err("available % has to be above 0, and 100 for one that never fails".into());
                }
                let spread = match self.repair.unwrap_or(RepairKind::Constant) {
                    RepairKind::Constant => time(1)?,
                    RepairKind::Uniform => json!({ "min": time(1)?, "max": time(2)? }),
                    RepairKind::Normal => json!({ "mean": time(1)?, "std": time(2)? }),
                    RepairKind::Erlang => json!({ "mean": time(1)?, "k": whole(2)? }),
                };
                let mut repair = Map::new();
                repair.insert(self.repair.unwrap_or(RepairKind::Constant).name().into(), spread);
                Ok(json!({ "availability": percent / 100.0, "repair_time": repair }))
            }
            "distribution" => {
                let by_destination = self.distribution.as_ref().is_some_and(|choice| choice.by_destination);
                let lines: Option<Vec<String>> = self.distribution.as_ref().and_then(|choice| choice.rules.as_ref()).map(|text| text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect());
                if lines.as_ref().is_some_and(Vec::is_empty) {
                    return Err("write a rule a line, such as `when class = Frame then to Line A`, or untick Rules".to_string());
                }
                let wrap = |then: Value| {
                    let then = if by_destination { json!({ "by_destination": { "then": then } }) } else { then };
                    match &lines {
                        Some(lines) if then == json!("round_robin") => json!({ "rules": lines }),
                        Some(lines) => json!({ "rules": lines, "default": then }),
                        None => then,
                    }
                };
                let Some(choice) = self.distribution.as_ref().filter(|choice| choice.weighted) else {
                    return Ok(wrap(json!("round_robin")));
                };
                let mut weighted = Map::new();
                weighted.insert("by".into(), json!(choice.by));
                weighted.insert("order".into(), json!(if choice.largest { "largest" } else { "smallest" }));
                weighted.insert("rank".into(), json!(if choice.successor { "successor" } else { "route" }));
                let counted: Vec<&String> = choice.drains.iter().filter(|(_, ticked)| *ticked).map(|(name, _)| name).collect();
                if !counted.is_empty() {
                    weighted.insert("drains".into(), json!(counted));
                }
                Ok(wrap(json!({ "weighted": weighted })))
            }
            "state_notes" => {
                let mut notes = Map::new();
                for (state, part) in self.states().into_iter().zip(&self.parts) {
                    if !part.text.trim().is_empty() {
                        notes.insert(state.name().into(), json!(part.text.trim()));
                    }
                }
                Ok(Value::Object(notes))
            }
            "dimensions" => {
                let mut size = Map::new();
                for (index, key) in ["length", "width", "height"].into_iter().enumerate() {
                    size.insert(key.into(), json!(float(index)?));
                }
                let rotation = float(3)?;
                if rotation != 0.0 {
                    size.insert("rotation_degrees".into(), json!(rotation));
                }
                Ok(Value::Object(size))
            }
            "stop_freezes_dwelling" => Ok(json!(text(0) == "true")),
            "accumulating" => Ok(json!(text(0) != "false")),
            "length" | "width" => match float(0)? {
                metres if metres > 0.0 => Ok(json!(metres)),
                _ => Err(format!("`{}` isn't a {}: type metres above 0", text(0), self.attribute)),
            },
            "booking_point" => booking_point(text(0)).map(|share| json!(share)),
            "speed" => match float(0)? {
                speed if speed > 0.0 => Ok(json!(speed)),
                _ => Err(format!("`{}` isn't a speed: type metres per second above 0", text(0))),
            },
            "gap" => match float(0)? {
                gap if gap >= 0.0 => Ok(json!(gap)),
                _ => Err(format!("`{}` isn't a gap: type metres, 0 or more", text(0))),
            },
            // Whether the segments make a sound path is the model file's rules to say.
            "path" => match &self.path {
                Some(path) => path.segments().map(|path| crabsim_model::path::to_json(&path)),
                None => Err("a path has rows of segments".to_string()),
            },
            "direction" => Ok(json!(text(0))),
            // Unset is only the geometric limit, which a model file says by leaving it out.
            "min_radius" if text(0).is_empty() => Ok(Value::Null),
            "min_radius" => match float(0)? {
                metres if metres > 0.0 => Ok(json!(metres)),
                _ => Err(format!("`{}` isn't a radius: type metres above 0, or leave it empty for no limit but half the width", text(0))),
            },
            "max_incline" => match float(0)? {
                degrees if degrees > 0.0 && degrees < 90.0 => Ok(json!(degrees)),
                _ => Err(format!("`{}` isn't an incline: type degrees above 0 and below 90", text(0))),
            },
            // None of them is no values of its own at all, which a model file says by leaving it out.
            "mu_values" => {
                let mut values = Map::new();
                for (index, key) in ["length", "width"].into_iter().enumerate() {
                    if !text(index).is_empty() {
                        match float(index)? {
                            metres if metres > 0.0 => values.insert(key.into(), json!(metres)),
                            _ => return Err(format!("`{}` isn't a {key}: type metres above 0, or leave it empty for the MU class's", text(index))),
                        };
                    }
                }
                if !text(2).is_empty() {
                    values.insert("booking_point".into(), json!(booking_point(text(2))?));
                }
                // No steps of its own takes the MU class's plan.
                if let Some(plan) = self.plan.as_ref().filter(|plan| !plan.rows.is_empty()) {
                    values.insert("work_plan".into(), plan.value()?);
                }
                Ok(if values.is_empty() { Value::Null } else { Value::Object(values) })
            }
            "work_plan" => self.plan.as_ref().map_or(Ok(json!([])), PlanRows::value),
            "exits" => Ok(self.exits.as_ref().map_or(json!({}), ExitLists::value)),
            // None is no detour, which a model file says by leaving it out.
            "detour" if text(0).is_empty() => Ok(Value::Null),
            _ => Ok(json!(text(0))),
        }
    }

    /// Whether the fields hold what `value` would give, so nothing waits to be applied.
    pub fn matches(&self, value: Option<&Value>) -> bool {
        // A path matches by its rows: the short form typed and not read yet changes nothing.
        if let Some(path) = &self.path {
            return Some(&path.rows) == Form::new(&self.attribute, value).path.as_ref().map(|path| &path.rows);
        }
        if self.attribute == "state_notes" {
            return *self == Form::state_notes(&self.states(), value);
        }
        if self.attribute == "distribution" {
            // Round robin set nowhere is round robin all the same.
            // The Drains counted are a set, whatever order the file lists them in.
            let sorted = |mut value: Value| {
                for at in ["/weighted/drains", "/by_destination/then/weighted/drains"] {
                    if let Some(Value::Array(names)) = value.pointer_mut(at) {
                        names.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
                    }
                }
                value
            };
            let given = value.cloned().unwrap_or_else(|| json!("round_robin"));
            let given = Form::distribution(Some(&given), &[]).value().unwrap_or_default();
            return self.value().ok().map(sorted) == Some(sorted(given));
        }
        if self.attribute == "exits" {
            // Each side's names are a set, and a side listing none lists nothing.
            let sorted = |value: Value| -> Value {
                let mut exits = Map::new();
                for (side, names) in value.as_object().into_iter().flatten() {
                    let mut names: Vec<&str> = names.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    names.sort_unstable();
                    if !names.is_empty() {
                        exits.insert(side.clone(), json!(names));
                    }
                }
                Value::Object(exits)
            };
            return self.value().ok().map(sorted) == Some(sorted(value.cloned().unwrap_or_else(|| json!({}))));
        }
        if self.attribute == "failures" {
            // Compared as the file would have them, each with its stream.
            let given = Form::new("failures", value);
            return self.value().ok() == given.value().ok() && self.value().is_ok();
        }
        // The names offered aren't part of the value.
        *self == Form { choices: self.choices.clone(), ..Form::new(&self.attribute, value) }
    }
}
