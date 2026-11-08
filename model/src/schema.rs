//! The published JSON Schema of a model file, generated from the types below. They mirror the
//! file's shape for `schemars`, while `read.rs` reads files by hand so it can report every problem
//! at once. Tests keep the two from drifting apart: every file the reader accepts must pass the
//! schema, the attributes must match, and the committed schema file must be the one generated here.
//!
//! The schema checks a file's shape, so an editor can point out mistakes as they are typed. The
//! reader's checks go further: names that must exist, classes going round in a circle, seconds
//! below 60, a Source that would create MUs forever. The operation catalogue can reuse these types
//! for its own input schemas, so the two never disagree.

use schemars::JsonSchema;
use serde_json::Value;

/// A time, written as crabsim's UI shows it: seconds, minutes and seconds, or hours, minutes and
/// seconds, with days in front and a fraction of a second at the end. Such as "90", "1:30",
/// "8:00:00", "2d 06:00:00", "3d" or "0:00:12.5".
#[derive(JsonSchema)]
#[schemars(extend("pattern" = r"^\s*(\d+\s*d(\s*((\d+:)?\d+:)?\d+(\.\d+)?)?|((\d+:)?\d+:)?\d+(\.\d+)?)\s*$"))]
pub struct Time(pub String);

/// A name: not empty, and with no spaces at either end.
#[derive(JsonSchema)]
#[schemars(extend("pattern" = r"^\S(.*\S)?$"))]
pub struct Name(pub String);

/// A crabsim model file. It holds the model only: a run's seed, length and warm-up are not part
/// of it.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "crabsim model file")]
pub struct ModelFile {
    /// Where this schema is, for editors. crabsim ignores it, keeps it when saving, and never
    /// needs the file it names to exist.
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// The format's version. Only 1 exists.
    #[schemars(extend("const" = 1))]
    pub format_version: u64,
    /// What the model is, as a whole.
    pub description: Option<String>,
    /// The model's own classes, in any order. Each derives from a built-in class (Source,
    /// Station, Buffer, Drain) or from another class here.
    pub classes: Option<Vec<ClassFile>>,
    /// The material-flow objects, in the order a run initialises them.
    pub instances: Vec<InstanceFile>,
}

/// A class of the model's own.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClassFile {
    /// Unique among classes, and not the name of a built-in class.
    pub name: Name,
    /// The class it derives from: Source, Station, Buffer, Drain or another class in the file.
    pub parent: String,
    /// Only the values this class changes from its parent.
    pub attributes: Option<AttributesFile>,
}

/// One material-flow object.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceFile {
    /// Unique among instances.
    pub name: Name,
    /// Source, Station, Buffer, Drain or a class in the file.
    pub class: String,
    /// The centre of its footprint, in metres. Visual only.
    pub position: Option<PositionFile>,
    /// The instances it passes MUs to, by name, in the order they are tried.
    pub successors: Option<Vec<String>>,
    /// Only the values set on this instance. Anything else comes from its class.
    pub attributes: Option<AttributesFile>,
}

/// A position in metres.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PositionFile {
    pub x: f64,
    pub y: f64,
    /// Height above the floor, 0 if left out.
    pub z: Option<f64>,
}

/// The values a class or instance sets. Each applies to some built-in classes only, and anything
/// left out is inherited.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttributesFile {
    /// Source: the time between creating an MU and offering it on. Must be set somewhere in the
    /// chain of classes. 0 offers each MU at once.
    pub interval: Option<Time>,
    /// Station: the time it works on each MU. 0 by default, a dummy Station that takes no time.
    pub process_time: Option<Time>,
    /// Station: random failures. Left out, it never fails.
    pub failures: Option<FailuresFile>,
    /// Station: how many MUs it holds from the start of a run, 0 or 1.
    #[schemars(range(max = 1))]
    pub starting_mus: Option<u64>,
    /// Buffer: how many MUs it holds. Must be set somewhere in the chain of classes.
    #[schemars(range(min = 1))]
    pub capacity: Option<u64>,
    /// Any object: its size in metres and rotation in degrees. Visual only.
    pub dimensions: Option<DimensionsFile>,
    /// Any object: free text saying what it is. Visual only.
    pub description: Option<String>,
}

/// Random failures. The time between failures follows from the availability and the repair time.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FailuresFile {
    /// The share of time available, above 0 and below 1.
    #[schemars(extend("exclusiveMinimum" = 0, "exclusiveMaximum" = 1))]
    pub availability: f64,
    pub repair_time: RepairTimeFile,
}

/// How long a repair takes: exactly one of these.
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum RepairTimeFile {
    /// Always the same time, above 0.
    Constant(Time),
    /// Evenly spread between `min` and `max`.
    Uniform { min: Time, max: Time },
    /// Normally distributed with this mean and standard deviation.
    Normal { mean: Time, std: Time },
    /// Erlang distributed with this mean and shape `k`.
    Erlang {
        mean: Time,
        #[schemars(range(min = 1))]
        k: u64,
    },
}

/// An object's size and rotation.
#[derive(JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DimensionsFile {
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub length: f64,
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub width: f64,
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub height: f64,
    /// Around the vertical axis, 0 if left out.
    pub rotation_degrees: Option<f64>,
}

/// The JSON Schema of a model file.
pub fn schema() -> Value {
    let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
    let mut schema = serde_json::to_value(generator.into_root_schema_for::<ModelFile>()).expect("a schema always serialises");
    tidy(&mut schema);
    schema
}

/// `schemars` lets an `Option` field be `null`, but a file leaves the field out instead, and
/// crabsim refuses `null`. So this takes `null` out of every choice of types. It also joins the
/// lines of descriptions, which come from wrapped doc comments.
fn tidy(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            if let Some(Value::Array(types)) = fields.get_mut("type") {
                types.retain(|kind| kind != "null");
                if types.len() == 1 {
                    let only = types.remove(0);
                    fields.insert("type".into(), only);
                }
            }
            if let Some(Value::Array(choices)) = fields.get_mut("anyOf") {
                choices.retain(|choice| choice.get("type").is_none_or(|kind| kind != "null"));
                if choices.len() == 1 {
                    let Value::Object(only) = choices.remove(0) else { unreachable!("a choice is a schema") };
                    fields.remove("anyOf");
                    fields.extend(only);
                }
            }
            if let Some(Value::String(description)) = fields.get_mut("description") {
                *description = description.replace('\n', " ");
            }
            fields.values_mut().for_each(tidy);
        }
        Value::Array(items) => items.iter_mut().for_each(tidy),
        _ => {}
    }
}

/// The JSON Schema as the committed file `models/model-file.schema.json` holds it.
pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&schema()).expect("a schema always serialises");
    text.push('\n');
    text
}
