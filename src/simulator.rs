#![allow(dead_code)]

use crate::error::RunTimeError;
use crate::event::{EcTime, EventTemplate, ScheduledEvent};
use crate::materialflow_objects::{Drain, SingleStation, Source};
use crate::flows::{EventInterface, MaterialFlow, ProcessState};
use crate::visualization::topology::TopologyVisualizer;
use crate::visualization::state::{StateVisualizer, StateSnapshot};
use std::{
    cell::RefCell,
    collections::{BinaryHeap, HashMap, HashSet},
    rc::Rc,
};
use std::net::TcpStream;
use std::io::Write;
use serde_json;

pub struct Simulator {
    pub material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    pub drains: Vec<Rc<RefCell<Drain>>>,
    pub object_id: u32,
    pub current_seq: u64,
    pub topology_printed: bool,
    pub live_inplace: bool,
    inplace_anchor_set: bool,
    pub renderer_stream: Option<TcpStream>,
    topology_sent: bool,
}

impl Simulator {
    pub fn new() -> Simulator {
        Simulator {
            material_flows: HashMap::new(),
            drains: Vec::new(),
            object_id: 0,
            current_seq: 0,
            topology_printed: false,
            live_inplace: false,
            inplace_anchor_set: false,
            renderer_stream: None,
            topology_sent: false,
        }
    }
    pub fn init(&mut self, eventcontroller: &mut EventController) {
        for object in self.material_flows.values() {
            let event_templates = object.borrow_mut().init().unwrap();
            for event_template in event_templates {
                eventcontroller.add_event(
                    eventcontroller.time + event_template.exec_time,
                    event_template,
                );
            }
        }
        for drain in &mut self.drains {
            drain.borrow_mut().init().unwrap();
        }
    }
    pub fn add_drain(&mut self, drain: Rc<RefCell<Drain>>) {
        // Assign a global unique object_id to drains as well
        let id = self.object_id;
        drain.borrow_mut().set_object_id(id);
        self.object_id += 1;
        self.drains.push(drain);
    }

    pub fn register_material_flow<T>(&mut self, obj: Rc<RefCell<T>>) -> u32
    where
        T: MaterialFlow + 'static,
    {
        let id = self.object_id;
        obj.borrow_mut().set_object_id(id);
        let mf: Rc<RefCell<dyn MaterialFlow>> = obj;
        self.material_flows.insert(id, mf);
        self.object_id += 1;
        id
    }

    pub fn print_network_topology(&self) {
        TopologyVisualizer::print_network_topology(&self.material_flows);
        if let Some(stream) = self.renderer_stream.as_ref() {
            let text = TopologyVisualizer::render_network_topology(&self.material_flows);
            let _ = stream.try_clone().and_then(|mut s| {
                let msg = serde_json::json!({"kind":"topology","text": text});
                let line = serde_json::to_string(&msg).unwrap();
                s.write_all(line.as_bytes())?; s.write_all(b"\n")?; s.flush()
            });
        }
    }

    pub fn print_materialflow_state(&mut self, current_time: EcTime) {
        // If an external renderer is connected, send snapshot instead of printing locally
        if let Some(stream) = self.renderer_stream.as_mut() {
            // Ensure topology is sent at least once upon first snapshot
            if !self.topology_sent {
                let text = TopologyVisualizer::render_network_topology(&self.material_flows);
                let topo_msg = serde_json::json!({"kind":"topology","text": text});
                if let Ok(line) = serde_json::to_string(&topo_msg) {
                    let _ = stream.write_all(line.as_bytes());
                    let _ = stream.write_all(b"\n");
                    let _ = stream.flush();
                }
                self.topology_sent = true;
            }
            let snapshot: StateSnapshot = StateVisualizer::collect_snapshot(&self.material_flows, current_time, self.current_seq);
            let msg = serde_json::json!({"kind":"snapshot","snapshot": snapshot});
            if let Ok(line) = serde_json::to_string(&msg) {
                let _ = stream.write_all(line.as_bytes());
                let _ = stream.write_all(b"\n");
                let _ = stream.flush();
                return;
            }
            // If sending failed, fall through to local printing as a fallback
        }
        if self.live_inplace {
            // Clear screen and move cursor home, then redraw topology + state
            print!("\x1B[2J\x1B[H");
            self.print_network_topology();
            StateVisualizer::print_materialflow_state(&self.material_flows, current_time, self.current_seq);
            let _ = std::io::Write::flush(&mut std::io::stdout());
            self.topology_printed = true;
            return;
        }

        if !self.topology_printed {
            self.print_network_topology();
            self.topology_printed = true;
        }
        StateVisualizer::print_materialflow_state(&self.material_flows, current_time, self.current_seq);
    }
}



impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EventController {
    pub event_queue: BinaryHeap<ScheduledEvent>,
    pub time: EcTime,
    pub endtime: Option<EcTime>,
    pub seq_counter: u64,
    pub seed: u64,
}

impl EventController {
    pub fn new(endtime: Option<EcTime>) -> EventController {
        EventController {
            event_queue: BinaryHeap::new(),
            time: EcTime::from(0),
            endtime: endtime,
            seq_counter: 0,
            seed: 0,
        }
    }

    pub fn add_event(&mut self, time: EcTime, event_template: EventTemplate) {
        let seq = self.seq_counter;
        self.seq_counter = self.seq_counter.wrapping_add(1);
        let mut tmpl = event_template;
        tmpl.seq = seq;
        self.event_queue.push(ScheduledEvent {
            time,
            template: tmpl,
            seq,
        });
    }

    pub fn init(&mut self, simulator: &mut Simulator) {
        // Initialize seeds and schedule initial events in a single pass.
        let mut seen_seeds: HashSet<u64> = HashSet::new();
        for (obj_id, object) in simulator.material_flows.iter() {
            let mut obj_mut = object.borrow_mut();
            // Compute a unique per-object seed derived from controller seed and object_id.
            let unique_seed = self.seed ^ (*obj_id as u64).wrapping_mul(0x9E3779B97F4A7C15);
            EventInterface::set_random_seed(&mut *obj_mut, unique_seed);
            debug_assert!(seen_seeds.insert(unique_seed), "Duplicate seed found for material_flow object_id {}", obj_id);
            let event_templates = obj_mut.init().unwrap();
            for event_template in event_templates {
                self.add_event(self.time + event_template.exec_time, event_template);
            }
        }
        // Assign unique seeds for drains based on their object_id
        for drain in simulator.drains.iter_mut() {
            let mut d = drain.borrow_mut();
            let unique_seed = self.seed ^ (*d.get_object_id() as u64).wrapping_mul(0x517CC1B727220A95);
            EventInterface::set_random_seed(&mut *d, unique_seed);
            debug_assert!(seen_seeds.insert(unique_seed), "Duplicate seed found for drain object_id {}", d.get_object_id());
            d.init().unwrap();
        }
    }

    pub fn run(&mut self, mut simulator: Simulator) -> Result<Simulator, RunTimeError> {
        while let Some(scheduled) = self.event_queue.pop() {
            let event_time = scheduled.time;
            let event_template = scheduled.template;
            // If an end time is set and the next event is at or beyond it, stop.
            if let Some(end) = self.endtime {
                if event_time.0 >= end.0 {
                    self.time = end;
                    break;
                }
            }
            // advance time and decrement remaining proc_time for processing objects
            let delta = if event_time.0 > self.time.0 {
                event_time - self.time
            } else {
                EcTime::from(0)
            };
            self.time = event_time;
            simulator.current_seq = event_template.seq;
            // decrement remaining proc time for stations and sources that are Processing
            for obj in simulator.material_flows.values() {
                let mut obj_mut = obj.borrow_mut();
                // try SingleStation
                if let Some(st) = <dyn MaterialFlow as MaterialFlow>::as_any(&mut *obj_mut)
                    .downcast_mut::<SingleStation>()
                {
                    if let ProcessState::Processing = st.state {
                        if st.remain_proc_time.0 > delta.0 {
                            st.remain_proc_time = st.remain_proc_time - delta;
                        } else {
                            st.remain_proc_time = EcTime::from(0);
                        }
                    }
                    continue;
                }
                // try Source
                if let Some(src) = <dyn MaterialFlow as MaterialFlow>::as_any(&mut *obj_mut)
                    .downcast_mut::<Source>()
                {
                    if let ProcessState::Processing = src.state {
                        if src.remain_proc_time.0 > delta.0 {
                            src.remain_proc_time = src.remain_proc_time - delta;
                        } else {
                            src.remain_proc_time = EcTime::from(0);
                        }
                    }
                }
            }
            for drain in &mut simulator.drains {
                drain.borrow_mut().update_current_time(self.time);
            }
            // Stop if endtime reached
            if self.time >= self.endtime.unwrap_or(EcTime(u128::MAX)) {
                break;
            }
            let obj_rc = simulator
                .material_flows
                .get(&event_template.object_id)
                .cloned();
            let eventtype_ref = &event_template.eventtype;
            if let Some(obj) = obj_rc {
                let mut queued_events: Vec<EventTemplate> = Vec::new();
                let mut obj_mut = obj.borrow_mut();
                log::info!(
                    "At {}: seq={} obj={} event={:#?}",
                    event_time.0,
                    event_template.seq,
                    event_template.object_id,
                    &event_template.eventtype
                );
                if let Ok(Some(evts)) = obj_mut.execute_event(&mut simulator, eventtype_ref) {
                    queued_events.extend(evts);
                }
                drop(obj_mut);
                for new_event in queued_events {
                    let scheduled_time = self.time + new_event.exec_time;
                    self.add_event(scheduled_time, new_event);
                }
                // Only print each step when a renderer is attached or live_inplace is enabled.
                // Otherwise defer printing to the end of the run for a single final snapshot.
                if simulator.renderer_stream.is_some() || simulator.live_inplace {
                    simulator.print_materialflow_state(self.time);
                }
            }
        }
        // If no external renderer and not live_inplace, emit one final snapshot at end.
        if simulator.renderer_stream.is_none() && !simulator.live_inplace {
            simulator.print_materialflow_state(self.time);
        }
        Ok(simulator)
    }
}

impl Default for EventController {
    fn default() -> Self {
        Self::new(None)
    }
}
