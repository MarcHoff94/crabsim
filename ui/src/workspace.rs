//! The workspace: what the window works on. A models folder, reached only through the operation
//! catalogue as the user (`Source::Ui`), its model files, the model open and the seed runs use.
//! Without a folder, or when another window has it, the window runs the built-in demo. Plain Rust
//! with no Bevy types, so it is tested without a window. See spec issue 63.
//!
//! `check` is the one seam for noticing that the open model file changed, such as by an agent: it
//! compares the file's size and modified time, and only when they differ reads it again through
//! the catalogue and compares its hash. A file watcher could take its place later.

use crabsim::{demo, Clock, Dimensions, LiveSession, ModelDescription, ObjectKind, Position};
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
    /// The model the window runs, laid out.
    description: ModelDescription,
    /// Its path, or none for the demo.
    opened: Option<String>,
    /// The open file as last read.
    read: Option<Read>,
    /// The open file's model since it changed, not used yet.
    pending: Option<ModelDescription>,
    problems: Option<ProblemReport>,
    seed: u64,
}

impl Workspace {
    /// The workspace on `models`, a models folder, or on the demo without one. A folder another
    /// window has, or that can't be opened, leaves the demo and a banner saying why.
    pub fn open(models: Option<&Path>) -> Workspace {
        let mut workspace = Workspace {
            catalogue: None,
            banner: None,
            models: Vec::new(),
            description: demo::model(),
            opened: None,
            read: None,
            pending: None,
            problems: None,
            seed: 0,
        };
        let Some(models) = models else { return workspace };
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

    /// The path of the model open, or none for the demo.
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

    /// The model the window runs, with every object placed.
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
