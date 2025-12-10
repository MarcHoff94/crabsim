use crate::error::MaterialflowError;
use crate::event::{EventTemplate, EventType};
use std::time::Duration;
use super::flows::BaseObject;
use crate::flows::{EventInterface, MaterialFlow, MoveableUnit};
use crate::simulator::Simulator;
use std::{cell::RefCell, rc::Rc};

pub struct Drain {
    pub base: BaseObject,
    pub content: Option<MoveableUnit>,
    pub pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub current_time: Duration,
    pub exits: Vec<Duration>,
    pub exit_ids: Vec<u32>,
    pub exit_interval: Duration,
    pub entrance_locked: bool,
    pub blockedlist: Vec<u32>,
}
impl Drain {
    pub fn new(object_id: u32, pred: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> Drain {
        Drain {
            base: BaseObject { object_id, random_seed: 0 },
            content: None,
            pred,
            current_time: Duration::from_millis(0),
            exits: Vec::new(),
            exit_ids: Vec::new(),
            exit_interval: Duration::from_millis(0),
            entrance_locked: false,
            blockedlist: Vec::new(),
        }
    }
    pub fn update_current_time(&mut self, time: Duration) {
        self.current_time = time;
    }
    pub fn add_blocked(&mut self, blocked_id: u32) {
        if !self.blockedlist.contains(&blocked_id) {
            log::debug!(
                "{}: adding blocked predecessor {}",
                self.base.object_id,
                blocked_id
            );
            self.blockedlist.push(blocked_id);
        } else {
            log::debug!(
                "{}: predecessor {} already in blockedlist",
                self.base.object_id,
                blocked_id
            );
        }
    }
}

impl EventInterface for Drain {
    fn init(&mut self) -> Result<Vec<EventTemplate>, crate::error::InitError> {
        self.exits = Vec::new();
        self.exit_ids = Vec::new();
        self.exit_interval = Duration::from_millis(0);
        self.entrance_locked = false;
        Ok(Vec::with_capacity(0))
    }
    fn execute_event(
        &mut self,
        _simulator: &mut Simulator,
        eventtype: &EventType,
    ) -> Result<Option<Vec<EventTemplate>>, crate::error::RunTimeError> {
        match eventtype {
            // EventType::MUExit => {
            //     let mut blocked_events = Vec::new();
            //     while let Some(blocked_id) = self.blockedlist.pop() {
            //         blocked_events.push(EventTemplate {
            //             eventtype: EventType::MUExit,
            //             exec_time: 0.into(),
            //             object_id: blocked_id,
            //             seq: 0,
            //         });
            //     }
            //     let mut events = blocked_events;
            //     if let Ok(Some(event_template)) = self.process_mu_exit() {
            //         events.push(event_template);
            //     }
            //     if !events.is_empty() {
            //         Ok(Some(events))
            //     } else {
            //         Ok(None)
            //     }
            // }
            _ => Err(crate::error::RunTimeError {}),
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
    fn as_any(&self) -> &dyn std::any::Any {
        self
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
    ) -> Result<Option<EventTemplate>, MaterialflowError> {
        let mu_id = mu.id;
        self.exits.push(self.current_time);
        self.exit_ids.push(mu_id);
        log::info!(
            "MU {} exited at time {}.{:03}s",
            mu_id,
            self.current_time.as_secs(),
            self.current_time.subsec_millis()
        );
        let mut intervals: Vec<Duration> = Vec::new();
        for (i, exit) in self.exits.iter().enumerate() {
            if i == 0 {
                continue;
            }
            let prev = self.exits[i - 1];
            let diff_ms = exit.as_millis().saturating_sub(prev.as_millis());
            intervals.push(Duration::from_millis(diff_ms as u64));
        }
        if !intervals.is_empty() {
            let total_ms: u128 = intervals.iter().map(|d| d.as_millis()).sum();
            let avg_ms = total_ms / intervals.len() as u128;
            self.exit_interval = Duration::from_millis(avg_ms as u64);
            log::info!(
                "Average exit interval: {}.{:03}s",
                self.exit_interval.as_secs(),
                self.exit_interval.subsec_millis()
            );
        }
        // let mut blocked_events = Vec::new();
        // while let Some(blocked_id) = self.blockedlist.pop() {
        //     blocked_events.push(EventTemplate {
        //         eventtype: EventType::BlockedRetry,
        //         exec_time: 0.into(),
        //         object_id: blocked_id,
        //         seq: 0,
        //     });
        // }
        // if !blocked_events.is_empty() {
        //     return Ok(Some(blocked_events.remove(0)));
        // }
        Ok(None)
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        Ok(None)
    }
    fn add_blocked(&mut self, id_blocked: u32) {
        self.blockedlist.push(id_blocked);
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        
        //predecessors do not get notified from drain because drain is never blocking them
        Ok(None)
        // let mut blocked_events = Vec::new();
        // while let Some(blocked_id) = self.blockedlist.pop() {
        //     blocked_events.push(EventTemplate {
        //         eventtype: EventType::BlockedRetry,
        //         exec_time: 0.into(),
        //         object_id: blocked_id,
        //         seq: 0,
        //     });
        // }
        // if !blocked_events.is_empty() {
        //     Ok(Some(blocked_events))
        // } else {
        //     Ok(None)
        // }
    }
}
