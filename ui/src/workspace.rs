//! The workspace: what the window works on. A models folder, reached only through the operation
//! catalogue as the user (`Source::Ui`), its model files, the model open and the seed runs use.
//! Until a model is chosen, or when the folder can't be opened or another window has it, no model
//! is open. Plain Rust with no Bevy types, so it is tested without a window. See spec issue 63.
//!
//! `check` is the one seam for noticing that the open model file changed, such as by an agent: it
//! compares the file's size and modified time, and only when they differ reads it again through
//! the catalogue and compares its hash. A file watcher could take its place later.

use crate::editing::{ActRefusal, Queue, QueueAction};
use crabsim::{Clock, Dimensions, LiveSession, ModelDescription, ObjectKind, Position};
use crabsim_catalogue::{Catalogue, ErrorKind, Settings, Source};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// An agent holds the editor right: the message names it and its purpose. A take-over for the
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
    /// Its path, until one is opened none.
    opened: Option<String>,
    /// The open file as last read.
    read: Option<Read>,
    /// The open file's model since it changed, not used yet.
    pending: Option<ModelDescription>,
    problems: Option<ProblemReport>,
    seed: u64,
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
            opened: None,
            read: None,
            pending: None,
            problems: None,
            seed: 0,
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
            Some((description, read)) => {
                self.description = description;
                self.opened = Some(path.to_string());
                self.read = Some(read);
                self.pending = None;
                self.problems = None;
                true
            }
            None => false,
        }
    }

    /// The model file at `path`, laid out, and what was read, or none with its problems kept.
    fn load(&mut self, path: &str) -> Option<(ModelDescription, Read)> {
        let catalogue = self.catalogue.as_ref()?;
        let stamp = catalogue.folder().model_file(path).ok().and_then(|file| stamp_of(&file));
        let read = match catalogue.call("read_model", json!({ "path": path })) {
            Ok(read) => read,
            Err(error) => {
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
                Some((description, known))
            }
            Err(problems) => {
                let summary = catalogue.call("describe_model", json!({ "path": path })).map_or_else(|error| vec![error.to_string()], |summary| summarise(&summary));
                self.problems = Some(ProblemReport { path: path.to_string(), summary, problems: problems.iter().map(ToString::to_string).collect() });
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
            Some((description, read)) => {
                self.read = Some(read);
                self.pending = Some(description);
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
        if let Some(description) = self.pending.take() {
            self.description = description;
        }
    }

    /// Every attribute the instance's built-in class has, with its value and where it comes
    /// from, as `get_attributes` gives them.
    pub fn attributes(&self, instance: &str) -> Result<Vec<AttributeView>, String> {
        let path = self.opened.as_deref().ok_or("no model file is open")?;
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        let got = catalogue.call("get_attributes", json!({ "path": path, "target": { "instance": instance } })).map_err(|error| error.to_string())?;
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

    /// Takes one attribute set on the instance back to what its class gives, saving the model
    /// file. The instance's other attributes stay as they are.
    pub fn restore(&mut self, instance: &str, attribute: &str, take_over: bool) -> Result<(), Refusal> {
        self.change(json!({ "change": "restore_inheritance", "target": { "instance": instance }, "attribute": attribute }), take_over)
    }

    /// One change to the open model file, through `edit_model`.
    fn change(&mut self, change: Value, take_over: bool) -> Result<(), Refusal> {
        let path = self.opened.clone().ok_or(Refusal::NoModel)?;
        let catalogue = self.catalogue.as_ref().ok_or(Refusal::NoModel)?;
        catalogue.set_take_over(take_over);
        let changed = catalogue.call("edit_model", json!({ "path": path, "changes": [change] }));
        catalogue.set_take_over(false);
        changed.map(|_| ()).map_err(|error| match error.kind {
            ErrorKind::NotEditor => Refusal::NotEditor(error.message.clone()),
            ErrorKind::ChangedOutside => Refusal::ChangedOutside(error.message.clone()),
            _ => Refusal::Invalid(error.to_string()),
        })
    }

    /// The folder's edit queue: every file someone edits or waits for, and the blocked agents.
    pub fn edit_queue(&self) -> Result<Queue, String> {
        let catalogue = self.catalogue.as_ref().ok_or("no models folder is open")?;
        let read = catalogue.call("get_edit_queue", json!({})).map_err(|error| error.to_string())?;
        Ok(Queue::from_json(&read))
    }

    /// Does one of the user's controls, if the queue of the open model and the blocked agents are
    /// still as `shown`, which is what the user decided on. Otherwise nothing is done, and the
    /// refusal holds the queue now and what changed. Gives the queue after.
    pub fn act(&mut self, shown: &Queue, action: QueueAction) -> Result<Queue, ActRefusal> {
        let path = self.opened.clone().ok_or_else(|| ActRefusal::Refused("no model file is open".into()))?;
        let now = self.edit_queue().map_err(ActRefusal::Refused)?;
        if let Some(what) = shown.changes_on(&path, &now) {
            return Err(ActRefusal::Changed { now, what });
        }
        let waiting: Vec<String> = now.file(&path).queue.into_iter().map(|agent| agent.agent).collect();
        let place = |agent: &str| waiting.iter().position(|waiting| waiting == agent).ok_or_else(|| ActRefusal::Refused(format!("{agent} isn't waiting for {path}")));
        let (operation, input) = match &action {
            QueueAction::Take => ("take_model", json!({ "path": path })),
            QueueAction::Release => ("release_model", json!({ "path": path })),
            QueueAction::Give(agent) => ("give_model", json!({ "path": path, "agent": agent })),
            QueueAction::Up(agent) => match place(agent)? {
                0 => return Err(ActRefusal::Refused(format!("{agent} is first in line already"))),
                at => ("move_in_queue", json!({ "path": path, "agent": agent, "place": at - 1 })),
            },
            QueueAction::Down(agent) => match place(agent)? {
                at if at + 1 >= waiting.len() => return Err(ActRefusal::Refused(format!("{agent} is last in line already"))),
                at => ("move_in_queue", json!({ "path": path, "agent": agent, "place": at + 1 })),
            },
            QueueAction::Remove(agent) => ("remove_from_queue", json!({ "path": path, "agent": agent })),
            QueueAction::Block(agent) => ("block_agent", json!({ "agent": agent })),
            QueueAction::Unblock(agent) => ("unblock_agent", json!({ "agent": agent })),
        };
        let catalogue = self.catalogue.as_ref().ok_or_else(|| ActRefusal::Refused("no models folder is open".into()))?;
        catalogue.call(operation, input).map_err(|error| ActRefusal::Refused(error.to_string()))?;
        self.edit_queue().map_err(ActRefusal::Refused)
    }

    /// The folder's edit state on disk, which the window may watch for changes to the queue.
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

    /// A live session on the model open, with the seed, on `clock`.
    pub fn session<C: Clock>(&self, clock: C) -> LiveSession {
        let model = self.description.clone();
        let seed = self.seed;
        LiveSession::new(move || model.build(seed), clock)
    }

    /// The catalogue on the folder, for what the window asks of it.
    pub fn catalogue(&self) -> Option<&Catalogue> {
        self.catalogue.as_ref()
    }
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
/// them, in file order. Visual only, and never saved to the file.
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
