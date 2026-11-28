//! Reading a file's JSON into a `Model`, noting every problem with its shape as it goes rather than
//! stopping at the first. What can't be read is left at its default, and noted in `Broken`, so the
//! checks that follow don't report it a second time.

use crate::{Attributes, Bend, Direction, DistributionStrategy, MuValues, Segment, WeightedStrategy, WEIGHABLE, BUILT_IN_CLASSES, Class, Connection, Failures, Instance, Model, Problem, RepairTime, State, StateNotes, WorkStep, FORMAT_VERSION, escape};
use crabsim::time::parse_time;
use crate::{exit_strategy_from_name, start_from_name, Capacity, Exits, ExitStrategy, Side, StartMode};
use crabsim::{Dimensions, Position, SimTime};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// What couldn't be read, so the checks leave it alone.
#[derive(Debug, Default)]
pub(crate) struct Broken {
    /// Instances whose stream couldn't be read, given another one so the rest can be checked.
    pub streams: HashSet<usize>,
    /// Instances, by index, whose name couldn't be read.
    pub names: HashSet<usize>,
    /// Instances whose class couldn't be read.
    pub classes: HashSet<usize>,
    /// Attributes of instances that were there but couldn't be read.
    pub attributes: HashSet<(usize, &'static str)>,
    /// Connections, by index, whose `from` or `to` couldn't be read.
    pub connections: HashSet<usize>,
    /// Connections whose `exit` couldn't be read.
    pub exits: HashSet<usize>,
    /// Connections whose `entrance` couldn't be read.
    pub entrances: HashSet<usize>,
    /// Classes, by index, whose name couldn't be read.
    pub class_names: HashSet<usize>,
    /// Classes whose parent couldn't be read.
    pub class_parents: HashSet<usize>,
    /// Attributes of classes that were there but couldn't be read.
    pub class_attributes: HashSet<(usize, &'static str)>,
}

const TOP_LEVEL: [&str; 10] = ["$schema", "format_version", "description", "classes", "instances", "connections", "hooks", "stages", "requests", "expectations"];
const EXPECTATION: [&str; 10] = ["id", "text", "stage", "assumptions", "source", "state", "question", "length", "warm_up", "observations"];
const REQUEST: [&str; 6] = ["id", "words", "restatement", "expectations", "accepted", "state"];
const RUN_SETTINGS: [&str; 3] = ["length", "warm_up", "observations"];
const INSTANCE: [&str; 5] = ["name", "class", "stream", "position", "attributes"];
const CONNECTION: [&str; 7] = ["from", "to", "exit", "entrance", "description", "from_side", "to_side"];
const CLASS: [&str; 3] = ["name", "parent", "attributes"];
const ATTRIBUTES: [&str; 33] = [
    "interval",
    "process_time",
    "failures",
    "starting_mus",
    "capacity",
    "start",
    "dwelling_time",
    "exit_strategy",
    "histogram_bar_width",
    "stop_freezes_dwelling",
    "dimensions",
    "description",
    "state_notes",
    "distribution",
    "mu_class",
    "mu_values",
    "length",
    "width",
    "booking_point",
    "path",
    "speed",
    "gap",
    "accumulating",
    "direction",
    "min_radius",
    "max_incline",
    "work_plan",
    "detour",
    "switch_time",
    "release_time",
    "exits",
    "on_entry",
    "on_exit",
];
/// The kinds of segment a path has (ADR 0024).
const SEGMENTS: [&str; 4] = ["straight", "arc", "transition", "helix"];
/// The values a Source can change for the MUs it makes.
const MU_VALUES: [&str; 4] = ["length", "width", "booking_point", "work_plan"];

pub(crate) fn read(text: &str) -> (Model, Vec<Problem>, Broken) {
    let mut reader = Reader::default();
    let model = match serde_json::from_str::<Value>(text) {
        Ok(value) => reader.model(&value),
        Err(error) => {
            reader.problem(String::new(), None, format!("this isn't JSON: {error}"));
            Model::default()
        }
    };
    (model, reader.problems, reader.broken)
}

#[derive(Default)]
struct Reader {
    problems: Vec<Problem>,
    broken: Broken,
}

impl Reader {
    fn problem(&mut self, pointer: String, name: Option<&str>, message: impl Into<String>) {
        self.problems.push(Problem { pointer, name: name.map(str::to_owned), message: message.into() });
    }

    /// The fields of an object, with a problem for each one not in `known`. `None` if it isn't one.
    fn object<'v>(&mut self, value: &'v Value, pointer: &str, name: Option<&str>, what: &str, known: &[&str]) -> Option<&'v Map<String, Value>> {
        let Some(fields) = value.as_object() else {
            self.problem(pointer.to_owned(), name, format!("{what} should be an object"));
            return None;
        };
        for key in fields.keys().filter(|key| !known.contains(&key.as_str())) {
            let guess = crate::suggest::did_you_mean(key, known.iter().copied());
            let message = match what {
                "attributes" => format!("unknown attribute `{key}`{guess}"),
                _ => format!("unknown field `{key}` in {what}{guess}"),
            };
            self.problem(format!("{pointer}/{}", escape(key)), name, message);
        }
        Some(fields)
    }

    fn model(&mut self, value: &Value) -> Model {
        let Some(fields) = self.object(value, "", None, "a model file", &TOP_LEVEL) else {
            return Model::default();
        };
        match fields.get("format_version") {
            None => self.problem(String::new(), None, format!("`format_version` is missing: this crabsim reads version {FORMAT_VERSION}")),
            Some(version) if version.as_u64() == Some(FORMAT_VERSION) => {}
            // Nothing else is read: every instance's successors would be a problem of its own.
            Some(version) if version.as_u64() == Some(1) => {
                self.problem(
                    "/format_version".into(),
                    None,
                    format!(
                        "format version 1 isn't read any more: since version {FORMAT_VERSION}, connections are their own list beside the instances, each with `from`, `to`, its `exit` and its `entrance`, instead of each instance's `successors`. model_file_format shows the format"
                    ),
                );
                return Model::default();
            }
            Some(version) => self.problem(
                "/format_version".into(),
                None,
                format!("format version {version} isn't supported: this crabsim reads version {FORMAT_VERSION}"),
            ),
        }
        let schema = fields.get("$schema").and_then(|value| self.text(value, "/$schema", None));
        let description = fields.get("description").and_then(|value| self.text(value, "/description", None));
        let classes = match fields.get("classes") {
            None => Vec::new(),
            Some(Value::Array(classes)) => classes.iter().enumerate().map(|(index, value)| self.class(index, value)).collect(),
            Some(_) => {
                self.problem("/classes".into(), None, "`classes` should be a list");
                Vec::new()
            }
        };
        let instances = match fields.get("instances") {
            None => {
                self.problem(String::new(), None, "`instances` is missing");
                Vec::new()
            }
            Some(Value::Array(instances)) => instances.iter().enumerate().map(|(index, value)| self.instance(index, value)).collect(),
            Some(_) => {
                self.problem("/instances".into(), None, "`instances` should be a list");
                Vec::new()
            }
        };
        let connections = match fields.get("connections") {
            None => Vec::new(),
            Some(Value::Array(connections)) => connections.iter().enumerate().map(|(index, value)| self.connection(index, value)).collect(),
            Some(_) => {
                self.problem("/connections".into(), None, "`connections` should be a list");
                Vec::new()
            }
        };
        let hooks = match fields.get("hooks") {
            None => crate::ModelHooks::default(),
            Some(value) => self.model_hooks(value),
        };
        let expectations = match fields.get("expectations") {
            None => Vec::new(),
            Some(Value::Array(expectations)) => expectations.iter().enumerate().map(|(index, value)| self.expectation(index, value)).collect(),
            Some(_) => {
                self.problem("/expectations".into(), None, "`expectations` should be a list");
                Vec::new()
            }
        };
        let requests = match fields.get("requests") {
            None => Vec::new(),
            Some(Value::Array(requests)) => requests.iter().enumerate().map(|(index, value)| self.request(index, value)).collect(),
            Some(_) => {
                self.problem("/requests".into(), None, "`requests` should be a list");
                Vec::new()
            }
        };
        let mut stages = crate::Stages::default();
        if let Some(value) = fields.get("stages") {
            if let Some(fields) = self.object(value, "/stages", None, "`stages`", &["verification", "validation"]) {
                for stage in [crate::CheckStage::Verification, crate::CheckStage::Validation] {
                    if let Some(value) = fields.get(stage.name()) {
                        let pointer = format!("/stages/{}", stage.name());
                        if let Some(settings) = self.object(value, &pointer, None, "a stage's settings", &RUN_SETTINGS) {
                            *stages.of_mut(stage) = self.run_settings(settings, &pointer, None);
                        }
                    }
                }
            }
        }
        Model { schema, description, classes, instances, connections, hooks, expectations, requests, stages }
    }

    /// How a check runs: its length, warm-up and observations, each optional.
    fn run_settings(&mut self, fields: &Map<String, Value>, pointer: &str, name: Option<&str>) -> crate::RunSettings {
        let mut run = crate::RunSettings::default();
        if let Some(value) = fields.get("length") {
            run.length = self.time(value, &format!("{pointer}/length"), name);
        }
        if let Some(value) = fields.get("warm_up") {
            run.warm_up = self.time(value, &format!("{pointer}/warm_up"), name);
        }
        if let Some(value) = fields.get("observations") {
            run.observations = self.count(value, &format!("{pointer}/observations"), name).map(|count| count.min(u64::from(u32::MAX)) as u32);
        }
        run
    }

    /// One request (ADR 0030): the user's words, an agent's restatement and its expectations.
    fn request(&mut self, index: usize, value: &Value) -> crate::Request {
        let pointer = format!("/requests/{index}");
        let mut request = crate::Request::default();
        let Some(fields) = self.object(value, &pointer, None, "a request", &REQUEST) else {
            return request;
        };
        let id = fields.get("id").and_then(|value| value.as_str()).map(str::to_owned);
        let name = id.as_deref();
        if let Some(value) = fields.get("id") {
            request.id = self.text(value, &format!("{pointer}/id"), name).unwrap_or_default();
        }
        if let Some(value) = fields.get("words") {
            request.words = self.text(value, &format!("{pointer}/words"), name).unwrap_or_default();
        }
        request.restatement = fields.get("restatement").and_then(|value| self.text(value, &format!("{pointer}/restatement"), name));
        match fields.get("expectations") {
            None => {}
            Some(Value::Array(ids)) => {
                for (place, value) in ids.iter().enumerate() {
                    if let Some(id) = self.text(value, &format!("{pointer}/expectations/{place}"), name) {
                        request.expectations.push(id);
                    }
                }
            }
            Some(_) => self.problem(format!("{pointer}/expectations"), name, "`expectations` should be a list of expectations' ids"),
        }
        if let Some(value) = fields.get("accepted") {
            request.accepted = self.yes_or_no(value, &format!("{pointer}/accepted"), name).unwrap_or(false);
        }
        match fields.get("state").map(|value| value.as_str()) {
            None | Some(Some("open")) => {}
            Some(Some("built")) => request.built = true,
            Some(_) => self.problem(
                format!("{pointer}/state"),
                name,
                "a request's `state` is `open` or `built`: whether it is met is what crabsim's checks say, not the file",
            ),
        }
        request
    }

    /// One expectation (ADR 0030). Its line is checked once the whole model is read, since it
    /// names instances and classes, and kept in its canonical form when it parses.
    fn expectation(&mut self, index: usize, value: &Value) -> crate::Expectation {
        let pointer = format!("/expectations/{index}");
        let mut expectation = crate::Expectation::new(String::new(), String::new());
        let Some(fields) = self.object(value, &pointer, None, "an expectation", &EXPECTATION) else {
            return expectation;
        };
        let id = fields.get("id").and_then(|value| value.as_str()).map(str::to_owned);
        // A missing id or text is left empty, which the checks report as they do an empty one.
        for key in ["id", "text"] {
            if let Some(value) = fields.get(key) {
                if let Some(text) = self.text(value, &format!("{pointer}/{key}"), id.as_deref()) {
                    if key == "id" { expectation.id = text } else { expectation.text = text }
                }
            }
        }
        if let Ok(check) = crate::expect::Check::parse(&expectation.text) {
            expectation.text = check.text();
        }
        if let Some(value) = fields.get("stage") {
            match value.as_str().and_then(crate::CheckStage::named) {
                Some(stage) => expectation.stage = stage,
                None => self.problem(format!("{pointer}/stage"), id.as_deref(), "`stage` is `verification` or `validation`"),
            }
        }
        expectation.assumptions = fields.get("assumptions").and_then(|value| self.text(value, &format!("{pointer}/assumptions"), id.as_deref()));
        expectation.source = fields.get("source").and_then(|value| self.text(value, &format!("{pointer}/source"), id.as_deref()));
        if let Some(value) = fields.get("state") {
            match value.as_str().and_then(crate::Agreement::named) {
                Some(state) => expectation.state = state,
                None => self.problem(format!("{pointer}/state"), id.as_deref(), "`state` is `proposed`, `accepted` or `questioned`"),
            }
        }
        expectation.question = fields.get("question").and_then(|value| self.text(value, &format!("{pointer}/question"), id.as_deref()));
        expectation.run = self.run_settings(fields, &pointer, id.as_deref());
        expectation
    }

    fn connection(&mut self, index: usize, value: &Value) -> Connection {
        let pointer = format!("/connections/{index}");
        let mut connection = Connection::new(String::new(), String::new(), 0, 0);
        let Some(fields) = self.object(value, &pointer, None, "a connection", &CONNECTION) else {
            self.broken.connections.insert(index);
            self.broken.exits.insert(index);
            self.broken.entrances.insert(index);
            return connection;
        };
        for key in ["from", "to"] {
            match fields.get(key).map(|value| value.as_str()) {
                Some(Some(text)) => {
                    if key == "from" { connection.from = text.to_owned() } else { connection.to = text.to_owned() }
                }
                Some(None) => {
                    self.problem(format!("{pointer}/{key}"), None, format!("`{key}` should be an instance's name"));
                    self.broken.connections.insert(index);
                }
                None => {
                    self.problem(pointer.clone(), None, format!("`{key}` is missing"));
                    self.broken.connections.insert(index);
                }
            }
        }
        for key in ["exit", "entrance"] {
            let place = match fields.get(key) {
                Some(value) => match value.as_u64().filter(|&place| place >= 1) {
                    Some(place) => Some(place),
                    None => {
                        self.problem(format!("{pointer}/{key}"), None, format!("{value} is not a place: write a whole number, 1 or more"));
                        None
                    }
                },
                None => {
                    self.problem(pointer.clone(), None, format!("`{key}` is missing"));
                    None
                }
            };
            match (key, place) {
                ("exit", Some(place)) => connection.exit = place,
                ("exit", None) => {
                    self.broken.exits.insert(index);
                }
                (_, Some(place)) => connection.entrance = place,
                (_, None) => {
                    self.broken.entrances.insert(index);
                }
            }
        }
        if let Some(description) = fields.get("description") {
            connection.description = self.text(description, &format!("{pointer}/description"), None);
        }
        for key in ["from_side", "to_side"] {
            if let Some(value) = fields.get(key) {
                let side = self.side(value, &format!("{pointer}/{key}"), None);
                if key == "from_side" { connection.from_side = side } else { connection.to_side = side }
            }
        }
        connection
    }

    /// A cross conveyor's side: `front`, `back`, `left` or `right`.
    fn side(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Side> {
        let side = value.as_str().and_then(Side::from_name);
        if side.is_none() {
            self.problem(pointer.to_owned(), name, "a side is \"front\", \"back\", \"left\" or \"right\"");
        }
        side
    }

    /// A cross conveyor's exits: each side's list of destinations, such as
    /// `{"left": ["Mill"], "right": ["Rework", "Shipping"]}`.
    fn exits(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Exits> {
        const SAY: &str = r#"exits list each exit side's destinations, such as {"left": ["Mill"], "right": ["Rework", "Shipping"]}"#;
        let Some(fields) = value.as_object() else {
            self.problem(pointer.to_owned(), name, SAY);
            return None;
        };
        let mut exits = Exits::new();
        let mut ok = true;
        for (key, list) in fields {
            let at = format!("{pointer}/{}", crate::escape(key));
            let Some(side) = Side::from_name(key) else {
                self.problem(at, name, format!("`{key}` is no side: a side is \"front\", \"back\", \"left\" or \"right\""));
                ok = false;
                continue;
            };
            let names = list.as_array().and_then(|items| items.iter().map(|item| item.as_str().map(str::to_owned)).collect::<Option<Vec<String>>>());
            match names {
                Some(names) => {
                    exits.insert(side, names);
                }
                None => {
                    self.problem(at, name, SAY);
                    ok = false;
                }
            }
        }
        ok.then_some(exits)
    }

    fn instance(&mut self, index: usize, value: &Value) -> Instance {
        let pointer = format!("/instances/{index}");
        let readable_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
        let name = readable_name.as_deref();
        let mut instance = Instance::new(String::new(), String::new());
        let Some(fields) = self.object(value, &pointer, name, "an instance", &INSTANCE) else {
            self.broken.names.insert(index);
            self.broken.classes.insert(index);
            return instance;
        };
        match fields.get("name").map(|value| value.as_str()) {
            Some(Some(text)) => instance.name = text.to_owned(),
            Some(None) => {
                self.problem(format!("{pointer}/name"), None, "`name` should be text");
                self.broken.names.insert(index);
            }
            None => {
                self.problem(pointer.clone(), None, "`name` is missing");
                self.broken.names.insert(index);
            }
        }
        match fields.get("class").map(|value| value.as_str()) {
            Some(Some(text)) => instance.class = text.to_owned(),
            Some(None) => {
                self.problem(format!("{pointer}/class"), name, "`class` should be text");
                self.broken.classes.insert(index);
            }
            None => {
                self.problem(pointer.clone(), name, "`class` is missing");
                self.broken.classes.insert(index);
            }
        }
        if let Some(stream) = fields.get("stream") {
            match stream.as_u64() {
                Some(stream) => instance.stream = Some(stream),
                None => {
                    self.problem(format!("{pointer}/stream"), name, format!("{stream} is not a stream: write a whole number, 0 or more"));
                    self.broken.streams.insert(index);
                }
            }
        }
        if let Some(position) = fields.get("position") {
            instance.position = self.position(position, &format!("{pointer}/position"), name);
        }
        if let Some(attributes) = fields.get("attributes") {
            let (attributes, broken) = self.attributes(attributes, &format!("{pointer}/attributes"), name);
            instance.attributes = attributes;
            self.broken.attributes.extend(broken.into_iter().map(|key| (index, key)));
        }
        instance
    }

    fn class(&mut self, index: usize, value: &Value) -> Class {
        let pointer = format!("/classes/{index}");
        let readable_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
        let name = readable_name.as_deref();
        let mut class = Class::new(String::new(), String::new());
        let Some(fields) = self.object(value, &pointer, name, "a class", &CLASS) else {
            self.broken.class_names.insert(index);
            self.broken.class_parents.insert(index);
            return class;
        };
        for (key, broken) in [("name", &mut self.broken.class_names), ("parent", &mut self.broken.class_parents)] {
            let (at, message) = match fields.get(key).map(|value| value.as_str()) {
                Some(Some(_)) if key == "parent" && class.refines_built_in() => {
                    class.parent = class.name.clone();
                    (format!("{pointer}/parent"), refinement_with_parent(&class.name))
                }
                Some(Some(text)) => {
                    if key == "name" { class.name = text.to_owned() } else { class.parent = text.to_owned() }
                    continue;
                }
                Some(None) => (format!("{pointer}/{key}"), format!("`{key}` should be text")),
                // A class named like a built-in is the model's own values for it, with no parent.
                None if key == "parent" && BUILT_IN_CLASSES.contains(&class.name.as_str()) => {
                    class.parent = class.name.clone();
                    continue;
                }
                None => (pointer.clone(), format!("`{key}` is missing")),
            };
            broken.insert(index);
            self.problems.push(Problem { pointer: at, name: if key == "name" { None } else { readable_name.clone() }, message });
        }
        if let Some(attributes) = fields.get("attributes") {
            let (attributes, broken) = self.attributes(attributes, &format!("{pointer}/attributes"), name);
            class.attributes = attributes;
            self.broken.class_attributes.extend(broken.into_iter().map(|key| (index, key)));
        }
        class
    }

    /// The attributes read, and those that were there but couldn't be read.
    fn attributes(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> (Attributes, Vec<&'static str>) {
        let mut attributes = Attributes::default();
        let mut broken = Vec::new();
        let Some(fields) = self.object(value, pointer, name, "attributes", &ATTRIBUTES) else {
            return (attributes, broken);
        };
        for key in ATTRIBUTES {
            let Some(value) = fields.get(key) else { continue };
            let at = format!("{pointer}/{key}");
            let read = match key {
                "interval" => self.time(value, &at, name).map(|time| attributes.interval = Some(time)),
                "process_time" if value.is_object() => self.process_time_rules(value, &at, name).map(|(lines, default)| {
                    attributes.process_time = Some(default);
                    attributes.process_time_rules = Some(lines);
                }),
                "process_time" => self.time(value, &at, name).map(|time| attributes.process_time = Some(time)),
                "failures" => self.failure_list(value, &at, name).map(|failures| attributes.failures = Some(failures)),
                "starting_mus" => self.count(value, &at, name).map(|count| attributes.starting_mus = Some(count)),
                "capacity" => self.capacity(value, &at, name).map(|capacity| attributes.capacity = Some(capacity)),
                "start" => self.start(value, &at, name).map(|start| attributes.start = Some(start)),
                "dwelling_time" => self.time(value, &at, name).map(|time| attributes.dwelling_time = Some(time)),
                "exit_strategy" => self.exit_strategy(value, &at, name).map(|strategy| attributes.exit_strategy = Some(strategy)),
                "histogram_bar_width" => self.count(value, &at, name).map(|width| attributes.histogram_bar_width = Some(width)),
                "stop_freezes_dwelling" => self.yes_or_no(value, &at, name).map(|freezes| attributes.stop_freezes_dwelling = Some(freezes)),
                "dimensions" => self.dimensions(value, &at, name).map(|dimensions| attributes.dimensions = Some(dimensions)),
                "description" => self.text(value, &at, name).map(|text| attributes.description = Some(text)),
                "state_notes" => self.state_notes(value, &at, name).map(|notes| attributes.state_notes = Some(notes)),
                "distribution" => self.distribution(value, &at, name).map(|strategy| attributes.distribution = Some(strategy)),
                "mu_class" => self.text(value, &at, name).map(|class| attributes.mu_class = Some(class)),
                "mu_values" => self.mu_values(value, &at, name).map(|values| attributes.mu_values = Some(values)),
                "length" => self.number_value(value, &at, name).map(|length| attributes.length = Some(length)),
                "width" => self.number_value(value, &at, name).map(|width| attributes.width = Some(width)),
                "booking_point" => self.number_value(value, &at, name).map(|share| attributes.booking_point = Some(share)),
                "path" => self.path(value, &at, name).map(|path| attributes.path = Some(path)),
                "speed" => self.number_value(value, &at, name).map(|speed| attributes.speed = Some(speed)),
                "gap" => self.number_value(value, &at, name).map(|gap| attributes.gap = Some(gap)),
                "accumulating" => self.yes_or_no(value, &at, name).map(|accumulating| attributes.accumulating = Some(accumulating)),
                "direction" => self.direction(value, &at, name).map(|direction| attributes.direction = Some(direction)),
                "min_radius" => self.number_value(value, &at, name).map(|radius| attributes.min_radius = Some(radius)),
                "max_incline" => self.number_value(value, &at, name).map(|degrees| attributes.max_incline = Some(degrees)),
                "work_plan" => self.work_plan(value, &at, name).map(|plan| attributes.work_plan = Some(plan)),
                "detour" => self.text(value, &at, name).map(|detour| attributes.detour = Some(detour)),
                "switch_time" => self.time(value, &at, name).map(|time| attributes.switch_time = Some(time)),
                "release_time" => self.time(value, &at, name).map(|time| attributes.release_time = Some(time)),
                "exits" => self.exits(value, &at, name).map(|exits| attributes.exits = Some(exits)),
                "on_entry" => self.code_hook(value, &at, name).map(|hook| attributes.on_entry = Some(hook)),
                "on_exit" => self.code_hook(value, &at, name).map(|hook| attributes.on_exit = Some(hook)),
                _ => unreachable!("every attribute is read"),
            };
            if read.is_none() {
                broken.push(key);
            }
        }
        (attributes, broken)
    }

    fn text(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<String> {
        let text = value.as_str().map(str::to_owned);
        if text.is_none() {
            self.problem(pointer.to_owned(), name, "this should be text");
        }
        text
    }

    fn time(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<SimTime> {
        let time = value.as_str().and_then(parse_time);
        if time.is_none() {
            let message = match value.as_str() {
                None => format!("{value} is not a time: write it as text, such as \"90\", \"1:30\", \"8:00:00\" or \"2d 06:00:00\""),
                Some(text) if text.chars().any(|c| c.is_alphabetic() && c != 'd') => format!(
                    "{value} is not a time: a time has no unit letters, apart from d for days, and is written such as \"10\", \"0:10\", \"8:00:00\" or \"2d 06:00:00\""
                ),
                Some(_) => format!("{value} is not a time, which is written such as \"90\", \"1:30\", \"8:00:00\" or \"2d 06:00:00\""),
            };
            self.problem(pointer.to_owned(), name, message);
        }
        time
    }

    fn yes_or_no(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<bool> {
        let answer = value.as_bool();
        if answer.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} should be true or false"));
        }
        answer
    }

    fn count(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<u64> {
        let count = value.as_u64();
        if count.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} should be a whole number, 0 or more"));
        }
        count
    }

    /// A whole number, or unlimited, which is written `"unlimited"` and also read from -1.
    fn capacity(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Capacity> {
        if value.as_str() == Some("unlimited") || value.as_i64() == Some(-1) {
            return Some(Capacity::Unlimited);
        }
        let capacity = value.as_u64().map(Capacity::Limited);
        if capacity.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} should be a whole number from 1, or \"unlimited\""));
        }
        capacity
    }

    fn start(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<StartMode> {
        let start = value.as_str().and_then(start_from_name);
        if start.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} is not a start: use \"individual\" or \"batch\""));
        }
        start
    }

    fn exit_strategy(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<ExitStrategy> {
        let strategy = value.as_str().and_then(exit_strategy_from_name);
        if strategy.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} is not an exit strategy: use \"first_in_first_out\" or \"last_in_first_out\""));
        }
        strategy
    }

    fn number(&mut self, fields: &Map<String, Value>, key: &str, pointer: &str, name: Option<&str>) -> Option<f64> {
        match fields.get(key) {
            None => {
                self.problem(pointer.to_owned(), name, format!("`{key}` is missing"));
                None
            }
            Some(value) => {
                let number = value.as_f64();
                if number.is_none() {
                    self.problem(format!("{pointer}/{key}"), name, format!("{value} should be a number"));
                }
                number
            }
        }
    }

    fn number_value(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<f64> {
        let number = value.as_f64();
        if number.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} should be a number"));
        }
        number
    }

    fn direction(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Direction> {
        let direction = value.as_str().and_then(Direction::from_name);
        if direction.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} is not a direction: use \"forward\" or \"backward\""));
        }
        direction
    }

    /// A conveyor's path: a list of segments, each one of the kinds of `SEGMENTS`, such as
    /// `{"straight": 10}` or `{"arc": {"radius": 1.5, "angle": 90}}`. Whether the values make a
    /// sound path is checked later.
    fn path(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Vec<Segment>> {
        const SAY: &str = r#"a path is a list of segments, such as [{"straight": 10}, {"arc": {"radius": 1.5, "angle": 90}}], in metres and degrees"#;
        let Some(items) = value.as_array() else {
            self.problem(pointer.to_owned(), name, SAY);
            return None;
        };
        let mut path = Vec::new();
        let mut ok = true;
        for (index, item) in items.iter().enumerate() {
            let at = format!("{pointer}/{index}");
            let Some(fields) = item.as_object() else {
                self.problem(at, name, SAY);
                ok = false;
                continue;
            };
            let mut kinds = fields.iter();
            let (Some((kind, body)), None) = (kinds.next(), kinds.next()) else {
                self.problem(at, name, "a segment is one of its kinds: straight, arc, transition or helix, such as {\"straight\": 10}");
                ok = false;
                continue;
            };
            match self.segment(kind, body, &format!("{at}/{}", escape(kind)), name) {
                Some(segment) => path.push(segment),
                None => ok = false,
            }
        }
        ok.then_some(path)
    }

    /// One segment of the kind `kind`, from what it holds.
    fn segment(&mut self, kind: &str, body: &Value, at: &str, name: Option<&str>) -> Option<Segment> {
        if kind == "straight" && body.is_number() {
            return self.number_value(body, at, name).map(Segment::straight);
        }
        let fields: &[&str] = match kind {
            "straight" => &["length", "rise"],
            "arc" => &["radius", "angle", "rise"],
            "transition" => &["length", "to_radius", "rise"],
            "helix" => &["radius", "turns", "rise"],
            other => {
                let guess = crate::suggest::did_you_mean(other, SEGMENTS);
                self.problem(at.to_owned(), name, format!("unknown segment `{other}`{guess}: a segment is a straight, an arc, a transition or a helix"));
                return None;
            }
        };
        let what = format!("a {kind}");
        let fields_read = self.object(body, at, name, &what, fields)?;
        let rise = match fields_read.get("rise") {
            Some(rise) => self.number_value(rise, &format!("{at}/rise"), name),
            // A helix always climbs or falls: a flat turn is an arc.
            None if kind == "helix" => self.number(fields_read, "rise", at, name),
            None => Some(0.0),
        };
        match kind {
            "straight" => Some(Segment::Straight { length: self.number(fields_read, "length", at, name)?, rise: rise? }),
            "arc" => {
                let radius = self.number(fields_read, "radius", at, name);
                let angle = self.number(fields_read, "angle", at, name);
                Some(Segment::Arc { radius: radius?, angle: angle?, rise: rise? })
            }
            "transition" => {
                let length = self.number(fields_read, "length", at, name);
                let to = match fields_read.get("to_radius") {
                    None => {
                        self.problem(at.to_owned(), name, "`to_radius` is missing: the radius it eases to, or \"straight\"");
                        None
                    }
                    Some(Value::String(word)) if word == "straight" => Some(Bend::Straight),
                    Some(value) => match value.as_f64() {
                        Some(radius) => Some(Bend::Radius(radius)),
                        None => {
                            self.problem(format!("{at}/to_radius"), name, format!("{value} should be a radius in metres, or \"straight\""));
                            None
                        }
                    },
                };
                Some(Segment::Transition { length: length?, to: to?, rise: rise? })
            }
            _ => {
                let radius = self.number(fields_read, "radius", at, name);
                let turns = self.number(fields_read, "turns", at, name);
                Some(Segment::Helix { radius: radius?, turns: turns?, rise: rise? })
            }
        }
    }

    /// A Source's own values for its MUs, each optional. The values that can be read are kept.
    fn mu_values(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<MuValues> {
        let fields = self.object(value, pointer, name, "MU values", &MU_VALUES)?;
        let mut values = MuValues::default();
        let mut read = |key: &str| fields.get(key).and_then(|value| self.number_value(value, &format!("{pointer}/{key}"), name));
        values.length = read("length");
        values.width = read("width");
        values.booking_point = read("booking_point");
        values.work_plan = fields.get("work_plan").and_then(|plan| self.work_plan(plan, &format!("{pointer}/work_plan"), name));
        Some(values)
    }

    /// A work plan: a list of steps, each `{"at": name}` with an optional `"process_time"`. Whether
    /// the names are places in the model is checked later.
    fn work_plan(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Vec<WorkStep>> {
        const SAY: &str = r#"a work plan is a list of steps, such as [{"at": "Saw", "process_time": "0:40"}, {"at": "Shipping"}]"#;
        let Some(items) = value.as_array() else {
            self.problem(pointer.to_owned(), name, SAY);
            return None;
        };
        let mut plan = Vec::new();
        let mut ok = true;
        for (index, item) in items.iter().enumerate() {
            let at = format!("{pointer}/{index}");
            let Some(fields) = self.object(item, &at, name, "a work plan step", &["at", "process_time"]) else {
                ok = false;
                continue;
            };
            let place = match fields.get("at") {
                None => {
                    self.problem(at.clone(), name, "`at` is missing: a step names where the MU goes, an instance or a class");
                    None
                }
                Some(place) => self.text(place, &format!("{at}/at"), name),
            };
            let process_time = match fields.get("process_time") {
                None => Some(None),
                Some(time) => self.time(time, &format!("{at}/process_time"), name).map(Some),
            };
            match (place, process_time) {
                (Some(place), Some(process_time)) => plan.push(WorkStep { at: place, process_time }),
                _ => ok = false,
            }
        }
        ok.then_some(plan)
    }

    fn position(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Position> {
        let fields = self.object(value, pointer, name, "a position", &["x", "y", "z"])?;
        let x = self.number(fields, "x", pointer, name);
        let y = self.number(fields, "y", pointer, name);
        let z = if fields.contains_key("z") { self.number(fields, "z", pointer, name) } else { Some(0.0) };
        Some(Position { x: x?, y: y?, z: z? })
    }

    /// `"round_robin"`, `{"weighted": {"by": ..., "order": ..., "rank": ..., "drains": [...]}}`
    /// with all but `by` optional, or `{"by_destination": {"then": ...}}` with either of the
    /// others as `then`, round robin when left out.
    fn distribution(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<DistributionStrategy> {
        const SAY: &str = r#"a distribution is "round_robin", {"weighted": {"by": "process_time"}}, with "order", "rank" and "drains" optional, or {"by_destination": {"then": "round_robin"}}"#;
        if value.as_str() == Some("round_robin") {
            return Some(DistributionStrategy::RoundRobin);
        }
        if let Some(text) = value.as_str() {
            self.problem(pointer.to_owned(), name, format!("unknown distribution `{text}`: {SAY}"));
            return None;
        }
        if value.get("code").is_some() {
            return self.code_distribution(value, pointer, name);
        }
        if value.get("rules").is_some() {
            return self.rules_distribution(value, pointer, name);
        }
        let fields = self.object(value, pointer, name, "a distribution", &["weighted", "by_destination"])?;
        if let Some(by_destination) = fields.get("by_destination") {
            let at = format!("{pointer}/by_destination");
            let fields = self.object(by_destination, &at, name, "a distribution by destination", &["then"])?;
            let then = match fields.get("then") {
                None => DistributionStrategy::RoundRobin,
                Some(then) => self.distribution(then, &format!("{at}/then"), name)?,
            };
            if matches!(then, DistributionStrategy::ByDestination(_) | DistributionStrategy::Code(_)) {
                self.problem(format!("{at}/then"), name, "by destination chooses among the ways that reach by \"round_robin\" or {\"weighted\": ...}, not by destination again");
                return None;
            }
            return Some(DistributionStrategy::ByDestination(Box::new(then)));
        }
        let Some(weighted) = fields.get("weighted") else {
            self.problem(pointer.to_owned(), name, SAY.to_string());
            return None;
        };
        let at = format!("{pointer}/weighted");
        let fields = self.object(weighted, &at, name, "a weighted distribution", &["by", "order", "rank", "drains"])?;
        let by = match fields.get("by").map(|by| by.as_str()) {
            None => {
                self.problem(at.clone(), name, "`by` is missing: weigh by process_time, interval, starting_mus or capacity");
                None
            }
            Some(by) => {
                let found = by.and_then(|by| WEIGHABLE.into_iter().find(|attribute| attribute.name() == by));
                if found.is_none() {
                    let guess = by.map_or(String::new(), |by| crate::suggest::did_you_mean(by, WEIGHABLE.iter().map(|attribute| attribute.name())));
                    self.problem(format!("{at}/by"), name, format!("weigh by process_time, interval, starting_mus or capacity{guess}"));
                }
                found
            }
        };
        let mut choice = |key: &str, options: [&str; 2]| match fields.get(key) {
            None => Some(0),
            Some(value) => {
                let found = value.as_str().and_then(|text| options.iter().position(|option| *option == text));
                if found.is_none() {
                    self.problem(format!("{at}/{key}"), name, format!("`{key}` is `{}` or `{}`", options[0], options[1]));
                }
                found
            }
        };
        let order = choice("order", ["smallest", "largest"]);
        let rank = choice("rank", ["route", "successor"]);
        let drains = match fields.get("drains") {
            None => Some(None),
            Some(Value::Array(names)) if names.iter().all(Value::is_string) => {
                if names.is_empty() {
                    self.problem(format!("{at}/drains"), name, "a list of drains can't be empty: leave `drains` out to count every Drain");
                    None
                } else {
                    Some(Some(names.iter().filter_map(|drain| drain.as_str().map(str::to_owned)).collect()))
                }
            }
            Some(_) => {
                self.problem(format!("{at}/drains"), name, "`drains` is a list of Drain names, such as [\"Out\"]");
                None
            }
        };
        use crabsim::distribution::{Order, Rank};
        Some(DistributionStrategy::Weighted(WeightedStrategy {
            by: by?,
            order: if order? == 0 { Order::Smallest } else { Order::Largest },
            rank: if rank? == 0 { Rank::Route } else { Rank::Successor },
            drains: drains?,
        }))
    }

    /// `{"function": "<module>.<function>", "with": {…}}`, `with` optional when the function has
    /// no parameters. Whether the module has the function, and the values fit its parameters, is
    /// checked against the code (`Model::code_problems`).
    fn code_call(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<crate::CodeCall> {
        let fields = self.object(value, pointer, name, "a call of code", &["function", "with"])?;
        let function = match fields.get("function") {
            None => {
                self.problem(pointer.to_owned(), name, "`function` is missing: name the code's function as \"<module>.<function>\"");
                None
            }
            Some(Value::String(function)) if crate::is_function_name(function) => Some(function.clone()),
            Some(other) => {
                self.problem(format!("{pointer}/function"), name, format!("{other} isn't a function's name: write \"<module>.<function>\", such as \"routing.every_nth\""));
                None
            }
        };
        let with = match fields.get("with") {
            None => Some(Map::new()),
            Some(Value::Object(with)) => Some(with.clone()),
            Some(_) => {
                self.problem(format!("{pointer}/with"), name, "`with` holds the function's parameters by name, such as {\"n\": 3}");
                None
            }
        };
        Some(crate::CodeCall { function: function?, with: with? })
    }

    /// `{"code": {…}, "at": ["front", "booking_point", "rear"]}`, `at` optional and the booking
    /// point alone by default.
    fn code_hook(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<crate::CodeHook> {
        let fields = self.object(value, pointer, name, "a hook", &["code", "at"])?;
        let code = match fields.get("code") {
            None => {
                self.problem(pointer.to_owned(), name, "`code` is missing: a hook is {\"code\": {\"function\": \"<module>.<function>\"}}");
                None
            }
            Some(code) => self.code_call(code, &format!("{pointer}/code"), name),
        };
        let at = match fields.get("at") {
            None => Some(vec![crabsim::TriggerPoint::BookingPoint]),
            Some(Value::Array(points)) if !points.is_empty() => {
                let read: Vec<Option<crabsim::TriggerPoint>> = points.iter().map(|point| point.as_str().and_then(crate::trigger_point_from_name)).collect();
                if read.iter().any(Option::is_none) {
                    self.problem(format!("{pointer}/at"), name, "a trigger point is \"front\", \"booking_point\" or \"rear\"");
                    None
                } else {
                    // Front first, each once.
                    Some(crabsim::TriggerPoint::ALL.into_iter().filter(|point| read.contains(&Some(*point))).collect())
                }
            }
            Some(_) => {
                self.problem(format!("{pointer}/at"), name, "`at` is a list of trigger points, such as [\"front\", \"rear\"]");
                None
            }
        };
        Some(crate::CodeHook { code: code?, at: at? })
    }

    /// `{"code": {…}, "default": <strategy>, "asked_on_entry": "booking_point"}`, the last two
    /// optional.
    fn code_distribution(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<DistributionStrategy> {
        let fields = self.object(value, pointer, name, "a distribution by code", &["code", "default", "asked_on_entry"])?;
        let code = self.code_call(fields.get("code").expect("looked for"), &format!("{pointer}/code"), name);
        let default = match fields.get("default") {
            None => Some(DistributionStrategy::RoundRobin),
            Some(default) => match self.distribution(default, &format!("{pointer}/default"), name)? {
                DistributionStrategy::Code(_) => {
                    self.problem(format!("{pointer}/default"), name, "code's default is a built-in strategy: \"round_robin\", {\"weighted\": …} or {\"by_destination\": …}");
                    None
                }
                strategy => Some(strategy),
            },
        };
        let asked_on_entry = match fields.get("asked_on_entry") {
            None => Some(None),
            Some(point) => match point.as_str().and_then(crate::trigger_point_from_name) {
                Some(point) => Some(Some(point)),
                None => {
                    self.problem(format!("{pointer}/asked_on_entry"), name, "`asked_on_entry` is a trigger point: \"front\", \"booking_point\" or \"rear\"");
                    None
                }
            },
        };
        Some(DistributionStrategy::Code(Box::new(crate::CodeDistribution { code: code?, default: default?, asked_on_entry: asked_on_entry? })))
    }

    /// `{"rules": ["when … then …", …], "default": …}` (ADR 0030): each line read, its names
    /// checked against the model by `Model::rule_problems`.
    fn rules_distribution(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<DistributionStrategy> {
        let fields = self.object(value, pointer, name, "a distribution by rules", &["rules", "default"])?;
        let at = format!("{pointer}/rules");
        let lines = match fields.get("rules") {
            Some(Value::Array(lines)) if !lines.is_empty() => {
                let mut read = Vec::new();
                for (place, line) in lines.iter().enumerate() {
                    let Some(text) = self.text(line, &format!("{at}/{place}"), name) else { continue };
                    match crate::rules::Line::parse(&text) {
                        Ok(line) => read.push(line.text()),
                        Err(message) => self.problem(format!("{at}/{place}"), name, message),
                    }
                }
                read
            }
            _ => {
                self.problem(at, name, "`rules` is a list of lines, at least one, such as [\"when class = Frame then to Line A\", \"otherwise default\"]");
                return None;
            }
        };
        let default = match fields.get("default") {
            None => Some(DistributionStrategy::RoundRobin),
            Some(default) => match self.distribution(default, &format!("{pointer}/default"), name)? {
                DistributionStrategy::Code(_) | DistributionStrategy::Rules(_) => {
                    self.problem(format!("{pointer}/default"), name, "rules' default is a built-in strategy: \"round_robin\", {\"weighted\": …} or {\"by_destination\": …}");
                    None
                }
                strategy => Some(strategy),
            },
        };
        Some(DistributionStrategy::Rules(Box::new(crate::rules::RuleDistribution { lines, default: default? })))
    }

    /// A Station's process time by MU, `{"rules": ["when … then 30s", …], "default": "20"}`
    /// (ADR 0030): each line read, its names checked against the model by `Model::rule_problems`,
    /// and the Station's own time, which a line's `default` and no line holding fall back on.
    fn process_time_rules(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<(Vec<String>, SimTime)> {
        let fields = self.object(value, pointer, name, "a process time by rules", &["rules", "default"])?;
        let at = format!("{pointer}/rules");
        let lines = match fields.get("rules") {
            Some(Value::Array(lines)) if !lines.is_empty() => {
                let mut read = Vec::new();
                for (place, line) in lines.iter().enumerate() {
                    let Some(text) = self.text(line, &format!("{at}/{place}"), name) else { continue };
                    match crate::rules::TimeLine::parse(&text) {
                        Ok(line) => read.push(line.text()),
                        Err(message) => self.problem(format!("{at}/{place}"), name, message),
                    }
                }
                Some(read)
            }
            _ => {
                self.problem(at, name, "`rules` is a list of lines, at least one, such as [\"when class = Frame then 30s\", \"when step = Deburr then 50s\"]");
                None
            }
        };
        let default = match fields.get("default") {
            Some(default) => self.time(default, &format!("{pointer}/default"), name),
            None => {
                self.problem(format!("{pointer}/default"), name, "`default` is the Station's own time, for MUs no line gives one, such as \"20\"");
                None
            }
        };
        Some((lines?, default?))
    }

    /// `{"before_init": {"code": {…}}, "after_init": …, "before_reset": …, "after_reset": …}`.
    fn model_hooks(&mut self, value: &Value) -> crate::ModelHooks {
        let mut hooks = crate::ModelHooks::default();
        let Some(fields) = self.object(value, "/hooks", None, "the model's hooks", &["before_init", "after_init", "before_reset", "after_reset"]) else {
            return hooks;
        };
        for (key, hook) in fields {
            let pointer = format!("/hooks/{key}");
            let Some(hook_fields) = self.object(hook, &pointer, None, "a hook", &["code"]) else { continue };
            let Some(code) = hook_fields.get("code") else {
                self.problem(pointer, None, "`code` is missing: a hook is {\"code\": {\"function\": \"<module>.<function>\"}}");
                continue;
            };
            let call = self.code_call(code, &format!("{pointer}/code"), None);
            match key.as_str() {
                "before_init" => hooks.before_init = call,
                "after_init" => hooks.after_init = call,
                "before_reset" => hooks.before_reset = call,
                "after_reset" => hooks.after_reset = call,
                _ => {}
            }
        }
        hooks
    }

    /// Notes by state. A key that isn't a state, or a note that isn't text, is a problem, and the
    /// notes that can be read are kept.
    fn state_notes(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<StateNotes> {
        let known: Vec<&str> = State::ALL.iter().map(|state| state.name()).collect();
        let Some(fields) = value.as_object() else {
            self.problem(pointer.to_owned(), name, r#"state notes should be an object, such as {"blocked": "waits for the conveyor"}"#);
            return None;
        };
        let mut notes = StateNotes::new();
        for (key, note) in fields {
            let at = format!("{pointer}/{}", escape(key));
            match State::from_name(key) {
                None => {
                    let guess = crate::suggest::did_you_mean(key, known.iter().copied());
                    self.problem(at, name, format!("unknown state `{key}`{guess}"));
                }
                Some(state) => {
                    if let Some(note) = self.text(note, &at, name) {
                        notes.insert(state, note);
                    }
                }
            }
        }
        Some(notes)
    }

    fn dimensions(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Dimensions> {
        let fields = self.object(value, pointer, name, "dimensions", &["length", "width", "height", "rotation_degrees"])?;
        let length = self.number(fields, "length", pointer, name);
        let width = self.number(fields, "width", pointer, name);
        let height = self.number(fields, "height", pointer, name);
        let rotation_degrees =
            if fields.contains_key("rotation_degrees") { self.number(fields, "rotation_degrees", pointer, name) } else { Some(0.0) };
        Some(Dimensions { length: length?, width: width?, height: height?, rotation_degrees: rotation_degrees? })
    }

    /// A list of named profiles, or the single object older files have, read as a list of one.
    fn failure_list(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Vec<Failures>> {
        let Some(items) = value.as_array() else {
            return self.failures(value, pointer, name, false).map(|failures| vec![failures]);
        };
        let mut all = Vec::new();
        let mut ok = true;
        for (index, item) in items.iter().enumerate() {
            match self.failures(item, &format!("{pointer}/{index}"), name, true) {
                Some(failures) => all.push(failures),
                None => ok = false,
            }
        }
        ok.then_some(all)
    }

    fn failures(&mut self, value: &Value, pointer: &str, name: Option<&str>, named: bool) -> Option<Failures> {
        let known: &[&str] = if named { &["name", "stream", "time_basis", "availability", "repair_time"] } else { &["availability", "repair_time"] };
        let fields = self.object(value, pointer, name, "failures", known)?;
        let profile_name = if named {
            match fields.get("name") {
                None => {
                    self.problem(pointer.to_owned(), name, "a failure profile needs a `name`");
                    None
                }
                Some(value) => self.text(value, &format!("{pointer}/name"), name),
            }
        } else {
            Some(crate::DEFAULT_FAILURES_NAME.to_owned())
        };
        let stream = match fields.get("stream") {
            None => Some(None),
            Some(stream) => match stream.as_u64() {
                Some(stream) => Some(Some(stream)),
                None => {
                    self.problem(format!("{pointer}/stream"), name, format!("{stream} is not a stream: write a whole number, 0 or more"));
                    None
                }
            },
        };
        let time_basis = match fields.get("time_basis") {
            None => Some(crate::TimeBasis::Simulation),
            Some(value) => match value.as_str().and_then(crate::time_basis_named) {
                Some(basis) => Some(basis),
                None => {
                    self.problem(format!("{pointer}/time_basis"), name, format!("{value} is not a time basis: use \"simulation\", \"working\" or \"processing\""));
                    None
                }
            },
        };
        let availability = self.number(fields, "availability", pointer, name);
        let repair_time = match fields.get("repair_time") {
            None => {
                self.problem(pointer.to_owned(), name, "`repair_time` is missing");
                None
            }
            Some(value) => self.repair_time(value, &format!("{pointer}/repair_time"), name),
        };
        Some(Failures { name: profile_name?, stream: stream?, time_basis: time_basis?, availability: availability?, repair_time: repair_time? })
    }

    fn repair_time(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<RepairTime> {
        const KINDS: [&str; 4] = ["constant", "uniform", "normal", "erlang"];
        let fields = self.object(value, pointer, name, "a repair time", &KINDS)?;
        let mut kinds = fields.iter().filter(|(key, _)| KINDS.contains(&key.as_str()));
        let (Some((kind, value)), None) = (kinds.next(), kinds.next()) else {
            self.problem(pointer.to_owned(), name, "a repair time should be exactly one of `constant`, `uniform`, `normal` or `erlang`");
            return None;
        };
        let at = format!("{pointer}/{kind}");
        match kind.as_str() {
            "constant" => self.time(value, &at, name).map(RepairTime::Constant),
            "uniform" => {
                let fields = self.object(value, &at, name, "a uniform repair time", &["min", "max"])?;
                let min = self.time_field(fields, "min", &at, name);
                let max = self.time_field(fields, "max", &at, name);
                Some(RepairTime::Uniform { min: min?, max: max? })
            }
            "normal" => {
                let fields = self.object(value, &at, name, "a normal repair time", &["mean", "std"])?;
                let mean = self.time_field(fields, "mean", &at, name);
                let std = self.time_field(fields, "std", &at, name);
                Some(RepairTime::Normal { mean: mean?, std: std? })
            }
            _ => {
                let fields = self.object(value, &at, name, "an Erlang repair time", &["mean", "k"])?;
                let mean = self.time_field(fields, "mean", &at, name);
                let k = match fields.get("k") {
                    None => {
                        self.problem(at.clone(), name, "`k` is missing");
                        None
                    }
                    Some(value) => self.count(value, &format!("{at}/k"), name),
                };
                Some(RepairTime::Erlang { mean: mean?, k: k? })
            }
        }
    }

    fn time_field(&mut self, fields: &Map<String, Value>, key: &str, pointer: &str, name: Option<&str>) -> Option<SimTime> {
        match fields.get(key) {
            None => {
                self.problem(pointer.to_owned(), name, format!("`{key}` is missing"));
                None
            }
            Some(value) => self.time(value, &format!("{pointer}/{key}"), name),
        }
    }
}

/// What to say about a class named like a built-in class that names a parent.
pub(crate) fn refinement_with_parent(name: &str) -> String {
    format!("a class named `{name}` is this model's own values for the built-in class, so it has no parent")
}
