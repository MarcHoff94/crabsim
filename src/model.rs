//! A minimal description of a model, so that building one takes no knowledge of how a run stores
//! its objects. It doesn't validate connections yet: illegal ones still panic while running.

use crate::flows::{FailureProfile, MoveableUnit};
use crate::materialflow_objects::{Buffer, Drain, ModelObject, SingleStation, Source};
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;

/// The objects of a model and how they connect. Each `add_` returns the new object's id, which
/// is also the order Init visits them in. One description can build any number of runs, such as
/// replications that differ only in their seed.
#[derive(Clone, Default)]
pub struct ModelDescription {
    objects: Vec<ModelObject>,
}

impl ModelDescription {
    pub fn new() -> ModelDescription {
        ModelDescription::default()
    }

    /// A Source that creates an MU every `interval`.
    pub fn add_source(&mut self, interval: SimTime) -> u32 {
        self.add(ModelObject::Source(Source::new(self.next_id(), interval, Vec::new())))
    }

    pub fn add_station(&mut self, process_time: SimTime, failures: FailureProfile) -> u32 {
        self.add(ModelObject::Station(SingleStation::new(self.next_id(), process_time, failures, None, Vec::new())))
    }

    /// A Station whose default state holds an MU, which it works on from Init.
    pub fn add_station_holding(&mut self, process_time: SimTime, failures: FailureProfile, mu_id: u32) -> u32 {
        let id = self.next_id();
        let mu = MoveableUnit { id: mu_id, location_id: id, left_source: SimTime::ZERO };
        self.add(ModelObject::Station(SingleStation::new(id, process_time, failures, Some(mu), Vec::new())))
    }

    pub fn add_buffer(&mut self, capacity: usize) -> u32 {
        self.add(ModelObject::Buffer(Buffer::new(self.next_id(), capacity, Vec::new())))
    }

    pub fn add_drain(&mut self) -> u32 {
        self.add(ModelObject::Drain(Drain::new(self.next_id())))
    }

    /// Adds `to` as the next successor of `from`. Successors are tried in the order added.
    pub fn connect(&mut self, from: u32, to: u32) {
        assert!((to as usize) < self.objects.len(), "object {to} is not in the model");
        let successors = match self.objects.get_mut(from as usize) {
            Some(ModelObject::Source(source)) => &mut source.succ,
            Some(ModelObject::Station(station)) => &mut station.succ,
            Some(ModelObject::Buffer(buffer)) => &mut buffer.succ,
            Some(_) => panic!("object {from} can't have successors"),
            None => panic!("object {from} is not in the model"),
        };
        successors.push(to);
    }

    /// Builds a run of this model, in its default state.
    pub fn build(&self, seed: u64) -> SimulationRun {
        SimulationRun::new(self.objects.clone(), seed)
    }

    fn next_id(&self) -> u32 {
        self.objects.len() as u32
    }

    fn add(&mut self, object: ModelObject) -> u32 {
        let id = self.next_id();
        self.objects.push(object);
        id
    }
}
