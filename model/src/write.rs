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
    if !class.refines_built_in() {
        fields.insert("parent".into(), json!(class.parent));
    }
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
        fields.insert("failures".into(), Value::Array(failures.iter().map(self::failures).collect()));
    }
    if let Some(count) = attributes.starting_mus {
        fields.insert("starting_mus".into(), json!(count));
    }
    if let Some(capacity) = attributes.capacity {
        fields.insert("capacity".into(), self::capacity(capacity));
    }
    if let Some(start) = attributes.start {
        fields.insert("start".into(), json!(crate::start_name(start)));
    }
    if let Some(dwelling_time) = attributes.dwelling_time {
        fields.insert("dwelling_time".into(), time(dwelling_time));
    }
    if let Some(strategy) = attributes.exit_strategy {
        fields.insert("exit_strategy".into(), json!(crate::exit_strategy_name(strategy)));
    }
    if let Some(width) = attributes.histogram_bar_width {
        fields.insert("histogram_bar_width".into(), json!(width));
    }
    if let Some(freezes) = attributes.stop_freezes_dwelling {
        fields.insert("stop_freezes_dwelling".into(), json!(freezes));
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
    if let Some(strategy) = &attributes.distribution {
        fields.insert("distribution".into(), distribution(strategy));
    }
    if let Some(class) = &attributes.mu_class {
        fields.insert("mu_class".into(), json!(class));
    }
    if let Some(values) = attributes.mu_values {
        fields.insert("mu_values".into(), mu_values(values));
    }
    for (key, value) in [("length", attributes.length), ("width", attributes.width), ("booking_point", attributes.booking_point)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(path) = &attributes.path {
        fields.insert("path".into(), self::path(path));
    }
    for (key, value) in [("speed", attributes.speed), ("gap", attributes.gap)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(accumulating) = attributes.accumulating {
        fields.insert("accumulating".into(), json!(accumulating));
    }
    Value::Object(fields)
}

/// A conveyor's path as a file writes it: its segments in order.
pub(crate) fn path(path: &[crate::Segment]) -> Value {
    Value::Array(
        path.iter()
            .map(|segment| match *segment {
                crate::Segment::Straight(metres) => json!({ "straight": metres }),
            })
            .collect(),
    )
}

/// A Source's own values for its MUs, as a file writes them: only those set.
pub(crate) fn mu_values(values: crate::MuValues) -> Value {
    let mut fields = Map::new();
    for (key, value) in [("length", values.length), ("width", values.width), ("booking_point", values.booking_point)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    Value::Object(fields)
}

/// A capacity as a file writes it: a number, or `"unlimited"`.
pub(crate) fn capacity(capacity: crate::Capacity) -> Value {
    match capacity {
        crate::Capacity::Limited(limit) => json!(limit),
        crate::Capacity::Unlimited => json!("unlimited"),
    }
}

/// A distribution strategy, with every field of a weighted one written out but `drains` when it
/// counts every Drain.
pub(crate) fn distribution(strategy: &crate::DistributionStrategy) -> Value {
    use crabsim::distribution::{Order, Rank};
    match strategy {
        crate::DistributionStrategy::RoundRobin => json!("round_robin"),
        crate::DistributionStrategy::Weighted(weighted) => {
            let mut fields = Map::new();
            fields.insert("by".into(), json!(weighted.by.name()));
            fields.insert("order".into(), json!(match weighted.order { Order::Smallest => "smallest", Order::Largest => "largest" }));
            fields.insert("rank".into(), json!(match weighted.rank { Rank::Route => "route", Rank::Successor => "successor" }));
            if let Some(drains) = &weighted.drains {
                fields.insert("drains".into(), json!(drains));
            }
            json!({ "weighted": fields })
        }
    }
}

fn failures(failures: &Failures) -> Value {
    let repair_time = match failures.repair_time {
        RepairTime::Constant(t) => json!({ "constant": time(t) }),
        RepairTime::Uniform { min, max } => json!({ "uniform": { "min": time(min), "max": time(max) } }),
        RepairTime::Normal { mean, std } => json!({ "normal": { "mean": time(mean), "std": time(std) } }),
        RepairTime::Erlang { mean, k } => json!({ "erlang": { "mean": time(mean), "k": k } }),
    };
    let mut fields = Map::new();
    fields.insert("name".into(), json!(failures.name));
    if let Some(stream) = failures.stream {
        fields.insert("stream".into(), json!(stream));
    }
    if failures.time_basis != crate::TimeBasis::Simulation {
        fields.insert("time_basis".into(), json!(crate::time_basis_name(failures.time_basis)));
    }
    fields.insert("availability".into(), json!(failures.availability));
    fields.insert("repair_time".into(), repair_time);
    Value::Object(fields)
}
