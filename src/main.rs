use splay_tree::SplayMap;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    ops::Add,
    cmp::Ordering,
    fmt
};

fn main() {
    let mut simulator = Simulator::new();
    let x = Rc::new(RefCell::new(SingleStation::new(1,60.into(), None, Vec::new(), Vec::new())));
    let y = Rc::new(RefCell::new(SingleStation::new(2,75.into(), None, Vec::new(), vec![x.clone()])));
    let source = Rc::new(RefCell::new(Source::new(0,30.into(), vec![x.clone()])));
    x.borrow_mut().succ = vec![y.clone()];
    simulator.add_obj(source);
    simulator.add_obj(x);
    simulator.add_obj(y);
    let mut eventcontroller = EventController::new();
    eventcontroller.init(&mut simulator);
    println!("{:#?}", eventcontroller.events);
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
            let event_templates = object.borrow_mut().init().unwrap();
            event_templates.iter().for_each(|event_template| {
                eventcontroller.add_event(eventcontroller.time + event_template.exec_time, Event {eventtype: event_template.eventtype.clone(), object: Rc::clone(object)});
            });
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
    fn init(&mut self, simulator: &mut Simulator) {
        for object in simulator.objects.values() {
            let event_templates = object.borrow_mut().init().unwrap();
            for event_template in event_templates {
                self.add_event(self.time + event_template.exec_time, Event {eventtype: event_template.eventtype, object: Rc::clone(object)});
            }
        }
    }
    fn run(&mut self, mut simulator: Simulator) -> Result<Simulator, RunTimeError> {
        while let Some((event_time, mut event_group)) = self.events.take_smallest() {
            self.time = event_time;
            while let Some((event_id, event)) = event_group.events.take_smallest() {
                println!("At {}: {:#?}", event_time.0, event.eventtype);
                match event.object.borrow_mut().execute_event(&mut simulator, event.eventtype) {
                    Ok(new_events) => {
                        if let Some(events) = new_events {
                            for new_event in events {
                                self.add_event(self.time + new_event.exec_time, Event {eventtype: new_event.eventtype, object: Rc::clone(simulator.objects.get(&new_event.object_id).unwrap())});
                            }
                        }
                    }
                    Err(e) => return Err(e),
                };

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

impl fmt::Debug for EventGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventGroup")
            .field("event_id", &self.event_id)
            .field("events_count", &self.events.len())
            .finish()
    }
}


struct Event { 
    eventtype: EventType,
    object: Rc<RefCell<dyn EventInterface>>,
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Event")
            .field("eventtype", &self.eventtype)
            // Skip the object field since it contains a trait object
            .finish()
    }
}


struct EventTemplate {
    eventtype: EventType,
    exec_time: EcTime,
    object_id: u32,  
}

impl EventTemplate {
    fn new(eventtype: EventType, exec_time: EcTime, object_id: u32) -> EventTemplate {
        EventTemplate {
            eventtype,
            exec_time,
            object_id,
        }
    }    
}

trait EventInterface {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError>;

    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError>;
}
trait MaterialFlow: EventInterface{
    fn is_entrance_allowed(&self) -> bool;
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<EventTemplate, MaterialflowError>;
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError>;
}

struct SingleStation {
    object_id: u32,
    proc_time: EcTime,
    remain_proc_time: EcTime,
    content: Option<MoveableUnit>,
    succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    succ_pointer: usize, //index of object in self.succ that MU will be moved to
    pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    blockedlist: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    entrancelocked: bool,
    exitlocked: bool,
    state: ProcessState,
    interruption: InterruptionState,
}

impl SingleStation {
    fn new(object_id: u32,
        proc_time: EcTime, 
        content: Option<MoveableUnit>,
        succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
        pred: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> SingleStation {
            SingleStation {
                object_id,
                proc_time,
                remain_proc_time: 0.into(),
                content,
                succ,
                succ_pointer: 0,
                pred,
                blockedlist: Vec::new(),
                entrancelocked: false,
                exitlocked: false,
                state: ProcessState::Idle,
                interruption: InterruptionState::None,
            }
    }
}

impl MaterialFlow for SingleStation {
    fn is_entrance_allowed(&self) -> bool {
        if self.entrancelocked {
            return false;
        }
        match self.state {
            ProcessState::Idle => {
                match self.interruption {
                    InterruptionState::None => {
                        return true;
                    },
                    InterruptionState::Failed | InterruptionState::Paused => {
                        return false;
                    }
                }
            }
            ProcessState::Processing | ProcessState::Blocked => {
                return false;
            },
        }
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<EventTemplate, MaterialflowError> {
        self.content = Some(mu);
        self.remain_proc_time = self.proc_time;
        self.entrancelocked = true;
        self.state = ProcessState::Processing;
        return Ok(EventTemplate {eventtype: EventType::MUExit, exec_time: self.proc_time.clone(), object_id: self.object_id.clone()});   
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                if self.succ.len() == 0 {
                    self.state = ProcessState::Blocked;
                    return Ok(None);
                }
                let mut i: usize = 0;
                let mut curr_succ_pointer = self.succ_pointer.clone();
                while i <= self.succ.len() -1 {
                    let succ_obj = self.succ.get(curr_succ_pointer).unwrap();
                    if succ_obj.borrow().is_entrance_allowed() {
                        match succ_obj.borrow_mut().process_mu_entrance(self.content.take().unwrap()) {
                            Ok(mut next_event_template) => {
                                self.succ_pointer += 1;
                                if self.succ_pointer == self.succ.len() - 1 {
                                    self.succ_pointer = 0;
                                }                                
                                return Ok(Some(next_event_template))
                            },
                            Err(e) => {
                                return Err(e)
                            }
                        }
                    }
                    i += 1;
                    if curr_succ_pointer == self.succ.len() - 1 {
                        curr_succ_pointer = 0;
                    } else {
                        curr_succ_pointer += 1;
                    }
                }
                self.state = ProcessState::Blocked;
                return Ok(None);
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                return Ok(None);
            }
        }
    }
}
impl EventInterface for SingleStation {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        Ok(Vec::new())
    }
    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match eventtype {
            EventType::MUExit => {
                match self.process_mu_exit() {
                    Ok(Some(event_template)) => {
                        return Ok(Some(vec![event_template]))
                    },
                    Ok(None) => {
                        return Ok(None)
                    },
                    Err(e) => {
                        return Err(RunTimeError{})
                    }
                }
            }
            _ => {
                return Err(RunTimeError{})
            }
        }
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
struct Source {
    object_id: u32,
    content: Option<MoveableUnit>,
    proc_time: EcTime,
    remain_proc_time: EcTime,
    succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    succ_pointer: usize,
    state: ProcessState,
    interruption: InterruptionState,
    mu_id: u32,//ongoing number used for new MUs
}
impl Source {
    fn new(object_id: u32, proc_time: EcTime, succ: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> Source {
        Source {
            object_id,
            content: None,
            proc_time,
            remain_proc_time: 0.into(),
            succ,
            succ_pointer: 0,
            state: ProcessState::Idle,
            interruption: InterruptionState::None,
            mu_id: 0,
        }
    }
    fn create_mu(&mut self) -> Result<(), MaterialflowError> {
        self.mu_id = 1;
        if self.content.is_none() {
            self.content = Some(MoveableUnit {id: self.mu_id, location_id: 0});
        } else {
            return Err(MaterialflowError::NoMU);
        }
        self.state = ProcessState::Processing;
        return Ok(())
    }
}
impl EventInterface for Source {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        match self.create_mu() {
            Ok(_) => {
                return Ok(vec![EventTemplate {eventtype: EventType::MUExit, exec_time: self.proc_time, object_id: self.object_id}])
                },
            Err(e) => {
                return Err(InitError{})
            }
        }
    }
        
    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match eventtype {
            EventType::MUExit => {
                match self.process_mu_exit() {
                    Ok(Some(event_template)) => {
                        return Ok(Some(vec![event_template]))
                    },
                    Ok(None) => {
                        return Ok(None)
                    },
                    Err(e) => {
                        return Err(RunTimeError{})
                    }
                }
            }
            _ => {
                return Err(RunTimeError{})
            }
        }
    }
}
impl MaterialFlow for Source {
    fn is_entrance_allowed(&self) -> bool {
        false
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<EventTemplate, MaterialflowError> {
        panic!("Source should not be processing MU entrance");       
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                if self.succ.len() == 0 {
                    self.state = ProcessState::Blocked;
                    return Ok(None);
                }
                let mut i: usize = 0;
                let mut curr_succ_pointer = self.succ_pointer.clone();
                while i <= self.succ.len() -1 {
                    let succ_obj = self.succ.get(curr_succ_pointer).unwrap();
                    if succ_obj.borrow().is_entrance_allowed() {
                        match succ_obj.borrow_mut().process_mu_entrance(self.content.take().unwrap()) {
                            Ok(next_event_template) => {
                                self.succ_pointer += 1;
                                if self.succ_pointer > self.succ.len() - 1 {
                                    self.succ_pointer = 0;
                                }                                
                                return Ok(Some(next_event_template))
                            },
                            Err(e) => {
                                return Err(e)
                            }
                        }
                    }
                    i += 1;
                    if curr_succ_pointer == self.succ.len() - 1 {
                        curr_succ_pointer = 0;
                    } else {
                        curr_succ_pointer += 1;
                    }
                }
                self.state = ProcessState::Blocked;
                return Ok(None);
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                return Ok(None);
            }
        }
    }
}
#[derive(Debug, Clone)]
enum EventType {
    MUEntrance,
    MUExit,
    Finished,
    StartFailure,
    EndFailure,
}
#[derive(Debug)]
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
enum MaterialflowError {
    EntranceDenied,
    NoMU,
}
#[derive(Debug)]
enum EventControllerError {
    EventNotFound,
    TimeNotFound,
}
#[derive(Debug)]
enum ProcessState {
    Processing,
    Blocked,
    Idle,
}
enum InterruptionState {
    None,
    Failed,
    Paused,
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
