use splay_tree::SplayMap;
use std::{
    cell::RefCell, cmp::Ordering, collections::HashMap, fmt, ops::{Add, Div, Sub}, rc::{Rc, Weak}
};


fn main() {
    let mut simulator = Simulator::new();
    let mut eventcontroller = EventController::new();
    let x = Rc::new(RefCell::new(SingleStation::new(1,60.into(), None, Vec::new(), Vec::new())));
    let y = Rc::new(RefCell::new(SingleStation::new(2,75.into(), None, Vec::new(), vec![x.clone()])));
    let source = Rc::new(RefCell::new(Source::new(0,30.into(), vec![x.clone()])));
    let drain = Rc::new(RefCell::new(Drain::new(3, vec![y.clone()])));
    x.borrow_mut().succ = vec![y.clone()];
    y.borrow_mut().succ = vec![drain.clone()];
    simulator.add_obj(source);
    simulator.add_obj(x);
    simulator.add_obj(y);
    simulator.add_drain(drain);
    
    eventcontroller.init(&mut simulator);
    // println!("{:#?}", eventcontroller.events);
    simulator = eventcontroller.run(simulator).unwrap();

}

struct Simulator {
    objects: HashMap<u32, Rc<RefCell<dyn EventInterface>>>,
    drains: Vec<Rc<RefCell<Drain>>>,
    object_ID: u32, //used to give an ID to new objects as key for self.objects
}

impl Simulator {
    fn new() -> Simulator {
        Simulator {
            objects: HashMap::new(),
            drains: Vec::new(),
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
        for drain in &mut self.drains {
            drain.borrow_mut().init().unwrap();
        }
    }
    fn add_obj(&mut self, obj: Rc<RefCell<dyn EventInterface>>) {
        self.objects.insert(self.object_ID, obj);
        self.object_ID += 1;
    }
    fn add_drain(&mut self, drain: Rc<RefCell<Drain>>) {
        self.drains.push(drain);
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
                simulator.drains.iter_mut().for_each(|drain| {
                    drain.borrow_mut().update_current_time(self.time);
                });
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
            println!("{:#?}", self.events);  
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
trait MaterialFlow: EventInterface {
    fn is_entrance_allowed(&self) -> bool;
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError>;
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError>;
    fn add_blocked(&mut self, blocked: Weak<RefCell<dyn MaterialFlow>>);
}


#[derive(Clone)]
struct SingleStation {
    object_id: u32,
    proc_time: EcTime,
    remain_proc_time: EcTime,
    content: Option<MoveableUnit>,
    succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    succ_pointer: usize, //index of object in self.succ that MU will be moved to
    pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    blockedlist: Vec<Weak<RefCell<dyn MaterialFlow>>>,
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
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        self.content = Some(mu);
        self.remain_proc_time = self.proc_time;
        self.entrancelocked = true;
        self.state = ProcessState::Processing;
        return Ok(Some(EventTemplate {eventtype: EventType::MUExit, exec_time: self.proc_time.clone(), object_id: self.object_id.clone()}));   
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                if let Some(event_template) = try_move_to_successor(self)? {
                    self.entrancelocked = false;
                    self.state = ProcessState::Idle;
                    
                    // Try to get MU from blocked objects
                    if !self.blockedlist.is_empty() {
                        if let Some(blocked_obj) = self.blockedlist[0].upgrade() {
                            if let Ok(Some(blocked_event)) = blocked_obj.borrow_mut().process_mu_exit() {
                                self.blockedlist.remove(0);
                                return Ok(Some(blocked_event));
                            }
                        }
                        self.blockedlist.remove(0);
                    }
                    
                    return Ok(Some(event_template));
                }
                Ok(None)
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                Ok(None)
            }
        }
    }
    fn add_blocked(&mut self, blocked: Weak<RefCell<dyn MaterialFlow>>) {
        self.blockedlist.push(blocked);
    }
}

// Helper function to try moving MU to successor
fn try_move_to_successor(station: &mut SingleStation) -> Result<Option<EventTemplate>, MaterialflowError> {
    if station.succ.len() == 0 {
        station.state = ProcessState::Blocked;
        return Ok(None);
    }
    
    let mut i: usize = 0;
    let mut curr_succ_pointer = station.succ_pointer;
    
    while i < station.succ.len() {
        let succ_obj = station.succ.get(curr_succ_pointer).unwrap();
        if succ_obj.borrow().is_entrance_allowed() {
            if let Some(mu) = station.content.take() {
                match succ_obj.borrow_mut().process_mu_entrance(mu) {
                    Ok(Some(next_event_template)) => {
                        curr_succ_pointer += 1;
                        if station.succ.len() == curr_succ_pointer {
                            station.succ_pointer = 0;
                        } else {
                            station.succ_pointer = curr_succ_pointer;
                        }
                        return Ok(Some(next_event_template));
                    },
                    Ok(None) => return Ok(None),
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        } 
        i += 1;
        curr_succ_pointer += 1;
        if station.succ.len() < curr_succ_pointer {
            curr_succ_pointer = 0;
        }
    }

    let station_ref = Rc::new(RefCell::new(station.clone())) as Rc<RefCell<dyn MaterialFlow>>;
    station.succ.get(station.succ_pointer).unwrap().borrow_mut().add_blocked(Rc::downgrade(&station_ref));
    
    station.state = ProcessState::Blocked;
    Ok(None)
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
    fn create_mu(&mut self) -> Result<EventTemplate, MaterialflowError> {
        self.mu_id += 1;
        if self.content.is_none() {
            self.content = Some(MoveableUnit {id: self.mu_id, location_id: 0});
            self.state = ProcessState::Processing;
            return Ok(EventTemplate {eventtype: EventType::MUExit, exec_time: self.proc_time, object_id: self.object_id});
        } else {
            return Err(MaterialflowError::NoMU);
        }
        
    }
}

impl Clone for Source {
    fn clone(&self) -> Self {
        Source {
            object_id: self.object_id,
            content: self.content.clone(),
            proc_time: self.proc_time,
            remain_proc_time: self.remain_proc_time,
            succ: self.succ.clone(),  // Vec<Rc> implements Clone
            succ_pointer: self.succ_pointer,
            state: self.state.clone(),
            interruption: self.interruption.clone(),
            mu_id: self.mu_id,
        }
    }
}

impl EventInterface for Source {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        match self.create_mu() {
            Ok(next_event) => {
                return Ok(vec![next_event])
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
                        let next_mu_event = match self.create_mu() {
                            Ok(val) => val,
                            Err(e) => return Err(RunTimeError{}),
                        };
                        return Ok(Some(vec![next_mu_event,event_template]))
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
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        panic!("Source should not be processing MU entrance");       
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                if let Some(event_template) = try_move_to_successor_source(self)? {
                    return Ok(Some(event_template));
                }
                Ok(None)
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                Ok(None)
            }
        }
    }

    fn add_blocked(&mut self, _blocked: Weak<RefCell<dyn MaterialFlow>>) {
        panic!("Source cannot receive blocked objects");
    }
}

// Add helper function for Source
fn try_move_to_successor_source(source: &mut Source) -> Result<Option<EventTemplate>, MaterialflowError> {
    if source.succ.len() == 0 {
        source.state = ProcessState::Blocked;
        return Ok(None);
    }
    
    let mut i: usize = 0;
    let mut curr_succ_pointer = source.succ_pointer;
    
    while i < source.succ.len() {
        let succ_obj = source.succ.get(curr_succ_pointer).unwrap();
        if succ_obj.borrow().is_entrance_allowed() {
            if let Some(mu) = source.content.take() {
                match succ_obj.borrow_mut().process_mu_entrance(mu) {
                    Ok(Some(next_event_template)) => {
                        curr_succ_pointer += 1;
                        if source.succ.len() == curr_succ_pointer {
                            source.succ_pointer = 0;
                        } else {
                            source.succ_pointer = curr_succ_pointer;
                        }
                        return Ok(Some(next_event_template));
                    },
                    Ok(None) => return Ok(None),
                    Err(e) => {
                        return Err(e);
                    }
                }
            }
        }
        i += 1;
        curr_succ_pointer += 1;
        if source.succ.len() < curr_succ_pointer {
            curr_succ_pointer = 0;
        }
    }
    
    let source_ref = Rc::new(RefCell::new(source.clone())) as Rc<RefCell<dyn MaterialFlow>>;
    source.succ.get(source.succ_pointer).unwrap().borrow_mut().add_blocked(Rc::downgrade(&source_ref));
    source.state = ProcessState::Blocked;
    Ok(None)
}

struct Drain {
    object_id: u32,
    content: Option<MoveableUnit>,
    pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    current_time: EcTime,
    exits: Vec<EcTime>,
    exit_interval: EcTime,
    entrance_locked: bool,
    blockedlist: Vec<Weak<RefCell<dyn MaterialFlow>>>,  // Add blockedlist
}
impl Drain {
    fn new(object_id: u32, pred: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> Drain {
        Drain {
            object_id,
            content: None,
            pred,
            current_time: 0.into(),
            exits: Vec::from([0.into()]),
            exit_interval: 0.into(),
            entrance_locked: false,
            blockedlist: Vec::new(),  // Initialize blockedlist
        }
        
    }
    fn update_current_time(&mut self, time: EcTime) {
        self.current_time = time;
    }
}
impl EventInterface for Drain {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        self.exits = Vec::new();
        self.exit_interval = 0.into();
        self.entrance_locked = false;
        Ok(Vec::with_capacity(0))
    }
    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        return Err(RunTimeError{});
        
    }
}
impl MaterialFlow for Drain {
    fn is_entrance_allowed(&self) -> bool {
        if self.entrance_locked {
            return false;
        }
        true
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        self.exits.push(self.current_time.clone());
        let mut intervals: Vec<EcTime> = Vec::with_capacity(self.exits.len()-1);
        for (i, exit) in self.exits.iter().enumerate() {
            if i == 0 {
                continue;
            }
            intervals.push(*exit - self.exits[i-1]);
        }
        self.exit_interval = intervals.iter().sum::<EcTime>() / intervals.len() as u128;
        println!("Average exit interval: {}", self.exit_interval.0);

        // Check blocked list after processing
        while !self.blockedlist.is_empty() && self.is_entrance_allowed() {
            if let Some(blocked_obj) = self.blockedlist[0].upgrade() {
                if let Ok(Some(blocked_event)) = blocked_obj.borrow_mut().process_mu_exit() {
                    self.blockedlist.remove(0);
                    return Ok(Some(blocked_event));
                }
            }
            self.blockedlist.remove(0);
        }
        
        return Ok(None);
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        Ok(None)
    }
    fn add_blocked(&mut self, blocked: Weak<RefCell<dyn MaterialFlow>>) {
        self.blockedlist.push(blocked);
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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
enum ProcessState {
    Processing,
    Blocked,
    Idle,
}
#[derive(Clone)]
enum InterruptionState {
    None,
    Failed,
    Paused,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EcTime(u128);

impl std::iter::Sum for EcTime {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(EcTime(0), |acc, x| EcTime(acc.0 + x.0))
    }
}

impl<'a> std::iter::Sum<&'a EcTime> for EcTime {
    fn sum<I: Iterator<Item = &'a EcTime>>(iter: I) -> Self {
        iter.fold(EcTime(0), |acc, x| EcTime(acc.0 + x.0))
    }
}

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
impl Sub<EcTime> for EcTime {
    type Output = EcTime;
    fn sub(self, other: EcTime) -> EcTime {
        EcTime(self.0 - other.0)
    }
}
impl Sub<u128> for EcTime {
    type Output = EcTime;
    fn sub(self, other: u128) -> EcTime {
        EcTime(self.0 - other)
    }
}
impl Div<u128> for EcTime {
    type Output = EcTime;
    fn div(self, other: u128) -> EcTime {
        EcTime(self.0 / other)
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
