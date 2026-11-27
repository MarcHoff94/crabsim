//! What code can do during a call, in plain Rust: the interface's reads and actions over the
//! engine's `HookRun`. The WebAssembly imports and the native guest both come here, so a module
//! behaves the same run either way.

use crate::{Binding, LogLine, Shared, Value};
use crabsim::hooks::{CallKind, HookCall};
use crabsim::statistics::MachineState;
use crabsim::{HookRun, ObjectKind, ObjectState, SimTime, TriggerPoint};

/// A call in progress: the run, the bound function and the call, and what the code has been
/// refused last.
pub struct Host<'h, 'r> {
    run: &'h mut HookRun<'r>,
    shared: &'h Shared,
    binding: &'h Binding,
    call: &'h HookCall,
    random: &'h mut Vec<Option<u64>>,
    /// Why the last action was refused, for the code to read.
    refusal: String,
    /// The last object state read, until the next action, since reading a list one item at a time
    /// would otherwise build the state again for each.
    cached: Option<ObjectState>,
}

/// An object's kind as the interface numbers it.
pub fn kind_code(kind: ObjectKind) -> i32 {
    match kind {
        ObjectKind::Source => 0,
        ObjectKind::Station => 1,
        ObjectKind::Buffer => 2,
        ObjectKind::Drain => 3,
        ObjectKind::Conveyor => 4,
        ObjectKind::CrossConveyor => 5,
    }
}

/// What an object is doing, as the interface numbers it.
pub fn state_code(state: MachineState) -> i32 {
    match state {
        MachineState::Idle => 0,
        MachineState::Busy | MachineState::BusyNotFull | MachineState::BusyPartlyBlocked => 1,
        MachineState::Blocked => 2,
        MachineState::Receiving => 3,
        MachineState::Failed => 4,
        MachineState::Paused => 5,
    }
}

/// The figures code can read from an object's statistics, by number.
pub mod statistic {
    pub const BUSY_SHARE: i32 = 0;
    pub const BLOCKED_SHARE: i32 = 1;
    pub const IDLE_SHARE: i32 = 2;
    pub const FAILED_SHARE: i32 = 3;
    pub const PAUSED_SHARE: i32 = 4;
    pub const MUS_IN: i32 = 5;
    pub const MUS_OUT: i32 = 6;
    pub const MEAN_CONTENTS: i32 = 7;
    pub const MUS_PER_HOUR: i32 = 8;
}

impl<'h, 'r> Host<'h, 'r> {
    pub(crate) fn new(run: &'h mut HookRun<'r>, shared: &'h Shared, binding: &'h Binding, call: &'h HookCall, random: &'h mut Vec<Option<u64>>) -> Host<'h, 'r> {
        Host { run, shared, binding, call, random, refusal: String::new(), cached: None }
    }

    // Reading.

    /// The simulation time now, in milliseconds.
    pub fn now(&self) -> i64 {
        self.run.time().as_millis() as i64
    }

    pub fn object_count(&self) -> u32 {
        self.run.object_count()
    }

    /// The object's kind (`kind_code`), or -1 if there is no such object.
    pub fn object_kind(&self, object: u32) -> i32 {
        self.shared.kinds.get(object as usize).map_or(-1, |&kind| kind_code(kind))
    }

    pub fn object_name(&self, object: u32) -> Option<&str> {
        self.shared.names.get(object as usize).map(String::as_str)
    }

    pub fn find_object(&self, name: &str) -> Option<u32> {
        self.shared.by_name.get(name).copied()
    }

    /// Its successors in exit order.
    pub fn successors(&self, object: u32) -> &[u32] {
        self.shared.successors.get(object as usize).map_or(&[], Vec::as_slice)
    }

    pub fn predecessors(&self, object: u32) -> &[u32] {
        self.shared.predecessors.get(object as usize).map_or(&[], Vec::as_slice)
    }

    /// A Station's process time now, in milliseconds.
    pub fn process_time(&self, object: u32) -> Option<i64> {
        self.run.object(object)?.process_time().map(|time| time.as_millis() as i64)
    }

    fn state(&mut self, object: u32) -> Option<&ObjectState> {
        if self.cached.as_ref().is_none_or(|state| state.id != object) {
            self.cached = Some(self.run.state(object)?);
        }
        self.cached.as_ref()
    }

    /// What it is doing (`state_code`), or -1 if there is no such object.
    pub fn object_state(&mut self, object: u32) -> i32 {
        self.state(object).map_or(-1, |state| state_code(MachineState::of(state.interrupted, &state.process)))
    }

    /// How many MUs it holds.
    pub fn mus_held(&mut self, object: u32) -> u32 {
        self.state(object).map_or(0, |state| state.contents.len() as u32)
    }

    /// The MU it holds at this place, for a Buffer in the order they came in, for a conveyor front
    /// first.
    pub fn content(&mut self, object: u32, index: u32) -> Option<u32> {
        self.state(object)?.contents.get(index as usize).copied()
    }

    /// A figure of its statistics (`statistic`), NaN if it has none.
    pub fn statistic(&mut self, object: u32, which: i32) -> f64 {
        let now = self.run.time();
        let Some(state) = self.state(object) else {
            return f64::NAN;
        };
        let figures = &state.statistics;
        let share = |time: SimTime| {
            let measured = figures.measured_time();
            if measured == SimTime::ZERO { f64::NAN } else { time.as_millis() as f64 / measured.as_millis() as f64 }
        };
        match which {
            statistic::BUSY_SHARE => share(figures.time_busy),
            statistic::BLOCKED_SHARE => share(figures.time_blocked),
            statistic::IDLE_SHARE => share(figures.time_idle),
            statistic::FAILED_SHARE => share(figures.time_failed),
            statistic::PAUSED_SHARE => share(figures.time_paused),
            statistic::MUS_IN => figures.mus_in as f64,
            statistic::MUS_OUT => figures.mus_out as f64,
            statistic::MEAN_CONTENTS => figures.average_contents().unwrap_or(f64::NAN),
            statistic::MUS_PER_HOUR => figures.mus_per_hour(now).unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }

    /// The MU this call is about, if it is this one: a copy, so it is there even where the MU is
    /// gone, such as in a Drain.
    fn call_mu(&self, mu: u32) -> Option<&crabsim::MoveableUnit> {
        match &self.call.kind {
            CallKind::Entry { mu: called, .. } | CallKind::Exit { mu: called, .. } | CallKind::Distribution { mu: called, .. } if called.id == mu => Some(called),
            _ => None,
        }
    }

    /// The MU's kind, the place of its MU class in the model's list of MU kinds.
    pub fn mu_kind(&mut self, mu: u32) -> Option<u32> {
        if let Some(called) = self.call_mu(mu) {
            return Some(called.kind() as u32);
        }
        let holder = self.run.location(mu)?;
        let state = self.state(holder)?;
        let at = state.contents.iter().position(|&held| held == mu)?;
        state.content_kinds.get(at).map(|&kind| kind as u32)
    }

    /// The name of the MU class an MU kind stands for.
    pub fn mu_class_name(&self, kind: u32) -> Option<&str> {
        self.shared.options.mu_classes.get(kind as usize).map(String::as_str)
    }

    /// The object holding the MU, if it is in the run.
    pub fn mu_location(&self, mu: u32) -> Option<u32> {
        self.run.location(mu)
    }

    /// When the MU left its Source, in milliseconds, if this call is about it.
    pub fn mu_left_source(&self, mu: u32) -> Option<i64> {
        self.call_mu(mu).map(|called| called.left_source().as_millis() as i64)
    }

    /// This hook's parameter at this place, as a whole number: an int, a bool (1 or 0), a time in
    /// milliseconds or an object's id. 0 if it isn't one.
    pub fn param_int(&self, index: u32) -> i64 {
        match self.binding.values.get(index as usize) {
            Some(Value::Int(value)) => *value,
            Some(Value::Float(value)) => *value as i64,
            _ => 0,
        }
    }

    pub fn param_float(&self, index: u32) -> f64 {
        match self.binding.values.get(index as usize) {
            Some(Value::Float(value)) => *value,
            Some(Value::Int(value)) => *value as f64,
            _ => f64::NAN,
        }
    }

    pub fn param_text(&self, index: u32) -> &str {
        match self.binding.values.get(index as usize) {
            Some(Value::Text(text)) => text,
            _ => "",
        }
    }

    /// The next random number in `[0, 1)` from the object's own stream for code, which comes from
    /// the run's seed and the object's stream (ADR 0012), apart from the numbers the object itself
    /// draws, so code asking for them changes none of the model's own.
    pub fn random(&mut self, object: u32) -> f64 {
        let Some(stream) = self.run.stream(object) else {
            return f64::NAN;
        };
        let seed = self.run.seed();
        let state = self.random[object as usize].get_or_insert_with(|| splitmix64(seed ^ splitmix64(stream ^ 0xC0DE_C0DE_C0DE_C0DE)));
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        (splitmix64(*state) >> 11) as f64 / (1u64 << 53) as f64
    }

    // Acting.

    fn acted(&mut self, outcome: Result<(), crabsim::Refusal>) -> bool {
        self.cached = None;
        match outcome {
            Ok(()) => true,
            Err(crabsim::Refusal(reason)) => {
                self.refusal = reason;
                false
            }
        }
    }

    /// Why the last action was refused.
    pub fn refusal(&self) -> &str {
        &self.refusal
    }

    pub fn move_mu(&mut self, mu: u32, to: u32) -> bool {
        let outcome = self.run.move_mu(mu, to);
        self.acted(outcome)
    }

    /// `point`: 1 front, 2 booking point, 3 rear.
    pub fn place_mu(&mut self, mu: u32, to: u32, at: f64, point: i32) -> bool {
        let point = match point {
            1 => TriggerPoint::Front,
            2 => TriggerPoint::BookingPoint,
            3 => TriggerPoint::Rear,
            _ => return self.acted(Err(crabsim::Refusal(format!("{point} isn't a trigger point: 1 front, 2 booking point, 3 rear")))),
        };
        let outcome = self.run.place_mu(mu, to, at, point);
        self.acted(outcome)
    }

    pub fn set_process_time(&mut self, object: u32, millis: i64) -> bool {
        let outcome = match u64::try_from(millis) {
            Ok(millis) => self.run.set_process_time(object, SimTime::from_millis(millis)),
            Err(_) => Err(crabsim::Refusal(format!("a process time can't be negative: {millis} ms"))),
        };
        self.acted(outcome)
    }

    pub fn set_work_time(&mut self, object: u32, mu: u32, millis: i64) -> bool {
        let outcome = match u64::try_from(millis) {
            Ok(millis) => self.run.set_work_time(object, mu, SimTime::from_millis(millis)),
            Err(_) => Err(crabsim::Refusal(format!("a work time can't be negative: {millis} ms"))),
        };
        self.acted(outcome)
    }

    pub fn pause(&mut self, object: u32, on: bool) -> bool {
        let outcome = self.run.pause(object, on);
        self.acted(outcome)
    }

    pub fn stop(&mut self, object: u32, on: bool) -> bool {
        let outcome = self.run.stop(object, on);
        self.acted(outcome)
    }

    /// Calls this hook's function again, with its parameters, `after` milliseconds from now, as a
    /// timer with `data`.
    pub fn call_at(&mut self, after: i64, data: u32) -> bool {
        let outcome = match u64::try_from(after) {
            Ok(after) => self.run.call_at(SimTime::from_millis(after), self.call.function, data),
            Err(_) => Err(crabsim::Refusal(format!("a timer can't go off in the past: {after} ms from now"))),
        };
        self.acted(outcome)
    }

    /// Logs a line, with the time, the hook's object and its function.
    pub fn log(&self, message: &str) {
        if let Some(sink) = &self.shared.options.log {
            let object = match &self.call.kind {
                CallKind::Entry { object, .. } | CallKind::Exit { object, .. } | CallKind::Distribution { object, .. } => self.object_name(*object).map(str::to_string),
                _ => None,
            };
            sink(&LogLine { time: self.run.time(), object, function: self.binding.name.clone(), message: message.to_string() });
        }
    }
}

fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
