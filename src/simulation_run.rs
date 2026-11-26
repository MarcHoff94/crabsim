use crate::error::{AdvanceFailure, RunError};
use crate::event::{EventContext, EventType, ScheduledEvent, NO_OBJECT};
use crate::flows::{EventInterface, MaterialFlow};
use crate::hooks::{CallKind, Decision, HookCall, HookError, HookFunction, HookTable, Hooks, Lifecycle, Refusal};
use crate::report::ObjectState;
use crate::distribution::DistributionRule;
use crate::materialflow_objects::{ByDestination, Distributing, ModelObject, OtherObjects, Ranked};
use crate::routing::Routing;
use std::sync::Arc;
use crate::report::{Occurrence, OccurrenceKind, Recording, RunState};
use crate::statistics::{ObjectStatistics, Periods};
use crate::time::SimTime;
use std::collections::BinaryHeap;
use std::mem::size_of;

/// How many events a run processes at one simulation time before it reports a zero-time
/// loop. Far more than a legitimate model has happening at one instant, however large.
pub const ZERO_TIME_EVENT_LIMIT: u64 = 1_000_000;

/// What a bounded advance did.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundedAdvance {
    pub occurrences: Vec<Occurrence>,
    /// The clock reached the target. If not, events are still due by then.
    pub reached: bool,
}

/// One execution of a model, from its Init until it is reset or discarded.
///
/// A run only advances when asked, so how fast it goes is up to the caller. Every advance
/// returns the occurrences it was asked to record, which is none by default, and `state` gives
/// the fixed-size results at any time, including after an error. So a run with nothing else
/// attached is a headless run. It executes on one thread at a time, and can move between threads.
///
/// A clone is a checkpoint: it continues exactly as the original would have.
#[derive(Clone)]
pub struct SimulationRun {
    /// Indexed by object id.
    objects: Vec<ModelObject>,
    queue: BinaryHeap<ScheduledEvent>,
    time: SimTime,
    next_seq: u64,
    /// The simulation time `events_at_this_time` is counting.
    counted_time: SimTime,
    events_at_this_time: u64,
    seed: u64,
    /// Each object's stream, by id, which with the seed gives its random numbers.
    streams: Vec<u64>,
    /// Each object's distribution strategy, by id, from which the run ranks the successors of
    /// the weighted ones (ADR 0016).
    rules: Vec<DistributionRule>,
    /// The model's work plans and destinations (spec issue 148), which every event's context
    /// reads.
    routing: Arc<Routing>,
    initialised: bool,
    recording: Recording,
    statistics_since: SimTime,
    /// User code (ADR 0029), or `None` in a model without it. Taken out of the run while it runs.
    hooks: Option<Box<dyn Hooks>>,
    /// Which objects have which hooks, shared with every event's context.
    hook_table: Option<Arc<HookTable>>,
    /// A failure of the hooks around Reset, which can't fail, reported by the next Init.
    hook_error: Option<RunError>,
    /// The end of the measuring period in progress, or the end of time without periods, so
    /// checking for one costs the per-event path a single comparison.
    period_end: SimTime,
    /// How long measuring periods are, a setting Reset keeps.
    measuring_period: Option<SimTime>,
    periods: Option<Periods>,
    context: EventContext,
}

impl SimulationRun {
    /// A run of these objects, whose ids must match their positions. `ModelDescription` builds
    /// them that way.
    pub(crate) fn new(objects: Vec<ModelObject>, seed: u64, streams: Vec<u64>, rules: Vec<DistributionRule>, routing: Arc<Routing>) -> SimulationRun {
        debug_assert_eq!(objects.len(), streams.len(), "one stream per object");
        debug_assert_eq!(objects.len(), rules.len(), "one distribution strategy per object");
        let mut run = SimulationRun {
            streams,
            rules,
            objects,
            queue: BinaryHeap::new(),
            time: SimTime::ZERO,
            next_seq: 0,
            counted_time: SimTime::ZERO,
            events_at_this_time: 0,
            seed,
            initialised: false,
            recording: Recording::NOTHING,
            statistics_since: SimTime::ZERO,
            hooks: None,
            hook_table: None,
            hook_error: None,
            period_end: SimTime::MAX,
            measuring_period: None,
            periods: None,
            context: EventContext::new(1, Recording::NOTHING).with_routing(routing.clone()),
            routing,
        };
        run.route_by_destination();
        run.rank_successors();
        run
    }

    /// Gives the run user code and which objects have which hooks (ADR 0029). An object with a
    /// distribution hook keeps its own strategy, which the hook can hand the choice back to.
    pub(crate) fn attach_hooks(&mut self, table: HookTable, hooks: Box<dyn Hooks>) {
        for (id, function) in table.distributions() {
            if let Some(distribution) = self.objects.get_mut(id as usize).and_then(ModelObject::distribution_mut) {
                let builtin = std::mem::take(distribution);
                *distribution = Distributing::Code(Box::new(crate::hooks::Coded::new(function, builtin)));
            }
        }
        self.hook_table = Some(Arc::new(table));
        self.context = self.new_context(1);
        self.hooks = Some(hooks);
    }

    /// A context for events from now on, handing out MU ids from `first_mu_id`.
    fn new_context(&self, first_mu_id: u32) -> EventContext {
        EventContext::new(first_mu_id, self.recording).with_routing(self.routing.clone()).with_hooks(self.hook_table.clone())
    }

    /// Gives every object routing by destination its table of which exits reach each
    /// destination (ADR 0025). Done once, when the run is built: connections don't change.
    fn route_by_destination(&mut self) {
        let tables = crate::distribution::reaching(&self.objects, &self.rules, &self.routing);
        for (object, table) in self.objects.iter_mut().zip(tables) {
            if let (Some(table), Some(distribution)) = (table, object.distribution_mut()) {
                *distribution = Distributing::ByDestination(Box::new(ByDestination::new(table)));
            }
        }
    }

    /// Ranks the successors of every weighted object again from the objects' values now, with
    /// their turns among equals starting afresh. Done when the run is built, at Reset and when a
    /// value changes, never per event. Costs nothing in a model of round robin only.
    fn rank_successors(&mut self) {
        if self.rules.iter().all(|rule| rule.weighted().is_none()) {
            return;
        }
        let rankings = crate::distribution::rankings(&self.objects, &self.rules);
        for (object, ranking) in self.objects.iter_mut().zip(rankings) {
            if let (Some(ranking), Some(distribution)) = (ranking, object.distribution_mut().map(Distributing::builtin_mut)) {
                let ranked = Distributing::Weighted(Ranked::new(ranking.groups));
                match distribution {
                    Distributing::ByDestination(by) => by.then = ranked,
                    _ => *distribution = ranked,
                }
            }
        }
    }

    /// Sets which kinds of occurrence this run keeps, from now on. It is a setting rather than
    /// part of a run's progress, so Reset leaves it alone.
    pub fn set_recording(&mut self, recording: Recording) {
        self.recording = recording;
        self.context.set_recording(recording);
    }

    /// Starts every object's statistics afresh from the current time, such as at the end of a
    /// warm-up. It changes nothing else, so random numbers and event order stay the same.
    pub fn clear_statistics(&mut self) {
        for object in &mut self.objects {
            object.clear_statistics(self.time);
        }
        self.statistics_since = self.time;
        self.start_periods();
    }

    /// Measures in periods of `length` from when statistics begin, each period's figures one
    /// sample, so one run gives a confidence interval from the spread between its periods (see
    /// `RunState::periods`). `None` or zero measures no periods. It is a setting, so Reset keeps
    /// it. Set during a run, periods start afresh from the current time. A period ends after
    /// every event at its end time, and costs reading every object's statistics once.
    pub fn set_measuring_period(&mut self, length: Option<SimTime>) {
        self.measuring_period = length.filter(|&length| length > SimTime::ZERO);
        if self.initialised {
            self.start_periods();
        }
    }

    pub fn measuring_period(&self) -> Option<SimTime> {
        self.measuring_period
    }

    fn start_periods(&mut self) {
        self.periods = self.measuring_period.map(|length| Periods::new(length, self.time, self.statistics_at(self.time)));
        self.period_end = self.periods.as_ref().map_or(SimTime::MAX, Periods::end);
    }

    /// Completes every period that ends before `time`, or by `time` if `inclusive`.
    #[cold]
    fn close_periods(&mut self, time: SimTime, inclusive: bool) {
        while self.period_end < time || (inclusive && self.period_end == time) {
            let statistics = self.statistics_at(self.period_end);
            let Some(periods) = &mut self.periods else { return };
            periods.close(statistics);
            self.period_end = periods.end();
        }
    }

    fn statistics_at(&self, time: SimTime) -> Vec<ObjectStatistics> {
        self.objects.iter().map(|object| object.report(time).statistics).collect()
    }

    /// Changes how long a Station works on each MU from now on. The MU in work keeps its time,
    /// and the next MU uses the new one. Reset returns to the configured process time. It
    /// schedules nothing, so it costs nothing per event.
    pub fn set_process_time(&mut self, id: u32, process_time: SimTime) -> Result<(), RunError> {
        match self.objects.get_mut(id as usize) {
            Some(object) if object.is_station() => {
                if let Some((now, _)) = object.process_times_mut() {
                    *now = process_time;
                }
                // A weighted object may weigh it.
                self.rank_successors();
                Ok(())
            }
            Some(_) => Err(RunError::NotAStation(id)),
            None => Err(RunError::UnknownObject(id)),
        }
    }

    /// Makes what changed during the run part of the default state, so Reset keeps it. So far
    /// that is each Station's process time.
    pub fn keep_changes_as_default(&mut self) {
        for object in &mut self.objects {
            if let Some((now, configured)) = object.process_times_mut() {
                *configured = *now;
            }
        }
    }

    /// Sets time to 0, calls every object's init in id order and queues the first events.
    /// Nothing is processed yet, including events due at t=0.
    pub fn init(&mut self) -> Result<Vec<Occurrence>, RunError> {
        if self.initialised {
            return Err(RunError::AlreadyInitialised);
        }
        if let Some(error) = self.hook_error.take() {
            return Err(error);
        }
        let highest_starting_mu = self.state().objects.iter().flat_map(|o| o.contents.clone()).max().unwrap_or(0);
        self.context = self.new_context(highest_starting_mu + 1);
        self.context.time = SimTime::ZERO;
        self.context.record(OccurrenceKind::RunInitialised);
        self.lifecycle(Lifecycle::BeforeInit)?;

        for index in 0..self.objects.len() {
            let id = index as u32;
            let object = &mut self.objects[index];
            object.set_random_seed(self.seed ^ self.streams[index].wrapping_mul(0x9E3779B97F4A7C15));
            let events = object
                .init(&mut self.context)
                .map_err(|_| RunError::ObjectFailed { object_id: id, time: SimTime::ZERO })?;
            for event in events {
                self.push_event(event.exec_time, event.eventtype, event.object_id);
            }
        }
        self.initialised = true;
        // The hooks on entry of MUs that came in during Init, then the one after it.
        if self.context.has_calls() {
            self.run_calls()?;
        }
        self.lifecycle(Lifecycle::AfterInit)?;
        self.start_periods();
        Ok(self.context.take_occurrences())
    }

    /// Calls the model's hook at this moment of the run's life, if it has one, and whatever its
    /// actions set off.
    fn lifecycle(&mut self, moment: Lifecycle) -> Result<(), RunError> {
        if let Some(function) = self.hook_table.as_ref().and_then(|table| table.lifecycle(moment)) {
            self.context.note(HookCall { function, kind: CallKind::Lifecycle(moment) });
        }
        if self.context.has_calls() { self.run_calls() } else { Ok(()) }
    }

    /// A timer user code started has come: its function is called.
    #[cold]
    #[inline(never)]
    fn timer(&mut self, function: HookFunction, data: u32) -> Result<(), RunError> {
        self.context.note(HookCall { function, kind: CallKind::Timer { data } });
        self.run_calls()
    }

    /// Makes the calls of user code noted so far, first noted first, and those they set off, each
    /// counting as an event at this time. A distribution's answer goes to its object at once. Kept
    /// apart from the per-event path, which a model without code never leaves.
    #[cold]
    #[inline(never)]
    fn run_calls(&mut self) -> Result<(), RunError> {
        let outcome = self.make_calls();
        if outcome.is_err() {
            self.context.drop_calls();
        }
        outcome
    }

    fn make_calls(&mut self) -> Result<(), RunError> {
        while let Some(call) = self.context.next_call() {
            self.count_at_this_time()?;
            let Some(mut hooks) = self.hooks.take() else {
                continue;
            };
            let access = match call.kind {
                CallKind::Lifecycle(Lifecycle::BeforeReset) => Access::ReadOnly,
                CallKind::Lifecycle(Lifecycle::BeforeInit | Lifecycle::AfterReset) => Access::NotStarted,
                _ => Access::Running,
            };
            let answer = hooks.call(&call, &mut HookRun { run: self, access });
            self.hooks = Some(hooks);
            let decision = answer.map_err(|HookError(message)| RunError::hook(&call, self.time, message))?;
            if let CallKind::Distribution { object, mu } = &call.kind {
                self.deliver(*object, mu.id, decision.unwrap_or(Decision::Default), &call)?;
            }
        }
        Ok(())
    }

    /// Counts one more thing happening at this time, refusing once too many have.
    fn count_at_this_time(&mut self) -> Result<(), RunError> {
        if self.time != self.counted_time {
            self.counted_time = self.time;
            self.events_at_this_time = 0;
        }
        if self.events_at_this_time >= ZERO_TIME_EVENT_LIMIT {
            return Err(RunError::ZeroTimeLoop { time: self.time });
        }
        self.events_at_this_time += 1;
        Ok(())
    }

    /// Hands a distribution hook's answer to its object, which tries to pass the MU on at once.
    fn deliver(&mut self, object: u32, mu: u32, decision: Decision, call: &HookCall) -> Result<(), RunError> {
        let Some(target) = self.objects.get_mut(object as usize) else {
            return Err(RunError::UnknownObject(object));
        };
        if let Decision::Successor(to) = decision {
            if !target.successor_ids().contains(&to) {
                return Err(RunError::hook(call, self.time, format!("it chose object {to}, which isn't a successor of object {object}")));
            }
        }
        if let Some(Distributing::Code(coded)) = target.distribution_mut() {
            coded.decided = Some((mu, decision));
        }
        self.execute_now(object, EventType::Decided)
    }

    /// Has the object handle the event at once, at the current time, outside the queue.
    fn execute_now(&mut self, object_id: u32, eventtype: EventType) -> Result<(), RunError> {
        let index = object_id as usize;
        if index >= self.objects.len() {
            return Err(RunError::UnknownObject(object_id));
        }
        let (before, from_object) = self.objects.split_at_mut(index);
        let (object, after) = from_object.split_at_mut(1);
        let mut others = OtherObjects::new(before, after);
        let outcome = object[0].execute_event(&mut self.context, &eventtype, &mut others);
        self.queue_scheduled();
        outcome.map_err(|_| RunError::ObjectFailed { object_id, time: self.time })
    }

    /// Queues the events scheduled through the context since it was last emptied.
    fn queue_scheduled(&mut self) {
        let mut scheduled = self.context.take_scheduled();
        for new_event in scheduled.drain(..) {
            self.push_event(self.time + new_event.exec_time, new_event.eventtype, new_event.object_id);
        }
        self.context.return_scheduled(scheduled);
    }

    /// Processes the next event, if there is one.
    pub fn step(&mut self) -> Result<Vec<Occurrence>, AdvanceFailure> {
        let outcome = self.require_initialised().and_then(|()| self.process_next());
        self.hand_over(outcome).map(|((), occurrences)| occurrences)
    }

    /// Processes every event up to and including `until`, and leaves the clock there.
    pub fn advance_to(&mut self, until: SimTime) -> Result<Vec<Occurrence>, AdvanceFailure> {
        self.advance_bounded(until, usize::MAX).map(|advance| advance.occurrences)
    }

    /// Does what `advance_to` does, but processes at most `max_events` events. If that leaves
    /// events due by `until`, the clock stays at the last one processed, and advancing again
    /// continues exactly as if this had not been split up.
    pub fn advance_bounded(&mut self, until: SimTime, max_events: usize) -> Result<BoundedAdvance, AdvanceFailure> {
        let outcome = self.require_initialised().and_then(|()| self.process_up_to(until, max_events));
        self.hand_over(outcome).map(|(reached, occurrences)| BoundedAdvance { occurrences, reached })
    }

    /// Processes at most `max_events` events, however far apart in time they are, and leaves the
    /// clock at the last one. Advancing again continues exactly as if this had not been split up.
    pub fn advance_events(&mut self, max_events: usize) -> Result<Vec<Occurrence>, AdvanceFailure> {
        let outcome = self.require_initialised().and_then(|()| {
            for _ in 0..max_events {
                if self.queue.is_empty() {
                    break;
                }
                self.process_next()?;
            }
            Ok(())
        });
        self.hand_over(outcome).map(|((), occurrences)| occurrences)
    }

    /// Hands over whatever has been recorded and not yet returned. Only an advance cut short by
    /// a panic leaves anything behind.
    pub(crate) fn take_recorded(&mut self) -> Vec<Occurrence> {
        self.context.take_occurrences()
    }

    /// About how much memory a clone of this run takes.
    pub(crate) fn checkpoint_bytes(&self) -> u64 {
        const PER_OBJECT_ON_THE_HEAP: usize = 64;
        // A Buffer's MUs are the one thing whose size isn't known from the model.
        let in_buffers: usize = self.objects.iter().map(|object| if let ModelObject::Buffer(buffer) = object { buffer.content_bytes() } else { 0 }).sum();
        let objects = self.objects.len() * (size_of::<ModelObject>() + PER_OBJECT_ON_THE_HEAP) + in_buffers;
        let periods = self.periods.as_ref().map_or(0, Periods::bytes);
        (size_of::<SimulationRun>() + objects + periods + self.queue.len() * size_of::<ScheduledEvent>()) as u64
    }

    /// Hands over everything an advance recorded, however it ended.
    fn hand_over<T>(&mut self, outcome: Result<T, RunError>) -> Result<(T, Vec<Occurrence>), AdvanceFailure> {
        let occurrences = self.context.take_occurrences();
        match outcome {
            Ok(value) => Ok((value, occurrences)),
            Err(error) => Err(AdvanceFailure { error, occurrences }),
        }
    }

    /// Returns every object to its default state, ready for a new run. Objects stay in the model.
    ///
    /// With user code, the hook before Reset sees the run that ended, the code's memory is cleared,
    /// and the hook after Reset sees the default state. Reset can't fail, so a failure of either
    /// is what the next Init reports.
    pub fn reset(&mut self) {
        self.hook_error = None;
        if self.hooks.is_some() {
            if let Err(error) = self.lifecycle(Lifecycle::BeforeReset) {
                self.hook_error = Some(error);
            }
        }
        for object in &mut self.objects {
            object.reset();
            if let Some(Distributing::Code(coded)) = object.distribution_mut() {
                coded.forget();
            }
        }
        self.queue.clear();
        self.time = SimTime::ZERO;
        self.next_seq = 0;
        self.counted_time = SimTime::ZERO;
        self.events_at_this_time = 0;
        self.initialised = false;
        self.statistics_since = SimTime::ZERO;
        self.periods = None;
        self.period_end = SimTime::MAX;
        self.context = self.new_context(1);
        self.rank_successors();
        if let Some(hooks) = &mut self.hooks {
            hooks.clear();
            if let Err(error) = self.lifecycle(Lifecycle::AfterReset) {
                self.hook_error.get_or_insert(error);
            }
        }
    }

    /// Queues an event from outside the model at a given simulation time.
    pub fn schedule_event(&mut self, at: SimTime, eventtype: EventType, object_id: u32) -> Result<(), RunError> {
        self.require_initialised()?;
        if at < self.time {
            return Err(RunError::EventInThePast);
        }
        self.push_event(at, eventtype, object_id);
        Ok(())
    }

    pub fn time(&self) -> SimTime {
        self.time
    }

    pub fn is_initialised(&self) -> bool {
        self.initialised
    }

    /// How many events this run has processed since Init, stale ones included. Worked out from
    /// the sequence counter, so counting costs the per-event path nothing.
    pub fn events_processed(&self) -> u64 {
        self.next_seq - self.queue.len() as u64
    }

    pub fn next_event_time(&self) -> Option<SimTime> {
        self.queue.peek().map(|scheduled| scheduled.time)
    }

    pub fn state(&self) -> RunState {
        RunState {
            time: self.time,
            initialised: self.initialised,
            statistics_since: self.statistics_since,
            objects: self.objects.iter().map(|object| object.report(self.time)).collect(),
            periods: self.periods.as_ref().map(Periods::report),
        }
    }

    pub fn objects(&self) -> &[ModelObject] {
        &self.objects
    }

    pub fn object(&self, id: u32) -> Option<&ModelObject> {
        self.objects.get(id as usize)
    }

    fn require_initialised(&self) -> Result<(), RunError> {
        if self.initialised {
            Ok(())
        } else {
            Err(RunError::NotInitialised)
        }
    }

    fn push_event(&mut self, time: SimTime, eventtype: EventType, object_id: u32) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.queue.push(ScheduledEvent { time, seq, object_id, eventtype });
    }

    /// Returns whether the clock reached `until`, which it does once no event is due by then.
    fn process_up_to(&mut self, until: SimTime, max_events: usize) -> Result<bool, RunError> {
        for _ in 0..max_events {
            if !self.has_event_by(until) {
                break;
            }
            self.process_next()?;
        }
        if self.has_event_by(until) {
            return Ok(false);
        }
        if until > crate::flows::TIME_LIMIT {
            return Err(RunError::BeyondTimeLimit { time: until });
        }
        if until > self.time {
            self.time = until;
        }
        if self.time >= self.period_end {
            self.close_periods(self.time, true);
        }
        Ok(true)
    }

    fn has_event_by(&self, time: SimTime) -> bool {
        self.next_event_time().is_some_and(|next| next <= time)
    }

    /// Processes the next event, unless that would be too many without time moving. The
    /// event then stays queued, so a run in a zero-time loop keeps refusing until Reset.
    fn process_next(&mut self) -> Result<(), RunError> {
        let Some(time) = self.next_event_time() else {
            return Ok(());
        };
        if time > self.period_end {
            self.close_periods(time, false);
        }
        if time != self.counted_time {
            if time > crate::flows::TIME_LIMIT {
                return Err(RunError::BeyondTimeLimit { time });
            }
            self.counted_time = time;
            self.events_at_this_time = 0;
        }
        if self.events_at_this_time >= ZERO_TIME_EVENT_LIMIT {
            return Err(RunError::ZeroTimeLoop { time });
        }
        self.events_at_this_time += 1;
        match self.queue.pop() {
            Some(scheduled) => self.process(scheduled),
            None => Ok(()),
        }
    }

    fn process(&mut self, event: ScheduledEvent) -> Result<(), RunError> {
        self.time = event.time;
        self.context.time = self.time;
        self.context.seq = event.seq;
        log::info!("At {}s: seq={} obj={} event={:?}", self.time, event.seq, event.object_id, event.eventtype);

        let index = event.object_id as usize;
        if index >= self.objects.len() {
            // A timer is queued for no object, so the check every event makes finds it too.
            return match event.eventtype {
                EventType::Timer { function, data } => self.timer(function, data),
                _ => Err(RunError::UnknownObject(event.object_id)),
            };
        }
        let (before, from_object) = self.objects.split_at_mut(index);
        let (object, after) = from_object.split_at_mut(1);
        let mut others = OtherObjects::new(before, after);
        let outcome = object[0].execute_event(&mut self.context, &event.eventtype, &mut others);
        // Taken out and handed back, so the list keeps its memory from event to event.
        let mut scheduled = self.context.take_scheduled();
        if outcome.is_err() {
            self.context.return_scheduled(scheduled);
            return Err(RunError::ObjectFailed { object_id: event.object_id, time: self.time });
        }
        for new_event in scheduled.drain(..) {
            self.push_event(self.time + new_event.exec_time, new_event.eventtype, new_event.object_id);
        }
        self.context.return_scheduled(scheduled);
        if self.context.has_calls() {
            return self.run_calls();
        }
        Ok(())
    }
}

/// What a call of user code may do to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Before Reset: read and log only.
    ReadOnly,
    /// Before Init and after Reset, when objects haven't started: read, set values and start
    /// timers, but not move MUs or interrupt objects.
    NotStarted,
    Running,
}

/// The run, as user code sees it during a call (ADR 0029): it reads any object, and changes the
/// run only through these actions, each carried out by the engine's own code, or refused with a
/// reason the code is given.
pub struct HookRun<'a> {
    run: &'a mut SimulationRun,
    access: Access,
}

impl HookRun<'_> {
    /// The simulation time now.
    pub fn time(&self) -> SimTime {
        self.run.time
    }

    pub fn object_count(&self) -> u32 {
        self.run.objects.len() as u32
    }

    pub fn object(&self, id: u32) -> Option<&ModelObject> {
        self.run.objects.get(id as usize)
    }

    /// The object's whole state now, as `SimulationRun::state` gives it.
    pub fn state(&self, id: u32) -> Option<ObjectState> {
        self.object(id).map(|object| object.report(self.run.time))
    }

    fn allow(&self, needs_running: bool) -> Result<(), Refusal> {
        match self.access {
            Access::ReadOnly => Err(Refusal::new("the hook before Reset may only read and log")),
            Access::NotStarted if needs_running => Err(Refusal::new("objects haven't started yet: do this from the hook after Init on")),
            _ => Ok(()),
        }
    }

    fn exists(&self, id: u32) -> Result<(), Refusal> {
        if id < self.object_count() { Ok(()) } else { Err(Refusal::new(format!("object {id} is not in the model"))) }
    }

    /// Moves the MU from the object holding it into another that takes it now, connected or not,
    /// whatever the MU is doing: work on it stops. Its old object's hook on exit and its new
    /// object's hook on entry run after this call.
    pub fn move_mu(&mut self, mu: u32, from: u32, to: u32) -> Result<(), Refusal> {
        self.allow(true)?;
        self.exists(from)?;
        self.exists(to)?;
        if from == to {
            return Err(Refusal::new(format!("MU {mu} would move from object {from} into itself")));
        }
        let run = &mut *self.run;
        let target = &run.objects[to as usize];
        if target.reads_mu_kind() {
            return Err(Refusal::new(format!("object {to} is a conveyor, and crabsim can't move an MU onto a conveyor from code yet")));
        }
        if !target.is_entrance_allowed() {
            return Err(Refusal::new(format!("object {to} doesn't take an MU now")));
        }
        let (before, from_object) = run.objects.split_at_mut(from as usize);
        let (object, after) = from_object.split_at_mut(1);
        let mut others = OtherObjects::new(before, after);
        let taken = object[0].take_out(mu, &mut run.context, &mut others)?;
        run.context.moved(&taken, from, to);
        let entered = others.get_mut(to).enter(taken, &mut run.context).expect("an object that takes an MU now takes it");
        if let Some(entered) = entered {
            run.context.schedule(entered);
        }
        run.queue_scheduled();
        Ok(())
    }

    /// Changes a Station's process time from its next MU on, as an intervention does.
    pub fn set_process_time(&mut self, object: u32, process_time: SimTime) -> Result<(), Refusal> {
        self.allow(false)?;
        self.run.set_process_time(object, process_time).map_err(|error| Refusal::new(error.to_string()))
    }

    /// Gives the MU in work at a Station this much work from now: from a hook on entry, its own
    /// process time.
    pub fn set_work_time(&mut self, object: u32, mu: u32, work: SimTime) -> Result<(), Refusal> {
        self.allow(true)?;
        self.exists(object)?;
        let run = &mut *self.run;
        run.objects[object as usize].set_work_time(mu, work, &mut run.context)?;
        run.queue_scheduled();
        Ok(())
    }

    /// Pauses the object, or ends that.
    pub fn pause(&mut self, object: u32, on: bool) -> Result<(), Refusal> {
        self.interrupt(object, if on { EventType::PauseOn } else { EventType::PauseOff })
    }

    /// Stops the object by hand, or ends that.
    pub fn stop(&mut self, object: u32, on: bool) -> Result<(), Refusal> {
        self.interrupt(object, if on { EventType::StopOn } else { EventType::StopOff })
    }

    fn interrupt(&mut self, object: u32, eventtype: EventType) -> Result<(), Refusal> {
        self.allow(true)?;
        self.exists(object)?;
        self.run.execute_now(object, eventtype).map_err(|error| Refusal::new(error.to_string()))
    }

    /// Calls `function` again `after` from now, with `data`.
    pub fn call_at(&mut self, after: SimTime, function: HookFunction, data: u32) -> Result<(), Refusal> {
        self.allow(false)?;
        let at = self.run.time + after;
        self.run.push_event(at, EventType::Timer { function, data }, NO_OBJECT);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materialflow_objects::records_then_fails::RecordsThenFails;

    fn secs(s: u64) -> SimTime {
        SimTime::from_secs(s)
    }

    fn fails_at_five_seconds() -> SimulationRun {
        let mut run = SimulationRun::new(vec![ModelObject::RecordsThenFails(RecordsThenFails::new(secs(5)))], 0, vec![0], vec![DistributionRule::RoundRobin], Arc::default());
        run.set_recording(Recording::ALL);
        run
    }

    #[test]
    fn an_error_from_a_step_carries_what_the_event_recorded_before_it_failed() {
        let mut run = fails_at_five_seconds();
        run.init().unwrap();
        let failure = run.step().unwrap_err();
        assert_eq!(failure.error, RunError::ObjectFailed { object_id: 0, time: secs(5) });
        assert_eq!(failure.occurrences, vec![Occurrence { time: secs(5), kind: OccurrenceKind::Blocked { object_id: 0 } }]);
    }

    #[test]
    fn occurrences_carried_by_an_error_are_not_reported_again() {
        let mut run = fails_at_five_seconds();
        run.init().unwrap();
        run.step().unwrap_err();
        run.schedule_event(secs(6), EventType::StartFailure(0), 0).unwrap();
        let failure = run.step().unwrap_err();
        assert_eq!(failure.occurrences, vec![Occurrence { time: secs(6), kind: OccurrenceKind::Blocked { object_id: 0 } }]);
    }
}
