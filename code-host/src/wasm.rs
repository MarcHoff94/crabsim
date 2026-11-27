//! WebAssembly modules, run by wasmtime: compiling and checking them, the sandbox (fuel, a memory
//! cap, nothing imported but crabsim's own functions), and copying an instance for a checkpoint.

use crate::host::Host;
use crate::{Options, Signature, INTERFACE_VERSION};
use std::sync::Arc;
use wasmtime::{Caller, Config, Engine, Extern, InstancePre, Linker, Memory, Mutability, Store, Trap, TypedFunc};

/// The import module's name.
const IMPORTS: &str = "crabsim";
/// Every function crabsim offers a module.
const IMPORT_NAMES: [&str; 32] = [
    "now", "object_count", "object_kind", "object_name", "find_object", "successor_count", "successor", "predecessor_count", "predecessor", "process_time",
    "object_state", "mus_held", "content", "statistic", "mu_kind", "mu_class_name", "mu_location", "mu_left_source", "param_int", "param_float", "param_text",
    "random", "move_mu", "place_mu", "set_process_time", "set_work_time", "pause", "stop", "call_at", "refusal", "log", "fail",
];
/// The custom section holding the functions a module declares, a line each, every line beginning
/// with the interface version it was built against: `1|every_nth(n:Int,left:Object)`.
const FUNCTIONS_SECTION: &str = "crabsim-functions";
/// What a hook function's export is called: this, then the function's name.
pub(crate) const EXPORT_PREFIX: &str = "crabsim:";

/// The engine and crabsim's imports, made once for every module a library loads.
pub(crate) struct Runtime {
    engine: Engine,
    linker: Linker<StoreData>,
}

/// A module compiled and checked, ready to be instantiated for each run.
pub(crate) struct Compiled {
    engine: Engine,
    pre: InstancePre<StoreData>,
    pub(crate) signatures: Vec<Signature>,
    /// The mutable globals it exports, which a checkpoint copies with its memory.
    mutable_globals: Vec<String>,
    has_initialize: bool,
}

/// What a module's store holds: the call in progress, and what went wrong in it.
pub(crate) struct StoreData {
    /// The `Host` of the call in progress, null between calls. Only the imports read it, and they
    /// run only inside `Instance::call`, which sets it for exactly that call.
    host: *mut (),
    memory_limit: usize,
    /// Whether the memory cap refused the module more memory.
    refused_memory: bool,
    /// What the code said when it panicked.
    failed: Option<String>,
}

// SAFETY: `host` is null except during `Instance::call`, which holds `&mut Instance` and so the
// store, on one thread, for the whole call; nothing reads it anywhere else.
unsafe impl Send for StoreData {}
unsafe impl Sync for StoreData {}

impl wasmtime::ResourceLimiter for StoreData {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        if desired > self.memory_limit {
            self.refused_memory = true;
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= 1 << 20)
    }
}

/// The host of the call in progress.
///
/// # Safety
/// Only from an import, which runs inside `Instance::call` while `host` points at its `Host`.
unsafe fn host<'c>(caller: &Caller<'_, StoreData>) -> &'c mut Host<'c, 'c> {
    let host = caller.data().host;
    assert!(!host.is_null(), "crabsim's imports run only during a call");
    // SAFETY: the caller's guarantee; `Host`'s lifetimes are those of the call, which outlives
    // every import it makes.
    unsafe { &mut *(host as *mut Host<'c, 'c>) }
}

fn memory(caller: &mut Caller<'_, StoreData>) -> wasmtime::Result<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => Ok(memory),
        _ => Err(wasmtime::Error::msg("the module exports no memory")),
    }
}

fn read_text(caller: &mut Caller<'_, StoreData>, ptr: i32, len: i32) -> wasmtime::Result<String> {
    let memory = memory(caller)?;
    let (start, len) = (ptr as u32 as usize, len as u32 as usize);
    let bytes = memory.data(&caller).get(start..start + len).ok_or_else(|| wasmtime::Error::msg("text outside the module's memory"))?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

/// Writes as much of `text` as fits in `cap` bytes at `ptr` and returns its whole length, so the
/// code can ask again with more room.
fn write_text(caller: &mut Caller<'_, StoreData>, ptr: i32, cap: i32, text: &str) -> wasmtime::Result<i32> {
    let memory = memory(caller)?;
    let (start, cap) = (ptr as u32 as usize, cap as u32 as usize);
    let n = text.len().min(cap);
    let target = memory.data_mut(&mut *caller).get_mut(start..start + n).ok_or_else(|| wasmtime::Error::msg("room outside the module's memory"))?;
    target.copy_from_slice(&text.as_bytes()[..n]);
    Ok(text.len() as i32)
}

impl Runtime {
    pub(crate) fn new() -> Result<Runtime, String> {
        let mut config = Config::new();
        config.consume_fuel(true);
        // The same results on every machine: NaNs are made canonical and relaxed SIMD
        // deterministic.
        config.cranelift_nan_canonicalization(true);
        config.relaxed_simd_deterministic(true);
        let engine = Engine::new(&config).map_err(|error| format!("WebAssembly can't start: {error}"))?;
        let mut linker = Linker::new(&engine);
        define_imports(&mut linker).map_err(|error| format!("crabsim's imports: {error}"))?;
        Ok(Runtime { engine, linker })
    }

    /// Compiles a module and checks it is a crabsim code module this crabsim can run.
    pub(crate) fn compile(&self, bytes: &[u8]) -> Result<Compiled, String> {
        let mut declared = String::new();
        let mut mutable_globals_defined = 0usize;
        for payload in wasmparser::Parser::new(0).parse_all(bytes) {
            match payload.map_err(|error| format!("isn't a WebAssembly module: {error}"))? {
                wasmparser::Payload::Version { encoding: wasmparser::Encoding::Component, .. } => {
                    return Err("is a WebAssembly component; crabsim runs plain modules (see docs/design/code-interface.md)".to_string());
                }
                wasmparser::Payload::CustomSection(section) if section.name() == FUNCTIONS_SECTION => {
                    declared.push_str(&String::from_utf8_lossy(section.data()));
                }
                wasmparser::Payload::GlobalSection(globals) => {
                    for global in globals {
                        if global.map_err(|error| error.to_string())?.ty.mutable {
                            mutable_globals_defined += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        let mut signatures: Vec<Signature> = Vec::new();
        for line in declared.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let (version, text) = line.split_once('|').ok_or_else(|| format!("declares `{line}` without an interface version"))?;
            if version.trim() != INTERFACE_VERSION.to_string() {
                return Err(format!("was built against crabsim's interface version {}, and this crabsim offers version {INTERFACE_VERSION}: build it again against this crabsim's crabsim-guest", version.trim()));
            }
            let signature = Signature::parse(text)?;
            if signatures.iter().any(|other| other.name == signature.name) {
                return Err(format!("declares the function `{}` twice", signature.name));
            }
            signatures.push(signature);
        }
        // The linker puts the declarations in an order of its own, so they are kept by name.
        signatures.sort_by(|a, b| a.name.cmp(&b.name));
        if signatures.is_empty() {
            return Err("isn't a crabsim code module: it declares no hook functions (write them with crabsim-guest's `hook!`)".to_string());
        }
        let module = wasmtime::Module::new(&self.engine, bytes).map_err(|error| format!("doesn't compile: {error:#}"))?;
        for import in module.imports() {
            if import.module() != IMPORTS || !IMPORT_NAMES.contains(&import.name()) {
                return Err(format!(
                    "imports `{}::{}`, which crabsim doesn't offer: code gets no files, network, clock or randomness of its own (build it for wasm32-unknown-unknown)",
                    import.module(),
                    import.name()
                ));
            }
        }
        let mut exported_mutable = Vec::new();
        let mut has_memory = false;
        let mut has_initialize = false;
        for export in module.exports() {
            match export.ty() {
                wasmtime::ExternType::Global(global) if global.mutability() == Mutability::Var => exported_mutable.push(export.name().to_string()),
                wasmtime::ExternType::Memory(_) if export.name() == "memory" => has_memory = true,
                wasmtime::ExternType::Func(_) if export.name() == "_initialize" => has_initialize = true,
                _ => {}
            }
        }
        if !has_memory {
            return Err("exports no memory named `memory`".to_string());
        }
        // A compiler keeps its shadow stack's pointer in a global it doesn't export, which is back
        // where it began between calls. Any other state must be in memory or an exported global,
        // or a checkpoint couldn't copy it.
        if mutable_globals_defined > exported_mutable.len() + 1 {
            return Err("keeps state in globals it doesn't export, which a checkpoint couldn't copy".to_string());
        }
        for signature in &signatures {
            let name = format!("{EXPORT_PREFIX}{}", signature.name);
            match module.get_export(&name) {
                Some(wasmtime::ExternType::Func(func)) if func.params().len() == 4 && func.params().all(|p| p.is_i32()) && func.results().len() == 1 && func.results().all(|r| r.is_i32()) => {}
                Some(_) => return Err(format!("exports `{name}` with another type than crabsim calls hooks with")),
                None => return Err(format!("declares the function `{}` but exports no `{name}`", signature.name)),
            }
        }
        let pre = self.linker.instantiate_pre(&module).map_err(|error| format!("can't be instantiated: {error:#}"))?;
        Ok(Compiled { engine: self.engine.clone(), pre, signatures, mutable_globals: exported_mutable, has_initialize })
    }
}

/// A module's instance for one run, its memory kept between calls.
pub(crate) struct Instance {
    compiled: Arc<Compiled>,
    store: Store<StoreData>,
    memory: Memory,
    /// The mutable globals it exports, in the order of `Compiled::mutable_globals`.
    globals: Vec<wasmtime::Global>,
    /// Their values after the last call, which a copy is given, since reading one needs the
    /// store mutably.
    global_values: Vec<wasmtime::Val>,
    functions: Vec<TypedFunc<(i32, i32, i32, i32), i32>>,
    fuel: u64,
}

impl Instance {
    pub(crate) fn new(compiled: Arc<Compiled>, options: &Options) -> Result<Instance, String> {
        let data = StoreData { host: std::ptr::null_mut(), memory_limit: options.memory_limit, refused_memory: false, failed: None };
        let mut store = Store::new(&compiled.engine, data);
        store.limiter(|data| data);
        store.set_fuel(options.fuel_per_call).map_err(|error| error.to_string())?;
        let instance = compiled.pre.instantiate(&mut store).map_err(|error| format!("can't be instantiated: {error:#}"))?;
        let memory = instance.get_memory(&mut store, "memory").ok_or("exports no memory")?;
        let functions = compiled
            .signatures
            .iter()
            .map(|signature| instance.get_typed_func(&mut store, &format!("{EXPORT_PREFIX}{}", signature.name)).map_err(|error| error.to_string()))
            .collect::<Result<_, _>>()?;
        let globals: Vec<wasmtime::Global> = compiled
            .mutable_globals
            .iter()
            .map(|name| instance.get_global(&mut store, name).ok_or_else(|| format!("exports no global `{name}`")))
            .collect::<Result<_, _>>()?;
        if compiled.has_initialize {
            let initialize = instance.get_typed_func::<(), ()>(&mut store, "_initialize").map_err(|error| error.to_string())?;
            initialize.call(&mut store, ()).map_err(|error| format!("its `_initialize` failed: {error:#}"))?;
        }
        let global_values = globals.iter().map(|global| global.get(&mut store)).collect();
        Ok(Instance { compiled, store, memory, globals, global_values, functions, fuel: options.fuel_per_call })
    }

    /// A copy that continues exactly as this one would: a fresh instance given this one's memory
    /// and exported globals. Between calls that is all the state a module has.
    pub(crate) fn duplicate(&self) -> Instance {
        let options = Options { fuel_per_call: self.fuel, memory_limit: self.store.data().memory_limit, ..Options::default() };
        let mut copy = Instance::new(self.compiled.clone(), &options).expect("a module that was instantiated once is instantiated again");
        let pages = self.memory.size(&self.store);
        let fresh_pages = copy.memory.size(&copy.store);
        if pages > fresh_pages {
            copy.memory.grow(&mut copy.store, pages - fresh_pages).expect("the memory a module had fits its cap");
        }
        copy.memory.data_mut(&mut copy.store).copy_from_slice(self.memory.data(&self.store));
        for (value, copied) in self.global_values.iter().zip(&copy.globals) {
            copied.set(&mut copy.store, *value).expect("the same global takes the same value");
        }
        copy.global_values = self.global_values.clone();
        copy
    }

    /// Calls a hook function with the call passed as four numbers, and returns its answer.
    pub(crate) fn call(&mut self, function: usize, host: &mut Host<'_, '_>, args: (i32, i32, i32, i32)) -> Result<i32, String> {
        let data = self.store.data_mut();
        data.host = host as *mut Host<'_, '_> as *mut ();
        data.failed = None;
        data.refused_memory = false;
        self.store.set_fuel(self.fuel).map_err(|error| error.to_string())?;
        let outcome = self.functions[function].call(&mut self.store, args);
        for (global, value) in self.globals.iter().zip(&mut self.global_values) {
            *value = global.get(&mut self.store);
        }
        let data = self.store.data_mut();
        data.host = std::ptr::null_mut();
        outcome.map_err(|error| {
            if let Some(message) = data.failed.take() {
                format!("panicked: {message}")
            } else if error.downcast_ref::<Trap>() == Some(&Trap::OutOfFuel) {
                format!("ran out of fuel: a call may run {} instructions", self.fuel)
            } else if data.refused_memory {
                format!("ran out of memory: a module may take {} bytes", data.memory_limit)
            } else if let Some(trap) = error.downcast_ref::<Trap>() {
                format!("trapped: {trap}")
            } else {
                format!("{error:#}")
            }
        })
    }
}

/// crabsim's functions, which a module imports from `crabsim`: what `Host` offers, over numbers
/// and the module's memory.
fn define_imports(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    // Each import's body runs inside a call, so `host` is safe there.
    macro_rules! import {
        ($name:literal, |$caller:ident $(, $arg:ident : $ty:ty)*| -> $ret:ty $body:block) => {
            #[allow(unused_mut)]
            linker.func_wrap(IMPORTS, $name, |mut $caller: Caller<'_, StoreData> $(, $arg: $ty)*| -> wasmtime::Result<$ret> {
                $body
            })?;
        };
    }
    import!("now", |caller| -> i64 { Ok(unsafe { host(&caller) }.now()) });
    import!("object_count", |caller| -> i32 { Ok(unsafe { host(&caller) }.object_count() as i32) });
    import!("object_kind", |caller, o: i32| -> i32 { Ok(unsafe { host(&caller) }.object_kind(o as u32)) });
    import!("object_name", |caller, o: i32, ptr: i32, cap: i32| -> i32 {
        let host = unsafe { host(&caller) };
        match host.object_name(o as u32) {
            Some(name) => write_text(&mut caller, ptr, cap, name),
            None => Ok(-1),
        }
    });
    import!("find_object", |caller, ptr: i32, len: i32| -> i32 {
        let name = read_text(&mut caller, ptr, len)?;
        Ok(unsafe { host(&caller) }.find_object(&name).map_or(-1, |id| id as i32))
    });
    import!("successor_count", |caller, o: i32| -> i32 { Ok(unsafe { host(&caller) }.successors(o as u32).len() as i32) });
    import!("successor", |caller, o: i32, i: i32| -> i32 { Ok(unsafe { host(&caller) }.successors(o as u32).get(i as u32 as usize).map_or(-1, |&id| id as i32)) });
    import!("predecessor_count", |caller, o: i32| -> i32 { Ok(unsafe { host(&caller) }.predecessors(o as u32).len() as i32) });
    import!("predecessor", |caller, o: i32, i: i32| -> i32 { Ok(unsafe { host(&caller) }.predecessors(o as u32).get(i as u32 as usize).map_or(-1, |&id| id as i32)) });
    import!("process_time", |caller, o: i32| -> i64 { Ok(unsafe { host(&caller) }.process_time(o as u32).unwrap_or(-1)) });
    import!("object_state", |caller, o: i32| -> i32 { Ok(unsafe { host(&caller) }.object_state(o as u32)) });
    import!("mus_held", |caller, o: i32| -> i32 { Ok(unsafe { host(&caller) }.mus_held(o as u32) as i32) });
    import!("content", |caller, o: i32, i: i32| -> i64 { Ok(unsafe { host(&caller) }.content(o as u32, i as u32).map_or(-1, i64::from)) });
    import!("statistic", |caller, o: i32, which: i32| -> f64 { Ok(unsafe { host(&caller) }.statistic(o as u32, which)) });
    import!("mu_kind", |caller, mu: i32| -> i32 { Ok(unsafe { host(&caller) }.mu_kind(mu as u32).map_or(-1, |kind| kind as i32)) });
    import!("mu_class_name", |caller, kind: i32, ptr: i32, cap: i32| -> i32 {
        let host = unsafe { host(&caller) };
        match host.mu_class_name(kind as u32) {
            Some(name) => write_text(&mut caller, ptr, cap, name),
            None => Ok(-1),
        }
    });
    import!("mu_location", |caller, mu: i32| -> i32 { Ok(unsafe { host(&caller) }.mu_location(mu as u32).map_or(-1, |id| id as i32)) });
    import!("mu_left_source", |caller, mu: i32| -> i64 { Ok(unsafe { host(&caller) }.mu_left_source(mu as u32).unwrap_or(-1)) });
    import!("param_int", |caller, i: i32| -> i64 { Ok(unsafe { host(&caller) }.param_int(i as u32)) });
    import!("param_float", |caller, i: i32| -> f64 { Ok(unsafe { host(&caller) }.param_float(i as u32)) });
    import!("param_text", |caller, i: i32, ptr: i32, cap: i32| -> i32 {
        let host = unsafe { host(&caller) };
        let text = host.param_text(i as u32);
        write_text(&mut caller, ptr, cap, text)
    });
    import!("random", |caller, o: i32| -> f64 { Ok(unsafe { host(&caller) }.random(o as u32)) });
    import!("move_mu", |caller, mu: i32, to: i32| -> i32 { Ok(unsafe { host(&caller) }.move_mu(mu as u32, to as u32) as i32) });
    import!("place_mu", |caller, mu: i32, to: i32, at: f64, point: i32| -> i32 { Ok(unsafe { host(&caller) }.place_mu(mu as u32, to as u32, at, point) as i32) });
    import!("set_process_time", |caller, o: i32, millis: i64| -> i32 { Ok(unsafe { host(&caller) }.set_process_time(o as u32, millis) as i32) });
    import!("set_work_time", |caller, o: i32, mu: i32, millis: i64| -> i32 { Ok(unsafe { host(&caller) }.set_work_time(o as u32, mu as u32, millis) as i32) });
    import!("pause", |caller, o: i32, on: i32| -> i32 { Ok(unsafe { host(&caller) }.pause(o as u32, on != 0) as i32) });
    import!("stop", |caller, o: i32, on: i32| -> i32 { Ok(unsafe { host(&caller) }.stop(o as u32, on != 0) as i32) });
    import!("call_at", |caller, after: i64, data: i32| -> i32 { Ok(unsafe { host(&caller) }.call_at(after, data as u32) as i32) });
    import!("refusal", |caller, ptr: i32, cap: i32| -> i32 {
        let host = unsafe { host(&caller) };
        write_text(&mut caller, ptr, cap, host.refusal())
    });
    import!("log", |caller, ptr: i32, len: i32| -> () {
        let message = read_text(&mut caller, ptr, len)?;
        unsafe { host(&caller) }.log(&message);
        Ok(())
    });
    import!("fail", |caller, ptr: i32, len: i32| -> () {
        let message = read_text(&mut caller, ptr, len)?;
        caller.data_mut().failed = Some(message);
        Ok(())
    });
    Ok(())
}
