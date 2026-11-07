use crate::error::MaterialflowError;
use crate::event::{EventContext, EventTemplate, EventType};
use crate::observation::{ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::StatisticsKeeper;
use crate::time::SimTime;
use super::flows::BaseObject;
use crate::materialflow_objects::OtherObjects;
use crate::flows::{list_blocked, EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, StopCtrl};

#[derive(Clone)]
pub struct Drain {
    pub base: BaseObject,
    pub statistics: StatisticsKeeper,
    pub entrance_locked: bool,
    pub blockedlist: Vec<u32>,
}
impl Drain {
    pub fn new(object_id: u32) -> Drain {
        Drain {
            base: BaseObject { object_id, random_seed: 0 },
            statistics: StatisticsKeeper::for_drain(),
            entrance_locked: false,
            blockedlist: Vec::new(),
        }
    }
}

impl EventInterface for Drain {
    fn init(&mut self, _context: &mut EventContext) -> Result<Vec<EventTemplate>, crate::error::InitError> {
        Ok(Vec::new())
    }
    fn execute_event(
        &mut self,
        _context: &mut EventContext,
        _eventtype: &EventType,
        _others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, crate::error::RunTimeError> {
        Err(crate::error::RunTimeError {})
    }
    fn reset(&mut self) {
        self.statistics.reset();
        self.entrance_locked = false;
        self.blockedlist.clear();
    }
    fn clear_statistics(&mut self, now: SimTime) {
        self.statistics.clear(now);
    }
    fn report(&self, now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Drain,
            contents: Vec::new(),
            process: ProcessState::Idle,
            stop: None,
            interruptions: Vec::new(),
            remaining: None,
            blocked_predecessors: self.blockedlist.clone(),
            successors: Vec::new(),
            capacity: 0,
            process_time: None,
            configured_process_time: None,
            statistics: self.statistics.read(now),
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.base.object_id
    }
    fn set_object_id(&mut self, id: u32) {
        self.base.object_id = id;
    }
    fn set_random_seed(&mut self, seed: u64) {
        self.base.random_seed = seed;
    }
}

impl StopCtrl for Drain {
    fn set_stop(&mut self, _failure_start: &SimTime, _reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
        todo!("StopCtrl is not implemented for Drain yet")
    }
    fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        todo!("StopCtrl is not implemented for Drain yet")
    }
}

impl MaterialFlow for Drain {
    fn is_entrance_allowed(&self) -> bool {
        !self.entrance_locked
    }
    fn is_exit_allowed(&self) -> bool {
        !self.entrance_locked
    }
    fn process_mu_entrance(
        &mut self,
        mu: MoveableUnit,
        context: &mut EventContext,
    ) -> Result<Option<EventTemplate>, MaterialflowError> {
        let lead_time = context.time.saturating_sub(mu.left_source);
        self.statistics.count_exit(context.time, lead_time);
        context.record(OccurrenceKind::MuLeft { mu_id: mu.id, at: self.base.object_id, lead_time });
        log::info!(
            "MU {} exited at time {}.{:03}s",
            mu.id,
            context.time.as_secs(),
            context.time.subsec_millis()
        );
        Ok(None)
    }
    fn add_blocked(&mut self, id_blocked: u32) {
        list_blocked(&mut self.blockedlist, id_blocked);
    }
    fn remove_blocked(&mut self, id_blocked: u32) {
        self.blockedlist.retain(|&id| id != id_blocked);
    }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        //predecessors do not get notified from drain because drain is never blocking them
        Ok(None)
    }
}
