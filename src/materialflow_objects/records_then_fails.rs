//! A material-flow object for tests, since no real object fails part way through an event.

use super::flows::{BaseObject, EventInterface, MaterialFlow, MoveableUnit, ProcessState};
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::{ObjectKind, ObjectState, OccurrenceKind};
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
    ) -> Result<(), RunTimeError> {
        context.record(OccurrenceKind::Blocked { object_id: self.base.object_id });
        Err(RunTimeError {})
    }
    fn reset(&mut self) {}
    fn report(&self, _now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Drain,
            contents: Vec::new(),
            content_kinds: Vec::new(),
            process: ProcessState::Idle,
            interrupted: None,
            failure_profiles: Vec::new(),
            interruptions: Vec::new(),
            remaining: None,
            work: Vec::new(),
            blocked_predecessors: Vec::new(),
            successors: Vec::new(),
            capacity: crate::report::Capacity::Limited(0),
            process_time: None,
            configured_process_time: None,
            statistics: Default::default(),
            fill: None,
            positions: Vec::new(),
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
    fn add_blocked(&mut self, _id_blocked: u32, _since: SimTime, _answering_retry: bool) {}
    fn relist_blocked(&mut self, _id_blocked: u32, _next: Option<SimTime>) {}
    fn notify_blocked_predecessors(&mut self, _context: &mut EventContext) -> Result<(), MaterialflowError> {
        Ok(())
    }
}
