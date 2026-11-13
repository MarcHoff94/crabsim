//! What a failed operation gives back: a kind an agent can act on, a message saying why, and for
//! an invalid model every problem in it.

use schemars::JsonSchema;
use serde::Serialize;
use std::fmt;

/// What kind of failure an operation had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The input doesn't fit the operation, such as a missing field or a length that isn't a time.
    InvalidInput,
    /// The model has problems, each listed with where it is.
    InvalidModel,
    /// No such model file, folder, run, object or class.
    NotFound,
    /// The path leaves the models folder, or names crabsim's own `.crabsim` folder.
    OutsideModelsFolder,
    /// The target already exists, and the operation never overwrites.
    AlreadyExists,
    /// Something went wrong while running or storing, such as a file that can't be written.
    RunError,
    /// You may not change this model file now: claim it first with claim_model, or the user has
    /// taken it and your claim is paused.
    NotEditor,
    /// Someone else holds this model file, so your claim is refused. Claims aren't queued.
    ModelInUse,
    /// The model file changed since your last change, by someone else: read it again.
    ModelChanged,
    /// The user has blocked this agent from changing models in this folder.
    Blocked,
    /// The file was written outside crabsim since crabsim last saved it, or crabsim never saved
    /// it: adopt_model accepts it as it is.
    ChangedOutside,
    /// The file isn't the version `expected_hash` names any more: read it again.
    Conflict,
    /// A crabsim window has the models folder open, so it alone changes models and who edits them
    /// there, and no second window may open it.
    FolderInUse,
    /// The model file's change history holds nothing to undo, or nothing to redo.
    NothingToUndo,
    /// The change to undo or redo was made by someone else: an agent undoes and redoes only its
    /// own latest changes.
    NotYours,
}

/// One thing wrong with a model, as `crabsim-model` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ModelProblem {
    /// A JSON Pointer to where it is in the file, such as `/instances/3/attributes/process_time`.
    /// Empty for the file as a whole.
    pub pointer: String,
    /// The instance or class it is about, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub message: String,
}

impl From<&crabsim_model::Problem> for ModelProblem {
    fn from(problem: &crabsim_model::Problem) -> ModelProblem {
        ModelProblem { pointer: problem.pointer.clone(), name: problem.name.clone(), message: problem.message.clone() }
    }
}

/// A failed operation.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct OperationError {
    pub kind: ErrorKind,
    pub message: String,
    /// For `invalid_model`, every problem in the model, in file order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<ModelProblem>,
}

impl OperationError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> OperationError {
        OperationError { kind, message: message.into(), problems: Vec::new() }
    }

    pub fn invalid_input(message: impl Into<String>) -> OperationError {
        OperationError::new(ErrorKind::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> OperationError {
        OperationError::new(ErrorKind::NotFound, message)
    }

    pub fn run_error(message: impl Into<String>) -> OperationError {
        OperationError::new(ErrorKind::RunError, message)
    }

    pub fn invalid_model(what: &str, problems: &[crabsim_model::Problem]) -> OperationError {
        OperationError {
            kind: ErrorKind::InvalidModel,
            message: format!("{what} has {} problem{}", problems.len(), if problems.len() == 1 { "" } else { "s" }),
            problems: problems.iter().map(ModelProblem::from).collect(),
        }
    }

    /// The error as JSON, as front-ends hand it on.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("an error always serialises")
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        for problem in &self.problems {
            let pointer = if problem.pointer.is_empty() { "/" } else { &problem.pointer };
            match &problem.name {
                Some(name) => write!(f, "\n{pointer} \"{name}\": {}", problem.message)?,
                None => write!(f, "\n{pointer}: {}", problem.message)?,
            }
        }
        Ok(())
    }
}

impl std::error::Error for OperationError {}

impl From<rusqlite::Error> for OperationError {
    fn from(error: rusqlite::Error) -> OperationError {
        OperationError::run_error(format!("the results store: {error}"))
    }
}
