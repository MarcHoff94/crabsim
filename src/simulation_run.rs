use crate::error::{AdvanceFailure, RunError};
use crate::event::{EventContext, EventTemplate, EventType, ScheduledEvent};
use crate::flows::MaterialFlow;
use crate::observation::{Occurrence, OccurrenceKind, RunState};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BinaryHeap},
    rc::Rc,
    time::Duration,
};

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
/// A run only advances when asked, so how fast it goes is up to the caller. Every
/// advance returns the occurrences it produced. All of a run's objects live on one
/// thread.
pub struct SimulationRun {
    objects: BTreeMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    next_object_id: u32,
    queue: BinaryHeap<ScheduledEvent>,
    time: Duration,
    next_seq: u64,
    /// The simulation time `events_at_this_time` is counting.
    counted_time: Duration,
    events_at_this_time: u64,
    seed: u64,
    initialised: bool,
    context: EventContext,
}

impl SimulationRun {
    pub fn new(seed: u64) -> SimulationRun {
        SimulationRun {
            objects: BTreeMap::new(),
            next_object_id: 0,
            queue: BinaryHeap::new(),
            time: Duration::ZERO,
            next_seq: 0,
            counted_time: Duration::ZERO,
            events_at_this_time: 0,
            seed,
            initialised: false,
            context: EventContext::new(1),
        }
    }

    /// Adds an object to the model and returns its id, which replaces any id it was built with.
    pub fn add_object<T>(&mut self, object: Rc<RefCell<T>>) -> u32
    where
        T: MaterialFlow + 'static,
    {
        let id = self.next_object_id;
        self.next_object_id += 1;
        object.borrow_mut().set_object_id(id);
        self.objects.insert(id, object);
        id
    }

    /// Sets time to 0, calls every object's init in id order and queues the first events.
    /// Nothing is processed yet, including events due at t=0.
    pub fn init(&mut self) -> Result<Vec<Occurrence>, RunError> {
        if self.initialised {
            return Err(RunError::AlreadyInitialised);
        }
        let highest_starting_mu = self.state().objects.iter().flat_map(|o| o.contents.clone()).max().unwrap_or(0);
        self.context = EventContext::new(highest_starting_mu + 1);
        self.context.time = Duration::ZERO;
        self.context.record(OccurrenceKind::RunInitialised);

        let objects: Vec<(u32, Rc<RefCell<dyn MaterialFlow>>)> =
            self.objects.iter().map(|(id, object)| (*id, object.clone())).collect();
        for (id, object) in objects {
            let mut object = object.borrow_mut();
            object.set_random_seed(self.seed ^ (id as u64).wrapping_mul(0x9E3779B97F4A7C15));
            let events = object
                .init(&mut self.context)
                .map_err(|_| RunError::ObjectFailed { object_id: id, time: Duration::ZERO })?;
            for event in events {
                self.push_event(event.exec_time, event);
            }
        }
        self.initialised = true;
        Ok(self.context.take_occurrences())
    }

    /// Processes the next event, if there is one.
    pub fn step(&mut self) -> Result<Vec<Occurrence>, AdvanceFailure> {
        let outcome = self.require_initialised().and_then(|()| self.process_next());
        self.hand_over(outcome).map(|((), occurrences)| occurrences)
    }

    /// Processes every event up to and including `until`, and leaves the clock there.
    pub fn advance_to(&mut self, until: Duration) -> Result<Vec<Occurrence>, AdvanceFailure> {
        self.advance_bounded(until, usize::MAX).map(|advance| advance.occurrences)
    }

    /// Does what `advance_to` does, but processes at most `max_events` events. If that leaves
    /// events due by `until`, the clock stays at the last one processed, and advancing again
    /// continues exactly as if this had not been split up.
    pub fn advance_bounded(&mut self, until: Duration, max_events: usize) -> Result<BoundedAdvance, AdvanceFailure> {
        let outcome = self.require_initialised().and_then(|()| self.process_up_to(until, max_events));
        self.hand_over(outcome).map(|(reached, occurrences)| BoundedAdvance { occurrences, reached })
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
    pub fn reset(&mut self) {
        for object in self.objects.values() {
            object.borrow_mut().reset();
        }
        self.queue.clear();
        self.time = Duration::ZERO;
        self.next_seq = 0;
        self.counted_time = Duration::ZERO;
        self.events_at_this_time = 0;
        self.initialised = false;
        self.context = EventContext::new(1);
    }

    /// Queues an event from outside the model at a given simulation time.
    pub fn schedule_event(&mut self, at: Duration, eventtype: EventType, object_id: u32) -> Result<(), RunError> {
        self.require_initialised()?;
        if at < self.time {
            return Err(RunError::EventInThePast);
        }
        self.push_event(at, EventTemplate { eventtype, exec_time: Duration::ZERO, object_id, seq: 0 });
        Ok(())
    }

    pub fn time(&self) -> Duration {
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

    pub fn next_event_time(&self) -> Option<Duration> {
        self.queue.peek().map(|scheduled| scheduled.time)
    }

    pub fn state(&self) -> RunState {
        RunState {
            time: self.time,
            initialised: self.initialised,
            objects: self.objects.values().map(|object| object.borrow().report(self.time)).collect(),
        }
    }

    pub fn objects(&self) -> &BTreeMap<u32, Rc<RefCell<dyn MaterialFlow>>> {
        &self.objects
    }

    fn require_initialised(&self) -> Result<(), RunError> {
        if self.initialised {
            Ok(())
        } else {
            Err(RunError::NotInitialised)
        }
    }

    fn push_event(&mut self, time: Duration, mut event: EventTemplate) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        event.seq = seq;
        self.queue.push(ScheduledEvent { time, template: event, seq });
    }

    /// Returns whether the clock reached `until`, which it does once no event is due by then.
    fn process_up_to(&mut self, until: Duration, max_events: usize) -> Result<bool, RunError> {
        for _ in 0..max_events {
            if !self.has_event_by(until) {
                break;
            }
            self.process_next()?;
        }
        if self.has_event_by(until) {
            return Ok(false);
        }
        if until > self.time {
            self.time = until;
        }
        Ok(true)
    }

    fn has_event_by(&self, time: Duration) -> bool {
        self.next_event_time().is_some_and(|next| next <= time)
    }

    /// Processes the next event, unless that would be too many without time moving. The
    /// event then stays queued, so a run in a zero-time loop keeps refusing until Reset.
    fn process_next(&mut self) -> Result<(), RunError> {
        let Some(time) = self.next_event_time() else {
            return Ok(());
        };
        if time != self.counted_time {
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

    fn process(&mut self, scheduled: ScheduledEvent) -> Result<(), RunError> {
        let event = scheduled.template;
        self.time = scheduled.time;
        self.context.time = self.time;
        self.context.seq = event.seq;
        let object = self
            .objects
            .get(&event.object_id)
            .cloned()
            .ok_or(RunError::UnknownObject(event.object_id))?;
        log::info!(
            "At {}.{:03}s: seq={} obj={} event={:?}",
            self.time.as_secs(),
            self.time.subsec_millis(),
            event.seq,
            event.object_id,
            event.eventtype
        );
        let new_events = object
            .borrow_mut()
            .execute_event(&mut self.context, &event.eventtype)
            .map_err(|_| RunError::ObjectFailed { object_id: event.object_id, time: self.time })?;
        for new_event in new_events.unwrap_or_default() {
            self.push_event(self.time + new_event.exec_time, new_event);
        }
        Ok(())
    }
}
