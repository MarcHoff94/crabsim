//! A minimal description of a model, so that building one takes no knowledge of how a run stores
//! its objects. It doesn't validate connections yet: illegal ones still panic while running.
//!
//! It also holds what a view needs and a run doesn't: each object's name, position and
//! dimensions. A run never sees them, so they can't change a result and cost a headless run
//! nothing.

use crate::distribution::{DistributionRule, Ranking};
use crate::flows::{FailureProfile, MoveableUnit, MuKindId};
use crate::materialflow_objects::{Buffer, Drain, ModelObject, MultiStation, SingleStation, Source, StartMode};
use crate::materialflow_objects::ExitStrategy;
use crate::random_numbers::Distribution;
use crate::report::{Capacity, ObjectKind};
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;

/// Where an object stands, in metres: the centre of its footprint. `z` is its height above the
/// floor, 0 until there is a 3D view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// An object's real size in metres, and its rotation around the vertical axis in degrees. Visual
/// only: an MU moves instantly whatever the distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimensions {
    pub length: f64,
    pub width: f64,
    pub height: f64,
    pub rotation_degrees: f64,
}

impl Dimensions {
    /// The size an object has until it is given one, by type. These are placeholders for drawing,
    /// not measurements of anything.
    pub fn default_for(kind: ObjectKind) -> Dimensions {
        let (length, width, height) = match kind {
            ObjectKind::Source | ObjectKind::Drain => (1.0, 1.0, 1.0),
            ObjectKind::Station => (2.0, 1.5, 1.5),
            ObjectKind::Buffer => (2.0, 1.0, 1.0),
        };
        Dimensions { length, width, height, rotation_degrees: 0.0 }
    }
}

/// What an MU is made like (ADR 0021): its MU class's values, with its Source's changes. Only
/// geometry-aware objects read it, so for every object today it changes no result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuKind {
    /// Its footprint's length along the way it travels, in metres.
    pub length: f64,
    /// Its footprint's width across the way it travels, in metres.
    pub width: f64,
    /// Where its booking point is, as a share of its length from its front: 0 at the front, 1 at
    /// the rear (ADR 0023).
    pub booking_point: f64,
}

impl Default for MuKind {
    /// The built-in MU class's values: a tote of 0.6 by 0.4 m, booked at its centre.
    fn default() -> MuKind {
        MuKind { length: 0.6, width: 0.4, booking_point: 0.5 }
    }
}

/// What a view needs to know about one object, beyond what the run needs.
#[derive(Clone, Default)]
struct Layout {
    name: Option<String>,
    position: Option<Position>,
    dimensions: Option<Dimensions>,
}

/// The objects of a model and how they connect. Each `add_` returns the new object's id, which
/// is also the order Init visits them in. One description can build any number of runs, such as
/// replications that differ only in their seed.
#[derive(Clone)]
pub struct ModelDescription {
    objects: Vec<ModelObject>,
    /// One entry per object, by id.
    layout: Vec<Layout>,
    /// Each object's stream, by id: with the run's seed, it gives the object's random numbers.
    /// Its id unless set, so a model built in code draws as it always did.
    streams: Vec<u64>,
    /// Each object's distribution strategy, by id: round robin unless set.
    distributions: Vec<DistributionRule>,
    /// The kinds of MU in the model, each once, by `MuKindId`. The first is the built-in MU's,
    /// which starting MUs and every Source not given another have.
    mu_kinds: Vec<MuKind>,
}

impl Default for ModelDescription {
    fn default() -> ModelDescription {
        ModelDescription { objects: Vec::new(), layout: Vec::new(), streams: Vec::new(), distributions: Vec::new(), mu_kinds: vec![MuKind::default()] }
    }
}

impl ModelDescription {
    pub fn new() -> ModelDescription {
        ModelDescription::default()
    }

    /// The model's kinds of MU, by `MuKindId`: the built-in MU's first.
    pub fn mu_kinds(&self) -> &[MuKind] {
        &self.mu_kinds
    }

    /// Adds a kind of MU, or finds the same one already there, and returns its id. Sets the first,
    /// the built-in MU's, when the model changes the built-in MU class's values: a kind equal to
    /// it is it.
    ///
    /// # Panics
    /// Past 65,536 kinds, which an id can't count.
    pub fn add_mu_kind(&mut self, kind: MuKind) -> MuKindId {
        if let Some(found) = self.mu_kinds.iter().position(|known| *known == kind) {
            return found as MuKindId;
        }
        assert!(self.mu_kinds.len() <= usize::from(MuKindId::MAX), "a model can have at most 65,536 kinds of MU");
        self.mu_kinds.push(kind);
        (self.mu_kinds.len() - 1) as MuKindId
    }

    /// Changes the built-in MU's values, which starting MUs and every Source not given another
    /// kind have, as a model does by refining the built-in MU class.
    pub fn set_built_in_mu_kind(&mut self, kind: MuKind) {
        self.mu_kinds[0] = kind;
    }

    /// Makes every MU the Source `id` creates of this kind.
    ///
    /// # Panics
    /// If the model has no such kind, or `id` isn't a Source.
    pub fn set_mu_kind(&mut self, id: u32, kind: MuKindId) {
        assert!(usize::from(kind) < self.mu_kinds.len(), "the model has no MU kind {kind}");
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Source(source)) => source.mu_kind = kind,
            Some(_) => panic!("object {id} is not a Source"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// The kind of every MU the Source `id` creates.
    pub fn mu_kind(&self, id: u32) -> MuKindId {
        match self.object(id) {
            ModelObject::Source(source) => source.mu_kind,
            _ => panic!("object {id} is not a Source"),
        }
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
        let mu = MoveableUnit { id: mu_id, left_source: SimTime::ZERO, kind: 0 };
        self.add(ModelObject::Station(SingleStation::new(id, process_time, failures, Some(mu), Vec::new())))
    }

    /// Gives a Station a capacity, from 1, and says when it begins work on its MUs. Capacity 1
    /// keeps the cheaper `SingleStation`, whatever the start, since a batch of one starts on
    /// entry. Its process time, failures, successors and starting MUs carry over.
    pub fn set_station_capacity(&mut self, id: u32, capacity: u64, start: StartMode) {
        assert!(capacity >= 1, "a Station's capacity is at least 1");
        let object = self.objects.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model"));
        let failures = object.failure_profiles().to_vec();
        let (proc_time, succ, mus) = match object {
            ModelObject::Station(station) => (station.configured_proc_time, station.succ.clone(), station.starting_content.iter().map(|mu| mu.id).collect()),
            ModelObject::MultiStation(station) => (station.configured_proc_time, station.succ.clone(), station.starting_mus.clone()),
            _ => panic!("object {id} is not a Station"),
        };
        *object = if capacity == 1 {
            let mu = mus.first().map(|&mu_id| MoveableUnit { id: mu_id, left_source: SimTime::ZERO, kind: 0 });
            ModelObject::Station(SingleStation::new(id, proc_time, FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), mu, succ))
        } else {
            let mut station = MultiStation::new(id, proc_time, FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), capacity, start);
            station.succ = succ;
            station.set_starting_mus(mus);
            ModelObject::MultiStation(station)
        };
        object.set_failure_profiles(failures);
    }

    /// Builds every Station with capacity 1 as a `MultiStation`, for tests that check it behaves
    /// exactly like a `SingleStation`.
    #[cfg(test)]
    pub(crate) fn with_general_stations(&self) -> ModelDescription {
        let mut model = self.clone();
        for object in &mut model.objects {
            if let ModelObject::Station(station) = object {
                let mut general = MultiStation::new(station.base.object_id, station.configured_proc_time, FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), 1, StartMode::Individual);
                general.interruptions.set_failure_profiles(station.interruptions.failure_profiles().to_vec());
                general.succ = station.succ.clone();
                general.set_starting_mus(station.starting_content.iter().map(|mu| mu.id).collect());
                *object = ModelObject::MultiStation(general);
            }
        }
        model
    }

    /// A Buffer that holds up to `capacity` MUs, empty in its default state, passing them on
    /// first in, first out with no dwelling time.
    pub fn add_buffer(&mut self, capacity: Capacity) -> u32 {
        self.add(ModelObject::Buffer(Buffer::new(self.next_id(), capacity, Vec::new())))
    }

    /// Sets the MUs a Buffer or Station holds in its default state, by id, in the order they
    /// count as entered at Init. A Station of capacity 1 holds at most one.
    pub fn set_starting_mus(&mut self, id: u32, mus: &[u32]) {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Buffer(buffer)) => buffer.set_starting_mus(mus.to_vec()),
            Some(ModelObject::MultiStation(station)) => station.set_starting_mus(mus.to_vec()),
            Some(ModelObject::Station(station)) => {
                assert!(mus.len() <= 1, "a Station of capacity 1 holds one MU at most");
                station.starting_content = mus.first().map(|&mu_id| MoveableUnit { id: mu_id, left_source: SimTime::ZERO, kind: 0 });
                station.content = station.starting_content.clone();
            }
            Some(_) => panic!("object {id} can't hold starting MUs"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// Sets how long each MU stays in a Buffer before it may leave, counted from its entry.
    pub fn set_dwelling_time(&mut self, id: u32, dwelling_time: SimTime) {
        self.buffer_mut(id).set_dwelling_time(dwelling_time);
    }

    /// Sets the width of a Buffer's fill histogram's bars, in MUs, or `None` for automatic: one
    /// level per bar, doubling whenever more MUs come than the 64 bars reach.
    pub fn set_histogram_bar_width(&mut self, id: u32, width: Option<u64>) {
        self.buffer_mut(id).set_histogram_bar_width(width);
    }

    /// Sets which of the MUs that may leave a Buffer it passes on next.
    pub fn set_exit_strategy(&mut self, id: u32, exit_strategy: ExitStrategy) {
        self.buffer_mut(id).exit_strategy = exit_strategy;
    }

    /// Makes an interruption of a Buffer freeze the dwelling time of its MUs, rather than let it
    /// run on, which is the default.
    pub fn set_stop_freezes_dwelling(&mut self, id: u32, freezes: bool) {
        self.buffer_mut(id).stop_freezes_dwelling = freezes;
    }

    /// Gives an object these failure profiles, replacing any it had, which for a Station
    /// includes the one it was added with. Several can fail at once, each with its own stream.
    pub fn set_failure_profiles(&mut self, id: u32, failures: Vec<FailureProfile>) {
        self.objects.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model")).set_failure_profiles(failures);
    }

    /// Gives a Source, Buffer or Drain one failure profile, or none with `None`. A Station gets its
    /// failures when it is added, or through `set_failure_profiles`.
    pub fn set_failures(&mut self, id: u32, failures: Option<FailureProfile>) {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Source(source)) => source.set_failures(failures),
            Some(ModelObject::Buffer(buffer)) => buffer.set_failures(failures),
            Some(ModelObject::Drain(drain)) => drain.set_failures(failures),
            Some(_) => panic!("object {id} is a Station, whose failures come with it"),
            None => panic!("object {id} is not in the model"),
        }
    }

    fn buffer_mut(&mut self, id: u32) -> &mut Buffer {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Buffer(buffer)) => buffer,
            Some(_) => panic!("object {id} is not a Buffer"),
            None => panic!("object {id} is not in the model"),
        }
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
            Some(ModelObject::MultiStation(station)) => &mut station.succ,
            Some(ModelObject::Buffer(buffer)) => &mut buffer.succ,
            Some(_) => panic!("object {from} can't have successors"),
            None => panic!("object {from} is not in the model"),
        };
        successors.push(to);
    }

    /// Builds a run of this model, in its default state. Names, positions and dimensions stay
    /// behind: the run has no use for them.
    pub fn build(&self, seed: u64) -> SimulationRun {
        SimulationRun::new(self.objects.clone(), seed, self.streams.clone(), self.distributions.clone())
    }

    /// Sets how the object picks the successor for its MU (ADR 0016). Round robin unless set.
    pub fn set_distribution(&mut self, id: u32, rule: DistributionRule) {
        *self.distributions.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model")) = rule;
    }

    /// How the object picks the successor for its MU.
    pub fn distribution(&self, id: u32) -> &DistributionRule {
        &self.distributions[id as usize]
    }

    /// How a weighted object ranks its successors, as a run of this model starts: `None` for one
    /// that takes them round robin.
    pub fn ranking(&self, id: u32) -> Option<Ranking> {
        match self.distribution(id) {
            DistributionRule::RoundRobin => None,
            DistributionRule::Weighted(rule) => Some(crate::distribution::ranking(&self.objects, id, rule)),
        }
    }

    /// Sets the object's stream: with the run's seed, it gives the object's random numbers, so
    /// two models in which an object has the same stream draw the same numbers for it with the
    /// same seed, wherever it stands among the objects. Streams should be unique: a model file
    /// guarantees that.
    pub fn set_stream(&mut self, id: u32, stream: u64) {
        *self.streams.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model")) = stream;
    }

    /// The object's stream: its id unless set.
    pub fn stream(&self, id: u32) -> u64 {
        self.streams[id as usize]
    }

    /// How many objects the model has. Their ids run from 0 to one less than this.
    pub fn object_count(&self) -> u32 {
        self.objects.len() as u32
    }

    pub fn kind(&self, id: u32) -> ObjectKind {
        match self.object(id) {
            ModelObject::Source(_) => ObjectKind::Source,
            ModelObject::Station(_) | ModelObject::MultiStation(_) => ObjectKind::Station,
            ModelObject::Buffer(_) => ObjectKind::Buffer,
            ModelObject::Drain(_) => ObjectKind::Drain,
            #[cfg(test)]
            ModelObject::RecordsThenFails(_) => ObjectKind::Station,
        }
    }

    /// The longest mean time between failures or mean repair time among the model's Stations
    /// that fail, which a measuring period should be long against. `None` if none fail.
    pub fn longest_failure_time(&self) -> Option<SimTime> {
        self.objects
            .iter()
            .flat_map(|object| object.failure_profiles())
            .filter(|failures| failures.active)
            .map(|failures| failures.mtbf.max(SimTime::from_millis_f64(failures.dist_sampler_mttr.params.mean())))
            .max()
    }

    /// The object's successors, in the order they were connected, which is the order they are
    /// tried in. Empty for a Drain.
    pub fn successors(&self, id: u32) -> &[u32] {
        match self.object(id) {
            ModelObject::Source(source) => &source.succ,
            ModelObject::Station(station) => &station.succ,
            ModelObject::MultiStation(station) => &station.succ,
            ModelObject::Buffer(buffer) => &buffer.succ,
            ModelObject::Drain(_) => &[],
            #[cfg(test)]
            ModelObject::RecordsThenFails(_) => &[],
        }
    }

    pub fn set_name(&mut self, id: u32, name: impl Into<String>) {
        self.layout_mut(id).name = Some(name.into());
    }

    pub fn name(&self, id: u32) -> Option<&str> {
        self.layout(id).name.as_deref()
    }

    pub fn set_position(&mut self, id: u32, position: Position) {
        self.layout_mut(id).position = Some(position);
    }

    /// Where the object stands, if it has been placed.
    pub fn position(&self, id: u32) -> Option<Position> {
        self.layout(id).position
    }

    pub fn set_dimensions(&mut self, id: u32, dimensions: Dimensions) {
        self.layout_mut(id).dimensions = Some(dimensions);
    }

    /// The object's dimensions, or its type's default size if it hasn't been given any.
    pub fn dimensions(&self, id: u32) -> Dimensions {
        self.layout(id).dimensions.unwrap_or_else(|| Dimensions::default_for(self.kind(id)))
    }

    fn next_id(&self) -> u32 {
        self.objects.len() as u32
    }

    fn add(&mut self, object: ModelObject) -> u32 {
        let id = self.next_id();
        self.objects.push(object);
        self.layout.push(Layout::default());
        self.streams.push(u64::from(id));
        self.distributions.push(DistributionRule::RoundRobin);
        id
    }

    fn object(&self, id: u32) -> &ModelObject {
        self.objects.get(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model"))
    }

    fn layout(&self, id: u32) -> &Layout {
        self.layout.get(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model"))
    }

    fn layout_mut(&mut self, id: u32) -> &mut Layout {
        self.layout.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model"))
    }
}
