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
    if !model.hooks.is_empty() {
        let mut hooks = Map::new();
        for (key, _, call) in model.hooks.each() {
            if let Some(call) = call {
                hooks.insert(key.into(), json!({ "code": code_call(call) }));
            }
        }
        file.insert("hooks".into(), Value::Object(hooks));
    }
    if !model.stages.is_empty() {
        let mut stages = Map::new();
        for stage in [crate::CheckStage::Verification, crate::CheckStage::Validation] {
            let settings = model.stages.of(stage);
            if !settings.is_empty() {
                let mut fields = Map::new();
                run_settings(settings, &mut fields);
                stages.insert(stage.name().into(), Value::Object(fields));
            }
        }
        file.insert("stages".into(), Value::Object(stages));
    }
    if !model.requests.is_empty() {
        file.insert("requests".into(), Value::Array(model.requests.iter().map(request).collect()));
    }
    if !model.expectations.is_empty() {
        file.insert("expectations".into(), Value::Array(model.expectations.iter().map(expectation).collect()));
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(file)).expect("a model always serialises");
    text.push('\n');
    text
}

/// An expectation, its stage left out while it is verification, the default.
fn expectation(expectation: &crate::Expectation) -> Value {
    let mut fields = Map::new();
    fields.insert("id".into(), json!(expectation.id));
    fields.insert("text".into(), json!(expectation.text));
    if expectation.stage != crate::CheckStage::default() {
        fields.insert("stage".into(), json!(expectation.stage.name()));
    }
    if let Some(assumptions) = &expectation.assumptions {
        fields.insert("assumptions".into(), json!(assumptions));
    }
    if let Some(source) = &expectation.source {
        fields.insert("source".into(), json!(source));
    }
    if expectation.state != crate::Agreement::default() {
        fields.insert("state".into(), json!(expectation.state.name()));
    }
    if let Some(question) = &expectation.question {
        fields.insert("question".into(), json!(question));
    }
    run_settings(&expectation.run, &mut fields);
    Value::Object(fields)
}

/// A check's run settings, each only where given.
fn run_settings(run: &crate::RunSettings, fields: &mut Map<String, Value>) {
    if let Some(length) = run.length {
        fields.insert("length".into(), time(length));
    }
    if let Some(warm_up) = run.warm_up {
        fields.insert("warm_up".into(), time(warm_up));
    }
    if let Some(observations) = run.observations {
        fields.insert("observations".into(), json!(observations));
    }
}

/// A request, open and not accepted left out as the defaults.
fn request(request: &crate::Request) -> Value {
    let mut fields = Map::new();
    fields.insert("id".into(), json!(request.id));
    fields.insert("words".into(), json!(request.words));
    if let Some(restatement) = &request.restatement {
        fields.insert("restatement".into(), json!(restatement));
    }
    if !request.expectations.is_empty() {
        fields.insert("expectations".into(), json!(request.expectations));
    }
    if request.accepted {
        fields.insert("accepted".into(), json!(true));
    }
    if request.built {
        fields.insert("state".into(), json!("built"));
    }
    Value::Object(fields)
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
    if let Some(side) = connection.from_side {
        fields.insert("from_side".into(), json!(side.name()));
    }
    if let Some(side) = connection.to_side {
        fields.insert("to_side".into(), json!(side.name()));
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
    if let Some(values) = &attributes.mu_values {
        fields.insert("mu_values".into(), mu_values(values));
    }
    for (key, value) in [("length", attributes.length), ("width", attributes.width), ("booking_point", attributes.booking_point)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(path) = &attributes.path {
        fields.insert("path".into(), crate::path::to_json(path));
    }
    for (key, value) in [("speed", attributes.speed), ("gap", attributes.gap)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(accumulating) = attributes.accumulating {
        fields.insert("accumulating".into(), json!(accumulating));
    }
    if let Some(direction) = attributes.direction {
        fields.insert("direction".into(), json!(direction.name()));
    }
    for (key, value) in [("min_radius", attributes.min_radius), ("max_incline", attributes.max_incline)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(plan) = &attributes.work_plan {
        fields.insert("work_plan".into(), work_plan(plan));
    }
    if let Some(detour) = &attributes.detour {
        fields.insert("detour".into(), json!(detour));
    }
    for (key, value) in [("switch_time", attributes.switch_time), ("release_time", attributes.release_time)] {
        if let Some(value) = value {
            fields.insert(key.into(), time(value));
        }
    }
    if let Some(exits) = &attributes.exits {
        fields.insert("exits".into(), crate::write::exits(exits));
    }
    if let Some(hook) = &attributes.on_entry {
        fields.insert("on_entry".into(), code_hook(hook));
    }
    if let Some(hook) = &attributes.on_exit {
        fields.insert("on_exit".into(), code_hook(hook));
    }
    Value::Object(fields)
}

/// A cross conveyor's exits as a file writes them: by side, front, back, left, right.
pub(crate) fn exits(exits: &crate::Exits) -> Value {
    Value::Object(exits.iter().map(|(side, names)| (side.name().to_string(), json!(names))).collect())
}

/// A Source's own values for its MUs, as a file writes them: only those set.
pub(crate) fn mu_values(values: &crate::MuValues) -> Value {
    let mut fields = Map::new();
    for (key, value) in [("length", values.length), ("width", values.width), ("booking_point", values.booking_point)] {
        if let Some(value) = value {
            fields.insert(key.into(), json!(value));
        }
    }
    if let Some(plan) = &values.work_plan {
        fields.insert("work_plan".into(), work_plan(plan));
    }
    Value::Object(fields)
}

/// A work plan as a file writes it: each step `{"at": ...}`, with its process time if it has one.
pub(crate) fn work_plan(plan: &[crate::WorkStep]) -> Value {
    Value::Array(
        plan.iter()
            .map(|step| {
                let mut fields = Map::new();
                fields.insert("at".into(), json!(step.at));
                if let Some(process_time) = step.process_time {
                    fields.insert("process_time".into(), time(process_time));
                }
                Value::Object(fields)
            })
            .collect(),
    )
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
        crate::DistributionStrategy::ByDestination(then) => json!({ "by_destination": { "then": distribution(then) } }),
        crate::DistributionStrategy::Code(code) => {
            let mut fields = Map::new();
            fields.insert("code".into(), code_call(&code.code));
            if code.default != crate::DistributionStrategy::RoundRobin {
                fields.insert("default".into(), distribution(&code.default));
            }
            if let Some(point) = code.asked_on_entry {
                fields.insert("asked_on_entry".into(), json!(crate::trigger_point_name(point)));
            }
            Value::Object(fields)
        }
    }
}

/// A call of code, with `with` left out when it is empty.
pub(crate) fn code_call(call: &crate::CodeCall) -> Value {
    let mut fields = Map::new();
    fields.insert("function".into(), json!(call.function));
    if !call.with.is_empty() {
        fields.insert("with".into(), Value::Object(call.with.clone()));
    }
    Value::Object(fields)
}

/// A hook on entry or exit, with `at` left out when it is the booking point alone.
pub(crate) fn code_hook(hook: &crate::CodeHook) -> Value {
    let mut fields = Map::new();
    fields.insert("code".into(), code_call(&hook.code));
    if hook.at != [crabsim::TriggerPoint::BookingPoint] {
        fields.insert("at".into(), json!(hook.at.iter().map(|&point| crate::trigger_point_name(point)).collect::<Vec<_>>()));
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
