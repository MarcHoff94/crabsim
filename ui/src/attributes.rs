//! The inspector's fields for an instance's attributes: a model file's value as text to edit, and
//! the text back as the value a model file writes, checked as far as the field can say. Whatever
//! the field lets through, the model file's own rules check when `edit_model` saves it. Plain Rust
//! with no egui types, so it is tested without a window. See spec issue 63.

use crabsim::time::parse_time;
use serde_json::{json, Map, Value};

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
    /// Failures without a value start from a typical profile, to be applied only if wanted.
    pub fn new(attribute: &str, value: Option<&Value>) -> Form {
        let (parts, repair) = match attribute {
            "failures" => {
                let failures = value.cloned().unwrap_or_else(|| json!({ "availability": 0.95, "repair_time": { "erlang": { "mean": "00:01:00", "k": 2 } } }));
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
        Form { attribute: attribute.to_string(), parts, repair }
    }

    /// Chooses how the repair time is spread, keeping what was typed.
    pub fn set_repair(&mut self, kind: RepairKind) {
        self.repair = Some(kind);
        let [first, second] = kind.labels();
        self.parts[1].label = first;
        self.parts[2].label = second;
    }

    /// The value as a model file writes it, or what is wrong with what was typed.
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
                if percent <= 0.0 || percent >= 100.0 {
                    return Err("available % has to be above 0 and below 100: restore the attribute for one that never fails".into());
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
        *self == Form::new(&self.attribute, value)
    }
}
