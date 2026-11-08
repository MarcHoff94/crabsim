//! Reading a file's JSON into a `Model`, noting every problem with its shape as it goes rather than
//! stopping at the first. What can't be read is left at its default, and noted in `Broken`, so the
//! checks that follow don't report it a second time.

use crate::{Attributes, Class, Failures, Instance, Model, Problem, RepairTime, FORMAT_VERSION, escape};
use crabsim::time::parse_time;
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
    /// Instances whose successors couldn't all be read.
    pub successors: HashSet<usize>,
    /// Classes, by index, whose name couldn't be read.
    pub class_names: HashSet<usize>,
    /// Classes whose parent couldn't be read.
    pub class_parents: HashSet<usize>,
    /// Attributes of classes that were there but couldn't be read.
    pub class_attributes: HashSet<(usize, &'static str)>,
}

const TOP_LEVEL: [&str; 5] = ["$schema", "format_version", "description", "classes", "instances"];
const INSTANCE: [&str; 6] = ["name", "class", "stream", "position", "successors", "attributes"];
const CLASS: [&str; 3] = ["name", "parent", "attributes"];
const ATTRIBUTES: [&str; 7] = ["interval", "process_time", "failures", "starting_mus", "capacity", "dimensions", "description"];

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
        Model { schema, description, classes, instances }
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
        if let Some(successors) = fields.get("successors") {
            match successors.as_array() {
                Some(list) => {
                    for (place, successor) in list.iter().enumerate() {
                        match successor.as_str() {
                            Some(successor) => instance.successors.push(successor.to_owned()),
                            None => {
                                self.problem(format!("{pointer}/successors/{place}"), name, "a successor should be an instance's name");
                                self.broken.successors.insert(index);
                            }
                        }
                    }
                }
                None => {
                    self.problem(format!("{pointer}/successors"), name, "`successors` should be a list of names");
                    self.broken.successors.insert(index);
                }
            }
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
                Some(Some(text)) => {
                    if key == "name" { class.name = text.to_owned() } else { class.parent = text.to_owned() }
                    continue;
                }
                Some(None) => (format!("{pointer}/{key}"), format!("`{key}` should be text")),
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
                "process_time" => self.time(value, &at, name).map(|time| attributes.process_time = Some(time)),
                "failures" => self.failures(value, &at, name).map(|failures| attributes.failures = Some(failures)),
                "starting_mus" => self.count(value, &at, name).map(|count| attributes.starting_mus = Some(count)),
                "capacity" => self.count(value, &at, name).map(|count| attributes.capacity = Some(count)),
                "dimensions" => self.dimensions(value, &at, name).map(|dimensions| attributes.dimensions = Some(dimensions)),
                "description" => self.text(value, &at, name).map(|text| attributes.description = Some(text)),
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

    fn count(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<u64> {
        let count = value.as_u64();
        if count.is_none() {
            self.problem(pointer.to_owned(), name, format!("{value} should be a whole number, 0 or more"));
        }
        count
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

    fn position(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Position> {
        let fields = self.object(value, pointer, name, "a position", &["x", "y", "z"])?;
        let x = self.number(fields, "x", pointer, name);
        let y = self.number(fields, "y", pointer, name);
        let z = if fields.contains_key("z") { self.number(fields, "z", pointer, name) } else { Some(0.0) };
        Some(Position { x: x?, y: y?, z: z? })
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

    fn failures(&mut self, value: &Value, pointer: &str, name: Option<&str>) -> Option<Failures> {
        let fields = self.object(value, pointer, name, "failures", &["availability", "repair_time"])?;
        let availability = self.number(fields, "availability", pointer, name);
        let repair_time = match fields.get("repair_time") {
            None => {
                self.problem(pointer.to_owned(), name, "`repair_time` is missing");
                None
            }
            Some(value) => self.repair_time(value, &format!("{pointer}/repair_time"), name),
        };
        Some(Failures { availability: availability?, repair_time: repair_time? })
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
