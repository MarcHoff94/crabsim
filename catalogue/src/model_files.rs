//! Operations on the model files in the models folder: validate, list, read, write, copy and
//! delete, and the format they're written in. They touch only `.json` files inside the folder,
//! never `.crabsim`.

use crate::error::{ErrorKind, ModelProblem, OperationError};
use crate::{entry, Catalogue, Effect, Entry, Operation};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) fn entries() -> Vec<Entry> {
    let mut entries = vec![
        entry::<ValidateModel>(),
        entry::<ListModels>(),
        entry::<ReadModel>(),
        entry::<WriteModel>(),
        entry::<CopyModel>(),
        entry::<DeleteModel>(),
        entry::<ModelFileFormat>(),
    ];
    entries.extend(crate::edit::entries());
    entries.extend(crate::commands::entries());
    entries.extend(crate::macros::entries());
    entries.extend(crate::editing::entries());
    entries.extend(crate::history::entries());
    entries.extend(crate::jobs::entries());
    entries.extend(crate::bottleneck::entries());
    entries.extend(crate::results::entries());
    entries.extend(crate::expectations::entries());
    entries.extend(crate::stages::entries());
    entries.extend(crate::compare::entries());
    entries.extend(crate::graph::entries());
    entries.extend(crate::export::entries());
    entries.extend(crate::store::entries());
    entries
}

/// A model file's text. Its schema is about 7 KB, which a harness would carry in every session
/// for every operation a model passes through, so it comes from `model_file_format` alone, and a
/// harness never refuses a draft that doesn't fit it yet.
pub(crate) fn model_text_schema(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "string",
        "description": "A crabsim model file's text: JSON with `format_version` 2, `instances`, and optional `classes` and `connections`. model_file_format gives its schema and an example.",
        "contentMediaType": "application/json",
    })
}

/// Reads the model file at `path` in the models folder.
pub(crate) fn read_file(catalogue: &Catalogue, path: &str) -> Result<(std::path::PathBuf, String), OperationError> {
    let file = catalogue.folder().model_file(path)?;
    if !file.is_file() {
        return Err(OperationError::not_found(format!("there is no model file `{path}` in the models folder")));
    }
    let text = std::fs::read_to_string(&file).map_err(|error| OperationError::run_error(format!("`{path}` can't be read: {error}")))?;
    Ok((file, text))
}

/// A model's problems, and its warnings when it has none.
fn problems_and_warnings(text: &str) -> (Vec<ModelProblem>, Vec<ModelProblem>) {
    match crabsim_model::load(text) {
        Ok(model) => (Vec::new(), model.warnings().iter().map(ModelProblem::from).collect()),
        Err(problems) => (problems.iter().map(ModelProblem::from).collect(), Vec::new()),
    }
}

// --- validate_model ---

pub struct ValidateModel;

/// The model to check: either `path` or `text`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateModelInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: Option<String>,
    /// A model file's text, such as a draft not saved yet. model_file_format gives its schema and
    /// an example.
    // `schema_with` hides the `Option` from schemars, so `default` is what keeps `text` optional
    // in the schema.
    #[serde(default)]
    #[schemars(schema_with = "model_text_schema")]
    pub text: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ValidateModelOutput {
    /// No problems: the model can be run.
    pub valid: bool,
    /// Every problem, in file order, each with a JSON Pointer to where it is.
    pub problems: Vec<ModelProblem>,
    /// For a model without problems: what can run but is probably a mistake, in file order,
    /// each with a JSON Pointer: an instance no Source reaches, one with no route to a Drain, a
    /// Drain list naming something that isn't a Drain, and a weighted object whose weight is
    /// nothing on every route onward. Warnings never stop a run.
    pub warnings: Vec<ModelProblem>,
    /// For a file: it was changed outside crabsim, or crabsim never saved it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_outside: Option<bool>,
}

impl Operation for ValidateModel {
    const NAME: &'static str = "validate_model";
    const DESCRIPTION: &'static str = "Checks a crabsim model, given as a path in the models folder or as the file's text, and returns every problem in it at once, each with a JSON Pointer to where it is, the instance or class it's about, and a message. A model with no problems can be run. For one without problems it also gives warnings, in the same form: what can run but is probably a mistake, such as an instance no Source reaches, one with no route to a Drain, a weighted distribution's Drain list naming something that isn't a Drain, a weight that is nothing on every route onward, a Buffer with no limit that can get MUs faster than it can pass them on, or an object routing by destination none of whose successors leads to where MUs can be going from it. Warnings never stop a run.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ValidateModelInput;
    type Output = ValidateModelOutput;

    fn run(catalogue: &Catalogue, input: ValidateModelInput) -> Result<ValidateModelOutput, OperationError> {
        let (text, changed_outside) = match (input.path, input.text) {
            (Some(path), None) => {
                let text = read_file(catalogue, &path)?.1;
                let changed = crate::editing::changed_outside(catalogue, &path, &text)?;
                (text, Some(changed))
            }
            (None, Some(text)) => (text, None),
            _ => return Err(OperationError::invalid_input("give either `path` or `text`, not both and not neither")),
        };
        let (problems, warnings) = problems_and_warnings(&text);
        Ok(ValidateModelOutput { valid: problems.is_empty(), problems, warnings, changed_outside })
    }
}

// --- list_models ---

pub struct ListModels;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListModelsInput {
    /// A subfolder of the models folder to list, such as `plant`. Left out, the whole folder.
    pub folder: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListModelsOutput {
    /// The paths of the model files, relative to the models folder, sorted, including those in
    /// subfolders.
    pub models: Vec<String>,
}

impl Operation for ListModels {
    const NAME: &'static str = "list_models";
    const DESCRIPTION: &'static str = "Lists the model files in the models folder, or in one of its subfolders, including those further down: the .json files with `format_version` at their top level, also those with problems, but not other JSON such as a schema. Paths are relative to the models folder, with / between folders, and are what the other operations take.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ListModelsInput;
    type Output = ListModelsOutput;

    fn run(catalogue: &Catalogue, input: ListModelsInput) -> Result<ListModelsOutput, OperationError> {
        let start = match &input.folder {
            Some(folder) => catalogue.folder().subfolder(folder)?,
            None => catalogue.folder().root().to_path_buf(),
        };
        if !start.is_dir() {
            return Err(OperationError::not_found(format!("there is no folder `{}` in the models folder", input.folder.unwrap_or_default())));
        }
        let mut models = Vec::new();
        let mut folders = vec![start];
        while let Some(folder) = folders.pop() {
            let entries = std::fs::read_dir(&folder).map_err(|error| OperationError::run_error(format!("a folder can't be listed: {error}")))?;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = catalogue.folder().name_of(&path);
                let Ok(kind) = entry.file_type() else { continue };
                // Links aren't followed, so a link can't lead out or round in a circle. Files go
                // through the same check as every other operation, so `.crabsim` never shows.
                if kind.is_dir() {
                    folders.push(path);
                } else if catalogue.folder().model_file(&name).is_ok() && path.is_file() && is_model_file(&path) {
                    models.push(name);
                }
            }
        }
        models.sort();
        Ok(ListModelsOutput { models })
    }
}

/// Whether a `.json` file is meant as a model file: its top level has `format_version`. A file
/// that isn't JSON any more, such as one part way through being written by hand, counts when it
/// names `format_version` at all, so it is listed with its problems rather than vanishing. Other
/// JSON, such as a JSON Schema or settings, isn't listed.
fn is_model_file(path: &std::path::Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(json) => json.get("format_version").is_some(),
        Err(_) => text.contains("\"format_version\""),
    }
}

// --- read_model ---

pub struct ReadModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    /// A model file in the models folder, such as `line.json` or `plant/line.json`.
    pub path: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ReadModelOutput {
    pub path: String,
    /// The file's text, exactly as it is: JSON whose schema model_file_format gives.
    #[schemars(schema_with = "model_text_schema")]
    pub text: String,
    /// SHA-256 of the file's bytes, for `expected_hash` when changing it.
    pub file_hash: String,
    /// SHA-256 of the model in canonical form, as runs record it in `model_hash`, so it tells
    /// whether a run ran this model. Only for a valid model. For a file crabsim saved, it equals
    /// `file_hash`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    /// The file was changed outside crabsim, or crabsim never saved it: adopt_model before
    /// changing it.
    pub changed_outside: bool,
}

impl Operation for ReadModel {
    const NAME: &'static str = "read_model";
    const DESCRIPTION: &'static str = "Reads a model file in the models folder and returns its text exactly as it is, its `file_hash` for edit_model's or write_model's `expected_hash`, its `model_hash`, which runs record, to tell whether a run ran this model, and `changed_outside`: true when the file was written outside crabsim since crabsim last saved it, or crabsim never saved it, so it must be adopted with adopt_model before crabsim changes it.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = PathInput;
    type Output = ReadModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<ReadModelOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.path)?;
        let changed_outside = crate::editing::changed_outside(catalogue, &input.path, &text)?;
        let model_hash = crate::edit::model_hash(&text);
        Ok(ReadModelOutput { file_hash: crate::edit::hash(&text), model_hash, changed_outside, path: input.path, text })
    }
}

// --- write_model ---

pub struct WriteModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteModelInput {
    /// Where to save it in the models folder, such as `line.json`. Missing subfolders are made.
    pub path: String,
    #[schemars(schema_with = "model_text_schema")]
    pub text: String,
    /// Replace a file already at `path`. Without it, an existing file is refused: change it with
    /// edit_model instead.
    #[serde(default)]
    pub overwrite: bool,
    /// With `overwrite`, the hash of the file as you read it: refused as a conflict if it has
    /// changed since.
    pub expected_hash: Option<String>,
}

/// How a model was saved.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Saved {
    /// Valid, so saved in crabsim's one canonical form.
    Canonical,
    /// Has problems, so saved exactly as given.
    AsGiven,
}

#[derive(Serialize, JsonSchema)]
pub struct WriteModelOutput {
    pub path: String,
    pub saved: Saved,
    /// Every problem in the model, empty if it's valid.
    pub problems: Vec<ModelProblem>,
}

impl Operation for WriteModel {
    const NAME: &'static str = "write_model";
    const DESCRIPTION: &'static str = "Saves a whole model file in the models folder, such as a new model. A file already at the path is refused unless `overwrite` is true: to change a model, edit_model is usually better. A valid model is saved in crabsim's canonical form. A model with problems is saved exactly as given, so work in progress isn't lost, and its problems come back to fix. An agent needs its claim on the path first (claim_model), also for a new file. A file changed outside crabsim isn't replaced until adopt_model, and with `expected_hash` only the version you read is.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = WriteModelInput;
    type Output = WriteModelOutput;

    fn run(catalogue: &Catalogue, input: WriteModelInput) -> Result<WriteModelOutput, OperationError> {
        let file = catalogue.folder().model_file(&input.path)?;
        let mut editing = crate::editing::before_change(catalogue, &input.path, input.expected_hash.as_deref())?;
        if file.is_dir() {
            return Err(OperationError::invalid_input(format!("`{}` is a folder", input.path)));
        }
        if file.is_file() && !input.overwrite {
            return Err(OperationError::new(
                ErrorKind::AlreadyExists,
                format!("`{}` already exists: change it with edit_model, or give `overwrite: true` to replace it", input.path),
            ));
        }
        let (text, saved, problems) = match crabsim_model::load(&input.text) {
            Ok(model) => (model.save(), Saved::Canonical, Vec::new()),
            Err(problems) => (input.text, Saved::AsGiven, problems.iter().map(ModelProblem::from).collect()),
        };
        let before = std::fs::read_to_string(&file).ok();
        crate::agreeing::guard(catalogue, &input.path, before.as_deref(), Some(&text))?;
        write(&file, &text).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.path)))?;
        editing.record(&text);
        match &before {
            Some(before) => {
                let change = crate::history::Change { before: crate::history::Before::Text(before), after: &text, kind: crate::history::Kind::Written, wording: "written whole".to_string() };
                crate::history::keep(catalogue, &editing, &input.path, change);
            }
            // A new file starts with no history, whatever was at its path before.
            None => crate::history::forget(catalogue, &input.path),
        }
        Ok(WriteModelOutput { path: input.path, saved, problems })
    }
}

fn write(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(file, text)
}

// --- copy_model ---

pub struct CopyModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CopyModelInput {
    /// The model file to copy.
    pub from: String,
    /// Where the copy goes. Refused if a file is already there.
    pub to: String,
}

#[derive(Serialize, JsonSchema)]
pub struct CopyModelOutput {
    /// The copy's path.
    pub path: String,
    /// The copy, like its source, was changed outside crabsim, or crabsim never saved it: adopt it
    /// with adopt_model before changing it.
    pub changed_outside: bool,
}

impl Operation for CopyModel {
    const NAME: &'static str = "copy_model";
    const DESCRIPTION: &'static str = "Copies a model file in the models folder to a new path there, such as to start a variant. Never overwrites: refused if the target already exists. A copy of a file changed outside crabsim is too, as `changed_outside` says, and needs adopt_model before crabsim changes it.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
    type Input = CopyModelInput;
    type Output = CopyModelOutput;

    fn run(catalogue: &Catalogue, input: CopyModelInput) -> Result<CopyModelOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.from)?;
        let target = catalogue.folder().model_file(&input.to)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.to)))?;
        }
        // `create_new` refuses an existing file, also one that appeared since checking.
        let created = std::fs::OpenOptions::new().write(true).create_new(true).open(&target);
        let mut file = match created {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(OperationError::new(ErrorKind::AlreadyExists, format!("`{}` already exists: choose another path, or delete it first", input.to)));
            }
            Err(error) => return Err(OperationError::run_error(format!("`{}` can't be written: {error}", input.to))),
        };
        std::io::Write::write_all(&mut file, text.as_bytes())
            .map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.to)))?;
        // A copy of a file crabsim saved is crabsim's own too. A copy of one changed outside isn't,
        // so the outside change can't slip in through it.
        let mut editing = catalogue.editing()?;
        let recorded = editing.recorded(&crate::editing::key_of(catalogue, &input.from)?, &text);
        if recorded {
            editing.record_at(crate::editing::key_of(catalogue, &input.to)?, &text);
        }
        crate::history::forget(catalogue, &input.to);
        Ok(CopyModelOutput { path: input.to, changed_outside: !recorded })
    }
}

// --- delete_model ---

pub struct DeleteModel;

#[derive(Serialize, JsonSchema)]
pub struct DeleteModelOutput {
    /// The deleted file's path.
    pub deleted: String,
}

impl Operation for DeleteModel {
    const NAME: &'static str = "delete_model";
    const DESCRIPTION: &'static str = "Deletes a model file in the models folder. Files only, never folders. Results of runs of it stay in the results store. An agent needs its claim on the file first (claim_model), and no file changed outside crabsim is deleted until adopt_model.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = PathInput;
    type Output = DeleteModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<DeleteModelOutput, OperationError> {
        let (file, text) = read_file(catalogue, &input.path)?;
        let mut editing = crate::editing::before_change(catalogue, &input.path, None)?;
        crate::agreeing::guard(catalogue, &input.path, Some(&text), None)?;
        std::fs::remove_file(&file).map_err(|error| OperationError::run_error(format!("`{}` can't be deleted: {error}", input.path)))?;
        editing.forget();
        crate::history::forget(catalogue, &input.path);
        Ok(DeleteModelOutput { deleted: input.path })
    }
}

// --- model_file_format ---

pub struct ModelFileFormat;

/// A small model that uses every part of the format an agent starts with: a derived class with
/// failures, instances that inherit from it or set their own values, positions and connections.
const EXAMPLE: &str = r#"{
  "format_version": 2,
  "description": "A Source, two mills of one class in parallel, an inspection and a Drain",
  "classes": [
    {
      "name": "CNC mill",
      "parent": "Station",
      "attributes": {
        "process_time": "00:00:45",
        "failures": { "availability": 0.95, "repair_time": { "erlang": { "mean": "00:05:00", "k": 2 } } },
        "description": "5-axis mill"
      }
    }
  ],
  "instances": [
    { "name": "Arrivals", "class": "Source", "position": { "x": 0, "y": 0 }, "attributes": { "interval": "00:00:30" } },
    { "name": "Mill 1", "class": "CNC mill", "position": { "x": 4, "y": 2 } },
    { "name": "Mill 2", "class": "CNC mill", "position": { "x": 4, "y": -2 }, "attributes": { "process_time": "00:00:50" } },
    { "name": "Inspection", "class": "Station", "position": { "x": 8, "y": 0 }, "attributes": { "process_time": "00:00:20" } },
    { "name": "Shipping", "class": "Drain", "position": { "x": 12, "y": 0 } }
  ],
  "connections": [
    { "from": "Arrivals", "to": "Mill 1", "exit": 1, "entrance": 1 },
    { "from": "Arrivals", "to": "Mill 2", "exit": 2, "entrance": 1 },
    { "from": "Mill 1", "to": "Inspection", "exit": 1, "entrance": 1 },
    { "from": "Mill 2", "to": "Inspection", "exit": 1, "entrance": 2, "description": "the long way round" },
    { "from": "Inspection", "to": "Shipping", "exit": 1, "entrance": 1 }
  ]
}"#;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelFileFormatInput {}

/// How the built-in classes behave, which the schema can't say.
const SEMANTICS: &str = "An MU (moveable unit) is one item flowing through the model. Objects pass MUs along `connections` by push: an object holding a finished MU offers it on to its successors, the objects its connections lead to, and a move takes no simulated time. \
Each connection has an `exit`, its place among the connections out of its object, and an `entrance`, its place among those into the object it leads to, each numbered 1 to n per object with no gaps or repeats, so the list's own order means nothing. The successors are tried round-robin in exit order, starting after the one that took the last MU, skipping any that can't take one now. Nothing reads the entrance order yet, so it doesn't change results. When none can, the object is blocked: it keeps the MU and waits in line at each of them, and when a successor frees up, the predecessor whose MU has been ready to leave longest gets it first, whichever object holds it. \
A Source creates one MU, which takes its `interval`, then offers it on. It creates the next only once that one has left, so a blocked Source falls behind and loses arrivals rather than piling them up. Its interval is 0 by default, which creates the next MU as soon as one leaves, so such a Source must reach every Drain through an object that takes time. \
A Station is one machine that works on up to its `capacity` of MUs at once, 1 by default and at most 10,000, each for its `process_time`, then offers each on, blocked until a successor takes it. With `start` \"individual\", the default, each MU starts on entry; with \"batch\", work begins once it is full, all finishing together, and it takes no MU until it is completely empty again. A finished MU waiting to leave keeps its place, and finished MUs leave earliest finished first. Work outranks blocking: a Station is blocked only when nothing is in work and a finished MU can't leave. With `failures`, it fails at random: `availability` is the share of time it isn't failed, the time between failures follows from it and the mean repair time, and while it's failed all its work is frozen and it takes and releases nothing. A Station with a process time of 0 is a dummy Station that passes MUs straight on. `starting_mus` puts that many MUs in it at the start, at most its capacity: a full starting batch begins work at once, a part of one waits. \
A Buffer holds MUs, up to its `capacity`: a whole number from 1, or \"unlimited\", the default, which takes every MU and grows as far as it's filled (-1 is read as unlimited too). Each MU stays for the `dwelling_time`, 0 by default, counted from its own entry, and then the Buffer offers on one MU at a time, chosen by its `exit_strategy` among those whose dwelling time is over: \"first_in_first_out\", the default, or \"last_in_first_out\". `starting_mus` puts that many MUs in it at the start, at most its capacity. Its fill statistics include a histogram of 64 bars of how long it held each number of MUs: `histogram_bar_width` sets how many MUs each bar covers, the last then taking everything beyond, and left out it is automatic, one MU each, doubling as more come. A Buffer is idle when empty, holding while it has MUs, and blocked when an MU that may leave can't: its working_share is the time it was holding. A full Buffer takes no MU, so what feeds it is blocked. A Source with an interval of 0 must not reach a Buffer with no limit through objects that take no time. \
A Drain takes every MU it's offered at once, so it's never blocked, and measures throughput and lead time, from leaving the Source to entering the Drain. \nAny object can have `failures` and be interrupted: failed at random, stopped by hand or paused. While interrupted, a Station's work and a Source's interval freeze, a Drain refuses MUs, and a Buffer takes and releases nothing, its dwelling time running on unless `stop_freezes_dwelling` is true. An interrupted object waiting to pass an MU on keeps its place in line. \
MUs have classes too: an MU class derives from the built-in class MU, or from another MU class, and is never placed as an instance. It gives its MUs a footprint, `length` along the way it travels and `width` across it, in metres, 0.6 by 0.4 by default, and a `booking_point`, a share of its length from its front, 0 at the front and 1 at the rear, 0.5 by default, which decides when an MU passes from one object to the next. A Source makes MUs of its `mu_class`, MU by default, and `mu_values` changes any of those values for its MUs alone. Only objects whose geometry counts, such as conveyors, read an MU's values: a Source, Station, Buffer or Drain takes any MU. \
An MU class can have a `work_plan`, a list of steps such as [{\"at\": \"Saw\", \"process_time\": \"0:40\"}, {\"at\": \"Mill\"}, {\"at\": \"Shipping\"}], which its MUs follow, and a Source's `mu_values` can give its MUs their own. Each step names a destination: an instance, or a class standing for any of its instances or of classes derived from it, never an MU class. An MU's next destination is its plan's current step. Entering a Station its next step names, it is worked on for the step's `process_time`, or the Station's own without one, and moves on to its next step; entering any other object its next step names, such as a Drain, moves it on too. At a Station its next step doesn't name, it is worked on for the Station's own time and keeps its step, counted as off plan. A Station's `detour` names a destination every MU it works on goes to first, such as Rework, before carrying on with its plan: while it has a detour that is its next destination, and reaching it clears it. MUs started in an object have no plan. Only an object whose `distribution` is {\"by_destination\": {\"then\": \"round_robin\"}}, or with a weighted strategy as `then`, reads an MU's destination: it offers the MU only to its successors from which the MU's next destination can be reached along the connections, chosen among by `then`, and waits for one of them when they are all blocked. An MU with no destination left, or one none of them reaches, is offered to every successor. A Source, Station, Buffer or Conveyor can route by destination. \
Instances are built in file order. Each instance's random numbers come from the run's seed and its `stream`, which crabsim gives it when it's added and keeps through edits, so changing a model changes no other instance's random numbers and variants compare pair by pair. An instance the file gives no stream, as in a file from before streams, gets the next unused one in file order when the model is read, and crabsim writes them in when it saves the file. \
A Conveyor carries MUs along its `path`, a list of segments that starts at the Conveyor's position, heading along its rotation, each segment starting where the one before ends: a straight, such as {\"straight\": 10}, an arc, such as {\"arc\": {\"radius\": 1.5, \"angle\": 90}}, a transition curve easing from the bend before it to a radius or to straight, such as {\"transition\": {\"length\": 1, \"to_radius\": 1.5}}, and a helix, such as {\"helix\": {\"radius\": 1.2, \"turns\": 2, \"rise\": 3}}, angles and turns positive to the left. Any segment may have a `rise` in metres, negative falling. Lengths are in plan, and only the path's length in 3D changes results: bends and slopes don't slow MUs. A bend may be no tighter than half the Conveyor's width or its `min_radius`, and no segment steeper than its `max_incline`, 30 degrees by default. Its `direction`, \"forward\" by default or \"backward\", says which end MUs enter at: backward they enter at the path's last point. get_attributes gives a Conveyor's worked-out ends, length and bounds, so what comes after it can be placed where it ends. It moves them at its `speed` in metres per second, 0.5 by default, holding as many as fit along it with at least its `gap` between two, 0 by default, and refusing an MU wider than its `width`, 0.6 by default, or longer than its path. An MU enters with its booking point at the start, so the length from its front to its booking point plus the gap must be free there, and a predecessor refused waits as for any object. MUs close up behind one waiting at the end, unless `accumulating` is false, which stops every MU on the Conveyor as a whole when its first can't leave, to start together again. When an MU's front reaches the end it is offered to the successors, and one that takes it is receiving, its place taken, until the MU's booking point crosses the end, when the MU moves to it; with the booking point at the front that is at once. A transfer once begun always finishes, even if the successor is interrupted meanwhile. Entering by its booking point, an MU takes the conveyor's length over its speed to cross it when nothing stops it. An interrupted Conveyor stops every MU on it and takes and passes on none. A Conveyor can lead straight into another Conveyor: the second reserves the space under the MU's front from when it touches its start, an MU on both moves at the slower of their speeds, and the MUs behind it close up to its rear. An MU on two Conveyors, while one is interrupted, moves at the speed of the one its booking point is on. \
Statistics per object: working_share, blocked_share, idle_share, receiving_share, failed_share and paused_share of the time after the warm-up, MUs in and out, failures, and achieved availability. A Station with a capacity above 1 also has working_share's three parts, which add up to it: working_full_share (every place in work), working_not_full_share (working with free places) and working_partly_blocked_share (working while a finished MU can't leave). A model never has a bottleneck by name: look for the Station busy and full or stopped the most while what comes before it is blocked and what comes after it is idle, as find_bottleneck does. A Buffer also has mean_mus, min_mus, max_mus, median_mus and, with a limit, full_share, and run_results gives its histogram when it is named in objects. A Station that worked on MUs with a work plan or a detour has mus_on_plan, those worked on for their step or detour, and mus_off_plan, those on their way somewhere else, which shows MUs going the wrong way. A model's `expectations` say what it must achieve, each a check crabsim judges on its own after every run (ADR 0030), with an `id`, its `text` as one line, a `stage`, `verification` by default or `validation`, and optional `assumptions` and `source`. A line is `[every|mean] <instance, class or model>.<figure> <scope> <comparison> <value>`: an instance by its name; a class with `every`, each of its instances meeting it, or `mean`, their mean; or `model`, its Drains together. The figure is one of list_figures', with the scopes it can be read over: `since start` (after the warm-up), `now` (at the run's end), `last <time>` (the stretch before the end, such as `last 1h`) or `always`, a limit at every moment, such as `Buffer X.mus always <= 20`, whose first violation is reported with its time and MU. The comparison is >=, <=, >, <, = or !=, and the value has an explicit unit: a share as a percentage (`60%`), a rate with its unit (`4000/h`, `70/min`, `96000/d`), a time with its unit (`90s`, `30min`, `2h`, `1:30`), MUs plainly (`20`), a state by its name (`busy`). Such as `every Press.working_share since start >= 60%`, `mean Press.throughput_per_hour since start >= 4000/h` or `model.mean_lead_time_s last 8h <= 2h`. run_results gives each one's verdict: met, not met, undecided when an experiment's 95% interval straddles its value, with how many observations would decide it, or error when it couldn't be judged. A run with `deterministic` true has no failures, for verifying the model does what was specified. \
An expectation's `state` is `proposed`, the default, for one an agent wrote; `accepted` for one the user accepted or wrote, which only the user changes; or `questioned`, with its `question`, an agent's reason, still in force until the user answers. An expectation may give its own `length`, `warm_up` and `observations` for its check. The model's `requests` hold what the user asked for: an `id`, the user's own `words`, an agent's `restatement`, the `expectations` that check it by id, `accepted` once the user accepted it with them, and `state` `open` or `built`. Its `stages` give each stage's `length`, `warm_up` and `observations`: verification a day, no warm-up and one deterministic observation by default, validation a day and ten observations with randomness. check_stage checks a stage and get_status says where the model stands.";

#[derive(Serialize, JsonSchema)]
pub struct ModelFileFormatOutput {
    /// The model file's JSON Schema (draft 2020-12), with every field, attribute and rule.
    pub schema: serde_json::Value,
    /// How the built-in classes behave: Source, Station, Buffer and Drain, how MUs move, failures
    /// and what is measured.
    pub semantics: String,
    /// A small valid model in canonical form, as crabsim saves it.
    pub example: String,
}

impl Operation for ModelFileFormat {
    const NAME: &'static str = "model_file_format";
    const DESCRIPTION: &'static str = "Gives the format of crabsim's model files: their JSON Schema, with every field, attribute and rule, how the built-in classes Source, Station, Buffer, Drain and Conveyor behave and how MUs move between them, what MU classes are, and a small valid example in canonical form. Read it before writing or judging a model. Times are text such as \"90\", \"1:30\" or \"2d 06:00:00\", and an instance or class sets only the attributes it changes from its class.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ModelFileFormatInput;
    type Output = ModelFileFormatOutput;

    fn run(_: &Catalogue, _: ModelFileFormatInput) -> Result<ModelFileFormatOutput, OperationError> {
        let example = crabsim_model::load(EXAMPLE).expect("the example is a valid model").save();
        Ok(ModelFileFormatOutput { schema: crabsim_model::schema::schema(), semantics: SEMANTICS.to_string(), example })
    }
}
