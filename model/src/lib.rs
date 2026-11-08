//! crabsim's model files: a model as readable JSON that people, agents and scripts can write, with
//! classes and inheritance, validation that reports every problem at once, one canonical form for
//! saving, and building into the engine's `ModelDescription`. The format is described in
//! `docs/design/model-file.md`, and why it lives in its own crate in ADR 0009.
//!
//! ```no_run
//! let text = std::fs::read_to_string("models/demo.json").unwrap();
//! let model = crabsim_model::load(&text).expect("a valid model");
//! let mut run = model.to_description().build(7);
//! ```

mod build;
mod check;
mod read;
pub mod schema;
mod suggest;
mod write;

use crabsim::{Dimensions, ObjectKind, Position, SimTime};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt;

/// The only format version this crabsim reads and writes.
pub const FORMAT_VERSION: u64 = 1;

/// The built-in classes every chain of classes ends in.
pub const BUILT_IN_CLASSES: [&str; 4] = ["Source", "Station", "Buffer", "Drain"];

/// A model, as a file holds it: valid once `load` or `check` finds no problem.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// The `$schema` a file names, for editors. crabsim keeps it and otherwise ignores it.
    pub schema: Option<String>,
    /// About the model as a whole.
    pub description: Option<String>,
    /// The model's own classes, in any order.
    pub classes: Vec<Class>,
    /// In Init order: an instance's id is its place here.
    pub instances: Vec<Instance>,
}

/// A class of the user's own, derived from a built-in class or another derived class.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    /// Unique among classes, and not a built-in class's name.
    pub name: String,
    /// The class it derives from, built-in or derived.
    pub parent: String,
    /// Only the values this class changes from its parent.
    pub attributes: Attributes,
}

impl Class {
    pub fn new(name: impl Into<String>, parent: impl Into<String>) -> Class {
        Class { name: name.into(), parent: parent.into(), attributes: Attributes::default() }
    }
}

/// One material-flow object in a model.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Unique among instances.
    pub name: String,
    /// A built-in class or one of the model's own.
    pub class: String,
    /// Its stream: with a run's seed, it gives the instance's random numbers. It's given when the
    /// instance is placed and stays, whatever else changes in the model, so runs of two variants
    /// with the same seed draw the same numbers for it. Unique in the model. `load` gives every
    /// instance one: see [`Model::assign_streams`].
    pub stream: Option<u64>,
    /// The centre of its footprint, in metres. `None` leaves placing it to a view.
    pub position: Option<Position>,
    /// The instances it passes MUs to, by name, in the order they are tried.
    pub successors: Vec<String>,
    /// Only the values set on this instance, where its inheritance is cut. Anything else comes
    /// from its class.
    pub attributes: Attributes,
}

impl Instance {
    /// An instance of `class` called `name`, with nothing set on it.
    pub fn new(name: impl Into<String>, class: impl Into<String>) -> Instance {
        Instance { name: name.into(), class: class.into(), stream: None, position: None, successors: Vec::new(), attributes: Attributes::default() }
    }
}

/// The attributes a class or instance can set. `None` means not set here, so inherited.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Attributes {
    /// A Source's time between creating an MU and offering it on. Must be set somewhere.
    pub interval: Option<SimTime>,
    /// A Station's time working on each MU. 0 by default: a dummy Station.
    pub process_time: Option<SimTime>,
    /// A Station's random failures. None by default: it never fails.
    pub failures: Option<Failures>,
    /// How many MUs a Station holds from Init. 0 by default, at most 1 for now.
    pub starting_mus: Option<u64>,
    /// How many MUs a Buffer holds. Must be set somewhere, at least 1.
    pub capacity: Option<u64>,
    /// The object's size and rotation. Visual only. A default size by type.
    pub dimensions: Option<Dimensions>,
    /// Free text saying what the object is. Visual only.
    pub description: Option<String>,
}

impl Attributes {
    /// The attributes set here, as a file writes them, such as `{"process_time": "00:00:45"}`.
    pub fn to_json(&self) -> serde_json::Value {
        write::attributes(self)
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Attributes::default()
    }

    /// Whether this attribute is set here.
    pub fn is_set(&self, attribute: Attribute) -> bool {
        match attribute {
            Attribute::Interval => self.interval.is_some(),
            Attribute::ProcessTime => self.process_time.is_some(),
            Attribute::Failures => self.failures.is_some(),
            Attribute::StartingMus => self.starting_mus.is_some(),
            Attribute::Capacity => self.capacity.is_some(),
            Attribute::Dimensions => self.dimensions.is_some(),
            Attribute::Description => self.description.is_some(),
        }
    }

    /// Takes this attribute's value from `from`.
    fn take(&mut self, attribute: Attribute, from: &Attributes) {
        match attribute {
            Attribute::Interval => self.interval = from.interval,
            Attribute::ProcessTime => self.process_time = from.process_time,
            Attribute::Failures => self.failures = from.failures.clone(),
            Attribute::StartingMus => self.starting_mus = from.starting_mus,
            Attribute::Capacity => self.capacity = from.capacity,
            Attribute::Dimensions => self.dimensions = from.dimensions,
            Attribute::Description => self.description = from.description.clone(),
        }
    }
}

/// Every attribute, by name as a file writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attribute {
    Interval,
    ProcessTime,
    Failures,
    StartingMus,
    Capacity,
    Dimensions,
    Description,
}

impl Attribute {
    pub const ALL: [Attribute; 7] = [
        Attribute::Interval,
        Attribute::ProcessTime,
        Attribute::Failures,
        Attribute::StartingMus,
        Attribute::Capacity,
        Attribute::Dimensions,
        Attribute::Description,
    ];

    /// Its name in a file.
    pub fn name(self) -> &'static str {
        match self {
            Attribute::Interval => "interval",
            Attribute::ProcessTime => "process_time",
            Attribute::Failures => "failures",
            Attribute::StartingMus => "starting_mus",
            Attribute::Capacity => "capacity",
            Attribute::Dimensions => "dimensions",
            Attribute::Description => "description",
        }
    }

    /// Whether instances of this built-in class have it.
    pub fn applies_to(self, built_in: &str) -> bool {
        matches!(
            (built_in, self),
            (_, Attribute::Dimensions | Attribute::Description)
                | ("Source", Attribute::Interval)
                | ("Station", Attribute::ProcessTime | Attribute::Failures | Attribute::StartingMus)
                | ("Buffer", Attribute::Capacity)
        )
    }
}

/// Random failures: how much of the time the object is available, and how long a repair takes.
/// The time between failures follows from the two.
#[derive(Debug, Clone, PartialEq)]
pub struct Failures {
    /// Strictly between 0 and 1.
    pub availability: f64,
    pub repair_time: RepairTime,
}

/// How long a repair takes.
#[derive(Debug, Clone, PartialEq)]
pub enum RepairTime {
    Constant(SimTime),
    Uniform { min: SimTime, max: SimTime },
    Normal { mean: SimTime, std: SimTime },
    Erlang { mean: SimTime, k: u64 },
}

/// Where an instance's value of an attribute comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Set on the instance itself, which cuts its inheritance.
    Instance,
    /// Inherited from this class, the nearest in its chain that sets it.
    Class(String),
    /// Set nowhere in its chain, so the built-in class's default: a value, or none.
    BuiltIn,
}

/// An instance's values, worked out from its chain of classes, and where each comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The built-in class at the root of its chain.
    pub built_in: &'static str,
    /// Every value the instance has, with built-in defaults filled in.
    pub attributes: Attributes,
    origins: Vec<(Attribute, Origin)>,
}

impl Resolved {
    pub fn origin(&self, attribute: Attribute) -> &Origin {
        &self.origins.iter().find(|(a, _)| *a == attribute).expect("every attribute has an origin").1
    }
}

/// One thing wrong with a model file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// A JSON Pointer to where it is in the file, such as `/instances/3/attributes/process_time`.
    /// Empty for the file as a whole.
    pub pointer: String,
    /// The instance or class it is about, when there is one.
    pub name: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pointer = if self.pointer.is_empty() { "/" } else { &self.pointer };
        match &self.name {
            Some(name) => write!(f, "{pointer} \"{name}\": {}", self.message),
            None => write!(f, "{pointer}: {}", self.message),
        }
    }
}

/// Reads a model file, returning every problem in it, in file order, if there are any.
pub fn load(text: &str) -> Result<Model, Vec<Problem>> {
    let (mut model, mut problems, broken) = read::read(text);
    model.assign_streams();
    problems.extend(check::check(&model, &broken));
    problems.sort_by(|a, b| order(&a.pointer).cmp(&order(&b.pointer)));
    if problems.is_empty() { Ok(model) } else { Err(problems) }
}

/// Orders pointers as they come in the file: by each part, numbers as numbers. Classes come
/// before instances, as a saved file writes them.
fn order(pointer: &str) -> Vec<(u64, String)> {
    pointer
        .split('/')
        .map(|part| match part {
            "classes" => (0, String::new()),
            "instances" => (1, String::new()),
            _ => part.parse::<u64>().map_or((u64::MAX, part.to_string()), |n| (n, String::new())),
        })
        .collect()
}

impl Model {
    /// Every problem in a model built in code, as `load` would report it for the same model saved.
    pub fn check(&self) -> Vec<Problem> {
        check::check(self, &read::Broken::default())
    }

    /// The model in its one canonical form, as a file saves it, with every instance's stream.
    pub fn save(&self) -> String {
        let mut model = self.clone();
        model.assign_streams();
        write::write(&model)
    }

    /// Gives every instance without a stream one more than the highest stream in the model, in
    /// file order, starting from 0 if none has one. So a file from before streams numbers its
    /// instances by place, which is how their random numbers were drawn then, and a new instance
    /// never takes another's stream. Removing an instance leaves a gap.
    pub fn assign_streams(&mut self) {
        let first = self.instances.iter().filter_map(|instance| instance.stream).max().map_or(0, |highest| highest + 1);
        for (offset, instance) in self.instances.iter_mut().filter(|instance| instance.stream.is_none()).enumerate() {
            instance.stream = Some(first + offset as u64);
        }
    }

    /// The engine's description of this model, ready to build runs from, with every instance's
    /// values worked out from its classes. The model must be valid: `load` only gives valid
    /// models, and one built in code should pass `check` first.
    ///
    /// # Panics
    /// If the model has problems, which `check` would have reported.
    pub fn to_description(&self) -> crabsim::ModelDescription {
        let problems = self.check();
        assert!(problems.is_empty(), "the model has problems: {problems:?}");
        build::build(self)
    }

    pub fn class(&self, name: &str) -> Option<&Class> {
        self.classes.iter().find(|class| class.name == name)
    }

    /// The derived classes from `class` up to its built-in class, nearest first, and that built-in
    /// class. `None` if the chain reaches an unknown class or goes round in a circle.
    pub fn chain(&self, class: &str) -> Option<(Vec<&Class>, &'static str)> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut at = class;
        loop {
            if let Some(built_in) = BUILT_IN_CLASSES.iter().find(|&&built_in| built_in == at) {
                return Some((chain, built_in));
            }
            if !seen.insert(at) {
                return None;
            }
            let class = self.class(at)?;
            chain.push(class);
            at = &class.parent;
        }
    }

    /// The values of the instance at `index`, worked out from its chain of classes, and where
    /// each comes from. `None` if its class's chain is broken, which `check` reports.
    pub fn resolve(&self, index: usize) -> Option<Resolved> {
        let instance = self.instances.get(index)?;
        self.resolve_chain(Some(&instance.attributes), &instance.class)
    }

    /// The values a class gives its instances, worked out from its own chain, and where each
    /// comes from: from a class, the class itself included, or built in. `name` may be a built-in
    /// class. `None` if there is no such class or its chain is broken.
    pub fn resolve_class(&self, name: &str) -> Option<Resolved> {
        self.resolve_chain(None, name)
    }

    /// The values of something with `own` attributes, if it's an instance, made from `class`.
    fn resolve_chain(&self, own: Option<&Attributes>, class: &str) -> Option<Resolved> {
        let (chain, built_in) = self.chain(class)?;
        let mut attributes = Attributes::default();
        let mut origins = Vec::new();
        for attribute in Attribute::ALL {
            let origin = if let Some(own) = own.filter(|own| own.is_set(attribute)) {
                attributes.take(attribute, own);
                Origin::Instance
            } else if let Some(class) = chain.iter().find(|class| class.attributes.is_set(attribute)) {
                attributes.take(attribute, &class.attributes);
                Origin::Class(class.name.clone())
            } else {
                Origin::BuiltIn
            };
            origins.push((attribute, origin));
        }
        // Built-in defaults, where there are any.
        if built_in == "Station" {
            attributes.process_time.get_or_insert(SimTime::ZERO);
            attributes.starting_mus.get_or_insert(0);
        }
        let kind = match built_in {
            "Source" => ObjectKind::Source,
            "Station" => ObjectKind::Station,
            "Buffer" => ObjectKind::Buffer,
            _ => ObjectKind::Drain,
        };
        attributes.dimensions.get_or_insert(Dimensions::default_for(kind));
        Some(Resolved { built_in, attributes, origins })
    }
}

/// Escapes a key for a JSON Pointer.
pub(crate) fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
