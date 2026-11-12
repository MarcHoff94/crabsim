use crate::time::SimTime;
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::report::{ObjectKind, ObjectState};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::materialflow_objects::OtherObjects;
use super::flows::{BaseObject, ProcessState, InterruptionState, MaterialFlow, StopCtrl, SuccessorContainer, Distributing, MoveableUnit, EventInterface, list_blocked, retries_for_blocked, try_to_move_to_successor};

/// A buffer that can hold up to `capacity` moveable units.
/// It forwards units to successors when they become available.
#[derive(Clone)]
pub struct Buffer {
    pub base: BaseObject,
    pub capacity: usize,
    pub content: Vec<MoveableUnit>,
    pub succ: Vec<u32>,
    pub succ_pointer: usize,
    /// How it picks the successor for its MU. Set by the run, which ranks the successors.
    pub distribution: Distributing,
    pub blockedlist: Vec<u32>,
    pub entrancelocked: bool,
    pub exitlocked: bool,
    pub state: ProcessState,
    pub interruption: InterruptionState,
    pub statistics: StatisticsKeeper,
}

impl Buffer {
    pub fn new(
        object_id: u32,
        capacity: usize,
        succ: Vec<u32>,
    ) -> Buffer {
        Buffer {
            base: BaseObject { object_id, random_seed: 0 },
            capacity,
            content: Vec::new(),
            succ,
            succ_pointer: 0,
            distribution: Distributing::RoundRobin,
            blockedlist: Vec::new(),
            entrancelocked: false,
            exitlocked: false,
            state: ProcessState::Idle,
            interruption: InterruptionState::None,
            statistics: StatisticsKeeper::new(capacity as u64),
        }
    }

    fn handle_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, crate::error::RunTimeError> {
        match eventtype {
            EventType::MUExit(_) => {
                // Attempt to move one MU
                let res = self.process_mu_exit(context, others);
                match res {
                    Ok(Some(event_template)) => {
                        // schedule next exit if more MU
                        if !self.content.is_empty() {
                            Ok(Some(vec![event_template, EventTemplate {
                                eventtype: EventType::MUExit(ExitTicket::new(self.content[0].id)),
                                exec_time: SimTime::ZERO,
                                object_id: self.base.object_id,
                            }]))
                        } else {
                            Ok(Some(vec![event_template]))
                        }
                    }
                    Ok(None) => Ok(None),
                    Err(_) => Err(crate::error::RunTimeError {}),
                }
            }
            EventType::BlockedRetry { .. } => {
                if !self.is_exit_allowed() {
                    return Ok(None);
                }
                // Try to move MU again
                let res = self.process_mu_exit(context, others);
                match res {
                    Ok(Some(event_template)) => Ok(Some(vec![event_template])),
                    Ok(None) => Ok(None),
                    Err(_) => Err(crate::error::RunTimeError {}),
                }
            }
            _ => Ok(None),
        }
    }

    /// Tells the statistics what the Buffer ended up doing. A Buffer's blocking outranks its being
    /// busy, because it holds MUs while they can't leave.
    fn update_statistics(&mut self, now: SimTime) {
        let state = match (&self.state, self.content.is_empty()) {
            (ProcessState::Blocked, _) => ProcessState::Blocked,
            (_, false) => ProcessState::Processing,
            (_, true) => ProcessState::Idle,
        };
        self.statistics.observe(now, MachineState::of(&self.interruption, &state), self.content.len() as u64);
    }

    fn process_mu_exit(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<Option<EventTemplate>, crate::error::MaterialflowError> {
        // Buffer does not have a processing time; simply attempt to move one MU
        if let Some(mu) = self.take_content(context.time) {
            match try_to_move_to_successor(self, context, others) {
                Ok(Some(event_template)) => {
                    // MU moved; set state to Processing if more MU remain
                    if !self.content.is_empty() {
                        self.state = ProcessState::Processing;
                    } else {
                        self.state = ProcessState::Idle;
                    }
                    Ok(Some(event_template))
                }
                Ok(None) => {
                    // No successor could accept; put MU back
                    self.content.insert(0, mu);
                    self.state = ProcessState::Blocked;
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        } else {
            // No MU to move
            self.state = ProcessState::Idle;
            Ok(None)
        }
    }
}

impl StopCtrl for Buffer {
    fn set_stop(&mut self, _failure_start: &SimTime, _reason: InterruptionState) -> Result<Option<EventTemplate>, crate::error::MaterialflowError> {
        todo!("StopCtrl is not implemented for Buffer yet")
    }
    fn remove_stop(&mut self) -> Result<Option<EventTemplate>, crate::error::MaterialflowError> {
        todo!("StopCtrl is not implemented for Buffer yet")
    }
}

impl MaterialFlow for Buffer {
    fn is_entrance_allowed(&self) -> bool {
        !self.entrancelocked && self.content.len() < self.capacity
    }
    fn is_exit_allowed(&self) -> bool {
        !self.exitlocked && !self.content.is_empty()
    }
    fn process_mu_entrance(
        &mut self,
        mu: MoveableUnit,
        context: &mut EventContext,
    ) -> Result<Option<EventTemplate>, crate::error::MaterialflowError> {
        log::info!("MU {} entered Buffer {}", mu.id, self.base.object_id);
        self.content.push(mu);
        self.statistics.count_mu_in();
        // If this is the first MU and we are idle, schedule an exit attempt
        let exit = if self.state == ProcessState::Idle && !self.content.is_empty() {
            self.state = ProcessState::Processing;
            Some(EventTemplate {
                eventtype: EventType::MUExit(ExitTicket::new(self.content[0].id)),
                exec_time: SimTime::ZERO,
                object_id: self.base.object_id,
            })
        } else {
            None
        };
        self.update_statistics(context.time);
        Ok(exit)
    }
    fn add_blocked(&mut self, id_blocked: u32) {
        list_blocked(&mut self.blockedlist, id_blocked);
    }
    fn remove_blocked(&mut self, id_blocked: u32) {
        self.blockedlist.retain(|&id| id != id_blocked);
    }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, crate::error::MaterialflowError> {
        Ok(retries_for_blocked(&mut self.blockedlist, self.base.object_id))
    }
}

impl SuccessorContainer for Buffer {
    fn distribution_mut(&mut self) -> &mut Distributing {
        &mut self.distribution
    }
    fn successors(&self) -> &[u32] {
        &self.succ
    }
    fn succ_pointer(&self) -> usize {
        self.succ_pointer
    }
    fn succ_pointer_mut(&mut self) -> &mut usize {
        &mut self.succ_pointer
    }
    fn clear_content_and_idle(&mut self) {
        self.content.clear();
        self.state = ProcessState::Idle;
    }
    fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
        &mut self.statistics
    }
    fn take_content(&mut self, _now: SimTime) -> Option<MoveableUnit> {
        if !self.content.is_empty() {
            Some(self.content.remove(0))
        } else {
            None
        }
    }
}

impl EventInterface for Buffer {
    fn init(&mut self, _context: &mut EventContext) -> Result<Vec<EventTemplate>, crate::error::InitError> {
        Ok(Vec::new())
    }
    fn reset(&mut self) {
        self.content.clear();
        self.succ_pointer = 0;
        self.blockedlist.clear();
        self.entrancelocked = false;
        self.exitlocked = false;
        self.state = ProcessState::Idle;
        self.interruption = InterruptionState::None;
        self.statistics.reset();
    }
    fn clear_statistics(&mut self, now: SimTime) {
        self.statistics.clear(now);
    }
    fn report(&self, now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Buffer,
            contents: self.content.iter().map(|mu| mu.id).collect(),
            process: self.state.clone(),
            stop: self.interruption.cause(),
            // A Buffer can't be interrupted yet: its stops aren't built.
            interruptions: Vec::new(),
            remaining: None,
            blocked_predecessors: self.blockedlist.clone(),
            successors: self.succ.clone(),
            capacity: self.statistics.capacity(),
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
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, crate::error::RunTimeError> {
        let outcome = self.handle_event(context, eventtype, others);
        self.update_statistics(context.time);
        outcome
    }
}
