//! The inspector's fields for an instance's attributes: a model file's value as text to edit, and
//! the text back as the value a model file writes, checked as far as the field can say. Whatever
//! the field lets through, the model file's own rules check when `edit_model` saves it. Plain Rust
//! with no egui types, so it is tested without a window. See spec issue 63.

use crabsim::time::parse_time;
use crabsim_model::State;
use serde_json::{json, Map, Value};

/// A model file's value as one short line, such as "2 × 1.5 × 1.5 m" or "95 % available, repair
/// 00:01:00", for showing a class's values without fields.
pub fn summary(value: &Value) -> String {
    let number = |value: &Value| value.as_f64().map_or_else(|| value.to_string(), |n| format!("{n}"));
    let text = |value: &Value| value.as_str().map_or_else(|| value.to_string(), str::to_string);
    if value == "round_robin" {
        return "round robin".to_string();
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
    if let Some((_, label)) = EXIT_STRATEGIES.iter().chain(&STARTS).find(|(name, _)| value == *name) {
        return label.to_lowercase();
    }
    if let Value::Bool(freezes) = value {
        return if *freezes { "freezes" } else { "runs on" }.to_string();
    }
    // A conveyor's path, by its straights.
    if let Some(straights) = straights(value) {
        return format!("{} m straight", straights.iter().map(|&metres| self::number(metres)).collect::<Vec<_>>().join(" + "));
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
    if !fields.is_empty() && fields.keys().all(|key| ["length", "width", "booking_point"].contains(&key.as_str())) {
        let mut parts = Vec::new();
        for (key, label, unit) in [("length", "length", " m"), ("width", "width", " m"), ("booking_point", "booking point", "")] {
            if let Some(value) = fields.get(key) {
                parts.push(format!("{label} {}{unit}", number(value)));
            }
        }
        return parts.join(", ");
    }
    // State notes, by state.
    format!("notes on {}", fields.keys().cloned().collect::<Vec<_>>().join(", "))
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
        Form { attribute: "failures".into(), parts: Vec::new(), repair: None, distribution: None, options: &[], profiles: Some(profiles), choices: Vec::new() }
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

/// A path's straights in metres, if `value` is a path of straights only.
fn straights(value: &Value) -> Option<Vec<f64>> {
    let segments = value.as_array().filter(|segments| !segments.is_empty())?;
    segments.iter().map(|segment| segment.get("straight").and_then(Value::as_f64).filter(|_| segment.as_object().is_some_and(|fields| fields.len() == 1))).collect()
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

/// What can be weighed, by name in a model file, with its label in the window.
pub const WEIGHABLE: [(&str, &str); 4] = [("process_time", "Process time"), ("interval", "Interval"), ("starting_mus", "Starting MUs"), ("capacity", "Capacity")];

/// A distribution strategy's choices (ADR 0016).
#[derive(Debug, Clone, PartialEq)]
pub struct DistributionChoice {
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
            // Each may be left empty, taking the MU class's.
            "mu_values" => {
                let field = |key: &str| value.and_then(|values| values.get(key)).and_then(Value::as_f64).map_or(String::new(), number);
                (vec![part("length m", field("length")), part("width m", field("width")), part("booking point", field("booking_point"))], None)
            }
            "length" | "width" | "booking_point" | "speed" | "gap" => (vec![part("", value.and_then(Value::as_f64).map_or(String::new(), number))], None),
            // Its straights in metres, such as "4, 2.5".
            "path" => {
                let typed = value.and_then(straights).map_or(String::new(), |straights| straights.iter().map(|&metres| number(metres)).collect::<Vec<_>>().join(", "));
                (vec![part("straights m", typed)], None)
            }
            _ => (vec![part("", text_of(value))], None),
        };
        let options: &[(&str, &str)] = match attribute {
            "exit_strategy" => &EXIT_STRATEGIES,
            "start" => &STARTS,
            "stop_freezes_dwelling" => &DWELLING_IN_A_STOP,
            _ => &[],
        };
        Form { attribute: attribute.to_string(), parts, repair, distribution: None, options, profiles: None, choices: Vec::new() }
    }

    /// The fields for `attribute`, whatever it is: state notes need the `states` the object can be
    /// in, a distribution the model's `drains`, and an MU class the model's `mu_classes`.
    pub fn for_attribute(attribute: &str, value: Option<&Value>, states: &[State], drains: &[String], mu_classes: &[String]) -> Form {
        match attribute {
            "state_notes" => Form::state_notes(states, value),
            "distribution" => Form::distribution(value, drains),
            "mu_class" => Form { choices: mu_classes.to_vec(), ..Form::new(attribute, value) },
            attribute => Form::new(attribute, value),
        }
    }

    /// The choices of a distribution strategy, from its value as a model file writes it: round
    /// robin without one. The Drains to tick are the model's `drains`, then any other name the
    /// value counts, so saving again never drops one silently.
    pub fn distribution(value: Option<&Value>, drains: &[String]) -> Form {
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
            weighted: weighted.is_some(),
            by: field("by").unwrap_or("process_time").to_string(),
            largest: field("order") == Some("largest"),
            successor: field("rank") == Some("successor"),
            drains: ticks,
        };
        Form { attribute: "distribution".into(), parts: Vec::new(), repair: None, distribution: Some(choice), options: &[], profiles: None, choices: Vec::new() }
    }

    /// The fields for state notes: one per state the object can be in, labelled with the state and
    /// filled from `value`, the notes as a model file writes them.
    pub fn state_notes(states: &[State], value: Option<&Value>) -> Form {
        let parts = states
            .iter()
            .map(|state| part(state.label(), value.and_then(|notes| notes.get(state.name())).and_then(Value::as_str).unwrap_or_default()))
            .collect();
        Form { attribute: "state_notes".into(), parts, repair: None, distribution: None, options: &[], profiles: None, choices: Vec::new() }
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
            "interval" | "process_time" | "dwelling_time" => time(0),
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
                let Some(choice) = self.distribution.as_ref().filter(|choice| choice.weighted) else {
                    return Ok(json!("round_robin"));
                };
                let mut weighted = Map::new();
                weighted.insert("by".into(), json!(choice.by));
                weighted.insert("order".into(), json!(if choice.largest { "largest" } else { "smallest" }));
                weighted.insert("rank".into(), json!(if choice.successor { "successor" } else { "route" }));
                let counted: Vec<&String> = choice.drains.iter().filter(|(_, ticked)| *ticked).map(|(name, _)| name).collect();
                if !counted.is_empty() {
                    weighted.insert("drains".into(), json!(counted));
                }
                Ok(json!({ "weighted": weighted }))
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
            "path" => {
                let mut segments = Vec::new();
                for typed in text(0).split(',').map(str::trim) {
                    match typed.parse::<f64>() {
                        Ok(metres) if metres > 0.0 && metres.is_finite() => segments.push(json!({ "straight": metres })),
                        _ => return Err(format!("`{typed}` isn't a straight: type each in metres above 0, such as 10, or 4, 2.5")),
                    }
                }
                Ok(Value::Array(segments))
            }
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
                Ok(if values.is_empty() { Value::Null } else { Value::Object(values) })
            }
            _ => Ok(json!(text(0))),
        }
    }

    /// Whether the fields hold what `value` would give, so nothing waits to be applied.
    pub fn matches(&self, value: Option<&Value>) -> bool {
        if self.attribute == "state_notes" {
            return *self == Form::state_notes(&self.states(), value);
        }
        if self.attribute == "distribution" {
            // Round robin set nowhere is round robin all the same.
            // The Drains counted are a set, whatever order the file lists them in.
            let sorted = |mut value: Value| {
                if let Some(Value::Array(names)) = value.pointer_mut("/weighted/drains") {
                    names.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
                }
                value
            };
            let given = value.cloned().unwrap_or_else(|| json!("round_robin"));
            let given = Form::distribution(Some(&given), &[]).value().unwrap_or_default();
            return self.value().ok().map(sorted) == Some(sorted(given));
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
