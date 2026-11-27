//! Writing crabsim user code in Rust (ADR 0029, spec issue 156).
//!
//! A code module is a crate of type `cdylib` holding hook functions written with [`hook!`],
//! compiled for `wasm32-unknown-unknown` into a models folder's `code/` folder. A model's hooks
//! name its functions as `<module>.<function>` with parameter values, which crabsim checks against
//! what each function declares before the run:
//!
//! ```ignore
//! use crabsim_guest::*;
//!
//! hook! {
//!     /// Sends every n-th MU to `left` and the others to `right`.
//!     fn every_nth(_call: Call, n: Int, left: Object, right: Object) -> Answer {
//!         let count = with_state(|count: &mut i64| { *count += 1; *count });
//!         Answer::Successor(if count % n == 0 { left } else { right })
//!     }
//! }
//! ```
//!
//! Code reads the run through [`Object`], [`Mu`] and [`now`], and changes it only through actions,
//! such as [`Mu::move_to`], which crabsim carries out or refuses with a [`Refusal`]. It keeps its
//! own state in [`with_state`], which crabsim clears at Reset and copies into every checkpoint.
//!
//! The same crate compiles natively too, for debugging: a test registers its functions in a
//! `NativeModule` (see [`native`]) and runs the model with them in its own process.

mod sys;

use std::any::Any;

/// The interface version this crate speaks, which every function it declares carries.
pub const INTERFACE_VERSION: u32 = 1;

/// A whole-number parameter.
pub type Int = i64;
/// A number parameter.
pub type Float = f64;
/// A true or false parameter.
pub type Bool = bool;
/// A text parameter.
pub type Text = String;

/// A length of simulation time, or a point in it, in whole milliseconds, as crabsim keeps time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time(pub i64);

impl Time {
    pub const fn from_millis(millis: i64) -> Time {
        Time(millis)
    }
    pub const fn from_secs(secs: i64) -> Time {
        Time(secs * 1000)
    }
    pub fn from_secs_f64(secs: f64) -> Time {
        Time((secs * 1000.0).round() as i64)
    }
    pub const fn millis(self) -> i64 {
        self.0
    }
    pub fn secs(self) -> f64 {
        self.0 as f64 / 1000.0
    }
}

/// An object of the model, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Object(pub u32);

/// An MU in the run, by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mu(pub u32);

/// Why crabsim refused an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

fn done(outcome: i32) -> Result<(), Refusal> {
    if outcome != 0 { Ok(()) } else { Err(Refusal(sys::refusal())) }
}

fn some_id(id: i32) -> Option<u32> {
    (id >= 0).then_some(id as u32)
}

/// What a distribution answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// The object's own distribution strategy chooses, as without code.
    Default,
    /// The object keeps the MU until code moves it.
    Hold,
    /// This successor, which the MU then waits for alone.
    Successor(Object),
}

/// What a hook function returns: an answer, nothing (the default), or a result whose error ends
/// the run with its message, so actions can use `?`.
pub trait IntoAnswer {
    fn into_answer(self) -> i32;
}

impl IntoAnswer for Answer {
    fn into_answer(self) -> i32 {
        match self {
            Answer::Default => -1,
            Answer::Hold => -2,
            Answer::Successor(Object(id)) => id as i32,
        }
    }
}

impl IntoAnswer for () {
    fn into_answer(self) -> i32 {
        -1
    }
}

impl<T: IntoAnswer, E: std::fmt::Display> IntoAnswer for Result<T, E> {
    fn into_answer(self) -> i32 {
        match self {
            Ok(answer) => answer.into_answer(),
            Err(error) => panic!("{error}"),
        }
    }
}

/// Where along an MU a hook on entry or exit runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    Front,
    BookingPoint,
    Rear,
}

impl Point {
    fn of(code: i32) -> Point {
        match code {
            1 => Point::Front,
            3 => Point::Rear,
            _ => Point::BookingPoint,
        }
    }

    fn code(self) -> i32 {
        match self {
            Point::Front => 1,
            Point::BookingPoint => 2,
            Point::Rear => 3,
        }
    }
}

/// Why a hook function is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// The call's MU came into the call's object.
    Entry { at: Point },
    /// The call's MU left the call's object for `to`.
    Exit { at: Point, to: Object },
    /// The call's MU wants to leave the call's object: which successor gets it?
    Distribution,
    /// A timer the code started, with the number it gave.
    Timer { data: u32 },
    BeforeInit,
    AfterInit,
    BeforeReset,
    AfterReset,
}

/// A call of a hook function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub kind: CallKind,
    object: Option<Object>,
    mu: Option<Mu>,
}

impl Call {
    #[doc(hidden)]
    pub fn decode(kind: i32, object: i32, mu: i32, extra: i32) -> Call {
        let point = Point::of(kind >> 8);
        let kind = match kind & 0xff {
            0 => CallKind::Entry { at: point },
            1 => CallKind::Exit { at: point, to: Object(extra as u32) },
            2 => CallKind::Distribution,
            3 => CallKind::Timer { data: extra as u32 },
            4 => CallKind::BeforeInit,
            5 => CallKind::AfterInit,
            6 => CallKind::BeforeReset,
            _ => CallKind::AfterReset,
        };
        Call { kind, object: some_id(object).map(Object), mu: some_id(mu).map(Mu) }
    }

    /// The object whose hook this is, if it is an object's.
    pub fn object(&self) -> Option<Object> {
        self.object
    }

    /// The object whose hook this is.
    ///
    /// # Panics
    /// For the model's hooks and timers, which are no object's.
    pub fn this(&self) -> Object {
        self.object.expect("this hook is no object's")
    }

    /// The MU the call is about, if any.
    pub fn mu(&self) -> Option<Mu> {
        self.mu
    }
}

/// An object's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Source,
    Station,
    Buffer,
    Drain,
    Conveyor,
    CrossConveyor,
}

/// What an object is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    /// Busy, or for a Buffer, holding.
    Busy,
    Blocked,
    Receiving,
    /// Failed, or stopped by hand.
    Failed,
    Paused,
}

impl Object {
    pub fn id(self) -> u32 {
        self.0
    }

    fn n(self) -> i32 {
        self.0 as i32
    }

    pub fn name(self) -> String {
        sys::object_name(self.n()).unwrap_or_default()
    }

    pub fn kind(self) -> Option<Kind> {
        Some(match sys::object_kind(self.n()) {
            0 => Kind::Source,
            1 => Kind::Station,
            2 => Kind::Buffer,
            3 => Kind::Drain,
            4 => Kind::Conveyor,
            5 => Kind::CrossConveyor,
            _ => return None,
        })
    }

    /// Its successors, in the order of its exits.
    pub fn successors(self) -> Vec<Object> {
        (0..sys::successor_count(self.n())).filter_map(|i| self.successor(i as usize)).collect()
    }

    /// Its successor at this exit, counted from 0.
    pub fn successor(self, exit: usize) -> Option<Object> {
        some_id(sys::successor(self.n(), exit as i32)).map(Object)
    }

    pub fn predecessors(self) -> Vec<Object> {
        (0..sys::predecessor_count(self.n())).filter_map(|i| some_id(sys::predecessor(self.n(), i)).map(Object)).collect()
    }

    /// A Station's process time now.
    pub fn process_time(self) -> Option<Time> {
        let millis = sys::process_time(self.n());
        (millis >= 0).then_some(Time(millis))
    }

    pub fn state(self) -> State {
        match sys::object_state(self.n()) {
            1 => State::Busy,
            2 => State::Blocked,
            3 => State::Receiving,
            4 => State::Failed,
            5 => State::Paused,
            _ => State::Idle,
        }
    }

    /// How many MUs it holds.
    pub fn mus_held(self) -> usize {
        sys::mus_held(self.n()) as usize
    }

    /// The MUs it holds: a copy, so changing it changes nothing. For a Buffer in the order they
    /// came in, for a conveyor front first.
    pub fn contents(self) -> Vec<Mu> {
        (0..self.mus_held()).filter_map(|i| self.content(i)).collect()
    }

    /// The MU it holds at this place, counted from 0.
    pub fn content(self, index: usize) -> Option<Mu> {
        let id = sys::content(self.n(), index as i32);
        (id >= 0).then_some(Mu(id as u32))
    }

    fn statistic(self, which: i32) -> f64 {
        sys::statistic(self.n(), which)
    }

    /// The share of time it was busy since statistics began, NaN before any time passed.
    pub fn busy_share(self) -> f64 {
        self.statistic(0)
    }
    pub fn blocked_share(self) -> f64 {
        self.statistic(1)
    }
    pub fn idle_share(self) -> f64 {
        self.statistic(2)
    }
    pub fn failed_share(self) -> f64 {
        self.statistic(3)
    }
    pub fn paused_share(self) -> f64 {
        self.statistic(4)
    }
    pub fn mus_in(self) -> u64 {
        self.statistic(5) as u64
    }
    pub fn mus_out(self) -> u64 {
        self.statistic(6) as u64
    }
    pub fn mean_contents(self) -> f64 {
        self.statistic(7)
    }
    pub fn mus_per_hour(self) -> f64 {
        self.statistic(8)
    }

    /// The next random number in `[0, 1)` from this object's stream for code, which the seed
    /// gives, so the same seed gives the same numbers.
    pub fn random(self) -> f64 {
        sys::random(self.n())
    }

    /// Changes a Station's process time from its next MU on.
    pub fn set_process_time(self, time: Time) -> Result<(), Refusal> {
        done(sys::set_process_time(self.n(), time.0))
    }

    /// Gives the MU in work at this Station this much work from now.
    pub fn set_work_time(self, mu: Mu, time: Time) -> Result<(), Refusal> {
        done(sys::set_work_time(self.n(), mu.0 as i32, time.0))
    }

    /// Pauses it, or ends the pause.
    pub fn pause(self, on: bool) -> Result<(), Refusal> {
        done(sys::pause(self.n(), on as i32))
    }

    /// Stops it by hand, or ends the stop.
    pub fn stop(self, on: bool) -> Result<(), Refusal> {
        done(sys::stop(self.n(), on as i32))
    }
}

impl Mu {
    pub fn id(self) -> u32 {
        self.0
    }

    fn n(self) -> i32 {
        self.0 as i32
    }

    /// Its kind's place in the model's list of MU kinds.
    pub fn kind(self) -> Option<u32> {
        some_id(sys::mu_kind(self.n()))
    }

    /// The name of its MU class.
    pub fn class(self) -> Option<String> {
        self.kind().and_then(|kind| sys::mu_class_name(kind as i32))
    }

    /// The object holding it, if it is in the run.
    pub fn location(self) -> Option<Object> {
        some_id(sys::mu_location(self.n())).map(Object)
    }

    /// When it left its Source, if the call is about it.
    pub fn left_source(self) -> Option<Time> {
        let millis = sys::mu_left_source(self.n());
        (millis >= 0).then_some(Time(millis))
    }

    /// Moves it from wherever it is into `to`, connected or not, if `to` takes it now.
    pub fn move_to(self, to: Object) -> Result<(), Refusal> {
        done(sys::move_mu(self.n(), to.n()))
    }

    /// Puts it on a conveyor with its `point` `at` metres from the start.
    pub fn place_on(self, conveyor: Object, at: f64, point: Point) -> Result<(), Refusal> {
        done(sys::place_mu(self.n(), conveyor.n(), at, point.code()))
    }
}

/// The simulation time now.
pub fn now() -> Time {
    Time(sys::now())
}

/// The object with this name, if the model has one. Better given as an `Object` parameter, which
/// crabsim checks before the run.
pub fn find(name: &str) -> Option<Object> {
    some_id(sys::find_object(name)).map(Object)
}

/// Every object of the model, by id.
pub fn objects() -> impl Iterator<Item = Object> {
    (0..sys::object_count() as u32).map(Object)
}

/// Logs a line, which crabsim gives the time, the hook's object and its function.
pub fn log(message: &str) {
    sys::log(message)
}

/// Logs a formatted line.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log(&format!($($arg)*)) };
}

/// Calls this hook's function again, with its parameters, `after` from now, as a timer with
/// `data`.
pub fn call_after(after: Time, data: u32) -> Result<(), Refusal> {
    done(sys::call_at(after.0, data as i32))
}

/// Runs `f` with this module's state of type `T` for the run, made with `T::default()` the first
/// time. crabsim clears it at Reset and copies it into every checkpoint.
pub fn with_state<T: Any + Clone + Default + Send + Sync, R>(f: impl FnOnce(&mut T) -> R) -> R {
    #[cfg(target_arch = "wasm32")]
    {
        use std::any::TypeId;
        use std::cell::RefCell;
        thread_local! {
            static STATES: RefCell<Vec<(TypeId, Box<dyn Any>)>> = const { RefCell::new(Vec::new()) };
        }
        // Taken out while `f` runs, so `f` may use another type's state.
        let mut state: Box<dyn Any> = STATES.with_borrow_mut(|states| match states.iter().position(|(id, _)| *id == TypeId::of::<T>()) {
            Some(at) => states.swap_remove(at).1,
            None => Box::new(T::default()),
        });
        let result = f(state.downcast_mut::<T>().expect("kept under its own type"));
        STATES.with_borrow_mut(|states| states.push((TypeId::of::<T>(), state)));
        result
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crabsim_code::native::with_state(f)
    }
}

/// A parameter's type, read from the hook's values.
pub trait Param: Sized {
    fn read(index: i32) -> Self;
}

impl Param for i64 {
    fn read(index: i32) -> i64 {
        sys::param_int(index)
    }
}
impl Param for f64 {
    fn read(index: i32) -> f64 {
        sys::param_float(index)
    }
}
impl Param for bool {
    fn read(index: i32) -> bool {
        sys::param_int(index) != 0
    }
}
impl Param for Time {
    fn read(index: i32) -> Time {
        Time(sys::param_int(index))
    }
}
impl Param for Object {
    fn read(index: i32) -> Object {
        Object(sys::param_int(index) as u32)
    }
}
impl Param for String {
    fn read(index: i32) -> String {
        sys::param_text(index)
    }
}

#[doc(hidden)]
pub fn __run(kind: i32, object: i32, mu: i32, extra: i32, body: impl FnOnce(Call) -> i32) -> i32 {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static HOOKED: AtomicBool = AtomicBool::new(false);
        if !HOOKED.swap(true, Ordering::Relaxed) {
            std::panic::set_hook(Box::new(|info| {
                let message = match info.payload().downcast_ref::<&str>() {
                    Some(text) => text.to_string(),
                    None => info.payload().downcast_ref::<String>().cloned().unwrap_or_else(|| info.to_string()),
                };
                let at = info.location().map(|at| format!(" at {}:{}", at.file(), at.line())).unwrap_or_default();
                sys::fail(&format!("{message}{at}"));
            }));
        }
    }
    body(Call::decode(kind, object, mu, extra))
}

#[doc(hidden)]
pub const fn __bytes<const N: usize>(text: &str) -> [u8; N] {
    let bytes = text.as_bytes();
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[i];
        i += 1;
    }
    out
}

/// Writes hook functions. Each is `fn name(call: Call, param: Type, …) -> R`, its parameters of
/// the types `Int`, `Float`, `Bool`, `Time`, `Object` or `Text`, written by those names, and `R`
/// an [`Answer`], `()` or a `Result` of either. A model's hook names it with values for its
/// parameters by name, which crabsim checks before the run.
///
/// Each function is exported for crabsim, declared with its parameters in the module, and also a
/// module of the same name holding its `SIGNATURE`, to register it natively:
/// `NativeModule::new("routing").function(every_nth::SIGNATURE, every_nth)`.
#[macro_export]
macro_rules! hook {
    (@call $kind:ident, $object:ident, $mu:ident, $extra:ident, $call:ident $(, $param:ident : $ty:ident)* $(-> $ret:ty)? $body:block) => {{
        #[allow(unused_variables)]
        fn body($call: $crate::Call $(, $param: $ty)*) $(-> $ret)? $body
        $crate::__run($kind, $object, $mu, $extra, |call| {
            let mut _index = 0;
            $(let $param = <$ty as $crate::Param>::read({ _index += 1; _index - 1 });)*
            $crate::IntoAnswer::into_answer(body(call $(, $param)*))
        })
    }};
    ($( $(#[$meta:meta])* fn $name:ident($call:ident : Call $(, $param:ident : $ty:ident)* $(,)?) $(-> $ret:ty)? $body:block )*) => {
        $(
            #[allow(non_snake_case, dead_code)]
            pub mod $name {
                /// What crabsim reads of the function: its name and parameters.
                pub const SIGNATURE: &str = concat!(stringify!($name), "(", $(stringify!($param), ":", stringify!($ty), ",",)* ")");
            }

            #[cfg(target_arch = "wasm32")]
            const _: () = {
                const LINE: &str = concat!("1|", stringify!($name), "(", $(stringify!($param), ":", stringify!($ty), ",",)* ")\n");
                #[unsafe(link_section = "crabsim-functions")]
                #[used]
                static DECLARED: [u8; LINE.len()] = $crate::__bytes::<{ LINE.len() }>(LINE);
            };

            $(#[$meta])*
            #[cfg(target_arch = "wasm32")]
            #[unsafe(export_name = concat!("crabsim:", stringify!($name)))]
            pub extern "C" fn $name(kind: i32, object: i32, mu: i32, extra: i32) -> i32 {
                $crate::hook!(@call kind, object, mu, extra, $call $(, $param: $ty)* $(-> $ret)? $body)
            }

            // Natively a Rust function, so a panic unwinds to crabsim, which reports it.
            $(#[$meta])*
            #[cfg(not(target_arch = "wasm32"))]
            pub fn $name(kind: i32, object: i32, mu: i32, extra: i32) -> i32 {
                $crate::hook!(@call kind, object, mu, extra, $call $(, $param: $ty)* $(-> $ret)? $body)
            }
        )*
    };
}

/// Native mode: the module's functions registered for crabsim to call in the user's own process.
#[cfg(not(target_arch = "wasm32"))]
pub mod native {
    pub use crabsim_code::native::NativeModule;
    pub use crabsim_code::{Code, Library, Options};

    /// A native module of hook functions written with `hook!`:
    /// `native_module!("routing", [every_nth, shortest_queue])`.
    #[macro_export]
    macro_rules! native_module {
        ($module:expr, [$($($segment:ident)::+),* $(,)?]) => {{
            let module = $crate::native::NativeModule::new($module);
            $(let module = module.function($($segment)::+::SIGNATURE, $($segment)::+);)*
            module
        }};
    }
}
