//! Saving a model in its one canonical form: keys in a fixed order, fields an instance doesn't
//! need left out, times as `format_time_exact` writes them, two-space indentation and a final
//! newline. Saving, loading and saving again gives the same bytes.

use crate::{Attributes, Class, Connection, Failures, Instance, Model, RepairTime, FORMAT_VERSION};
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
    if !model.connections.is_empty() {
        // The list's order means nothing, so it is saved in one: by the place of the instance each
        // leaves, then by exit, which reads as each object's exits in turn.
        let place = |name: &str| model.instances.iter().position(|instance| instance.name == name).unwrap_or(usize::MAX);
        let mut connections: Vec<&Connection> = model.connections.iter().collect();
        connections.sort_by_key(|c| (place(&c.from), c.from.clone(), c.exit, place(&c.to), c.to.clone(), c.entrance));
        file.insert("connections".into(), Value::Array(connections.into_iter().map(connection).collect()));
    }
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
    if let Some(stream) = instance.stream {
        fields.insert("stream".into(), json!(stream));
    }
    if let Some(position) = instance.position {
        let mut at = Map::new();
        at.insert("x".into(), json!(position.x));
        at.insert("y".into(), json!(position.y));
        if position.z != 0.0 {
            at.insert("z".into(), json!(position.z));
        }
        fields.insert("position".into(), Value::Object(at));
    }
    if !instance.attributes.is_empty() {
        fields.insert("attributes".into(), attributes(&instance.attributes));
    }
    Value::Object(fields)
}

fn connection(connection: &Connection) -> Value {
    let mut fields = Map::new();
    fields.insert("from".into(), json!(connection.from));
    fields.insert("to".into(), json!(connection.to));
    fields.insert("exit".into(), json!(connection.exit));
    fields.insert("entrance".into(), json!(connection.entrance));
    if let Some(description) = &connection.description {
        fields.insert("description".into(), json!(description));
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
    if let Some(notes) = &attributes.state_notes {
        let notes: Map<String, Value> = notes.iter().map(|(state, note)| (state.name().to_string(), json!(note))).collect();
        fields.insert("state_notes".into(), Value::Object(notes));
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
