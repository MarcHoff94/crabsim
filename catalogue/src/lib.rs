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

mod error;
mod folder;
mod jobs;
mod model_files;
mod process;
mod results;
mod store;

pub use crate::error::{ErrorKind, ModelProblem, OperationError};
pub use crate::folder::{ModelsFolder, CRABSIM_FOLDER};
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
    /// The JSON Schema of its input, always an object.
    pub input_schema: Value,
    /// The JSON Schema of its output, always an object.
    pub output_schema: Value,
}

type Handler = fn(&Catalogue, Value) -> Result<Value, OperationError>;

struct Entry {
    info: OperationInfo,
    handler: Handler,
}

fn entry<O: Operation>() -> Entry {
    Entry {
        info: OperationInfo {
            name: O::NAME,
            description: O::DESCRIPTION,
            effect: O::EFFECT,
            input_schema: schema_for::<O::Input>(Nulls::Refuse),
            output_schema: schema_for::<O::Output>(Nulls::Allow),
        },
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
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Mcp => "mcp",
            Source::Cli => "cli",
        }
    }
}

/// How to open a catalogue.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The models folder: everything the operations touch is inside it.
    pub models: PathBuf,
    /// Who starts the runs, as the results store records it.
    pub source: Source,
    /// How many threads run observations. One per CPU core if left out.
    pub threads: Option<usize>,
}

impl Settings {
    pub fn new(models: impl Into<PathBuf>, source: Source) -> Settings {
        Settings { models: models.into(), source, threads: None }
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
    /// This process's lock, taken with the store. Last, so it is dropped last: until its jobs have
    /// stopped, no other process may take their runs for interrupted.
    process: std::sync::OnceLock<process::ProcessLock>,
}

impl Catalogue {
    /// The catalogue on a models folder. Refused if the folder can't be opened or holds source
    /// code: see [`ModelsFolder::open`].
    pub fn open(settings: Settings) -> Result<Catalogue, OperationError> {
        let folder = ModelsFolder::open(&settings.models)?;
        let threads = settings.threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |cores| cores.get()));
        let jobs = jobs::Jobs::new(folder.crabsim_folder(), threads);
        Ok(Catalogue { folder, source: settings.source, store: Mutex::new(None), entries: model_files::entries(), jobs, process: std::sync::OnceLock::new() })
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
        (entry.handler)(self, input)
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
            let lock = process::ProcessLock::acquire(&crabsim_folder)
                .map_err(|error| OperationError::run_error(format!("this process's lock in the results store's folder can't be made: {error}")))?;
            let lock = self.process.get_or_init(|| lock);
            process::tidy(&crabsim_folder, lock.id());
            opened.sweep(lock.id())?;
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
