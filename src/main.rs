use std::{
    cell::RefCell, cmp::Ordering, collections::{HashMap, VecDeque}, fmt, ops::{Add, Div, Sub}, rc::{Rc, Weak}
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
    simulator.objects.insert(simulator.object_id, source.clone() as Rc<RefCell<dyn EventInterface>>);
    simulator.material_flows.insert(simulator.object_id, source.clone() as Rc<RefCell<dyn MaterialFlow>>);
    simulator.object_id += 1;
    simulator.objects.insert(simulator.object_id, x.clone() as Rc<RefCell<dyn EventInterface>>);
    simulator.material_flows.insert(simulator.object_id, x.clone() as Rc<RefCell<dyn MaterialFlow>>);
    simulator.object_id += 1;
    simulator.objects.insert(simulator.object_id, y.clone() as Rc<RefCell<dyn EventInterface>>);
    simulator.material_flows.insert(simulator.object_id, y.clone() as Rc<RefCell<dyn MaterialFlow>>);
    simulator.object_id += 1;
    simulator.add_drain(drain.clone());
    eventcontroller.endtime = Some(500.into());
    eventcontroller.init(&mut simulator);
    let _simulator = eventcontroller.run(simulator).unwrap();
    println!("exit interval: {:#?}", _simulator.drains[0].borrow().exits);
}

struct Simulator {
    objects: HashMap<u32, Rc<RefCell<dyn EventInterface>>>,
    material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    drains: Vec<Rc<RefCell<Drain>>>,
    object_id: u32, //used to give an ID to new objects as key for self.objects
}

impl Simulator {
    fn new() -> Simulator {
        Simulator {
            objects: HashMap::new(),
            material_flows: HashMap::new(),
            drains: Vec::new(),
            object_id: 0,
        }
    }
    fn init(&mut self, eventcontroller: &mut EventController) {
        for (_id, object) in &self.objects {
            let event_templates = object.borrow_mut().init().unwrap();
            event_templates.iter().for_each(|event_template| {
                eventcontroller.add_event(eventcontroller.time + event_template.exec_time, event_template.clone());
            });
        }
        for drain in &mut self.drains {
            drain.borrow_mut().init().unwrap();
        }
    }
    // Only insert into objects map
    fn add_obj(&mut self, obj: Rc<RefCell<dyn EventInterface>>) {
        self.objects.insert(self.object_id, obj);
        self.object_id += 1;
    }
    fn add_drain(&mut self, drain: Rc<RefCell<Drain>>) {
        self.drains.push(drain);
    }   
}

struct EventController {
    event_queue: VecDeque<(EcTime, EventTemplate)>,
    time: EcTime,
    endtime: Option<EcTime>,
}

impl EventController {
    fn new() -> EventController {
        EventController {
            event_queue: VecDeque::new(),
            time: EcTime::from(0),
            endtime: None,
        }
    }
    fn add_event(&mut self, time: EcTime, event_template: EventTemplate) {
        self.event_queue.push_back((time, event_template));
        self.event_queue.make_contiguous().sort_by_key(|(t, _)| *t);
    }
    fn init(&mut self, simulator: &mut Simulator) {
        for object in simulator.objects.values() {
            let event_templates = object.borrow_mut().init().unwrap();
            for event_template in event_templates {
                self.add_event(self.time + event_template.exec_time, event_template.clone());
            }
        }
    }
    fn run(&mut self, mut simulator: Simulator) -> Result<Simulator, RunTimeError> {
        while let Some((event_time, event_template)) = self.event_queue.pop_front() {
            self.time = event_time;
            for drain in &mut simulator.drains {
                drain.borrow_mut().update_current_time(self.time);
            }
            if self.time > self.endtime.unwrap_or(EcTime(u128::MAX)) {
                break;
            }
            // Lookup and clone the object Rc before mutable borrow
            let obj_rc = simulator.objects.get(&event_template.object_id).cloned();
            let eventtype = event_template.eventtype.clone();
            if let Some(obj) = obj_rc {
                let mut queued_events: Vec<EventTemplate> = Vec::new();
                let mut obj_mut = obj.borrow_mut();
                println!("At {}: {:#?} | obj_id = {}", event_time.0, eventtype.clone(), event_template.object_id);
                if let Ok(new_events) = obj_mut.execute_event(&mut simulator, eventtype) {
                    if let Some(evts) = new_events {
                        queued_events.extend(evts);
                    }
                }
                drop(obj_mut);
                for new_event in queued_events {
                    let scheduled_time = self.time + new_event.exec_time;
                    self.add_event(scheduled_time, new_event);
                }
            }
        }
        Ok(simulator)
    }
}


#[derive(Debug, Clone)]
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

    fn get_object_id(&self) -> &u32;
    
    fn as_any(&self) -> &dyn std::any::Any;
}
trait MaterialFlow: EventInterface {
    fn is_entrance_allowed(&self) -> bool;
    fn process_mu_entrance(&mut self, _mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError>;
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError>;
    fn add_blocked(&mut self, id_blocked: u32);
    fn as_any(&mut self) -> &mut dyn std::any::Any;
}

// Trait for objects with successors and round-robin pointer
trait SuccessorContainer {
    fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>>;
    fn succ_pointer(&self) -> usize;
    fn succ_pointer_mut(&mut self) -> &mut usize;
    fn clear_content_and_idle(&mut self);
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
    blockedlist: Vec<u32>, // Store blocked object IDs directly
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
    fn add_blocked(&mut self, blocked_id: u32) {
        self.blockedlist.push(blocked_id);
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
    fn process_mu_entrance(&mut self, _mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        self.content = Some(_mu);
        self.remain_proc_time = self.proc_time;
        self.entrancelocked = true;
        self.state = ProcessState::Processing;
        return Ok(Some(EventTemplate {eventtype: EventType::MUExit, exec_time: self.proc_time.clone(), object_id: self.object_id.clone()}));   
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                Ok(None)
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                Ok(None)
            }
        }
    }
    fn add_blocked(&mut self, id_blocked: u32) {
        self.blockedlist.push(id_blocked);
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl EventInterface for SingleStation {
    fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
        Ok(Vec::new())
    }
    fn execute_event(&mut self, simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match eventtype {
            EventType::MUExit => {
                match try_to_move_to_successor(self) {
                    Ok(Some(event_template)) => {
                        self.entrancelocked = false;
                        self.state = ProcessState::Idle;
                        // Blockedlist handling: queue EventTemplates using IDs
                        let mut blocked_events = Vec::new();
                        while let Some(blocked_id) = self.blockedlist.pop() {
                            blocked_events.push(EventTemplate {
                                eventtype: EventType::MUExit,
                                exec_time: 0.into(),
                                object_id: blocked_id,
                            });
                        }
                        blocked_events.push(event_template);
                        return Ok(Some(blocked_events));
                    },
                    Ok(None) => {
                        return Ok(None)
                    },
                    Err(_) => {
                        return Err(RunTimeError{})
                    }
                }
            }
            _ => {
                return Err(RunTimeError{})
            }
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.object_id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl SuccessorContainer for SingleStation {
    fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>> { &self.succ }
    fn succ_pointer(&self) -> usize { self.succ_pointer }
    fn succ_pointer_mut(&mut self) -> &mut usize { &mut self.succ_pointer }
    fn clear_content_and_idle(&mut self) {
        self.content = None;
        self.entrancelocked = false;
        self.state = ProcessState::Idle;
    }
}

// Generalized function for both SingleStation and Source
fn try_to_move_to_successor<T: SuccessorContainer>(obj: &mut T) -> Result<Option<EventTemplate>, MaterialflowError> {
    let succ = obj.successors();
    if succ.is_empty() {
        return Ok(None);
    }
    let n = succ.len();
    let mut checked = 0;
    let mut curr_succ_pointer = obj.succ_pointer();
    while checked < n {
        let succ_obj = &succ[curr_succ_pointer];
        if succ_obj.borrow().is_entrance_allowed() {
            let succ_id = succ_obj.borrow().get_object_id().clone();
            curr_succ_pointer = (curr_succ_pointer + 1) % n;
            *obj.succ_pointer_mut() = curr_succ_pointer;
            obj.clear_content_and_idle();
            return Ok(Some(EventTemplate {
                eventtype: EventType::MUEntrance,
                exec_time: 0.into(),
                object_id: succ_id,
            }));
        }
        curr_succ_pointer = (curr_succ_pointer + 1) % n;
        checked += 1;
    }
    Ok(None)
}

// Helper function to try moving MU to successor

// Helper function to handle Source event logic and avoid nested mutable borrows
fn handle_source_mu_exit(source: &mut Source, simulator: &Simulator) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
    let event_template_result = try_to_move_to_successor(source);
    if let Ok(Some(event_template)) = event_template_result {
        let next_mu_event = match source.create_mu() {
            Ok(val) => val,
            Err(_) => return Err(RunTimeError{}),
        };
        Ok(Some(vec![next_mu_event,event_template]))
    } else if let Ok(None) = event_template_result {
        Ok(None)
    } else {
        Err(RunTimeError{})
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
            Err(_e) => {
                return Err(InitError{})
            }
        }
    }
    fn execute_event(&mut self, _simulator: &mut Simulator, eventtype: EventType) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        match eventtype {
            EventType::MUExit => {
                // Use self directly, do not search for rc_self
                // Use helper function to avoid nested mutable borrows
                return handle_source_mu_exit(self, _simulator);
            }
            _ => {
                return Err(RunTimeError{})
            }
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.object_id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl MaterialFlow for Source {
    fn is_entrance_allowed(&self) -> bool {
        false
    }
    fn process_mu_entrance(&mut self, _mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError> {
        panic!("Source should not be processing MU entrance");       
    }
    fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        match self.interruption {
            InterruptionState::None => {
                Ok(None)
            },
            InterruptionState::Failed | InterruptionState::Paused => {
                self.state = ProcessState::Blocked;
                Ok(None)
            }
        }
    }

    fn add_blocked(&mut self, id_blocked: u32) {
        panic!("Source cannot receive blocked objects");
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl SuccessorContainer for Source {
    fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>> { &self.succ }
    fn succ_pointer(&self) -> usize { self.succ_pointer }
    fn succ_pointer_mut(&mut self) -> &mut usize { &mut self.succ_pointer }
    fn clear_content_and_idle(&mut self) {
        self.content = None;
        self.state = ProcessState::Idle;
    }
}

// Add helper function for Source

struct Drain {
    object_id: u32,
    content: Option<MoveableUnit>,
    pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    current_time: EcTime,
    exits: Vec<EcTime>,
    exit_interval: EcTime,
    entrance_locked: bool,
    blockedlist: Vec<u32>, // Store blocked object IDs directly
}
impl Drain {
    fn new(object_id: u32, pred: Vec<Rc<RefCell<dyn MaterialFlow>>>) -> Drain {
        Drain {
            object_id,
            content: None,
            pred,
            current_time: 0.into(),
            exits: Vec::new(),
            exit_interval: 0.into(),
            entrance_locked: false,
            blockedlist: Vec::new(), // Initialize blockedlist
        }
        
    }
    fn update_current_time(&mut self, time: EcTime) {
        self.current_time = time;
    }
    fn add_blocked(&mut self, blocked_id: u32) {
        self.blockedlist.push(blocked_id);
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
        match eventtype {
            EventType::MUExit => {
                // Only queue EventTemplates for blocked objects, do not call process_mu_exit directly on them
                let mut blocked_events = Vec::new();
                while let Some(blocked_id) = self.blockedlist.pop() {
                    blocked_events.push(EventTemplate {
                        eventtype: EventType::MUExit,
                        exec_time: 0.into(),
                        object_id: blocked_id,
                    });
                }
                // Only queue the event for this object itself
                let mut events = blocked_events;
                if let Ok(Some(event_template)) = self.process_mu_exit() {
                    events.push(event_template);
                }
                if !events.is_empty() {
                    return Ok(Some(events));
                } else {
                    return Ok(None);
                }
            }
            _ => {
                return Err(RunTimeError{});
            }
        }
    }
    fn get_object_id(&self) -> &u32 {
        &self.object_id
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        self.content = Some(mu);
        self.entrance_locked = true;
        self.exits.push(self.current_time);
        println!("MU {} exited at time {}", self.content.as_ref().unwrap().id, self.current_time.0);
        let mut intervals: Vec<EcTime> = Vec::new();
        for (i, exit) in self.exits.iter().enumerate() {
            if i == 0 {
                continue;
            }
            intervals.push(*exit - self.exits[i-1]);
        }
        if !intervals.is_empty() {
            self.exit_interval = intervals.iter().sum::<EcTime>() / intervals.len() as u128;
            println!("Average exit interval: {}", self.exit_interval.0);
        }
        // Blockedlist handling: queue EventTemplates, do NOT call process_mu_exit
        let mut blocked_events = Vec::new();
        while let Some(blocked_id) = self.blockedlist.pop() {
            blocked_events.push(EventTemplate {
                eventtype: EventType::MUExit,
                exec_time: 0.into(),
                object_id: blocked_id,
            });
        }
        if !blocked_events.is_empty() {
            // Return only the first event, queue the rest in the main loop
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
