//! Operations on the model files in the models folder: validate, list, read, write, copy and
//! delete. They touch only `.json` files inside the folder, never `.crabsim`.

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
    ];
    entries.extend(crate::jobs::entries());
    entries.extend(crate::results::entries());
    entries.extend(crate::store::entries());
    entries
}

/// A model file's text, whose JSON follows the model file's own schema, so an operation's schema
/// and the model file's can't disagree. The schema rides along as `contentSchema`, which describes
/// the text without making a harness refuse a draft that doesn't fit it yet.
pub(crate) fn model_text_schema(generator: &mut SchemaGenerator) -> Schema {
    let model = generator.subschema_for::<crabsim_model::schema::ModelFile>();
    schemars::json_schema!({
        "type": "string",
        "description": "A crabsim model file's text: JSON with `format_version` 1, optional `classes` and `instances`.",
        "contentMediaType": "application/json",
        "contentSchema": model,
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
    /// A model file's text, such as a draft not saved yet.
    #[schemars(schema_with = "model_text_schema")]
    pub text: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ValidateModelOutput {
    /// No problems: the model can be run.
    pub valid: bool,
    /// Every problem, in file order, each with a JSON Pointer to where it is.
    pub problems: Vec<ModelProblem>,
}

impl Operation for ValidateModel {
    const NAME: &'static str = "validate_model";
    const DESCRIPTION: &'static str = "Checks a crabsim model, given as a path in the models folder or as the file's text, and returns every problem in it at once, each with a JSON Pointer to where it is, the instance or class it's about, and a message. A model with no problems can be run.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ValidateModelInput;
    type Output = ValidateModelOutput;

    fn run(catalogue: &Catalogue, input: ValidateModelInput) -> Result<ValidateModelOutput, OperationError> {
        let text = match (input.path, input.text) {
            (Some(path), None) => read_file(catalogue, &path)?.1,
            (None, Some(text)) => text,
            _ => return Err(OperationError::invalid_input("give either `path` or `text`, not both and not neither")),
        };
        let problems = problems_of(&text);
        Ok(ValidateModelOutput { valid: problems.is_empty(), problems })
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
    /// The file's text, exactly as it is.
    #[schemars(schema_with = "model_text_schema")]
    pub text: String,
}

impl Operation for ReadModel {
    const NAME: &'static str = "read_model";
    const DESCRIPTION: &'static str = "Reads a model file in the models folder and returns its text exactly as it is.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = PathInput;
    type Output = ReadModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<ReadModelOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.path)?;
        Ok(ReadModelOutput { path: input.path, text })
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
    const DESCRIPTION: &'static str = "Saves a model file in the models folder, replacing any file already there. A valid model is saved in crabsim's canonical form. A model with problems is saved exactly as given, so work in progress isn't lost, and its problems come back to fix.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = WriteModelInput;
    type Output = WriteModelOutput;

    fn run(catalogue: &Catalogue, input: WriteModelInput) -> Result<WriteModelOutput, OperationError> {
        let file = catalogue.folder().model_file(&input.path)?;
        if file.is_dir() {
            return Err(OperationError::invalid_input(format!("`{}` is a folder", input.path)));
        }
        let (text, saved, problems) = match crabsim_model::load(&input.text) {
            Ok(model) => (model.save(), Saved::Canonical, Vec::new()),
            Err(problems) => (input.text, Saved::AsGiven, problems.iter().map(ModelProblem::from).collect()),
        };
        write(&file, &text).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.path)))?;
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
    const DESCRIPTION: &'static str = "Deletes a model file in the models folder. Files only, never folders. Results of runs of it stay in the results store.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = PathInput;
    type Output = DeleteModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<DeleteModelOutput, OperationError> {
        let (file, _) = read_file(catalogue, &input.path)?;
        std::fs::remove_file(&file).map_err(|error| OperationError::run_error(format!("`{}` can't be deleted: {error}", input.path)))?;
        Ok(DeleteModelOutput { deleted: input.path })
    }
}
