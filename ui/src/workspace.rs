//! The workspace: what the window works on. A models folder, reached only through the operation
//! catalogue as the user (`Source::Ui`), its model files, the model open and the seed runs use.
//! Without a folder, or when another window has it, the window runs the built-in demo. Plain Rust
//! with no Bevy types, so it is tested without a window. See spec issue 63.

use crabsim::{demo, Clock, Dimensions, LiveSession, ModelDescription, ObjectKind, Position};
use crabsim_catalogue::{Catalogue, ErrorKind, Settings, Source};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::Path;

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
        let Some(catalogue) = &self.catalogue else { return false };
        let text = match catalogue.call("read_model", json!({ "path": path })) {
            Ok(read) => read["text"].as_str().unwrap_or_default().to_string(),
            Err(error) => {
                self.problems = Some(ProblemReport { path: path.to_string(), summary: Vec::new(), problems: vec![error.to_string()] });
                return false;
            }
        };
        match crabsim_model::load(&text) {
            Ok(model) => {
                let mut description = model.to_description();
                lay_out(&mut description);
                self.description = description;
                self.opened = Some(path.to_string());
                self.problems = None;
                true
            }
            Err(problems) => {
                let summary = catalogue.call("describe_model", json!({ "path": path })).map_or_else(|error| vec![error.to_string()], |summary| summarise(&summary));
                self.problems = Some(ProblemReport { path: path.to_string(), summary, problems: problems.iter().map(ToString::to_string).collect() });
                false
            }
        }
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
