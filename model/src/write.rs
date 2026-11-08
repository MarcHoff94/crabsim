//! Saving a model in its one canonical form: keys in a fixed order, fields an instance doesn't
//! need left out, times as `format_time_exact` writes them, two-space indentation and a final
//! newline. Saving, loading and saving again gives the same bytes.

use crate::{Attributes, Class, Failures, Instance, Model, RepairTime, FORMAT_VERSION};
use crabsim::time::format_time_exact;
use crabsim::SimTime;
use serde_json::{json, Map, Value};

pub(crate) fn write(model: &Model) -> String {
    let mut file = Map::new();
    if let Some(schema) = &model.schema {
        file.insert("$schema".into(), json!(schema));
    }
    file.insert("format_version".into(), json!(FORMAT_VERSION));
    if let Some(description) = &model.description {
        file.insert("description".into(), json!(description));
    }
    if !model.classes.is_empty() {
        file.insert("classes".into(), Value::Array(model.classes.iter().map(class).collect()));
    }
    file.insert("instances".into(), Value::Array(model.instances.iter().map(instance).collect()));
    let mut text = serde_json::to_string_pretty(&Value::Object(file)).expect("a model always serialises");
    text.push('\n');
    text
}

fn time(time: SimTime) -> Value {
    json!(format_time_exact(time))
}

fn class(class: &Class) -> Value {
    let mut fields = Map::new();
    fields.insert("name".into(), json!(class.name));
    fields.insert("parent".into(), json!(class.parent));
    if !class.attributes.is_empty() {
        fields.insert("attributes".into(), attributes(&class.attributes));
    }
    Value::Object(fields)
}

fn instance(instance: &Instance) -> Value {
    let mut fields = Map::new();
    fields.insert("name".into(), json!(instance.name));
    fields.insert("class".into(), json!(instance.class));
    if let Some(position) = instance.position {
        let mut at = Map::new();
        at.insert("x".into(), json!(position.x));
        at.insert("y".into(), json!(position.y));
        if position.z != 0.0 {
            at.insert("z".into(), json!(position.z));
        }
        fields.insert("position".into(), Value::Object(at));
    }
    if !instance.successors.is_empty() {
        fields.insert("successors".into(), json!(instance.successors));
    }
    if !instance.attributes.is_empty() {
        fields.insert("attributes".into(), attributes(&instance.attributes));
    }
    Value::Object(fields)
}

pub(crate) fn attributes(attributes: &Attributes) -> Value {
    let mut fields = Map::new();
    if let Some(interval) = attributes.interval {
        fields.insert("interval".into(), time(interval));
    }
    if let Some(process_time) = attributes.process_time {
        fields.insert("process_time".into(), time(process_time));
    }
    if let Some(failures) = &attributes.failures {
        fields.insert("failures".into(), self::failures(failures));
    }
    if let Some(count) = attributes.starting_mus {
        fields.insert("starting_mus".into(), json!(count));
    }
    if let Some(capacity) = attributes.capacity {
        fields.insert("capacity".into(), json!(capacity));
    }
    if let Some(dimensions) = attributes.dimensions {
        let mut size = Map::new();
        size.insert("length".into(), json!(dimensions.length));
        size.insert("width".into(), json!(dimensions.width));
        size.insert("height".into(), json!(dimensions.height));
        if dimensions.rotation_degrees != 0.0 {
            size.insert("rotation_degrees".into(), json!(dimensions.rotation_degrees));
        }
        fields.insert("dimensions".into(), Value::Object(size));
    }
    if let Some(description) = &attributes.description {
        fields.insert("description".into(), json!(description));
    }
    Value::Object(fields)
}

fn failures(failures: &Failures) -> Value {
    let repair_time = match failures.repair_time {
        RepairTime::Constant(t) => json!({ "constant": time(t) }),
        RepairTime::Uniform { min, max } => json!({ "uniform": { "min": time(min), "max": time(max) } }),
        RepairTime::Normal { mean, std } => json!({ "normal": { "mean": time(mean), "std": time(std) } }),
        RepairTime::Erlang { mean, k } => json!({ "erlang": { "mean": time(mean), "k": k } }),
    };
    json!({ "availability": failures.availability, "repair_time": repair_time })
}
