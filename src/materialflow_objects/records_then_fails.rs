//! A material-flow object for tests, since no real object fails part way through an event.

use super::flows::{BaseObject, EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, StopCtrl};
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::observation::{ObjectKind, ObjectState, OccurrenceKind};
use crate::materialflow_objects::OtherObjects;
use crate::time::SimTime;

/// Handles its only event at `at` by recording an occurrence and then failing.
#[derive(Clone)]
pub struct RecordsThenFails {
    pub base: BaseObject,
    pub at: SimTime,
}

impl RecordsThenFails {
    pub fn new(at: SimTime) -> RecordsThenFails {
        RecordsThenFails { base: BaseObject { object_id: 0, random_seed: 0 }, at }
    }
}

impl EventInterface for RecordsThenFails {
    fn init(&mut self, _context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
        Ok(vec![EventTemplate { eventtype: EventType::Finished, exec_time: self.at, object_id: self.base.object_id }])
    }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        _eventtype: &EventType,
        _others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        context.record(OccurrenceKind::Blocked { object_id: self.base.object_id });
        Err(RunTimeError {})
    }
    fn reset(&mut self) {}
    fn report(&self, _now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Drain,
            contents: Vec::new(),
            process: ProcessState::Idle,
            stop: None,
            remaining: None,
            blocked_predecessors: Vec::new(),
            successors: Vec::new(),
            capacity: 0,
            statistics: Default::default(),
        }
    }
    fn clear_statistics(&mut self, _now: SimTime) {}
    fn get_object_id(&self) -> &u32 {
        &self.base.object_id
    }
    fn set_object_id(&mut self, id: u32) {
        self.base.object_id = id;
    }
    fn set_random_seed(&mut self, _seed: u64) {}
}

impl StopCtrl for RecordsThenFails {
    fn set_stop(&mut self, _at: &SimTime, _reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
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
    fn remove_blocked(&mut self, _id_blocked: u32) {}
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        Ok(None)
    }
}
