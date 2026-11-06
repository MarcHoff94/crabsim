use crate::error::{AdvanceFailure, RunError};
use crate::event::{EventContext, EventType, ScheduledEvent};
use crate::flows::EventInterface;
use crate::materialflow_objects::{ModelObject, OtherObjects};
use crate::observation::{Occurrence, OccurrenceKind, RunState};
use crate::time::SimTime;
use std::collections::BinaryHeap;

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
/// advance returns the occurrences it produced. A run executes on one thread at a time,
/// and can move between threads.
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
    initialised: bool,
    context: EventContext,
}

impl SimulationRun {
    /// A run of these objects, whose ids must match their positions. `ModelDescription` builds
    /// them that way.
    pub(crate) fn new(objects: Vec<ModelObject>, seed: u64) -> SimulationRun {
        SimulationRun {
            objects,
            queue: BinaryHeap::new(),
            time: SimTime::ZERO,
            next_seq: 0,
            counted_time: SimTime::ZERO,
            events_at_this_time: 0,
            seed,
            initialised: false,
            context: EventContext::new(1),
        }
    }

    /// Sets time to 0, calls every object's init in id order and queues the first events.
    /// Nothing is processed yet, including events due at t=0.
    pub fn init(&mut self) -> Result<Vec<Occurrence>, RunError> {
        if self.initialised {
            return Err(RunError::AlreadyInitialised);
        }
        let highest_starting_mu = self.state().objects.iter().flat_map(|o| o.contents.clone()).max().unwrap_or(0);
        self.context = EventContext::new(highest_starting_mu + 1);
        self.context.time = SimTime::ZERO;
        self.context.record(OccurrenceKind::RunInitialised);

        for index in 0..self.objects.len() {
            let id = index as u32;
            let object = &mut self.objects[index];
            object.set_random_seed(self.seed ^ (id as u64).wrapping_mul(0x9E3779B97F4A7C15));
            let events = object
                .init(&mut self.context)
                .map_err(|_| RunError::ObjectFailed { object_id: id, time: SimTime::ZERO })?;
            for event in events {
                self.push_event(event.exec_time, event.eventtype, event.object_id);
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
        for object in &mut self.objects {
            object.reset();
        }
        self.queue.clear();
        self.time = SimTime::ZERO;
        self.next_seq = 0;
        self.counted_time = SimTime::ZERO;
        self.events_at_this_time = 0;
        self.initialised = false;
        self.context = EventContext::new(1);
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
            objects: self.objects.iter().map(|object| object.report(self.time)).collect(),
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
        if until > self.time {
            self.time = until;
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

    fn process(&mut self, event: ScheduledEvent) -> Result<(), RunError> {
        self.time = event.time;
        self.context.time = self.time;
        self.context.seq = event.seq;
        log::info!("At {}s: seq={} obj={} event={:?}", self.time, event.seq, event.object_id, event.eventtype);

        let index = event.object_id as usize;
        if index >= self.objects.len() {
            return Err(RunError::UnknownObject(event.object_id));
        }
        let (before, from_object) = self.objects.split_at_mut(index);
        let (object, after) = from_object.split_at_mut(1);
        let mut others = OtherObjects::new(before, after);
        let new_events = object[0]
            .execute_event(&mut self.context, &event.eventtype, &mut others)
            .map_err(|_| RunError::ObjectFailed { object_id: event.object_id, time: self.time })?;
        for new_event in new_events.unwrap_or_default() {
            self.push_event(self.time + new_event.exec_time, new_event.eventtype, new_event.object_id);
        }
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
        SimulationRun::new(vec![ModelObject::RecordsThenFails(RecordsThenFails::new(secs(5)))], 0)
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
        run.schedule_event(secs(6), EventType::StartFailure, 0).unwrap();
        let failure = run.step().unwrap_err();
        assert_eq!(failure.occurrences, vec![Occurrence { time: secs(6), kind: OccurrenceKind::Blocked { object_id: 0 } }]);
    }
}
