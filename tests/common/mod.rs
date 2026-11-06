//! A material-flow object for tests, since no real object fails part way through an event.

use crabsim::flows::{EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, StopCtrl};
use crabsim::{
    EventContext, EventTemplate, EventType, InitError, MaterialflowError, ObjectKind, ObjectState, OccurrenceKind,
    RunTimeError,
};
use std::time::Duration;

/// Handles its only event at `at` by recording an occurrence and then failing.
pub struct RecordsThenFails {
    pub id: u32,
    pub at: Duration,
}

impl EventInterface for RecordsThenFails {
    fn init(&mut self, _context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
        Ok(vec![EventTemplate { eventtype: EventType::Finished, exec_time: self.at, object_id: self.id, seq: 0 }])
    }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        _eventtype: &EventType,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        context.record(OccurrenceKind::Blocked { object_id: self.id });
        Err(RunTimeError {})
    }
    fn reset(&mut self) {}
    fn report(&self, _now: Duration) -> ObjectState {
        ObjectState {
            id: self.id,
            kind: ObjectKind::Drain,
            contents: Vec::new(),
            process: ProcessState::Idle,
            stop: None,
            remaining: None,
            blocked_predecessors: Vec::new(),
            successors: Vec::new(),
            mus_left: 0,
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.id
    }
    fn set_object_id(&mut self, id: u32) {
        self.id = id;
    }
    fn set_random_seed(&mut self, _seed: u64) {}
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StopCtrl for RecordsThenFails {
    fn set_stop(&mut self, _at: &Duration, _reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
        Ok(None)
    }
    fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        Ok(None)
    }
}

impl MaterialFlow for RecordsThenFails {
    fn is_entrance_allowed(&self) -> bool {
        false
    }
    fn is_exit_allowed(&self) -> bool {
        false
    }
    fn process_mu_entrance(
        &mut self,
        _mu: MoveableUnit,
        _context: &mut EventContext,
    ) -> Result<Option<EventTemplate>, MaterialflowError> {
        Err(MaterialflowError::EntranceDenied)
    }
    fn add_blocked(&mut self, _id_blocked: u32) {}
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        Ok(None)
    }
}
