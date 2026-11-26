//! User code's hooks in a run (ADR 0029, spec issue 156): where a run calls user code, what the
//! code is told, what it may answer, and what it may do to the run.
//!
//! The engine knows user code only as `Hooks`, a trait. Whatever runs the code, such as a
//! WebAssembly host, implements it outside the engine, so a run without hooks carries nothing of
//! it. An object never calls user code itself: at a hook point it notes a `HookCall` in its event
//! context, and the run makes the call right after the event, with the whole run at hand
//! (`HookRun`). Calls that a hook's actions set off are noted the same way and run after it, in
//! order, at the same simulation time. Nothing goes through the event queue, so a model without
//! hooks keeps exactly its events and their order.

use crate::flows::MoveableUnit;

/// Which of the code's functions a hook runs: an opaque number the code's host gives out. The
/// engine only passes it back.
pub type HookFunction = u32;

/// Where along an MU a hook on entry or exit runs (the glossary's Trigger point). Between two
/// objects without geometry all three are the same instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerPoint {
    Front,
    BookingPoint,
    Rear,
}

/// Which trigger points a hook runs at: once for each one set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triggers {
    pub front: bool,
    pub booking_point: bool,
    pub rear: bool,
}

impl Default for Triggers {
    /// The booking point alone: where the MU changes hands (ADR 0023).
    fn default() -> Triggers {
        Triggers { front: false, booking_point: true, rear: false }
    }
}

impl Triggers {
    /// The trigger points set, front first.
    pub fn points(self) -> impl Iterator<Item = TriggerPoint> {
        [(self.front, TriggerPoint::Front), (self.booking_point, TriggerPoint::BookingPoint), (self.rear, TriggerPoint::Rear)]
            .into_iter()
            .filter_map(|(set, point)| set.then_some(point))
    }
}

/// A function and the trigger points it runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hook {
    pub function: HookFunction,
    pub triggers: Triggers,
}

impl Hook {
    /// At the booking point alone.
    pub fn new(function: HookFunction) -> Hook {
        Hook { function, triggers: Triggers::default() }
    }
}

/// An object's hooks. A Drain has only `on_entry`, and a Source's entry is its MU being created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ObjectHooks {
    /// As an MU comes in.
    pub on_entry: Option<Hook>,
    /// As an MU leaves, after it has moved on.
    pub on_exit: Option<Hook>,
    /// Chooses the successor when an MU wants to leave, instead of the object's own distribution
    /// strategy, which its answer `Decision::Default` hands the choice back to.
    pub distribution: Option<HookFunction>,
}

/// A moment in a run's life, around crabsim's own Init and Reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// Objects in their default state, nothing queued.
    BeforeInit,
    /// Every object's Init done, its first events queued.
    AfterInit,
    /// The run that ended still there, and the code's memory: it may only read and log.
    BeforeReset,
    /// Objects back in their default state, the code's memory fresh.
    AfterReset,
}

impl Lifecycle {
    pub const ALL: [Lifecycle; 4] = [Lifecycle::BeforeInit, Lifecycle::AfterInit, Lifecycle::BeforeReset, Lifecycle::AfterReset];

    fn index(self) -> usize {
        self as usize
    }
}

/// Why the code is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    /// The MU came into `object`.
    Entry { object: u32, mu: MoveableUnit, at: TriggerPoint },
    /// The MU left `object` for `to`.
    Exit { object: u32, mu: MoveableUnit, at: TriggerPoint, to: u32 },
    /// The MU wants to leave `object`: which successor gets it? Answered with a `Decision`.
    Distribution { object: u32, mu: MoveableUnit },
    Lifecycle(Lifecycle),
    /// A timer the code started with `HookRun::call_at`, with the number it gave.
    Timer { data: u32 },
}

/// One call of user code. The MU it is about is a copy, so code can read it even where the MU is
/// gone by the time it runs, such as in a Drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCall {
    pub function: HookFunction,
    pub kind: CallKind,
}

/// A distribution's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// This successor, by object id, which the MU then waits for alone.
    Successor(u32),
    /// The object keeps the MU until code moves it.
    Hold,
    /// The object's own distribution strategy chooses, as without code.
    Default,
}

/// User code failed: its own message, such as a panic's. The run ends with `RunError::Hook`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookError(pub String);

/// Why the run refused an action, which the code is told and may act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl Refusal {
    pub(crate) fn new(reason: impl Into<String>) -> Refusal {
        Refusal(reason.into())
    }
}

/// User code, as a run sees it. One instance lives for the run: its memory is kept between calls,
/// cleared at Reset, and copied into every checkpoint, so going back stays exact.
pub trait Hooks: Send + Sync {
    /// Runs the call. A distribution call answers with a decision, and no answer means `Default`.
    /// Every other call's answer is ignored.
    fn call(&mut self, call: &HookCall, run: &mut crate::simulation_run::HookRun<'_>) -> Result<Option<Decision>, HookError>;
    /// Clears the code's memory, between the hooks before and after Reset.
    fn clear(&mut self);
    /// A copy for a checkpoint, which continues exactly as this one would.
    fn clone_hooks(&self) -> Box<dyn Hooks>;
}

impl Clone for Box<dyn Hooks> {
    fn clone(&self) -> Box<dyn Hooks> {
        self.clone_hooks()
    }
}

/// Which objects have which hooks, and the model's lifecycle hooks: fixed for a run, and shared
/// with every event's context, which reads it at each entry and exit.
#[derive(Debug, Clone, Default)]
pub struct HookTable {
    /// By object id. Shorter than the model where the last objects have none.
    objects: Vec<ObjectHooks>,
    lifecycle: [Option<HookFunction>; 4],
}

impl HookTable {
    pub fn set_object(&mut self, id: u32, hooks: ObjectHooks) {
        let id = id as usize;
        if self.objects.len() <= id {
            self.objects.resize(id + 1, ObjectHooks::default());
        }
        self.objects[id] = hooks;
    }

    pub fn set_lifecycle(&mut self, moment: Lifecycle, function: Option<HookFunction>) {
        self.lifecycle[moment.index()] = function;
    }

    pub fn object(&self, id: u32) -> ObjectHooks {
        self.objects.get(id as usize).copied().unwrap_or_default()
    }

    pub fn lifecycle(&self, moment: Lifecycle) -> Option<HookFunction> {
        self.lifecycle[moment.index()]
    }

    /// Whether anything at all would call code.
    pub fn is_empty(&self) -> bool {
        self.lifecycle.iter().all(Option::is_none) && self.objects.iter().all(|hooks| *hooks == ObjectHooks::default())
    }

    /// The objects with a distribution hook, and its function.
    pub(crate) fn distributions(&self) -> impl Iterator<Item = (u32, HookFunction)> + '_ {
        self.objects.iter().enumerate().filter_map(|(id, hooks)| hooks.distribution.map(|function| (id as u32, function)))
    }
}

/// The call, as a run error names it.
pub fn describe(call: &HookCall) -> String {
    match &call.kind {
        CallKind::Entry { object, mu, .. } => format!("the hook on entry of object {object}, for MU {}", mu.id),
        CallKind::Exit { object, mu, .. } => format!("the hook on exit of object {object}, for MU {}", mu.id),
        CallKind::Distribution { object, mu } => format!("the distribution hook of object {object}, for MU {}", mu.id),
        CallKind::Lifecycle(moment) => format!("the hook {}", match moment {
            Lifecycle::BeforeInit => "before Init",
            Lifecycle::AfterInit => "after Init",
            Lifecycle::BeforeReset => "before Reset",
            Lifecycle::AfterReset => "after Reset",
        }),
        CallKind::Timer { data } => format!("a timer ({data})"),
    }
}

/// An object whose successor code chooses: its function, the strategy `Decision::Default` hands
/// back to, and what it was asked and answered about its MUs. Boxed in `Distributing`, so only
/// objects with a distribution hook pay for it.
#[derive(Debug, Clone)]
pub struct Coded {
    pub function: HookFunction,
    /// The object's own strategy: round robin, weighted or by destination.
    pub builtin: crate::flows::Distributing,
    /// The MU a call is due or running for, so one MU is asked about once.
    pub(crate) asked: Option<u32>,
    /// The answer for an MU, kept until it leaves. MU ids are never reused, so an answer for an MU
    /// that has gone is never read again.
    pub(crate) decided: Option<(u32, Decision)>,
}

impl Coded {
    pub fn new(function: HookFunction, builtin: crate::flows::Distributing) -> Coded {
        Coded { function, builtin, asked: None, decided: None }
    }

    /// What was asked and answered goes, as at Reset.
    pub(crate) fn forget(&mut self) {
        self.asked = None;
        self.decided = None;
    }

    /// The answer for this MU, if there is one.
    pub(crate) fn decision(&self, mu: u32) -> Option<Decision> {
        self.decided.filter(|&(decided, _)| decided == mu).map(|(_, decision)| decision)
    }

    /// Whether a call is due or running for this MU and has no answer yet.
    pub(crate) fn awaits(&self, mu: u32) -> bool {
        self.asked == Some(mu) && self.decision(mu).is_none()
    }
}
