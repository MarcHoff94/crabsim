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
}

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
        let (parts, repair) = match attribute {
            "failures" => {
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
            _ => (vec![part("", text_of(value))], None),
        };
        Form { attribute: attribute.to_string(), parts, repair, distribution: None }
    }

    /// The fields for `attribute`, whatever it is: state notes need the `states` the object can be
    /// in, and a distribution the model's `drains`.
    pub fn for_attribute(attribute: &str, value: Option<&Value>, states: &[State], drains: &[String]) -> Form {
        match attribute {
            "state_notes" => Form::state_notes(states, value),
            "distribution" => Form::distribution(value, drains),
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
        Form { attribute: "distribution".into(), parts: Vec::new(), repair: None, distribution: Some(choice) }
    }

    /// The fields for state notes: one per state the object can be in, labelled with the state and
    /// filled from `value`, the notes as a model file writes them.
    pub fn state_notes(states: &[State], value: Option<&Value>) -> Form {
        let parts = states
            .iter()
            .map(|state| part(state.label(), value.and_then(|notes| notes.get(state.name())).and_then(Value::as_str).unwrap_or_default()))
            .collect();
        Form { attribute: "state_notes".into(), parts, repair: None, distribution: None }
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
            "interval" | "process_time" => time(0),
            "starting_mus" | "capacity" => whole(0).map(|n| json!(n)),
            "failures" => {
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
        *self == Form::new(&self.attribute, value)
    }
}
