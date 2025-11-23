use crate::error::MaterialflowError;
use crate::event::{EcTime, EventTemplate, EventType};
use crate::flows::{EventInterface, MaterialFlow, MoveableUnit};
use crate::simulator::Simulator;
use std::{cell::RefCell, rc::Rc};

pub struct Drain {
    pub object_id: u32,
    pub content: Option<MoveableUnit>,
    pub pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub current_time: EcTime,
    pub exits: Vec<EcTime>,
    pub exit_ids: Vec<u32>,
    pub exit_interval: EcTime,
    pub entrance_locked: bool,
    pub blockedlist: Vec<u32>,
}
impl Drain {
    pub fn new(object_id: u32, pred: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> Drain {
        Drain {
            object_id,
            content: None,
            pred,
            current_time: 0.into(),
            exits: Vec::new(),
            exit_ids: Vec::new(),
            exit_interval: 0.into(),
            entrance_locked: false,
            blockedlist: Vec::new(),
        }
    }
    pub fn update_current_time(&mut self, time: EcTime) {
        self.current_time = time;
    }
    pub fn add_blocked(&mut self, blocked_id: u32) {
        if !self.blockedlist.contains(&blocked_id) {
            log::debug!(
                "{}: adding blocked predecessor {}",
                self.object_id,
                blocked_id
            );
            self.blockedlist.push(blocked_id);
        } else {
            log::debug!(
                "{}: predecessor {} already in blockedlist",
                self.object_id,
                blocked_id
            );
        }
    }
}

impl EventInterface for Drain {
    fn init(&mut self) -> Result<Vec<EventTemplate>, crate::error::InitError> {
        self.exits = Vec::new();
        self.exit_ids = Vec::new();
        self.exit_interval = 0.into();
        self.entrance_locked = false;
        Ok(Vec::with_capacity(0))
    }
    fn execute_event(
        &mut self,
        _simulator: &mut Simulator,
        eventtype: EventType,
    ) -> Result<Option<Vec<EventTemplate>>, crate::error::RunTimeError> {
        match eventtype {
            EventType::MUExit => {
                let mut blocked_events = Vec::new();
                while let Some(blocked_id) = self.blockedlist.pop() {
                    blocked_events.push(EventTemplate {
                        eventtype: EventType::MUExit,
                        exec_time: 0.into(),
                        object_id: blocked_id,
                        seq: 0,
                    });
                }
                let mut events = blocked_events;
                if let Ok(Some(event_template)) = self.process_mu_exit() {
                    events.push(event_template);
                }
                if !events.is_empty() {
                    Ok(Some(events))
                } else {
                    Ok(None)
                }
            }
            _ => Err(crate::error::RunTimeError {}),
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.object_id
    }
    fn set_object_id(&mut self, id: u32) {
        self.object_id = id;
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
        // Drain doesn't perform exits in the same way as stations/sources.
        // Use entrance_locked as a proxy: if the drain is entrance-locked,
        // it's not ready to accept new content/propagate blocked retries.
        !self.entrance_locked
    }
    fn process_mu_entrance(
        &mut self,
        mu: MoveableUnit,
    ) -> Result<Option<EventTemplate>, MaterialflowError> {
        let mu_id = mu.id;
        self.exits.push(self.current_time);
        self.exit_ids.push(mu_id);
        log::info!("MU {} exited at time {}", mu_id, self.current_time.0);
        let mut intervals: Vec<EcTime> = Vec::new();
        for (i, exit) in self.exits.iter().enumerate() {
            if i == 0 {
                continue;
            }
            intervals.push(*exit - self.exits[i - 1]);
        }
        if !intervals.is_empty() {
            self.exit_interval = intervals.iter().sum::<EcTime>() / intervals.len() as u128;
            log::info!("Average exit interval: {}", self.exit_interval.0);
        }
        let mut blocked_events = Vec::new();
        while let Some(blocked_id) = self.blockedlist.pop() {
            blocked_events.push(EventTemplate {
                eventtype: EventType::BlockedRetry,
                exec_time: 0.into(),
                object_id: blocked_id,
                seq: 0,
            });
        }
        if !blocked_events.is_empty() {
            return Ok(Some(blocked_events.remove(0)));
        }
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
}
