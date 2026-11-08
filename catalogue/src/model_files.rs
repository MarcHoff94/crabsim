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
    entries.extend(crate::editing::entries());
    entries.extend(crate::jobs::entries());
    entries.extend(crate::results::entries());
    entries.extend(crate::store::entries());
    entries
}

/// A model file's text. Its schema is about 7 KB, which a harness would carry in every session
/// for every operation a model passes through, so it comes from `model_file_format` alone, and a
/// harness never refuses a draft that doesn't fit it yet.
pub(crate) fn model_text_schema(_: &mut SchemaGenerator) -> Schema {
    schemars::json_schema!({
        "type": "string",
        "description": "A crabsim model file's text: JSON with `format_version` 1, optional `classes` and `instances`. model_file_format gives its schema and an example.",
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

fn problems_of(text: &str) -> Vec<ModelProblem> {
    match crabsim_model::load(text) {
        Ok(_) => Vec::new(),
        Err(problems) => problems.iter().map(ModelProblem::from).collect(),
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
    /// For a file: it was changed outside crabsim, or crabsim never saved it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_outside: Option<bool>,
}

impl Operation for ValidateModel {
    const NAME: &'static str = "validate_model";
    const DESCRIPTION: &'static str = "Checks a crabsim model, given as a path in the models folder or as the file's text, and returns every problem in it at once, each with a JSON Pointer to where it is, the instance or class it's about, and a message. A model with no problems can be run.";
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
        let problems = problems_of(&text);
        Ok(ValidateModelOutput { valid: problems.is_empty(), problems, changed_outside })
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
    /// The paths of the `.json` files, relative to the models folder, sorted, including those in
    /// subfolders.
    pub models: Vec<String>,
}

impl Operation for ListModels {
    const NAME: &'static str = "list_models";
    const DESCRIPTION: &'static str = "Lists the model files (.json) in the models folder, or in one of its subfolders, including those further down. Paths are relative to the models folder, with / between folders, and are what the other operations take.";
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
                } else if catalogue.folder().model_file(&name).is_ok() && path.is_file() {
                    models.push(name);
                }
            }
        }
        models.sort();
        Ok(ListModelsOutput { models })
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
    /// SHA-256 of the text, for `expected_hash` when changing it.
    pub hash: String,
    /// The file was changed outside crabsim, or crabsim never saved it: adopt_model before
    /// changing it.
    pub changed_outside: bool,
}

impl Operation for ReadModel {
    const NAME: &'static str = "read_model";
    const DESCRIPTION: &'static str = "Reads a model file in the models folder and returns its text exactly as it is, its hash for edit_model's or write_model's `expected_hash`, and `changed_outside`: true when the file was written outside crabsim since crabsim last saved it, or crabsim never saved it, so it must be adopted with adopt_model before crabsim changes it.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = PathInput;
    type Output = ReadModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<ReadModelOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.path)?;
        let changed_outside = crate::editing::changed_outside(catalogue, &input.path, &text)?;
        Ok(ReadModelOutput { hash: crate::edit::hash(&text), changed_outside, path: input.path, text })
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
    const DESCRIPTION: &'static str = "Saves a whole model file in the models folder, such as a new model. A file already at the path is refused unless `overwrite` is true: to change a model, edit_model is usually better. A valid model is saved in crabsim's canonical form. A model with problems is saved exactly as given, so work in progress isn't lost, and its problems come back to fix. Only the file's editor may replace it (claim_model), not a file changed outside crabsim until adopt_model, and with `expected_hash` only the version you read.";
    const EFFECT: Effect = Effect::Destructive;
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
        write(&file, &text).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.path)))?;
        editing.record(&text);
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
}

impl Operation for CopyModel {
    const NAME: &'static str = "copy_model";
    const DESCRIPTION: &'static str = "Copies a model file in the models folder to a new path there, such as to start a variant. Never overwrites: refused if the target already exists.";
    const EFFECT: Effect = Effect::Additive;
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
        if editing.recorded(&crate::editing::key_of(catalogue, &input.from)?, &text) {
            editing.record_at(crate::editing::key_of(catalogue, &input.to)?, &text);
        }
        Ok(CopyModelOutput { path: input.to })
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
    const DESCRIPTION: &'static str = "Deletes a model file in the models folder. Files only, never folders. Results of runs of it stay in the results store. Only the file's editor may delete it (claim_model), and not a file changed outside crabsim until adopt_model.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = PathInput;
    type Output = DeleteModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<DeleteModelOutput, OperationError> {
        let (file, _) = read_file(catalogue, &input.path)?;
        let mut editing = crate::editing::before_change(catalogue, &input.path, None)?;
        std::fs::remove_file(&file).map_err(|error| OperationError::run_error(format!("`{}` can't be deleted: {error}", input.path)))?;
        editing.forget();
        Ok(DeleteModelOutput { deleted: input.path })
    }
}

// --- model_file_format ---

pub struct ModelFileFormat;

/// A small model that uses every part of the format an agent starts with: a derived class with
/// failures, instances that inherit from it or set their own values, positions and connections.
const EXAMPLE: &str = r#"{
  "format_version": 1,
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
    { "name": "Arrivals", "class": "Source", "position": { "x": 0, "y": 0 }, "successors": ["Mill 1", "Mill 2"], "attributes": { "interval": "00:00:30" } },
    { "name": "Mill 1", "class": "CNC mill", "position": { "x": 4, "y": 2 }, "successors": ["Inspection"] },
    { "name": "Mill 2", "class": "CNC mill", "position": { "x": 4, "y": -2 }, "successors": ["Inspection"], "attributes": { "process_time": "00:00:50" } },
    { "name": "Inspection", "class": "Station", "position": { "x": 8, "y": 0 }, "successors": ["Shipping"], "attributes": { "process_time": "00:00:20" } },
    { "name": "Shipping", "class": "Drain", "position": { "x": 12, "y": 0 } }
  ]
}"#;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelFileFormatInput {}

/// How the built-in classes behave, which the schema can't say.
const SEMANTICS: &str = "An MU (moveable unit) is one item flowing through the model. Objects pass MUs to their `successors` by push: an object holding a finished MU offers it on, and a move takes no simulated time. \
The successors are tried round-robin, starting after the one that took the last MU, skipping any that can't take one now. When none can, the object is blocked: it keeps the MU and waits, and when a successor frees up, the predecessor that has waited longest gets it first. \
A Source creates one MU, which takes its `interval`, then offers it on. It creates the next only once that one has left, so a blocked Source falls behind and loses arrivals rather than piling them up. An interval of 0 creates the next MU as soon as one leaves. \
A Station holds one MU at a time and works on it for its `process_time`, then offers it on, blocked until a successor takes it. With `failures`, it fails at random: `availability` is the share of time it isn't failed, the time between failures follows from it and the mean repair time, and while it's failed its work is frozen and it takes and releases nothing. A Station with a process time of 0 is a dummy Station that passes MUs straight on. `starting_mus` puts an MU in it at the start. \
A Buffer holds up to `capacity` MUs, but is known not to pass them on correctly yet: don't use it. \
A Drain takes every MU it's offered at once, so it's never blocked, and measures throughput and lead time, from leaving the Source to entering the Drain. Only Stations can fail so far. \
Instances are built in file order, and each object's random numbers come from the run's seed and its position among the instances, so keep the order of instances when comparing variants of a model. \
Statistics per object: working_share, blocked_share, idle_share, failed_share and paused_share of the time after the warm-up, MUs in and out, failures, and achieved availability. A model never has a bottleneck by name: look for the object that is busiest while what comes before it is blocked and what comes after it is idle.";

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
    const DESCRIPTION: &'static str = "Gives the format of crabsim's model files: their JSON Schema, with every field, attribute and rule, how the built-in classes Source, Station, Buffer and Drain behave and how MUs move between them, and a small valid example in canonical form. Read it before writing or judging a model. Times are text such as \"90\", \"1:30\" or \"2d 06:00:00\", and an instance or class sets only the attributes it changes from its class.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ModelFileFormatInput;
    type Output = ModelFileFormatOutput;

    fn run(_: &Catalogue, _: ModelFileFormatInput) -> Result<ModelFileFormatOutput, OperationError> {
        let example = crabsim_model::load(EXAMPLE).expect("the example is a valid model").save();
        Ok(ModelFileFormatOutput { schema: crabsim_model::schema::schema(), semantics: SEMANTICS.to_string(), example })
    }
}
