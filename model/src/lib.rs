//! crabsim's model files: a model as readable JSON that people, agents and scripts can write, with
//! validation that reports every problem at once, one canonical form for saving, and building into
//! the engine's `ModelDescription`. The format is described in `docs/design/model-file.md`, and
//! why it lives in its own crate in ADR 0009.
//!
//! ```no_run
//! let text = std::fs::read_to_string("models/demo.json").unwrap();
//! let model = crabsim_model::load(&text).expect("a valid model");
//! let mut run = model.to_description().build(7);
//! ```

mod build;
mod check;
mod read;
mod write;

use crabsim::{Dimensions, Position, SimTime};
use serde::Serialize;
use std::fmt;

/// The only format version this crabsim reads and writes.
pub const FORMAT_VERSION: u64 = 1;

/// The built-in classes every instance's chain of classes ends in.
pub const BUILT_IN_CLASSES: [&str; 4] = ["Source", "Station", "Buffer", "Drain"];

/// A model, as a file holds it: valid once `load` or `check` finds no problem.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// About the model as a whole.
    pub description: Option<String>,
    /// In Init order: an instance's id is its place here.
    pub instances: Vec<Instance>,
}

/// One material-flow object in a model.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Unique among instances.
    pub name: String,
    /// One of `BUILT_IN_CLASSES`, until derived classes exist.
    pub class: String,
    /// The centre of its footprint, in metres. `None` leaves placing it to a view.
    pub position: Option<Position>,
    /// The instances it passes MUs to, by name, in the order they are tried.
    pub successors: Vec<String>,
    /// Only the values set on this instance. Anything else comes from its class.
    pub attributes: Attributes,
}

impl Instance {
    /// An instance of `class` called `name`, with nothing set on it.
    pub fn new(name: impl Into<String>, class: impl Into<String>) -> Instance {
        Instance { name: name.into(), class: class.into(), position: None, successors: Vec::new(), attributes: Attributes::default() }
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
    pub(crate) fn is_empty(&self) -> bool {
        *self == Attributes::default()
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
    let (model, mut problems, broken) = read::read(text);
    problems.extend(check::check(&model, &broken));
    problems.sort_by(|a, b| order(&a.pointer).cmp(&order(&b.pointer)));
    if problems.is_empty() { Ok(model) } else { Err(problems) }
}

/// Orders pointers as they come in the file: by each part, numbers as numbers.
fn order(pointer: &str) -> Vec<(u64, String)> {
    pointer.split('/').map(|part| part.parse::<u64>().map_or((u64::MAX, part.to_string()), |n| (n, String::new()))).collect()
}

impl Model {
    /// Every problem in a model built in code, as `load` would report it for the same model saved.
    pub fn check(&self) -> Vec<Problem> {
        check::check(self, &read::Broken::default())
    }

    /// The model in its one canonical form, as a file saves it.
    pub fn save(&self) -> String {
        write::write(self)
    }

    /// The engine's description of this model, ready to build runs from. The model must be
    /// valid: `load` only gives valid models, and one built in code should pass `check` first.
    ///
    /// # Panics
    /// If the model has problems, which `check` would have reported.
    pub fn to_description(&self) -> crabsim::ModelDescription {
        let problems = self.check();
        assert!(problems.is_empty(), "the model has problems: {problems:?}");
        build::build(self)
    }
}

/// Escapes a key for a JSON Pointer.
pub(crate) fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
