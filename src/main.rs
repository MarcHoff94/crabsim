use splay_tree::SplayMap;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use std::ops::Add;
use std::cmp::Ordering;

fn main() {
    println!("Hello, world!");
    let mut simulator = Simulator::new();
    let x = Rc::new(RefCell::new(SingleStation::new(60.into(), None, None, None)));
    let y = Rc::new(RefCell::new(SingleStation::new(75.into(), None, None, Some(vec![x.clone()]))));
    x.borrow_mut().succ = Some(vec![y.clone()]);
    simulator.add_obj(x);
    simulator.add_obj(y);
    let mut eventcontroller = EventController::new();
    simulator = eventcontroller.run(simulator).unwrap();
}

struct Simulator {
    objects: HashMap<u32, Rc<RefCell<dyn EventInterface>>>,
    object_ID: u32, //used to give an ID to new objects as key for self.objects
}

impl Simulator {
    fn new() -> Simulator {
        Simulator {
            objects: HashMap::new(),
            object_ID: 0,
        }
    }
    fn init(&mut self, eventcontroller: &mut EventController) {
        for (id, object) in &self.objects {
            object.borrow_mut().init(eventcontroller).unwrap();
        }
    }
    fn add_obj(&mut self, obj: Rc<RefCell<dyn EventInterface>>) {
        self.objects.insert(self.object_ID, obj);
        self.object_ID += 1;
    }
}

struct EventController {
    events: SplayMap<EcTime, EventGroup>,
    time: EcTime,        // Current time in milliseconds
    endtime: Option<EcTime>, // Optional end time in milliseconds
}

impl EventController {
    fn new() -> EventController {
        EventController {
            events: SplayMap::new(),
            time: EcTime::from(0),
            endtime: None,
        }
    }
    fn add_event(&mut self, time: EcTime, event: Event) -> Result<(EcTime, u32), EventControllerError> {
        let event_id: u32;
        if let Some(event_group) = self.events.get_mut(&time) {
            event_id = event_group.add_event(event);
        } else {
            let mut new_event_group = EventGroup::new();
            event_id = new_event_group.add_event(event);
            self.events.insert(time, new_event_group);
        }
        Ok((time, event_id))
    }

    fn delete_event(&mut self, time: EcTime, event_id: u32) -> Result<(), EventControllerError> {
        if let Some(event_group) = self.events.get_mut(&time) {
            if event_group.remove_event(&event_id).is_none() {
                return Err(EventControllerError::EventNotFound);
            } 
            if event_group.events.is_empty() && self.events.remove(&time).is_none() {
                return Err(EventControllerError::TimeNotFound);
            }
        } else {
            return Err(EventControllerError::TimeNotFound);
        }
        Ok(())
    }
    
    fn run(&mut self, mut simulator: Simulator) -> Result<Simulator, RunTimeError> {
        while let Some((event_time, mut event_group)) = self.events.take_smallest() {
            self.time = event_time;
            println!("At {}:", event_time.0);
            while let Some((event_id, event)) = event_group.events.take_smallest() {
                event.object.borrow_mut().execute_event(&mut simulator, event.eventtype)?;
            }
        }
        Ok(simulator)
    }
}

struct EventGroup {
    event_id: u32,
    events: SplayMap<u32, Event> //key is EventGroup.event_id
}

impl EventGroup {
    fn new() -> EventGroup {
        EventGroup {event_id: 0, events: SplayMap::new()}
    }
    fn add_event(&mut self, event: Event) -> u32 {
        self.event_id += 1;
        self.events.insert(self.event_id, event);
        self.event_id
    }
    fn remove_event(&mut self, event_id: &u32) -> Option<Event> {
        self.events.remove(event_id)
    }
}

struct Event { 
    eventtype: EventType,
    object: Rc<RefCell<dyn EventInterface>>,
}

trait EventInterface {
    fn init(&mut self, event_controller: &mut EventController) -> Result<(), InitError>;

    fn schedule_next_events(&mut self) -> Option<Vec<(EcTime, Event)>>; /// Vec<(time, Event)> -> u32 is time until this event is executed

    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Rc<RefCell<dyn EventInterface>>>, RunTimeError>;
}

trait MaterialFlow: EventInterface {
    fn process_mu_entrance(&mut self, mu: Option<MoveableUnit>) -> Result<(EcTime, EventType), MaterialflowError>;
    fn process_mu_exit(&mut self) -> Result<(), MaterialflowError>;
}

struct SingleStation {
    proc_time: EcTime,
    remain_proc_time: EcTime,
    content: Option<MoveableUnit>,
    succ: Option<Vec<Rc<RefCell<dyn MaterialFlow>>>>, //id for object in simulator
    pred: Option<Vec<Rc<RefCell<dyn MaterialFlow>>>>, //id for object in simulator
    blockedlist: Vec<u16>, //index of object in self.pred that tried to move MU in station unsuccessfully
    entrancelocked: bool,
    exitlocked: bool,
    processing: bool,
    failed: bool,
    blocked: bool,
}

impl SingleStation {
    fn new(proc_time: EcTime, 
        content: Option<MoveableUnit>,
        succ: Option<Vec<Rc<RefCell<dyn MaterialFlow>>>>,
        pred: Option<Vec<Rc<RefCell<dyn MaterialFlow>>>>) -> SingleStation {
            SingleStation {
                proc_time,
                remain_proc_time: 0.into(),
                content,
                succ,
                pred,
                blockedlist: Vec::new(),
                entrancelocked: false,
                exitlocked: false,
                processing: false,
                failed: false,
                blocked: false,
            }
    }
}

impl MaterialFlow for SingleStation {
    fn process_mu_entrance(&mut self, mu: Option<MoveableUnit>) -> Result<(EcTime, EventType), MaterialflowError> {
        self.content = mu;
        self.processing = true;
        self.entrancelocked = true;
        Ok((EcTime::from(self.proc_time), EventType::MUEntrance))
    }
    fn process_mu_exit(&mut self) -> Result<(), MaterialflowError> {
        Ok(())
    }
}

impl EventInterface for SingleStation {
    fn init(&mut self, event_controller: &mut EventController) -> Result<(), InitError> {
        Ok(())
    }
    fn schedule_next_events(&mut self) -> Option<Vec<(EcTime, Event)>> {
        None
    }
    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Rc<RefCell<dyn EventInterface>>>, RunTimeError> {
        Err(RunTimeError{})
    }
}
// struct ParallelStation {

// }
// impl MaterialFlow for ParallelStation {
    
// }
// impl EventInterface for ParallelStation {
//     fn init(&mut self, event_controller: &mut EventController) -> Result<(), InitError> {
//         Ok(())
//     }
//     fn schedule_next_events(&mut self) -> Option<Vec<(u32, Event)>> {
//         None
//     }
//     fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Rc<RefCell<dyn EventInterface>>>, RunTimeError> {
//         Err(RunTimeError{})
//     }
// }

enum EventType {
    MUEntrance,
    MUExit,
    StartFailure,
    EndFailure,
}

struct MoveableUnit {
    id: u32,
    location_id: u32,
}

// Error definitions
#[derive(Debug)]
struct InitError;

#[derive(Debug)]
struct RunTimeError;

#[derive(Debug)]
struct MaterialflowError;

#[derive(Debug)]
enum EventControllerError {
    EventNotFound,
    TimeNotFound,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EcTime(u128);

impl Add<u128> for EcTime {
    type Output = EcTime;
    fn add(self, other: u128) -> EcTime {
        EcTime(self.0 + other)
    }
}

impl Add<EcTime> for EcTime {
    type Output = EcTime;
    fn add(self, other: EcTime) -> EcTime {
        EcTime(self.0 + other.0)
    }
}

impl Ord for EcTime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for EcTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl From<u128> for EcTime {
    fn from(value: u128) -> Self {
        EcTime(value)
    }
}
