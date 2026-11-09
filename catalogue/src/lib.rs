//! crabsim's operation catalogue: every capability as an operation with a name, a description, a
//! typed input and a typed output, defined once here. Front-ends, such as the MCP server and the
//! CLI, only translate between their protocol and [`Catalogue::operations`] and
//! [`Catalogue::call`] (ADR 0007). The design, including what the operations may touch, is in
//! `docs/design/operation-catalogue.md`.
//!
//! ```no_run
//! use crabsim_catalogue::{Catalogue, Settings, Source};
//! let catalogue = Catalogue::open(Settings::new("my-models", Source::Cli)).unwrap();
//! let listed = catalogue.call("list_models", serde_json::json!({})).unwrap();
//! ```
//!
//! What crabsim's operations touch: only `.json` model files inside one models folder, and
//! crabsim's own `.crabsim` folder there, which holds the results store. Never crabsim's code,
//! never commands, never the network.
//!
//! The user works through the UI or the CLI, and only one at a time changes a folder's models: a
//! UI's catalogue holds an OS lock on `.crabsim/user.lock` while it lives, a second UI is refused,
//! and the CLI may only read and run meanwhile (ADR 0011).

mod compare;
mod edit;
mod editing;
mod error;
mod export;
mod folder;
mod graph;
mod jobs;
mod model_files;
mod process;
mod results;
mod store;

pub use crate::editing::{EDITING_FILE, EDIT_TIMEOUT};
pub use crate::error::{ErrorKind, ModelProblem, OperationError};
pub use crate::folder::{ModelsFolder, CRABSIM_FOLDER, EXPORTS_FOLDER};
pub use crate::jobs::{observation_seed, RunState, MAX_OBSERVATIONS};
pub use crate::results::STATISTICS;
pub use crate::store::{Store, QUERY_ROW_LIMIT, STORE_FILE};

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// One thing crabsim can do, with typed input and output. Each is defined once, here, and every
/// front-end offers it.
pub trait Operation {
    /// Snake case, verb first, such as `validate_model`.
    const NAME: &'static str;
    /// What it does, written for an agent deciding whether and how to call it.
    const DESCRIPTION: &'static str;
    /// Whether it only reads, or may change or delete what's there, for a harness deciding
    /// whether to ask the user first.
    const EFFECT: Effect;
    /// Only for the user, through the UI or the CLI, such as revoking an agent's claim: the MCP
    /// server doesn't offer it, and the catalogue refuses it to an agent (ADR 0011).
    const USER_ONLY: bool = false;
    /// Changes a model file or who may change it, which the CLI is refused while a UI has the folder.
    const CHANGES_MODELS: bool = false;
    /// Always a JSON object.
    type Input: DeserializeOwned + JsonSchema;
    /// Always a JSON object.
    type Output: Serialize + JsonSchema;

    fn run(catalogue: &Catalogue, input: Self::Input) -> Result<Self::Output, OperationError>;
}

/// What an operation does to what's already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Changes nothing.
    ReadOnly,
    /// Adds or changes something, but never overwrites or deletes, such as starting a run.
    Additive,
    /// May overwrite or delete what's there, such as writing over a model file.
    Destructive,
}

/// An operation as front-ends see it.
#[derive(Debug, Clone, Serialize)]
pub struct OperationInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// Only for the user: a front-end for agents leaves it out.
    pub user_only: bool,
    /// The JSON Schema of its input, always an object.
    pub input_schema: Value,
    /// The JSON Schema of its output, always an object.
    pub output_schema: Value,
}

type Handler = fn(&Catalogue, Value) -> Result<Value, OperationError>;

struct Entry {
    info: OperationInfo,
    changes_models: bool,
    handler: Handler,
}

fn entry<O: Operation>() -> Entry {
    Entry {
        info: OperationInfo {
            name: O::NAME,
            description: O::DESCRIPTION,
            effect: O::EFFECT,
            user_only: O::USER_ONLY,
            input_schema: schema_for::<O::Input>(Nulls::Refuse),
            output_schema: schema_for::<O::Output>(Nulls::Allow),
        },
        changes_models: O::CHANGES_MODELS,
        handler: |catalogue, input| {
            let input: O::Input = serde_json::from_value(input)
                .map_err(|error| OperationError::invalid_input(format!("the input to `{}` doesn't fit: {error}", O::NAME)))?;
            let output = O::run(catalogue, input)?;
            Ok(serde_json::to_value(output).expect("an operation's output always serialises"))
        },
    }
}

/// Whether a schema lets an optional field be `null`.
#[derive(Clone, Copy, PartialEq)]
enum Nulls {
    /// For inputs: an optional field is left out, as in a model file, whose schema refuses
    /// `null` too, so the two schemas stay the same.
    Refuse,
    /// For outputs, which may hold `null` for a figure that doesn't exist.
    Allow,
}

/// A JSON Schema (draft 2020-12) of `T`, as a front-end publishes it.
fn schema_for<T: JsonSchema>(nulls: Nulls) -> Value {
    let generator = schemars::generate::SchemaSettings::draft2020_12().into_generator();
    let mut schema = serde_json::to_value(generator.into_root_schema_for::<T>()).expect("a schema always serialises");
    tidy(&mut schema, nulls);
    schema
}

/// Joins the lines of descriptions, which come from wrapped doc comments, and for inputs takes
/// `null` out of every choice of types, as `crabsim_model::schema` does for the model file.
fn tidy(value: &mut Value, nulls: Nulls) {
    match value {
        Value::Object(fields) => {
            if nulls == Nulls::Refuse {
                if let Some(Value::Array(types)) = fields.get_mut("type") {
                    types.retain(|kind| kind != "null");
                    if types.len() == 1 {
                        let only = types.remove(0);
                        fields.insert("type".into(), only);
                    }
                }
                if let Some(Value::Array(choices)) = fields.get_mut("anyOf") {
                    choices.retain(|choice| choice.get("type").is_none_or(|kind| kind != "null"));
                    if choices.len() == 1 {
                        let Value::Object(only) = choices.remove(0) else { unreachable!("a choice is a schema") };
                        fields.remove("anyOf");
                        fields.extend(only);
                    }
                }
                // `#[serde(default)]` on an optional field, which keeps it optional where a schema
                // is written by hand, makes its default `null`, which the input refuses.
                if fields.get("default") == Some(&Value::Null) {
                    fields.remove("default");
                }
            }
            if let Some(Value::String(description)) = fields.get_mut("description") {
                *description = description.replace('\n', " ");
            }
            fields.values_mut().for_each(|value| tidy(value, nulls));
        }
        Value::Array(items) => items.iter_mut().for_each(|value| tidy(value, nulls)),
        _ => {}
    }
}

/// Who started a simulation run, as the results store keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// An agent through the MCP server.
    Mcp,
    /// A person through the command line.
    Cli,
    /// A person through the UI, which holds the models folder while it is open.
    Ui,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Mcp => "mcp",
            Source::Cli => "cli",
            Source::Ui => "ui",
        }
    }

    /// The user, through the UI or the CLI, rather than an agent.
    pub fn is_user(self) -> bool {
        self != Source::Mcp
    }
}

/// The file in `.crabsim` a UI holds an OS lock on while it has the models folder open.
pub const USER_LOCK_FILE: &str = "user.lock";

/// How to open a catalogue.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The models folder: everything the operations touch is inside it.
    pub models: PathBuf,
    /// Who starts the runs, as the results store records it.
    pub source: Source,
    /// How many threads run observations. One per CPU core if left out.
    pub threads: Option<usize>,
    /// How long an agent's claim lasts unused.
    pub edit_timeout: std::time::Duration,
}

impl Settings {
    pub fn new(models: impl Into<PathBuf>, source: Source) -> Settings {
        Settings { models: models.into(), source, threads: None, edit_timeout: EDIT_TIMEOUT }
    }
}

/// Every operation, on one models folder.
pub struct Catalogue {
    folder: ModelsFolder,
    source: Source,
    /// Opened on first use, so a folder only gets `.crabsim` once something needs the store.
    store: Mutex<Option<Store>>,
    entries: Vec<Entry>,
    jobs: jobs::Jobs,
    edit_timeout: std::time::Duration,
    /// The name an agent's harness gave, shown with its claims.
    client_name: Mutex<String>,
    /// For the user: changes go through even while an agent holds the right, which keeps it.
    take_over: std::sync::atomic::AtomicBool,
    /// A UI's hold on the models folder, released when the catalogue is dropped or the process
    /// ends however it does.
    _user_lock: Option<std::fs::File>,
    /// This process's lock, taken with the store. Last, so it is dropped last: until its jobs have
    /// stopped, no other process may take their runs for interrupted.
    process: std::sync::OnceLock<process::ProcessLock>,
}

impl Catalogue {
    /// The catalogue on a models folder. Refused if the folder can't be opened or holds source
    /// code: see [`ModelsFolder::open`]. A UI's catalogue holds the folder, and is refused with
    /// `folder_in_use` if another UI already does.
    pub fn open(settings: Settings) -> Result<Catalogue, OperationError> {
        let folder = ModelsFolder::open(&settings.models)?;
        let user_lock = if settings.source == Source::Ui { Some(hold_folder(&folder)?) } else { None };
        let threads = settings.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |cores| cores.get()));
        let jobs = jobs::Jobs::new(folder.crabsim_folder(), threads);
        Ok(Catalogue {
            folder,
            source: settings.source,
            store: Mutex::new(None),
            entries: model_files::entries(),
            jobs,
            edit_timeout: settings.edit_timeout,
            client_name: Mutex::new("an agent".to_string()),
            take_over: std::sync::atomic::AtomicBool::new(false),
            _user_lock: user_lock,
            process: std::sync::OnceLock::new(),
        })
    }

    /// Every operation, with its description and schemas.
    pub fn operations(&self) -> impl Iterator<Item = &OperationInfo> {
        self.entries.iter().map(|entry| &entry.info)
    }

    /// Runs the operation called `name` on `input`, a JSON object, giving its output as JSON.
    pub fn call(&self, name: &str, input: Value) -> Result<Value, OperationError> {
        let entry = self.entries.iter().find(|entry| entry.info.name == name).ok_or_else(|| {
            let names: Vec<_> = self.operations().map(|operation| operation.name).collect();
            OperationError::invalid_input(format!("no operation is called `{name}`. There are: {}", names.join(", ")))
        })?;
        if entry.info.user_only && self.source == Source::Mcp {
            return Err(OperationError::invalid_input(format!("`{name}` is only for the user, through the UI or the CLI")));
        }
        if entry.changes_models && self.source == Source::Cli && ui_holds(&self.folder) {
            return Err(OperationError::new(
                ErrorKind::FolderInUse,
                format!("a crabsim window has this models folder open, so `{name}` is refused here: change models and who edits them in the window. Reading and running work here"),
            ));
        }
        (entry.handler)(self, input)
    }

    /// The name an agent's harness gave when it connected, such as `claude-code`, which the edit
    /// queue shows the user.
    pub fn set_client_name(&self, name: &str) {
        *self.client_name.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = name.to_string();
    }

    /// For the user only: lets the following changes go through while an agent holds the editor
    /// right, which the agent keeps, as the CLI's `--take-over` does for one command. An agent's
    /// catalogue ignores it.
    pub fn set_take_over(&self, on: bool) {
        self.take_over.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn take_over(&self) -> bool {
        self.source.is_user() && self.take_over.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn client_name(&self) -> String {
        self.client_name.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    pub(crate) fn edit_timeout(&self) -> std::time::Duration {
        self.edit_timeout
    }

    /// This process's lock id, taking the lock on first use.
    pub(crate) fn process_lock(&self) -> Result<&str, OperationError> {
        if let Some(lock) = self.process.get() {
            return Ok(lock.id());
        }
        let lock = process::ProcessLock::acquire(&self.folder.crabsim_folder())
            .map_err(|error| OperationError::run_error(format!("this process's lock in `.crabsim` can't be made: {error}")))?;
        Ok(self.process.get_or_init(|| lock).id())
    }

    pub fn folder(&self) -> &ModelsFolder {
        &self.folder
    }

    pub fn source(&self) -> Source {
        self.source
    }

    pub(crate) fn jobs(&self) -> &jobs::Jobs {
        &self.jobs
    }

    /// The state of a run this process is running, if it is one.
    pub(crate) fn jobs_status(&self, id: &str) -> Option<RunState> {
        self.jobs.state(id)
    }

    /// The results store, created on first use. Opening it also takes this process's lock and
    /// marks interrupted the runs of processes that have ended.
    pub(crate) fn store(&self) -> Result<StoreGuard<'_>, OperationError> {
        let mut store = self.store.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if store.is_none() {
            let crabsim_folder = self.folder.crabsim_folder();
            let opened = Store::open(&crabsim_folder)?;
            let id = self.process_lock()?;
            process::tidy(&crabsim_folder, id);
            opened.sweep(id)?;
            *store = Some(opened);
        }
        Ok(StoreGuard(store))
    }

    /// This process's lock id, as runs record it. Only once the store is open.
    pub(crate) fn process_id(&self) -> &str {
        self.process.get().map_or("", process::ProcessLock::id)
    }

    /// The results store, after marking interrupted the runs of processes that have ended.
    pub(crate) fn swept_store(&self) -> Result<StoreGuard<'_>, OperationError> {
        let store = self.store()?;
        store.sweep(self.process_id())?;
        Ok(store)
    }
}

/// Takes the models folder for a UI: an OS lock on `.crabsim/user.lock`, released however the
/// process ends.
fn hold_folder(folder: &ModelsFolder) -> Result<std::fs::File, OperationError> {
    let crabsim = folder.crabsim_folder();
    let fail = |error: std::io::Error| OperationError::run_error(format!("the models folder can't be held for this window: {error}"));
    std::fs::create_dir_all(&crabsim).map_err(fail)?;
    let file = std::fs::File::options().create(true).truncate(false).write(true).open(crabsim.join(USER_LOCK_FILE)).map_err(fail)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(OperationError::new(
            ErrorKind::FolderInUse,
            "another crabsim window has this models folder open. Open a copy of the folder to work on a variant",
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(fail(error)),
    }
}

/// Whether a UI has the models folder open. If the lock can't be tested, no UI counts as holding
/// it, so the CLI is never refused by mistake.
fn ui_holds(folder: &ModelsFolder) -> bool {
    let Ok(file) = std::fs::File::options().write(true).open(folder.crabsim_folder().join(USER_LOCK_FILE)) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// The results store, held for one operation.
pub(crate) struct StoreGuard<'a>(MutexGuard<'a, Option<Store>>);

impl std::ops::Deref for StoreGuard<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        self.0.as_ref().expect("opened before handing out")
    }
}

impl std::ops::DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        self.0.as_mut().expect("opened before handing out")
    }
}
