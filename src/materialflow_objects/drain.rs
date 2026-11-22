use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{MachineState, StatisticsKeeper};
use super::flows::FailureProfile;
use super::interruptions::{Interruptible, Interruptions};
use crate::time::SimTime;
use super::flows::BaseObject;
use crate::materialflow_objects::OtherObjects;
use crate::flows::{list_blocked, relist_blocked, retries_for_blocked, waiting_ids, Waiting, EventInterface, MaterialFlow, MoveableUnit, ProcessState};

#[derive(Clone)]
pub struct Drain {
    pub base: BaseObject,
    pub statistics: StatisticsKeeper,
    pub entrance_locked: bool,
    pub blockedlist: Vec<Waiting>,
    /// Its failures and pauses (ADR 0020). Interrupted, it refuses MUs.
    pub interruptions: Interruptions,
    /// MUs conveyors are handing over to it, whose booking points haven't arrived (ADR 0023).
    incoming: u16,
}
impl Drain {
    pub fn new(object_id: u32) -> Drain {
        Drain {
            base: BaseObject { object_id, random_seed: 0 },
            statistics: StatisticsKeeper::for_drain(),
            entrance_locked: false,
            blockedlist: Vec::new(),
            interruptions: Interruptions::new(None),
            incoming: 0,
        }
    }

    /// Gives it failures, or none with `None`.
    pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
        self.interruptions.set_failures(failures);
    }

    /// Idle, or receiving an MU from a conveyor (ADR 0023).
    fn process_state(&self) -> ProcessState {
        if self.incoming > 0 { ProcessState::Receiving } else { ProcessState::Idle }
    }

    /// Tells the statistics what it ended up doing: idle, receiving, or interrupted.
    fn update_statistics(&mut self, now: SimTime) {
        self.statistics.observe(now, MachineState::of(self.interruptions.most_serious(), &self.process_state()), 0);
    }
}

impl EventInterface for Drain {
    fn init(&mut self, _context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
        Ok(self.interruptions.first_failure(self.base.object_id).into_iter().collect())
    }
    /// Only interruptions are events for a Drain: it takes MUs as they are offered.
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<(), RunTimeError> {
        let outcome = self.handle_interruption(eventtype, context, others).unwrap_or(Err(RunTimeError {}));
        self.update_statistics(context.time);
        outcome
    }
    fn reset(&mut self) {
        self.statistics.reset();
        self.entrance_locked = false;
        self.blockedlist.clear();
        self.interruptions.reset();
        self.incoming = 0;
    }
    fn clear_statistics(&mut self, now: SimTime) {
        self.interruptions.clear_statistics(now, &self.statistics);
		self.statistics.clear(now);
    }
    fn report(&self, now: SimTime) -> ObjectState {
        ObjectState {
            id: self.base.object_id,
            kind: ObjectKind::Drain,
            contents: Vec::new(),
            process: self.process_state(),
            interrupted: self.interruptions.most_serious(),
            interruptions: self.interruptions.list(),
			failure_profiles: self.interruptions.profile_states(now, &self.statistics),
            remaining: None,
            work: Vec::new(),
            blocked_predecessors: waiting_ids(&self.blockedlist),
            successors: Vec::new(),
            capacity: Capacity::Limited(0),
            process_time: None,
            configured_process_time: None,
            statistics: self.statistics.read(now),
            fill: None,
            positions: Vec::new(),
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
        self.interruptions.reseed(seed);
    }
}

impl MaterialFlow for Drain {
    fn is_entrance_allowed(&self) -> bool {
        !self.entrance_locked && !self.interruptions.is_interrupted() && self.incoming < u16::MAX
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
        // An MU it was receiving arrives, even while it is interrupted. Only then does what it is
        // doing change.
        if self.incoming > 0 {
            self.incoming -= 1;
            self.update_statistics(context.time);
        }
        Ok(None)
    }
    fn reserve(&mut self, mu_id: u32, context: &mut EventContext) {
        self.incoming += 1;
        context.record(OccurrenceKind::Reserved { object_id: self.base.object_id, mu_id });
        self.update_statistics(context.time);
    }
    fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool) {
        list_blocked(&mut self.blockedlist, id_blocked, since, answering_retry);
    }
    fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>) {
        relist_blocked(&mut self.blockedlist, id_blocked, next);
    }
    fn notify_blocked_predecessors(&mut self, context: &mut EventContext) -> Result<(), MaterialflowError> {
        retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
        Ok(())
    }
}

impl Interruptible for Drain {
    fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32) {
        (&mut self.interruptions, &mut self.statistics, self.base.object_id)
    }
    /// It refuses MUs while interrupted, which `is_entrance_allowed` already says.
    fn stop_began(&mut self, _context: &mut EventContext) {}
    /// Back, it has room for those that waited for it, longest-waiting first.
    fn stop_ended(&mut self, context: &mut EventContext, _others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
        retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
        Ok(())
    }
}
