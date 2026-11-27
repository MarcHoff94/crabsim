//! crabsim's host for user code (ADR 0029, spec issue 156, PR 2): code modules, compiled to
//! WebAssembly and run by wasmtime in a sandbox, or Rust compiled natively for debugging, behind
//! the engine's `Hooks` trait, so the engine stays free of WebAssembly (ADR 0009).
//!
//! A `Library` holds the modules a model's hooks name, each with the functions it declares and
//! their parameters. `Library::bind` checks a hook's function and parameter values against them,
//! and `Code` runs the bound functions for a run: one instance of each module for the run, its
//! memory kept between calls, cleared at Reset and copied into every checkpoint. The interface
//! between crabsim and a module is in `docs/design/code-interface.md`, versioned by
//! `INTERFACE_VERSION`.

mod host;
pub mod native;
mod wasm;

use crabsim::hooks::{CallKind, Decision, HookCall, HookError, Hooks, Lifecycle};
use crabsim::{HookRun, ModelDescription, ObjectKind, SimTime, TriggerPoint};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub use host::Host;

/// The version of the interface between crabsim and a code module. A module built against
/// another is refused when it loads.
pub const INTERFACE_VERSION: u32 = 1;

/// The fuel a call may burn by default, in WebAssembly instructions: enough for any reasonable
/// hook, and a loop that never ends is found in a fraction of a second.
pub const DEFAULT_FUEL_PER_CALL: u64 = 10_000_000;

/// The memory a module's instance may take by default: 64 MiB.
pub const DEFAULT_MEMORY_LIMIT: usize = 64 << 20;

/// A parameter's type, as a module declares it and a model file gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    /// A whole number.
    Int,
    /// A number.
    Float,
    /// True or false, passed as 1 or 0.
    Bool,
    /// A length of simulation time, written as the UI shows time, passed as milliseconds.
    Time,
    /// An instance's name, resolved to its object when the model is built, so a misspelled name
    /// is a problem before the run.
    Object,
    /// Text.
    Text,
}

impl ParamType {
    /// Reads a type's name, as `int` or as the guest crate's type, `Int`.
    fn read(word: &str) -> Option<ParamType> {
        let word = word.rsplit("::").next().unwrap_or(word).trim().to_ascii_lowercase();
        Some(match word.as_str() {
            "int" => ParamType::Int,
            "float" => ParamType::Float,
            "bool" => ParamType::Bool,
            "time" => ParamType::Time,
            "object" => ParamType::Object,
            "text" => ParamType::Text,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            ParamType::Int => "int",
            ParamType::Float => "float",
            ParamType::Bool => "bool",
            ParamType::Time => "time",
            ParamType::Object => "object",
            ParamType::Text => "text",
        }
    }
}

/// A parameter a function declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub ty: ParamType,
}

/// A function a module declares, with its parameters in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    pub parameters: Vec<Parameter>,
}

impl Signature {
    /// Reads a signature as a module declares it: `every_nth(n:int,left:object)`.
    pub fn parse(text: &str) -> Result<Signature, String> {
        let text = text.trim();
        let (name, rest) = text.split_once('(').ok_or_else(|| format!("`{text}` has no parameter list"))?;
        let list = rest.strip_suffix(')').ok_or_else(|| format!("`{text}` doesn't end with `)`"))?;
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("`{name}` isn't a function's name"));
        }
        let mut parameters: Vec<Parameter> = Vec::new();
        for part in list.split(',').map(str::trim).filter(|part| !part.is_empty()) {
            let (param, ty) = part.split_once(':').ok_or_else(|| format!("parameter `{part}` of `{name}` has no type"))?;
            let ty = ParamType::read(ty.trim()).ok_or_else(|| format!("parameter `{param}` of `{name}` has type `{}`, which isn't int, float, bool, time, object or text", ty.trim()))?;
            let param = param.trim().to_string();
            if parameters.iter().any(|other| other.name == param) {
                return Err(format!("`{name}` has two parameters named `{param}`"));
            }
            parameters.push(Parameter { name: param, ty });
        }
        Ok(Signature { name: name.to_string(), parameters })
    }
}

impl std::fmt::Display for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parameters: Vec<String> = self.parameters.iter().map(|p| format!("{}: {}", p.name, p.ty.name())).collect();
        write!(f, "{}({})", self.name, parameters.join(", "))
    }
}

/// A parameter's value for one hook. A bool, a time and an object pass as `Int`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Text(String),
}

/// A module in a library: compiled WebAssembly or Rust compiled natively.
pub struct Module {
    name: String,
    hash: String,
    signatures: Vec<Signature>,
    kind: ModuleKind,
}

enum ModuleKind {
    Wasm(Arc<wasm::Compiled>),
    Native(Vec<native::NativeFunction>),
}

impl Module {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The SHA-256 of its file, in hex, which a run records so it can be told which code ran.
    /// `native` for a native module.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// The functions it declares, in its own order.
    pub fn functions(&self) -> &[Signature] {
        &self.signatures
    }

    pub fn function(&self, name: &str) -> Option<&Signature> {
        self.signatures.iter().find(|signature| signature.name == name)
    }

    pub fn is_native(&self) -> bool {
        matches!(self.kind, ModuleKind::Native(_))
    }
}

/// The code modules a model's hooks name.
#[derive(Default)]
pub struct Library {
    wasm: Option<wasm::Runtime>,
    modules: Vec<Module>,
}

impl Library {
    pub fn new() -> Library {
        Library::default()
    }

    /// Loads `<name>.wasm` from a models folder's `code/` folder, unless it is loaded already.
    pub fn load_from(&mut self, code_folder: &Path, name: &str) -> Result<(), String> {
        if self.module(name).is_some() {
            return Ok(());
        }
        let path = code_folder.join(format!("{name}.wasm"));
        let bytes = std::fs::read(&path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => format!("there is no code module `{name}`: no file code/{name}.wasm"),
            _ => format!("code/{name}.wasm can't be read: {error}"),
        })?;
        self.load_wasm(name, &bytes)
    }

    /// Loads a WebAssembly module from its bytes, under a name.
    pub fn load_wasm(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if self.module(name).is_some() {
            return Err(format!("there are two code modules named `{name}`"));
        }
        let runtime = match &mut self.wasm {
            Some(runtime) => runtime,
            None => self.wasm.insert(wasm::Runtime::new()?),
        };
        let compiled = runtime.compile(bytes).map_err(|error| format!("code module `{name}`: {error}"))?;
        let signatures = compiled.signatures.clone();
        self.modules.push(Module { name: name.to_string(), hash: sha256_hex(bytes), signatures, kind: ModuleKind::Wasm(Arc::new(compiled)) });
        Ok(())
    }

    /// Adds a module of Rust compiled natively, which runs in crabsim's own process, unsandboxed
    /// and without fuel, for debugging hooks in the user's own `cargo test` (spec issue 156).
    pub fn add_native(&mut self, module: native::NativeModule) -> Result<(), String> {
        if self.module(&module.name).is_some() {
            return Err(format!("there are two code modules named `{}`", module.name));
        }
        let (signatures, functions) = module.functions.into_iter().unzip();
        self.modules.push(Module { name: module.name, hash: "native".to_string(), signatures, kind: ModuleKind::Native(functions) });
        Ok(())
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|module| module.name == name)
    }

    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    /// Checks a hook's function, `<module>.<function>`, and its parameter values, given as JSON,
    /// against what the module declares. An object parameter's instance name is found through
    /// `object`, so a misspelled name is a problem now rather than in the run. Every problem is
    /// returned, each with the parameter it is about, if any.
    pub fn bind(&self, function: &str, with: &serde_json::Map<String, serde_json::Value>, object: &dyn Fn(&str) -> Option<u32>) -> Result<Binding, Vec<BindProblem>> {
        let problem = |parameter: Option<&str>, message: String| BindProblem { parameter: parameter.map(str::to_string), message };
        let Some((module_name, function_name)) = function.split_once('.') else {
            return Err(vec![problem(None, format!("`{function}` isn't `<module>.<function>`"))]);
        };
        let Some(module) = self.module(module_name) else {
            return Err(vec![problem(None, format!("there is no code module `{module_name}`"))]);
        };
        let Some(signature) = module.function(function_name) else {
            let known: Vec<&str> = module.signatures.iter().map(|signature| signature.name.as_str()).collect();
            return Err(vec![problem(None, format!("code module `{module_name}` has no function `{function_name}`; it has {}", if known.is_empty() { "none".to_string() } else { known.join(", ") }))]);
        };
        let mut problems = Vec::new();
        for name in with.keys() {
            if signature.parameters.iter().all(|parameter| &parameter.name != name) {
                problems.push(problem(Some(name), format!("`{function}` has no parameter `{name}`; its parameters are {signature}")));
            }
        }
        let mut values = Vec::new();
        for parameter in &signature.parameters {
            let name = parameter.name.as_str();
            let Some(value) = with.get(name) else {
                problems.push(problem(Some(name), format!("`{function}` needs its parameter `{name}`, of type {}", parameter.ty.name())));
                continue;
            };
            match value_of(parameter.ty, value, object) {
                Ok(value) => values.push(value),
                Err(message) => problems.push(problem(Some(name), format!("parameter `{name}` of `{function}`: {message}"))),
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        let module_index = self.modules.iter().position(|m| m.name == module_name).expect("found above");
        let function_index = module.signatures.iter().position(|s| s.name == function_name).expect("found above");
        Ok(Binding { module: module_index, function: function_index, name: function.to_string(), values })
    }
}

/// A problem with a hook's function or its parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindProblem {
    /// The parameter it is about, if any.
    pub parameter: Option<String>,
    pub message: String,
}

fn value_of(ty: ParamType, value: &serde_json::Value, object: &dyn Fn(&str) -> Option<u32>) -> Result<Value, String> {
    use serde_json::Value as Json;
    match (ty, value) {
        (ParamType::Int, Json::Number(number)) => number.as_i64().map(Value::Int).ok_or_else(|| format!("{number} isn't a whole number")),
        (ParamType::Float, Json::Number(number)) => number.as_f64().filter(|f| f.is_finite()).map(Value::Float).ok_or_else(|| format!("{number} isn't a number")),
        (ParamType::Bool, Json::Bool(b)) => Ok(Value::Int(*b as i64)),
        (ParamType::Time, Json::String(text)) => crabsim::time::parse_time(text).map(|time| Value::Int(time.as_millis() as i64)).ok_or_else(|| format!("`{text}` isn't a time, such as `1:30` or `2d 06:00:00`")),
        (ParamType::Object, Json::String(name)) => object(name).map(|id| Value::Int(id as i64)).ok_or_else(|| format!("there is no instance `{name}`")),
        (ParamType::Text, Json::String(text)) => Ok(Value::Text(text.clone())),
        (ty, other) => Err(format!("{other} isn't {}", match ty {
            ParamType::Int => "a whole number",
            ParamType::Float => "a number",
            ParamType::Bool => "true or false",
            ParamType::Time => "a time as text, such as \"1:30\"",
            ParamType::Object => "an instance's name",
            ParamType::Text => "text",
        })),
    }
}

/// A hook's function with its parameter values, checked against its module.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    module: usize,
    function: usize,
    /// `<module>.<function>`, as the model names it.
    name: String,
    values: Vec<Value>,
}

impl Binding {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }
}

/// A line code logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub time: SimTime,
    /// The object whose hook logged it, if any.
    pub object: Option<String>,
    /// `<module>.<function>`.
    pub function: String,
    pub message: String,
}

impl std::fmt::Display for LogLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}  {}  {}: {}", crabsim::time::format_time_exact(self.time), self.object.as_deref().unwrap_or("-"), self.function, self.message)
    }
}

/// Where log lines go.
pub type LogSink = Arc<dyn Fn(&LogLine) + Send + Sync>;

/// How code runs.
#[derive(Clone)]
pub struct Options {
    /// The fuel each call may burn, in WebAssembly instructions.
    pub fuel_per_call: u64,
    /// The memory each module's instance may take, in bytes.
    pub memory_limit: usize,
    /// Where log lines go. Nowhere by default.
    pub log: Option<LogSink>,
    /// The MU class of each MU kind, by its id, for code reading an MU's class.
    pub mu_classes: Vec<String>,
}

impl Default for Options {
    fn default() -> Options {
        Options { fuel_per_call: DEFAULT_FUEL_PER_CALL, memory_limit: DEFAULT_MEMORY_LIMIT, log: None, mu_classes: Vec::new() }
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options").field("fuel_per_call", &self.fuel_per_call).field("memory_limit", &self.memory_limit).field("log", &self.log.is_some()).finish()
    }
}

/// What every copy of a run's code shares: the library, the bound functions and the model.
pub(crate) struct Shared {
    library: Arc<Library>,
    /// By `HookFunction`.
    bindings: Vec<Binding>,
    names: Vec<String>,
    by_name: HashMap<String, u32>,
    kinds: Vec<ObjectKind>,
    successors: Vec<Vec<u32>>,
    predecessors: Vec<Vec<u32>>,
    options: Options,
}

/// The code of a run: the engine's `Hooks`, whose `HookFunction` is a binding's place in the list
/// it was made with.
pub struct Code {
    shared: Arc<Shared>,
    /// One per module in the library, made the first time one of its functions is bound.
    instances: Vec<Option<Instance>>,
    /// Each object's random numbers for code, made the first time code asks.
    random: Vec<Option<u64>>,
}

enum Instance {
    Wasm(wasm::Instance),
    Native(native::Instance),
}

impl Instance {
    fn new(module: &Module, options: &Options) -> Result<Instance, String> {
        Ok(match &module.kind {
            ModuleKind::Wasm(compiled) => Instance::Wasm(wasm::Instance::new(compiled.clone(), options)?),
            ModuleKind::Native(functions) => Instance::Native(native::Instance::new(functions.clone())),
        })
    }

    fn duplicate(&self) -> Instance {
        match self {
            Instance::Wasm(instance) => Instance::Wasm(instance.duplicate()),
            Instance::Native(instance) => Instance::Native(instance.duplicate()),
        }
    }
}

impl Code {
    /// The code for runs of `description`, its hooks numbered by their place in `bindings`.
    /// Instantiates every module a binding names, so a module that can't run is found now.
    pub fn new(library: Arc<Library>, bindings: Vec<Binding>, description: &ModelDescription, options: Options) -> Result<Code, String> {
        let count = description.object_count();
        let names: Vec<String> = (0..count).map(|id| description.name(id).unwrap_or_default().to_string()).collect();
        let by_name = names.iter().enumerate().map(|(id, name)| (name.clone(), id as u32)).collect();
        let kinds = (0..count).map(|id| description.kind(id)).collect();
        let successors: Vec<Vec<u32>> = (0..count).map(|id| description.successors(id).to_vec()).collect();
        let mut predecessors = vec![Vec::new(); count as usize];
        for (from, list) in successors.iter().enumerate() {
            for &to in list {
                predecessors[to as usize].push(from as u32);
            }
        }
        let mut instances: Vec<Option<Instance>> = library.modules.iter().map(|_| None).collect();
        for binding in &bindings {
            if instances[binding.module].is_none() {
                instances[binding.module] = Some(Instance::new(&library.modules[binding.module], &options)?);
            }
        }
        let shared = Shared { library, bindings, names, by_name, kinds, successors, predecessors, options };
        Ok(Code { shared: Arc::new(shared), instances, random: vec![None; count as usize] })
    }

    /// The modules the code uses, with their hashes.
    pub fn modules(&self) -> impl Iterator<Item = &Module> {
        self.shared.library.modules.iter().zip(&self.instances).filter(|(_, instance)| instance.is_some()).map(|(module, _)| module)
    }
}

/// How a call is passed to a module: its kind, with the trigger point in the second byte, the
/// object, the MU and one more number (`docs/design/code-interface.md`).
pub(crate) fn encode(call: &HookCall) -> (i32, i32, i32, i32) {
    let point = |at: TriggerPoint| match at {
        TriggerPoint::Front => 1 << 8,
        TriggerPoint::BookingPoint => 2 << 8,
        TriggerPoint::Rear => 3 << 8,
    };
    match &call.kind {
        CallKind::Entry { object, mu, at } => (abi::ENTRY | point(*at), *object as i32, mu.id as i32, -1),
        CallKind::Exit { object, mu, at, to } => (abi::EXIT | point(*at), *object as i32, mu.id as i32, *to as i32),
        CallKind::Distribution { object, mu } => (abi::DISTRIBUTION, *object as i32, mu.id as i32, -1),
        CallKind::Timer { data } => (abi::TIMER, -1, -1, *data as i32),
        CallKind::Lifecycle(moment) => (
            match moment {
                Lifecycle::BeforeInit => abi::BEFORE_INIT,
                Lifecycle::AfterInit => abi::AFTER_INIT,
                Lifecycle::BeforeReset => abi::BEFORE_RESET,
                Lifecycle::AfterReset => abi::AFTER_RESET,
            },
            -1,
            -1,
            -1,
        ),
    }
}

/// The numbers of the interface, shared with the guest crate's copy of them.
pub mod abi {
    pub const ENTRY: i32 = 0;
    pub const EXIT: i32 = 1;
    pub const DISTRIBUTION: i32 = 2;
    pub const TIMER: i32 = 3;
    pub const BEFORE_INIT: i32 = 4;
    pub const AFTER_INIT: i32 = 5;
    pub const BEFORE_RESET: i32 = 6;
    pub const AFTER_RESET: i32 = 7;
    /// A distribution's answers below 0.
    pub const DEFAULT: i32 = -1;
    pub const HOLD: i32 = -2;
}

impl Hooks for Code {
    fn call(&mut self, call: &HookCall, run: &mut HookRun<'_>) -> Result<Option<Decision>, HookError> {
        let shared = self.shared.clone();
        let binding = &shared.bindings[call.function as usize];
        let args = encode(call);
        let mut host = Host::new(run, &shared, binding, call, &mut self.random);
        let instance = self.instances[binding.module].as_mut().expect("every bound module has an instance");
        let answer = match instance {
            Instance::Wasm(instance) => instance.call(binding.function, &mut host, args),
            Instance::Native(instance) => instance.call(binding.function, &mut host, args),
        }
        .map_err(|message| HookError(format!("{}: {message}", binding.name)))?;
        if !matches!(call.kind, CallKind::Distribution { .. }) {
            return Ok(None);
        }
        match answer {
            abi::DEFAULT => Ok(Some(Decision::Default)),
            abi::HOLD => Ok(Some(Decision::Hold)),
            successor if successor >= 0 => Ok(Some(Decision::Successor(successor as u32))),
            other => Err(HookError(format!("{}: it answered {other}, which is no successor, hold or default", binding.name))),
        }
    }

    fn clear(&mut self) {
        for (module, instance) in self.shared.library.modules.iter().zip(&mut self.instances) {
            if instance.is_some() {
                *instance = Some(Instance::new(module, &self.shared.options).expect("a module that was instantiated once is instantiated again"));
            }
        }
        self.random.iter_mut().for_each(|random| *random = None);
    }

    fn clone_hooks(&self) -> Box<dyn Hooks> {
        Box::new(Code { shared: self.shared.clone(), instances: self.instances.iter().map(|instance| instance.as_ref().map(Instance::duplicate)).collect(), random: self.random.clone() })
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// For crabsim's own tests: building code modules from their sources.
#[doc(hidden)]
pub mod testing {
    use std::path::{Path, PathBuf};

    /// The repository's root, where `code-examples` is.
    pub fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// Builds a module crate for WebAssembly, into a target folder of its own, and gives the path
    /// of its `.wasm`. Cargo's lock on that folder keeps test binaries from building at once.
    pub fn build(crate_dir: &Path, name: &str) -> PathBuf {
        let target_dir = repository().join("target/wasm-modules");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = std::process::Command::new(cargo)
            .args(["build", "--release", "--target", "wasm32-unknown-unknown", "--quiet", "--target-dir"])
            .arg(&target_dir)
            .arg("--manifest-path")
            .arg(crate_dir.join("Cargo.toml"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .status()
            .expect("cargo runs");
        assert!(status.success(), "{name} builds for wasm32-unknown-unknown");
        target_dir.join(format!("wasm32-unknown-unknown/release/{name}.wasm"))
    }

    /// The example module `routing`, built from `code-examples/routing`.
    pub fn routing() -> PathBuf {
        static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        BUILT.get_or_init(|| build(&repository().join("code-examples/routing"), "routing")).clone()
    }
}
