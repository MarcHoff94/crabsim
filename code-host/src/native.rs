//! Native mode (spec issue 156): hooks written in Rust against the crabsim-guest crate, compiled
//! into the user's own program instead of to WebAssembly, so they can be debugged with breakpoints
//! from the user's own `cargo test`. They run in crabsim's process, without the sandbox or fuel,
//! and keep their state in `with_state`, which the host keeps per run, clears at Reset and copies
//! into checkpoints as a module's memory is.

use crate::host::Host;
use crate::Signature;
use std::any::{Any, TypeId};
use std::cell::Cell;
use std::collections::HashMap;

/// A hook function compiled natively, called with the same four numbers as a WebAssembly export.
pub type NativeFunction = fn(i32, i32, i32, i32) -> i32;

/// A module of native hook functions, each with the signature it would declare as WebAssembly.
pub struct NativeModule {
    pub(crate) name: String,
    pub(crate) functions: Vec<(Signature, NativeFunction)>,
}

impl NativeModule {
    pub fn new(name: impl Into<String>) -> NativeModule {
        NativeModule { name: name.into(), functions: Vec::new() }
    }

    /// Adds a function, declared as `name(param:type,…)`.
    ///
    /// # Panics
    /// If the signature can't be read, or its name is taken.
    pub fn function(mut self, signature: &str, function: NativeFunction) -> NativeModule {
        let signature = Signature::parse(signature).unwrap_or_else(|problem| panic!("{problem}"));
        assert!(self.functions.iter().all(|(other, _)| other.name != signature.name), "`{}` is in the module twice", signature.name);
        self.functions.push((signature, function));
        self
    }
}

/// State a native module keeps for a run.
trait State: Any + Send + Sync {
    fn clone_state(&self) -> Box<dyn State>;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any + Clone + Send + Sync> State for T {
    fn clone_state(&self) -> Box<dyn State> {
        Box::new(self.clone())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

type States = HashMap<TypeId, Box<dyn State>>;

pub(crate) struct Instance {
    functions: Vec<NativeFunction>,
    states: States,
}

thread_local! {
    /// The host and the states of the native call in progress on this thread, null outside one.
    static CURRENT: Cell<(*mut (), *mut States)> = const { Cell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
}

impl Instance {
    pub(crate) fn new(functions: Vec<NativeFunction>) -> Instance {
        Instance { functions, states: States::new() }
    }

    pub(crate) fn duplicate(&self) -> Instance {
        Instance { functions: self.functions.clone(), states: self.states.iter().map(|(&id, state)| (id, (**state).clone_state())).collect() }
    }

    pub(crate) fn call(&mut self, function: usize, host: &mut Host<'_, '_>, (kind, object, mu, extra): (i32, i32, i32, i32)) -> Result<i32, String> {
        let function = self.functions[function];
        let before = CURRENT.replace((host as *mut Host<'_, '_> as *mut (), &mut self.states as *mut States));
        let outcome = std::panic::catch_unwind(|| function(kind, object, mu, extra));
        CURRENT.set(before);
        outcome.map_err(|payload| {
            let message = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic".to_string());
            format!("panicked: {message}")
        })
    }
}

/// Runs `f` with the host of the native call in progress on this thread.
///
/// # Panics
/// Outside a call.
pub fn with_host<R>(f: impl FnOnce(&mut Host<'_, '_>) -> R) -> R {
    let (host, _) = CURRENT.get();
    assert!(!host.is_null(), "crabsim-guest's functions work only inside a hook");
    // SAFETY: `Instance::call` set it to the host of the call running on this thread, which lives
    // until the call returns, and the reference doesn't outlive `f`.
    f(unsafe { &mut *(host as *mut Host<'_, '_>) })
}

/// Runs `f` with the module's state of type `T` for this run, made with `T::default()` the first
/// time and gone at Reset.
///
/// # Panics
/// Outside a call.
pub fn with_state<T: Any + Clone + Default + Send + Sync, R>(f: impl FnOnce(&mut T) -> R) -> R {
    let (_, states) = CURRENT.get();
    assert!(!states.is_null(), "crabsim-guest's functions work only inside a hook");
    // SAFETY: as for `with_host`: the states of the module whose call is running on this thread.
    let states = unsafe { &mut *states };
    let state = states.entry(TypeId::of::<T>()).or_insert_with(|| Box::new(T::default()));
    f(state.as_any_mut().downcast_mut::<T>().expect("kept under its own type"))
}
