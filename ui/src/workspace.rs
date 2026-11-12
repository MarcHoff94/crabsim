//! The workspace: what the window works on. A models folder, reached only through the operation
//! catalogue as the user (`Source::Ui`), its model files, the model open and the seed runs use.
//! Until a model is chosen, or when the folder can't be opened or another window has it, no model
//! is open. Plain Rust with no Bevy types, so it is tested without a window. See spec issue 63.
//!
//! `check` is the one seam for noticing that the open model file changed, such as by an agent: it
//! compares the file's size and modified time, and only when they differ reads it again through
//! the catalogue and compares its hash. A file watcher could take its place later.
//!
//! Building a model is a set of actions here, each exactly one `edit_model` batch, the unit a later
//! undo will step through (spec issue 87). The window offers them only with no run in progress, and
//! picks up what they saved through `check`, as it does an agent's change.

use crate::editing::{ActRefusal, EditAction, Editors};
use crate::view::{Change, DEFAULT_MEASURING_PERIOD};
use crabsim::{Clock, Dimensions, LiveSession, ModelDescription, ObjectKind, Position, SimTime};
use crabsim_catalogue::{Catalogue, ErrorKind, Settings, Source};
use crabsim_model::{Attribute, Model, BUILT_IN_CLASSES};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The demo model, saved in a models folder made on a first start. Built in, so a downloaded
/// program has it without the repository.
const DEMO: &str = include_str!("../../models/demo.json");

/// One class in the Classes panel, as a tree by inheritance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassEntry {
    pub name: String,
    /// The class it derives from, none for a built-in class.
    pub parent: Option<String>,
    /// 0 for a built-in class, 1 for one derived from it, and so on.
    pub depth: usize,
    pub built_in: bool,
}

/// One thing Validate found: a problem, which stops the model running, or a warning, which can
/// run but is probably a mistake.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub warning: bool,
    /// Where it is in the file, such as `/instances/3/attributes/process_time`.
    pub pointer: String,
    /// The instance or class it's about.
    pub name: Option<String>,
    pub message: String,
    /// The instance it's about, by id in the open model, when it's about one.
    pub instance: Option<u32>,
    /// The class it's about, when it's about one.
    pub class: Option<String>,
}

/// Which of a connection's two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// Its place among the connections out of its object.
    Exit,
    /// Its place among the connections into its successor.
    Entrance,
}

/// A connection as the inspector shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionView {
    pub from: String,
    pub to: String,
    pub exit: u64,
    /// How many connections leave `from`.
    pub exits: u64,
    pub entrance: u64,
    /// How many connections lead into `to`.
    pub entrances: u64,
    pub description: Option<String>,
}

impl ConnectionView {
    /// Such as "2nd of Station A's 3 exits".
    pub fn exit_text(&self) -> String {
        format!("{} of {}'s {} exit{}", ordinal(self.exit), self.from, self.exits, if self.exits == 1 { "" } else { "s" })
    }

    /// Such as "1st of Station B's 2 entrances".
    pub fn entrance_text(&self) -> String {
        format!("{} of {}'s {} entrance{}", ordinal(self.entrance), self.to, self.entrances, if self.entrances == 1 { "" } else { "s" })
    }
}

/// 1st, 2nd, 3rd, 4th, 11th, 21st and so on.
pub fn ordinal(n: u64) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// What removing a class takes with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dependents {
    /// The classes deriving from it, at any depth.
    pub classes: Vec<String>,
    /// The instances of it or of those classes.
    pub instances: Vec<String>,
}

impl Dependents {
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty() && self.instances.is_empty()
    }
}

/// Space left between objects the window places itself, in metres.
const GAP: f64 = 2.0;

/// One model file in the folder, as the list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelEntry {
    /// Relative to the models folder, such as `plant/line.json`.
    pub path: String,
    /// How many problems it has: one that has any can't run.
    pub problems: usize,
}

/// Why a model chosen can't run: `describe_model`'s summary of it, and every problem.
#[derive(Debug, Clone, PartialEq)]
pub struct ProblemReport {
    pub path: String,
    pub summary: Vec<String>,
    /// Each as `pointer "name": message`, in file order.
    pub problems: Vec<String>,
}

/// What changed in the open model file since it was last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelChange {
    /// It is a different model now, waiting to be used: see `apply_pending`.
    Changed,
    /// It has problems now, so it can't be used, and the model in use stays.
    NowHasProblems,
}

/// Where an instance's value of an attribute comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueFrom {
    /// Set on the instance itself, which cuts its inheritance.
    Instance,
    /// Inherited from this class, the nearest that sets it.
    Class(String),
    /// The built-in class's default.
    BuiltIn,
    /// Set nowhere, and with no default.
    NotSet,
}

/// One attribute of an instance, as the inspector shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeView {
    /// Its name in a model file, such as `process_time`.
    pub attribute: String,
    /// As a model file writes it, if it has one.
    pub value: Option<Value>,
    pub from: ValueFrom,
}

/// Why a change to the model file was refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// An agent holds the file: the message names it, its goal and what it's doing. A take-over for the
    /// one change goes through, and the agent keeps its right.
    NotEditor(String),
    /// The file was written outside crabsim, so it must be adopted first.
    ChangedOutside(String),
    /// The value doesn't fit the model file's rules, with their message.
    Invalid(String),
    /// No model file is open.
    NoModel,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotEditor(message) | Refusal::ChangedOutside(message) | Refusal::Invalid(message) => write!(f, "{message}"),
            Refusal::NoModel => write!(f, "no model file is open"),
        }
    }
}

/// A file's size and modified time, which change whenever it is written.
type Stamp = (u64, SystemTime);

fn stamp_of(file: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(file).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// What the workspace knows about the open model file as it last read it.
struct Read {
    stamp: Option<Stamp>,
    hash: String,
    changed_outside: bool,
}

pub struct Workspace {
    /// Absent without a folder, or when it couldn't be opened.
    catalogue: Option<Catalogue>,
    /// Why there is no folder when one was asked for.
    banner: Option<String>,
    models: Vec<ModelEntry>,
    /// The model the window runs, laid out. Empty until one is opened.
    description: ModelDescription,
    /// The same model as its file has it, with its classes and connections, for building.
    model: Model,
    /// Its path, until one is opened none.
    opened: Option<String>,
    /// The open file as last read.
    read: Option<Read>,
    /// The open file's model since it changed, not used yet.
    pending: Option<(ModelDescription, Model)>,
    problems: Option<ProblemReport>,
    seed: u64,
    /// How long the measuring periods of runs are, a setting of runs rather than of the model.
    measuring_period: SimTime,
}

impl Workspace {
    /// The workspace on `models`, a models folder, with no model open yet. A folder another window
    /// has, or that can't be opened, leaves the workspace empty, with a banner saying why.
    pub fn open(models: &Path) -> Workspace {
        let mut workspace = Workspace {
            catalogue: None,
            banner: None,
            models: Vec::new(),
            description: ModelDescription::new(),
            model: Model::default(),
            opened: None,
            read: None,
            pending: None,
            problems: None,
            seed: 0,
            measuring_period: DEFAULT_MEASURING_PERIOD,
        };
        match Catalogue::open(Settings::new(models, Source::Ui)) {
            Ok(catalogue) => {
                workspace.catalogue = Some(catalogue);
                workspace.refresh();
            }
            Err(error) if error.kind == ErrorKind::FolderInUse => {
                workspace.banner = Some("Another crabsim window has this models folder open. Open a copy of the folder to work on a variant.".into());
            }
            Err(error) => workspace.banner = Some(format!("The models folder {} can't be opened: {error}", models.display())),
        }
        workspace
    }

    /// The workspace on `models`, as `open`, but a folder that doesn't exist yet is made first
    /// with the demo model saved in it through the catalogue, so a first start has a model to run
    /// and crabsim counts it as its own. A folder that exists is opened as it is, so a demo the
    /// user deleted doesn't come back.
    pub fn open_or_create(models: &Path) -> Workspace {
        if models.exists() {
            return Workspace::open(models);
        }
        if let Err(error) = std::fs::create_dir_all(models) {
            let mut workspace = Workspace::open(models);
            workspace.banner = Some(format!("The models folder {} can't be made: {error}", models.display()));
            return workspace;
        }
        let mut workspace = Workspace::open(models);
        if let Some(catalogue) = &workspace.catalogue {
            if let Err(error) = catalogue.call("write_model", json!({ "path": "demo.json", "text": DEMO })) {
                workspace.banner = Some(format!("The demo model can't be saved in the new models folder: {error}"));
            }
            workspace.refresh();
        }
        workspace
    }

    /// Why the window has no models folder, when one was asked for.
    pub fn banner(&self) -> Option<&str> {
        self.banner.as_deref()
    }

    pub fn has_folder(&self) -> bool {
        self.catalogue.is_some()
    }

    /// The folder's model files, sorted, each with how many problems it has.
    pub fn models(&self) -> &[ModelEntry] {
        &self.models
    }

    /// Lists the folder's model files again.
    pub fn refresh(&mut self) {
        let Some(catalogue) = &self.catalogue else { return };
        let listed = match catalogue.call("list_models", json!({})) {
            Ok(listed) => listed,
            Err(error) => {
                self.banner = Some(format!("The model files can't be listed: {error}"));
                return;
            }
        };
        self.models = strings(&listed["models"])
            .into_iter()
            .map(|path| {
                let problems = catalogue
                    .call("validate_model", json!({ "path": path }))
                    .map_or(1, |checked| checked["problems"].as_array().map_or(0, Vec::len));
                ModelEntry { path, problems }
            })
            .collect();
    }

    /// Opens the model at `path` to run, laid out. One with problems isn't opened: they are kept
    /// for the window to show, and the model open before stays. Returns whether it opened.
    pub fn open_model(&mut self, path: &str) -> bool {
        match self.load(path) {
            Some((description, model, read)) => {
                self.description = description;
                self.model = model;
                self.opened = Some(path.to_string());
                self.read = Some(read);
                self.pending = None;
                self.problems = None;
                true
            }
            None => false,
        }
    }

    /// The model file at `path`, laid out, as its file has it, and what was read, or none with its
    /// problems kept.
    fn load(&mut self, path: &str) -> Option<(ModelDescription, Model, Read)> {
        let catalogue = self.catalogue.as_ref()?;
        let stamp = catalogue.folder().model_file(path).ok().and_then(|file| stamp_of(&file));
        let read = match catalogue.call("read_model", json!({ "path": path })) {
            Ok(read) => read,
            Err(error) => {
                // The window's font has no arrow, so a connection's name reads "In to Mill" there.
                self.problems = Some(ProblemReport { path: path.to_string(), summary: Vec::new(), problems: vec![error.to_string()] });
                return None;
            }
        };
        let text = read["text"].as_str().unwrap_or_default();
        let known = Read { stamp, hash: read["file_hash"].as_str().unwrap_or_default().to_string(), changed_outside: read["changed_outside"] == true };
        match crabsim_model::load(text) {
            Ok(model) => {
                let mut description = model.to_description();
                lay_out(&mut description);
                Some((description, model, known))
            }
            Err(problems) => {
                let summary = catalogue.call("describe_model", json!({ "path": path })).map_or_else(|error| vec![error.to_string()], |summary| summarise(&summary));
                self.problems = Some(ProblemReport { path: path.to_string(), summary, problems: problems.iter().map(|problem| problem.to_string().replace(" → ", " to ")).collect() });
                // What was read is kept, so the same broken file isn't reported again.
                if self.opened.as_deref() == Some(path) {
                    self.read = Some(known);
                }
                None
            }
        }
    }

    /// Whether the open model file changed since it was last read: cheap when it didn't, since it
    /// compares the file's size and modified time first. A different model waits in `pending`
    /// until the window uses it with `apply_pending`. Each change is reported once.
    pub fn check(&mut self) -> Option<ModelChange> {
        let path = self.opened.clone()?;
        let file = self.watched_file()?;
        let stamp = stamp_of(&file);
        let read = self.read.as_mut()?;
        if stamp.is_some() && read.stamp == stamp {
            return None;
        }
        read.stamp = stamp;
        let hash = self.catalogue.as_ref()?.call("read_model", json!({ "path": path })).ok().and_then(|read| read["file_hash"].as_str().map(str::to_string));
        if hash.as_deref() == Some(self.read.as_ref()?.hash.as_str()) {
            return None;
        }
        match self.load(&path) {
            Some((description, model, read)) => {
                self.read = Some(read);
                self.pending = Some((description, model));
                self.problems = None;
                Some(ModelChange::Changed)
            }
            None => {
                self.pending = None;
                Some(ModelChange::NowHasProblems)
            }
        }
    }

    /// Whether the open file's model changed and the window hasn't used it yet.
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Uses the changed model: `description` and `session` give it from now on.
    pub fn apply_pending(&mut self) {
        if let Some((description, model)) = self.pending.take() {
            self.description = description;
            self.model = model;
        }
    }

    /// Every attribute the instance's built-in class has, with its value and where it comes
    /// from, as `get_attributes` gives them.
    pub fn attributes(&self, instance: &str) -> Result<Vec<AttributeView>, String> {
        self.attributes_of(json!({ "instance": instance }))
    }

    /// Every attribute a class gives its instances, with its value and where it comes from: the
    /// class itself, a class it derives from, or built in. `class` may be a built-in class.
    pub fn class_attributes(&self, class: &str) -> Result<Vec<AttributeView>, String> {
        let mut values = self.attributes_of(json!({ "class": class }))?;
        // What the class sets itself is set here, as on an instance.
        for view in &mut values {
            if view.from == ValueFrom::Class(class.to_string()) {
                view.from = ValueFrom::Instance;
            }
        }
        Ok(values)
    }

    /// Sets an attribute on a class of the model's own, which every instance following it and
    /// not setting it itself then takes.
    pub fn set_class_attribute(&mut self, class: &str, attribute: &str, value: Value, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "set_attribute", "target": { "class": class }, "attribute": attribute, "value": value }), take_over)
    }

    /// Takes an attribute set on a class back to what its parent gives.
    pub fn restore_class_attribute(&mut self, class: &str, attribute: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "restore_inheritance", "target": { "class": class }, "attribute": attribute }), take_over)
    }

    fn attributes_of(&self, target: Value) -> Result<Vec<AttributeView>, String> {
        let path = self.opened.as_deref().ok_or("no model file is open")?;
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        let got = catalogue.call("get_attributes", json!({ "path": path, "target": target })).map_err(|error| error.to_string())?;
        Ok(got["attributes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|attribute| AttributeView {
                attribute: attribute["attribute"].as_str().unwrap_or_default().to_string(),
                value: attribute.get("value").cloned(),
                from: match attribute["from"].as_str() {
                    Some("instance") => ValueFrom::Instance,
                    Some("class") => ValueFrom::Class(attribute["class"].as_str().unwrap_or_default().to_string()),
                    Some("built_in") => ValueFrom::BuiltIn,
                    _ => ValueFrom::NotSet,
                },
            })
            .collect())
    }

    /// Sets an attribute on the instance, which cuts its inheritance, saving the model file.
    /// `take_over` makes the one change while an agent holds the editor right, which it keeps.
    /// The window picks up the saved file through `check`.
    pub fn set_attribute(&mut self, instance: &str, attribute: &str, value: Value, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "set_attribute", "target": { "instance": instance }, "attribute": attribute, "value": value }), take_over)
    }

    /// Saves values changed during the run to the model file, as Reset keeping changes does: each
    /// set on its instance, all in one change, so either all are saved or none is. `take_over`
    /// makes the change while an agent holds the editor right, which it keeps. Nothing to keep
    /// saves nothing.
    pub fn keep_changes(&mut self, changes: &[Change], take_over: bool) -> Result<(), Refusal> {
        if changes.is_empty() {
            return Ok(());
        }
        let batch = changes
            .iter()
            .map(|change| {
                json!({
                    "change": "set_attribute",
                    "target": { "instance": change.name },
                    "attribute": change.attribute,
                    "value": crabsim::time::format_time_exact(change.value),
                })
            })
            .collect();
        self.edit(batch, None, take_over)
    }

    /// Takes one attribute set on the instance back to what its class gives, saving the model
    /// file. The instance's other attributes stay as they are.
    pub fn restore(&mut self, instance: &str, attribute: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "restore_inheritance", "target": { "instance": instance }, "attribute": attribute }), take_over)
    }

    /// Moves the instance to `to`, saving the model file, with every position rounded to the
    /// centimetre so the file stays readable. Instances the file leaves unplaced are saved where
    /// the window draws them too, in the same change, so moving one doesn't send the rest
    /// elsewhere. Refused as a conflict if the file changed since the window read it. `take_over`
    /// makes the one move while an agent holds the editor right.
    pub fn move_instance(&mut self, instance: &str, to: Position, take_over: bool) -> Result<(), Refusal> {
        self.move_instances(&[(instance.to_string(), to)], take_over)
    }

    /// Moves each instance to its position, all in one change, as `move_instance` does one.
    pub fn move_instances(&mut self, moves: &[(String, Position)], take_over: bool) -> Result<(), Refusal> {
        let path = self.opened.clone().ok_or(Refusal::NoModel)?;
        let catalogue = self.catalogue.as_ref().ok_or(Refusal::NoModel)?;
        let read = catalogue.call("read_model", json!({ "path": path })).map_err(|error| Refusal::Invalid(error.to_string()))?;
        let in_file = crabsim_model::load(read["text"].as_str().unwrap_or_default()).map_err(|_| Refusal::Invalid(format!("{path} has problems now")))?.to_description();
        let move_to = |name: &str, to: Position| json!({ "change": "move_instance", "name": name, "position": position_json(to) });
        let moved = |name: &str| moves.iter().any(|(moving, _)| moving == name);
        let mut changes: Vec<Value> = (0..in_file.object_count())
            .filter(|&id| in_file.position(id).is_none())
            .filter_map(|id| Some((self.description.name(id)?, self.description.position(id)?)))
            .filter(|(name, _)| !moved(name))
            .map(|(name, drawn)| move_to(name, drawn))
            .collect();
        changes.extend(moves.iter().map(|(name, to)| move_to(name, *to)));
        let expected = self.read.as_ref().map(|read| read.hash.clone());
        self.edit(changes, expected, take_over)
    }

    // ---------- Building ----------

    /// Makes an empty model file at `path`, `.json` added if it lacks it, saved by crabsim so it can
    /// be built on at once, lists it and opens it. Returns its path. A file already there is
    /// refused.
    pub fn new_model(&mut self, path: &str) -> Result<String, Refusal> {
        let path = with_json(path);
        let catalogue = self.catalogue.as_ref().ok_or(Refusal::NoModel)?;
        catalogue
            .call("write_model", json!({ "path": path, "text": NEW_MODEL }))
            .map_err(|error| Refusal::Invalid(error.to_string()))?;
        self.refresh();
        self.open_model(&path);
        Ok(path)
    }

    /// Copies the model file at `from` to `to`, `.json` added if it lacks it, and lists it. The
    /// model open stays open. Returns the copy's path. A file already there is refused.
    pub fn duplicate_model(&mut self, from: &str, to: &str) -> Result<String, Refusal> {
        let to = with_json(to);
        let catalogue = self.catalogue.as_ref().ok_or(Refusal::NoModel)?;
        catalogue.call("copy_model", json!({ "from": from, "to": to })).map_err(|error| Refusal::Invalid(error.to_string()))?;
        self.refresh();
        Ok(to)
    }

    /// The classes an instance can be placed from, as a tree by inheritance: each built-in class,
    /// which stands for the model's own values for it, followed by the model's classes derived
    /// from it, depth first, in file order. Buffer and the classes derived from it are left out
    /// until build step 7 fixes Buffer.
    pub fn classes(&self) -> Vec<ClassEntry> {
        fn below(model: &Model, parent: &str, depth: usize, into: &mut Vec<ClassEntry>) {
            for class in model.classes.iter().filter(|class| class.parent == parent && !class.refines_built_in()) {
                into.push(ClassEntry { name: class.name.clone(), parent: Some(parent.to_string()), depth, built_in: false });
                below(model, &class.name, depth + 1, into);
            }
        }
        let mut entries = Vec::new();
        for built_in in BUILT_IN_CLASSES.into_iter().filter(|&class| class != "Buffer") {
            entries.push(ClassEntry { name: built_in.to_string(), parent: None, depth: 0, built_in: true });
            below(&self.model, built_in, 1, &mut entries);
        }
        entries
    }

    /// Checks the open model file through `validate_model`: every problem, then every warning,
    /// each with the instance or class it points into.
    pub fn validate(&self) -> Result<Vec<Issue>, String> {
        let path = self.opened.as_deref().ok_or("no model file is open")?;
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        let validated = catalogue.call("validate_model", json!({ "path": path })).map_err(|error| error.to_string())?;
        let issues = |key: &str, warning: bool| -> Vec<Issue> {
            validated[key]
                .as_array()
                .into_iter()
                .flatten()
                .map(|found| {
                    let pointer = found["pointer"].as_str().unwrap_or_default().to_string();
                    let mut parts = pointer.split('/').skip(1);
                    let (list, index) = (parts.next(), parts.next().and_then(|index| index.parse::<usize>().ok()));
                    let instance = match (list, index) {
                        (Some("instances"), Some(index)) if index < self.model.instances.len() => Some(index as u32),
                        _ => None,
                    };
                    let class = match (list, index) {
                        (Some("classes"), Some(index)) => self.model.classes.get(index).map(|class| class.name.clone()),
                        _ => None,
                    };
                    Issue {
                        warning,
                        pointer,
                        name: found["name"].as_str().map(str::to_owned),
                        message: found["message"].as_str().unwrap_or_default().replace(" \u{2192} ", " to "),
                        instance,
                        class,
                    }
                })
                .collect()
        };
        let mut found = issues("problems", false);
        found.extend(issues("warnings", true));
        Ok(found)
    }

    /// The open model as its file gives it.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Asks `query_model` a question about the open model file, such as `{"kind": "paths", ...}`.
    pub fn query(&self, query: Value) -> Result<Value, String> {
        let path = self.opened.as_deref().ok_or("no model file is open")?;
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        catalogue.call("query_model", json!({ "path": path, "query": query })).map_err(|error| error.to_string().replace(" → ", " to "))
    }

    /// The names of the open model's Drains, in file order.
    pub fn drains(&self) -> Vec<String> {
        (0..self.model.instances.len())
            .filter(|&index| self.model.resolve(index).is_some_and(|resolved| resolved.built_in == "Drain"))
            .map(|index| self.model.instances[index].name.clone())
            .collect()
    }

    /// The built-in class at the root of `class`'s chain, if it has a sound one.
    pub fn built_in_of(&self, class: &str) -> Option<&'static str> {
        self.model.chain(class).map(|(_, built_in)| built_in)
    }

    /// The name an instance of `class` gets when placed: the class's name and one more than the
    /// highest number any instance called that way has, such as "Station 6".
    pub fn next_name(&self, class: &str) -> String {
        let prefix = format!("{class} ");
        let highest = self
            .model
            .instances
            .iter()
            .filter_map(|instance| instance.name.strip_prefix(&prefix)?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        format!("{class} {}", highest + 1)
    }

    /// Places an instance of `class` at `at`, to the centimetre, named by `next_name`, with
    /// nothing set on it, so it inherits every value. Its stream is given when the file is saved.
    /// Returns its name.
    pub fn add_instance(&mut self, class: &str, at: Position, take_over: bool) -> Result<String, Refusal> {
        let name = self.next_name(class);
        let mut change = json!({ "change": "add_instance", "name": name, "class": class, "position": position_json(at) });
        let (mut changes, values) = self.starting_values(class);
        if let Some(values) = values {
            change["attributes"] = values;
        }
        changes.push(change);
        self.edit(changes, None, take_over)?;
        Ok(name)
    }

    /// What building on `class` starts with, as the changes to make first and the values to set
    /// on what is built. A Station works 10 s on each MU to start with: the model's own Station
    /// gets it, where nothing in the model is built on Station yet, so every Station placed
    /// later inherits it and changing it there changes them all. A model that already has
    /// Stations without it keeps their values, and what is built gets it set on itself.
    fn starting_values(&self, class: &str) -> (Vec<Value>, Option<Value>) {
        let Some(built_in) = self.built_in_of(class) else { return (Vec::new(), None) };
        let Some(values) = template(built_in) else { return (Vec::new(), None) };
        if self.model.class(built_in).is_some() {
            return (Vec::new(), None);
        }
        let built_on = |name: &str| self.built_in_of(name) == Some(built_in);
        let in_use = self.model.instances.iter().any(|instance| built_on(&instance.class))
            || self.model.classes.iter().any(|class| built_on(&class.name));
        if in_use {
            return (Vec::new(), (class == built_in).then_some(values));
        }
        let changes = values
            .as_object()
            .into_iter()
            .flatten()
            .map(|(attribute, value)| json!({ "change": "set_attribute", "target": { "class": built_in }, "attribute": attribute, "value": value }))
            .collect();
        (changes, None)
    }

    /// Connects `from` to `to`, as `from`'s last exit and `to`'s last entrance. One the file's
    /// rules refuse, such as out of a Drain, is refused with their message.
    pub fn connect(&mut self, from: &str, to: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "connect", "from": from, "to": to }), take_over)
    }

    /// Removes the connection from `from` to `to`, closing up the places of the others.
    pub fn disconnect(&mut self, from: &str, to: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "disconnect", "from": from, "to": to }), take_over)
    }

    /// Moves the connection from `from` to `to` to `place` among its object's exits, or its
    /// successor's entrances, the others moving to make room.
    pub fn move_connection(&mut self, from: &str, to: &str, end: End, place: u64, take_over: bool) -> Result<(), Refusal> {
        let field = match end {
            End::Exit => "exit",
            End::Entrance => "entrance",
        };
        let mut change = json!({ "change": "set_connection", "from": from, "to": to });
        change[field] = json!(place);
        self.change(change, take_over)
    }

    /// Sets the connection's description, or removes it when `description` is empty.
    pub fn describe_connection(&mut self, from: &str, to: &str, description: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "set_connection", "from": from, "to": to, "description": description }), take_over)
    }

    /// The connection from `from` to `to` as the inspector shows it, with its places and how many
    /// there are at each end.
    pub fn connection(&self, from: &str, to: &str) -> Option<ConnectionView> {
        let connection = self.model.connections.iter().find(|connection| connection.from == from && connection.to == to)?;
        Some(ConnectionView {
            from: from.to_string(),
            to: to.to_string(),
            exit: connection.exit,
            exits: self.model.connections_out(from).len() as u64,
            entrance: connection.entrance,
            entrances: self.model.connections_in(to).len() as u64,
            description: connection.description.clone(),
        })
    }

    /// Every connection to and from the instance called `name`, as (from, to), which removing it
    /// takes with it, in the order the file lists them.
    pub fn instance_connections(&self, name: &str) -> Vec<(String, String)> {
        self.model
            .connections
            .iter()
            .filter(|connection| connection.from == name || connection.to == name)
            .map(|connection| (connection.from.clone(), connection.to.clone()))
            .collect()
    }

    /// Removes the instance and its connections, closing up the places of the instances they
    /// joined it to.
    pub fn remove_instance(&mut self, name: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "remove_instance", "name": name }), take_over)
    }

    /// Removes these instances and all their connections in one change.
    pub fn remove_instances(&mut self, names: &[String], take_over: bool) -> Result<(), Refusal> {
        let changes = names.iter().map(|name| json!({ "change": "remove_instance", "name": name })).collect();
        self.edit(changes, None, take_over)
    }

    /// Sets an attribute on each of these instances, in one change.
    pub fn set_attribute_on_all(&mut self, names: &[String], attribute: &str, value: Value, take_over: bool) -> Result<(), Refusal> {
        let changes = names
            .iter()
            .map(|name| json!({ "change": "set_attribute", "target": { "instance": name }, "attribute": attribute, "value": value }))
            .collect();
        self.edit(changes, None, take_over)
    }

    /// Makes a class called `name` from several instances of one class: derived from it, with the
    /// values every one of them sets alike, which then come off them as they all follow the new
    /// class. Values they set differently stay on each. One change. Instances of different
    /// classes are refused.
    pub fn make_class_from_all(&mut self, names: &[String], name: &str, take_over: bool) -> Result<(), Refusal> {
        let found: Vec<&crabsim_model::Instance> = names
            .iter()
            .map(|wanted| self.model.instances.iter().find(|instance| &instance.name == wanted).ok_or_else(|| Refusal::Invalid(format!("no instance is called `{wanted}`"))))
            .collect::<Result<_, _>>()?;
        let Some(first) = found.first() else { return Ok(()) };
        if found.iter().any(|instance| instance.class != first.class) {
            return Err(Refusal::Invalid("a class can only be made from instances of one class, so it can derive from it".into()));
        }
        let shared: Vec<Attribute> = Attribute::ALL
            .into_iter()
            .filter(|&attribute| first.attributes.is_set(attribute))
            .filter(|&attribute| found.iter().all(|instance| instance.attributes.to_json().get(attribute.name()) == first.attributes.to_json().get(attribute.name())))
            .collect();
        let values = first.attributes.to_json();
        let class_values: serde_json::Map<String, Value> = shared.iter().filter_map(|attribute| Some((attribute.name().to_string(), values.get(attribute.name())?.clone()))).collect();
        let mut changes = vec![json!({ "change": "add_class", "name": name, "parent": first.class, "attributes": class_values })];
        for instance in &found {
            for attribute in &shared {
                changes.push(json!({ "change": "restore_inheritance", "target": { "instance": instance.name }, "attribute": attribute.name() }));
            }
            changes.push(json!({ "change": "set_class", "name": instance.name, "class": name }));
        }
        self.edit(changes, None, take_over)
    }

    /// Renames the instance, and its connections with it.
    pub fn rename_instance(&mut self, name: &str, to: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "rename_instance", "name": name, "to": to }), take_over)
    }

    /// Adds a class called `name` derived from `parent`, a built-in or another class, setting
    /// nothing, so it gives exactly what its parent gives.
    pub fn derive_class(&mut self, parent: &str, name: &str, take_over: bool) -> Result<(), Refusal> {
        let mut change = json!({ "change": "add_class", "name": name, "parent": parent });
        let (mut changes, values) = self.starting_values(parent);
        if let Some(values) = values {
            change["attributes"] = values;
        }
        changes.push(change);
        self.edit(changes, None, take_over)
    }

    /// Adds a copy of the class, with its parent and values, not linked to it.
    pub fn duplicate_class(&mut self, name: &str, to: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "duplicate_class", "name": name, "to": to }), take_over)
    }

    /// Renames the class, and every class and instance naming it.
    pub fn rename_class(&mut self, name: &str, to: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "rename_class", "name": name, "to": to }), take_over)
    }

    /// What removing the class takes with it, as `remove_class` says: the classes deriving from
    /// it at any depth, and the instances of it or of them, each in file order.
    pub fn class_dependents(&self, name: &str) -> Dependents {
        if BUILT_IN_CLASSES.contains(&name) {
            return Dependents { classes: Vec::new(), instances: Vec::new() };
        }
        let mut family = vec![name.to_string()];
        let mut classes = Vec::new();
        loop {
            let more: Vec<String> = self
                .model
                .classes
                .iter()
                .filter(|class| family.contains(&class.parent) && !family.contains(&class.name))
                .map(|class| class.name.clone())
                .collect();
            if more.is_empty() {
                break;
            }
            family.extend(more.iter().cloned());
            classes.extend(more);
        }
        let order = |name: &String| self.model.classes.iter().position(|class| &class.name == name);
        classes.sort_by_key(order);
        let instances = self.model.instances.iter().filter(|instance| family.contains(&instance.class)).map(|instance| instance.name.clone()).collect();
        Dependents { classes, instances }
    }

    /// Removes the class with everything `class_dependents` names, and their connections, in one
    /// change. The window asks first, naming them.
    pub fn remove_class(&mut self, name: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "remove_class", "name": name, "delete_dependents": true }), take_over)
    }

    /// Makes a class of the instance's tuned values: a class called `name`, derived from the
    /// instance's class, with the values set on the instance, which then come off the instance as
    /// it becomes one of the new class. It behaves as before. One change, so a refusal changes
    /// nothing.
    pub fn make_class(&mut self, instance: &str, name: &str, take_over: bool) -> Result<(), Refusal> {
        let found = self.model.instances.iter().find(|candidate| candidate.name == instance).ok_or_else(|| Refusal::Invalid(format!("no instance is called `{instance}`")))?;
        let mut changes = vec![json!({ "change": "add_class", "name": name, "parent": found.class, "attributes": found.attributes.to_json() })];
        for attribute in Attribute::ALL.into_iter().filter(|&attribute| found.attributes.is_set(attribute)) {
            changes.push(json!({ "change": "restore_inheritance", "target": { "instance": instance }, "attribute": attribute.name() }));
        }
        changes.push(json!({ "change": "set_class", "name": instance, "class": name }));
        self.edit(changes, None, take_over)
    }

    /// One change to the open model file, through `edit_model`.
    fn change(&mut self, change: Value, take_over: bool) -> Result<(), Refusal> {
        self.edit(vec![change], None, take_over)
    }

    /// A batch of changes to the open model file, through `edit_model`, refused if the file's
    /// hash is no longer `expected_hash`.
    fn edit(&mut self, changes: Vec<Value>, expected_hash: Option<String>, take_over: bool) -> Result<(), Refusal> {
        let path = self.opened.clone().ok_or(Refusal::NoModel)?;
        let catalogue = self.catalogue.as_ref().ok_or(Refusal::NoModel)?;
        let mut input = json!({ "path": path, "changes": changes });
        if let Some(hash) = expected_hash {
            input["expected_hash"] = json!(hash);
        }
        catalogue.set_take_over(take_over);
        let changed = catalogue.call("edit_model", input);
        catalogue.set_take_over(false);
        changed.map(|_| ()).map_err(|error| match error.kind {
            ErrorKind::NotEditor => Refusal::NotEditor(error.message.clone()),
            ErrorKind::ChangedOutside => Refusal::ChangedOutside(error.message.clone()),
            // The window's font has no arrow, so a connection's name reads "In to Mill" there.
            _ => Refusal::Invalid(error.to_string().replace(" → ", " to ")),
        })
    }

    /// Who holds each model file in the folder, and the blocked agents.
    pub fn editors(&self) -> Result<Editors, String> {
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        let read = catalogue.call("get_editors", json!({})).map_err(|error| error.to_string())?;
        Ok(Editors::from_json(&read))
    }

    /// Does one of the user's controls, if who holds the model it acts on and the blocked agents are
    /// still as `shown`, which is what the user decided on. Otherwise nothing is done, and the
    /// refusal holds the state now and what changed. Gives the state after.
    pub fn act(&mut self, shown: &Editors, action: EditAction) -> Result<Editors, ActRefusal> {
        // Taking and releasing are for the open model. The rest need none open.
        let opened = self.opened.clone();
        let path = match &action {
            EditAction::Take | EditAction::Release => opened.clone().ok_or_else(|| ActRefusal::Refused("no model file is open".into()))?,
            EditAction::Revoke(path) => path.clone(),
            EditAction::Block(_) | EditAction::Unblock(_) => opened.clone().unwrap_or_default(),
        };
        let now = self.editors().map_err(ActRefusal::Refused)?;
        if let Some(what) = shown.changes_on(&path, opened.as_deref(), &now) {
            return Err(ActRefusal::Changed { now, what });
        }
        let (operation, input) = match &action {
            EditAction::Take => ("take_model", json!({ "path": path })),
            EditAction::Release => ("release_model", json!({ "path": path })),
            EditAction::Revoke(path) => ("revoke_claim", json!({ "path": path })),
            EditAction::Block(agent) => ("block_agent", json!({ "agent": agent })),
            EditAction::Unblock(agent) => ("unblock_agent", json!({ "agent": agent })),
        };
        let catalogue = self.catalogue.as_ref().ok_or_else(|| ActRefusal::Refused("no models folder is open".into()))?;
        catalogue.call(operation, input).map_err(|error| ActRefusal::Refused(error.to_string()))?;
        self.editors().map_err(ActRefusal::Refused)
    }

    /// The folder's edit state on disk, which the window may watch for changes to who holds what.
    pub fn editing_file(&self) -> Option<PathBuf> {
        Some(self.catalogue.as_ref()?.folder().crabsim_folder().join(crabsim_catalogue::EDITING_FILE))
    }

    /// The open model file on disk, which the window may watch for changes.
    pub fn watched_file(&self) -> Option<PathBuf> {
        self.catalogue.as_ref()?.folder().model_file(self.opened.as_deref()?).ok()
    }

    /// Whether the open model file was written outside crabsim, or crabsim never saved it, so
    /// crabsim refuses to change it until it's adopted.
    pub fn changed_outside(&self) -> bool {
        self.read.as_ref().is_some_and(|read| read.changed_outside)
    }

    /// Accepts the open model file as it is, so crabsim and agents may change it again.
    pub fn adopt(&mut self) -> Result<(), String> {
        let path = self.opened.clone().ok_or("no model file is open")?;
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        catalogue.call("adopt_model", json!({ "path": path })).map_err(|error| error.to_string())?;
        if let Some(read) = self.read.as_mut() {
            read.changed_outside = false;
        }
        Ok(())
    }

    /// The path of the model open, or none until one is opened.
    pub fn opened(&self) -> Option<&str> {
        self.opened.as_deref()
    }

    /// Why the model last chosen can't run, until one that can is opened.
    pub fn problems(&self) -> Option<&ProblemReport> {
        self.problems.as_ref()
    }

    /// Puts the problems away.
    pub fn dismiss_problems(&mut self) {
        self.problems = None;
    }

    /// The model the window runs, with every object placed. Empty until one is opened.
    pub fn description(&self) -> &ModelDescription {
        &self.description
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The seed every run from now on uses. It stays for the window, whatever model is open.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    pub fn measuring_period(&self) -> SimTime {
        self.measuring_period
    }

    /// The measuring period every run from now on uses, whose figures each count as one sample
    /// for the Results tab's intervals. It stays for the window, whatever model is open.
    pub fn set_measuring_period(&mut self, length: SimTime) {
        self.measuring_period = length;
    }

    /// A live session on the model open, with the seed and measuring period, on `clock`.
    pub fn session<C: Clock>(&self, clock: C) -> LiveSession {
        let model = self.description.clone();
        let (seed, period) = (self.seed, self.measuring_period);
        LiveSession::new(
            move || {
                let mut run = model.build(seed);
                run.set_measuring_period(Some(period));
                run
            },
            clock,
        )
    }

    /// The catalogue on the folder, for what the window asks of it.
    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.as_ref()
    }
}

/// A new model file: the model's own Station with what a Station starts with, and nothing else.
const NEW_MODEL: &str = r#"{"format_version": 2, "classes": [{"name": "Station", "attributes": {"process_time": "00:00:10"}}], "instances": []}"#;

/// The values building on a built-in class starts with: a Station works 10 s on each MU, where
/// the built-in Station's default is 0. See `Workspace::starting_values` for where they go.
pub fn template(class: &str) -> Option<Value> {
    match class {
        "Station" => Some(json!({ "process_time": "00:00:10" })),
        _ => None,
    }
}

/// A position as a model file writes it, to the centimetre so the file stays readable.
fn position_json(at: Position) -> Value {
    let round = |metres: f64| (metres * 100.0).round() / 100.0;
    let mut position = json!({ "x": round(at.x), "y": round(at.y) });
    if at.z != 0.0 {
        position["z"] = json!(round(at.z));
    }
    position
}

/// `path` with `.json` at the end.
fn with_json(path: &str) -> String {
    let path = path.trim();
    if path.ends_with(".json") { path.to_string() } else { format!("{path}.json") }
}

fn strings(list: &Value) -> Vec<String> {
    list.as_array().into_iter().flatten().filter_map(|item| item.as_str().map(str::to_string)).collect()
}

/// `describe_model`'s summary as short lines: size, Sources, Drains, and what is wrong with the flow.
fn summarise(summary: &Value) -> Vec<String> {
    let count = |what: &Value| what["count"].as_u64().unwrap_or_default();
    let names = |what: &Value| strings(&what["names"]).join(", ");
    let mut lines = vec![format!("{} instances", summary["instances"].as_u64().unwrap_or_default())];
    for (label, key) in [("Sources", "sources"), ("Drains", "drains"), ("Nothing reaches", "unreachable"), ("Dead ends", "dead_ends")] {
        let part = &summary[key];
        if count(part) > 0 {
            let more = count(part).saturating_sub(part["names"].as_array().map_or(0, |names| names.len() as u64));
            let more = if more > 0 { format!(" and {more} more") } else { String::new() };
            lines.push(format!("{label}: {}{more}", names(part)));
        }
    }
    lines
}

/// How far an object reaches from its centre along x and y, with its rotation.
fn extent(size: Dimensions) -> (f64, f64) {
    let (sin, cos) = size.rotation_degrees.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    ((cos * size.length + sin * size.width) / 2.0, (sin * size.length + cos * size.width) / 2.0)
}

/// Places every object the model leaves unplaced. If none is placed, in columns from left to right
/// by the fewest connections from a Source, each stacked from the top in file order, with what
/// nothing reaches in a column after the rest. If some are placed, the rest go in a row below
/// them, in file order. Visual only, and saved to the file only when the user moves an object.
pub fn lay_out(model: &mut ModelDescription) {
    let count = model.object_count();
    let unplaced: Vec<u32> = (0..count).filter(|&id| model.position(id).is_none()).collect();
    if unplaced.is_empty() {
        return;
    }
    let at = |x: f64, y: f64| Position { x, y, z: 0.0 };
    if unplaced.len() == count as usize {
        let column = columns(model);
        let last = column.iter().copied().max().unwrap_or(0);
        let mut x = 0.0;
        let mut previous_half: Option<f64> = None;
        for c in 0..=last {
            let members: Vec<u32> = (0..count).filter(|&id| column[id as usize] == c).collect();
            if members.is_empty() {
                continue;
            }
            let extents: Vec<(f64, f64)> = members.iter().map(|&id| extent(model.dimensions(id))).collect();
            let half = extents.iter().map(|e| e.0).fold(0.0, f64::max);
            if let Some(previous) = previous_half {
                x += previous + GAP + half;
            }
            previous_half = Some(half);
            // Stacked down from the top, then centred on y = 0.
            let height: f64 = extents.iter().map(|e| 2.0 * e.1).sum::<f64>() + GAP * (members.len() - 1) as f64;
            let mut top = height / 2.0;
            for (&id, &(_, half_y)) in members.iter().zip(&extents) {
                model.set_position(id, at(x, top - half_y));
                top -= 2.0 * half_y + GAP;
            }
        }
        return;
    }
    // A row below everything placed.
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    for id in (0..count).filter(|id| !unplaced.contains(id)) {
        let position = model.position(id).expect("placed");
        let (half_x, half_y) = extent(model.dimensions(id));
        min_x = min_x.min(position.x - half_x);
        min_y = min_y.min(position.y - half_y);
    }
    let extents: Vec<(f64, f64)> = unplaced.iter().map(|&id| extent(model.dimensions(id))).collect();
    let row_half = extents.iter().map(|e| e.1).fold(0.0, f64::max);
    let y = min_y - GAP - row_half;
    let mut left = min_x;
    for (&id, &(half_x, _)) in unplaced.iter().zip(&extents) {
        model.set_position(id, at(left + half_x, y));
        left += 2.0 * half_x + GAP;
    }
}

/// Each object's column: its fewest connections from a Source, and one past the last for what
/// nothing reaches.
fn columns(model: &ModelDescription) -> Vec<usize> {
    let count = model.object_count() as usize;
    let mut distance = vec![None; count];
    let mut queue = VecDeque::new();
    for id in 0..count as u32 {
        if model.kind(id) == ObjectKind::Source {
            distance[id as usize] = Some(0);
            queue.push_back(id);
        }
    }
    while let Some(at) = queue.pop_front() {
        let next = distance[at as usize].expect("queued, so reached") + 1;
        for &successor in model.successors(at) {
            if distance[successor as usize].is_none() {
                distance[successor as usize] = Some(next);
                queue.push_back(successor);
            }
        }
    }
    let beyond = distance.iter().flatten().max().map_or(0, |&furthest| furthest + 1);
    distance.into_iter().map(|d| d.unwrap_or(beyond)).collect()
}
