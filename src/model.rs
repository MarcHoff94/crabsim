//! A minimal description of a model, so that building one takes no knowledge of how a run stores
//! its objects. It doesn't validate connections yet: illegal ones still panic while running.
//!
//! It also holds what a view needs and a run doesn't: each object's name, position and
//! dimensions. A run never sees them, so they can't change a result and cost a headless run
//! nothing.

use crate::distribution::{DistributionRule, Ranking};
use crate::flows::{DestinationId, FailureProfile, MoveableUnit, MuKindId, NO_DESTINATION};
use crate::routing::{PlanStep, Routing};
use crate::materialflow_objects::cross::Cross;
use crate::materialflow_objects::{Buffer, Conveyor, Drain, ModelObject, MultiStation, Side, SingleStation, Source, StartMode};
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
            // A conveyor's length and width are its own, which `add_conveyor` gives it.
            ObjectKind::Conveyor => (5.0, 0.6, 0.5),
            // A conveyor's default width each way, so default conveyors meet every side.
            ObjectKind::CrossConveyor => (0.6, 0.6, 0.5),
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
    /// Each MU kind's work plan, by `MuKindId` (ADR 0026): empty for a kind without one. Two kinds
    /// alike but for their plans are two kinds.
    plans: Vec<Vec<PlanStep>>,
    /// The objects each destination stands for, by `DestinationId` (ADR 0025).
    destinations: Vec<Vec<u32>>,
    /// User code (ADR 0029), which every run gets a copy of, and which objects have which hooks.
    hooks: Option<Box<dyn crate::hooks::Hooks>>,
    hook_table: crate::hooks::HookTable,
}

impl Default for ModelDescription {
    fn default() -> ModelDescription {
        ModelDescription {
            objects: Vec::new(),
            layout: Vec::new(),
            streams: Vec::new(),
            distributions: Vec::new(),
            mu_kinds: vec![MuKind::default()],
            plans: vec![Vec::new()],
            destinations: Vec::new(),
            hooks: None,
            hook_table: crate::hooks::HookTable::default(),
        }
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

    /// Adds a kind of MU without a work plan, or finds the same one already there, and returns
    /// its id. A kind equal to the built-in MU's is it.
    ///
    /// # Panics
    /// Past 65,536 kinds, which an id can't count.
    pub fn add_mu_kind(&mut self, kind: MuKind) -> MuKindId {
        self.add_mu_kind_with_plan(kind, Vec::new())
    }

    /// Adds a kind of MU whose MUs follow this work plan (ADR 0026), or finds the same one already
    /// there, and returns its id.
    ///
    /// # Panics
    /// Past 65,536 kinds, or if a step names a destination the model doesn't have.
    pub fn add_mu_kind_with_plan(&mut self, kind: MuKind, plan: Vec<PlanStep>) -> MuKindId {
        assert!(plan.iter().all(|step| usize::from(step.at) < self.destinations.len()), "a work plan step names a destination the model doesn't have");
        if let Some(found) = self.mu_kinds.iter().zip(&self.plans).position(|(known, known_plan)| *known == kind && *known_plan == plan) {
            return found as MuKindId;
        }
        assert!(self.mu_kinds.len() <= usize::from(MuKindId::MAX), "a model can have at most 65,536 kinds of MU");
        self.mu_kinds.push(kind);
        self.plans.push(plan);
        (self.mu_kinds.len() - 1) as MuKindId
    }

    /// Changes the built-in MU's values, which starting MUs and every Source not given another
    /// kind have, as a model does by refining the built-in MU class.
    pub fn set_built_in_mu_kind(&mut self, kind: MuKind) {
        self.mu_kinds[0] = kind;
    }

    /// Gives the built-in MU a work plan, which every Source not given another kind's MUs follow.
    /// Starting MUs have no plan.
    pub fn set_built_in_work_plan(&mut self, plan: Vec<PlanStep>) {
        assert!(plan.iter().all(|step| usize::from(step.at) < self.destinations.len()), "a work plan step names a destination the model doesn't have");
        self.plans[0] = plan;
    }

    /// The work plan of an MU kind: empty for one without.
    pub fn work_plan(&self, kind: MuKindId) -> &[PlanStep] {
        &self.plans[usize::from(kind)]
    }

    /// Adds a destination standing for these objects, by id (ADR 0025): one instance, or every
    /// instance of a class. Returns its id, the same for the same objects.
    ///
    /// # Panics
    /// Past 65,535 destinations, or with an object not in the model.
    pub fn add_destination(&mut self, objects: &[u32]) -> DestinationId {
        assert!(objects.iter().all(|&id| (id as usize) < self.objects.len()), "a destination stands for objects in the model");
        let mut objects = objects.to_vec();
        objects.sort_unstable();
        objects.dedup();
        if let Some(found) = self.destinations.iter().position(|known| *known == objects) {
            return found as DestinationId;
        }
        assert!(self.destinations.len() < usize::from(NO_DESTINATION), "a model can have at most 65,535 destinations");
        self.destinations.push(objects);
        (self.destinations.len() - 1) as DestinationId
    }

    /// The objects each destination stands for, by id.
    pub fn destinations(&self) -> &[Vec<u32>] {
        &self.destinations
    }

    /// Gives a Station a detour (ADR 0026): every MU it works on goes to `destination` first,
    /// before carrying on with its plan. `NO_DESTINATION` takes it away.
    pub fn set_detour(&mut self, id: u32, destination: DestinationId) {
        assert!(destination == NO_DESTINATION || usize::from(destination) < self.destinations.len(), "the model has no destination {destination}");
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Station(station)) => station.detour = destination,
            Some(ModelObject::MultiStation(station)) => station.detour = destination,
            Some(_) => panic!("object {id} is not a Station"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// A Station's detour, or `NO_DESTINATION`.
    pub fn detour(&self, id: u32) -> DestinationId {
        match self.object(id) {
            ModelObject::Station(station) => station.detour,
            ModelObject::MultiStation(station) => station.detour,
            _ => NO_DESTINATION,
        }
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
        let mu = MoveableUnit::starting(mu_id);
        self.add(ModelObject::Station(SingleStation::new(id, process_time, failures, Some(mu), Vec::new())))
    }

    /// Gives a Station a capacity, from 1, and says when it begins work on its MUs. Capacity 1
    /// keeps the cheaper `SingleStation`, whatever the start, since a batch of one starts on
    /// entry. Its process time, failures, successors and starting MUs carry over.
    pub fn set_station_capacity(&mut self, id: u32, capacity: u64, start: StartMode) {
        assert!(capacity >= 1, "a Station's capacity is at least 1");
        let object = self.objects.get_mut(id as usize).unwrap_or_else(|| panic!("object {id} is not in the model"));
        let failures = object.failure_profiles().to_vec();
        let (proc_time, succ, mus, detour) = match object {
            ModelObject::Station(station) => (station.configured_proc_time, station.succ.clone(), station.starting_content.iter().map(|mu| mu.id).collect(), station.detour),
            ModelObject::MultiStation(station) => (station.configured_proc_time, station.succ.clone(), station.starting_mus.clone(), station.detour),
            _ => panic!("object {id} is not a Station"),
        };
        *object = if capacity == 1 {
            let mu = mus.first().map(|&mu_id| MoveableUnit::starting(mu_id));
            ModelObject::Station(SingleStation::new(id, proc_time, FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), mu, succ))
        } else {
            let mut station = MultiStation::new(id, proc_time, FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0), capacity, start);
            station.succ = succ;
            station.set_starting_mus(mus);
            ModelObject::MultiStation(station)
        };
        object.set_failure_profiles(failures);
        self.set_detour(id, detour);
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
                general.detour = station.detour;
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
                station.starting_content = mus.first().map(|&mu_id| MoveableUnit::starting(mu_id));
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
            Some(ModelObject::Conveyor(conveyor)) => conveyor.set_failures(failures),
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

    /// A straight conveyor of `length` and `width` in metres, moving its MUs at `speed` metres per
    /// second (spec issue 133), accumulating at its end, with no gap between MUs. Its length and
    /// width are its dimensions too, whatever is set later.
    pub fn add_conveyor(&mut self, length: f64, width: f64, speed: f64) -> u32 {
        let id = self.add(ModelObject::Conveyor(Box::new(Conveyor::new(self.next_id(), length, width, speed))));
        self.layout_mut(id).dimensions = Some(Dimensions { length, width, height: 0.5, rotation_degrees: 0.0 });
        id
    }

    /// Sets the least space between two MUs on a conveyor, in metres.
    pub fn set_conveyor_gap(&mut self, id: u32, gap: f64) {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Conveyor(conveyor)) => conveyor.gap = gap,
            Some(_) => panic!("object {id} is not a conveyor"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// Sets whether a conveyor accumulates: MUs behind a stopped one close up and stop while the
    /// rest still move, which is the default, or every MU stops as a whole when the first does.
    pub fn set_conveyor_accumulating(&mut self, id: u32, accumulating: bool) {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Conveyor(conveyor)) => conveyor.accumulating = accumulating,
            Some(_) => panic!("object {id} is not a conveyor"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// A cross conveyor (spec issue 148): `length` along its own x axis, from its back to its
    /// front, and `width` across, from its right to its left, in metres, moving its MUs at `speed`
    /// metres per second, one at a time, with a switch time of 2 s and no release time. Its
    /// connections name its sides, through `connect_at`.
    pub fn add_cross_conveyor(&mut self, length: f64, width: f64, speed: f64) -> u32 {
        let id = self.add(ModelObject::Conveyor(Box::new(Conveyor::new_cross(self.next_id(), length, width, speed))));
        self.layout_mut(id).dimensions = Some(Dimensions { length, width, height: 0.5, rotation_degrees: 0.0 });
        id
    }

    fn cross_mut(&mut self, id: u32) -> &mut Cross {
        match self.objects.get_mut(id as usize) {
            Some(ModelObject::Conveyor(conveyor)) => conveyor.cross_mut().unwrap_or_else(|| panic!("object {id} is not a cross conveyor")),
            Some(_) => panic!("object {id} is not a cross conveyor"),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// How long an MU turning at a cross conveyor stands at its centre.
    pub fn set_switch_time(&mut self, id: u32, switch_time: SimTime) {
        self.cross_mut(id).switch_time = switch_time;
    }

    /// How long a cross conveyor takes no MU after one that turned has left.
    pub fn set_release_time(&mut self, id: u32, release_time: SimTime) {
        self.cross_mut(id).release_time = release_time;
    }

    /// The destinations a cross conveyor's exit side `side` takes, replacing any it had: an MU
    /// whose next destination one of them stands for turns towards it, unless an exit straight
    /// across takes it too.
    pub fn set_exit_destinations(&mut self, id: u32, side: Side, destinations: &[DestinationId]) {
        let cross = self.cross_mut(id);
        cross.lists.retain(|(listed, _)| *listed != side);
        cross.lists.push((side, destinations.to_vec()));
    }

    pub fn add_drain(&mut self) -> u32 {
        self.add(ModelObject::Drain(Drain::new(self.next_id())))
    }

    /// Adds `to` as the next successor of `from`. Successors are tried in the order added.
    ///
    /// # Panics
    /// If either is a cross conveyor, whose connections name its sides: see `connect_at`.
    pub fn connect(&mut self, from: u32, to: u32) {
        self.connect_at(from, None, to, None);
    }

    /// Connects `from` to `to` as `connect` does, at `from_side` of `from` and `to_side` of `to`,
    /// which are given for an end that is a cross conveyor and only then. A side is an entry side
    /// or an exit side, never both, and has one connection.
    pub fn connect_at(&mut self, from: u32, from_side: Option<Side>, to: u32, to_side: Option<Side>) {
        assert!((to as usize) < self.objects.len(), "object {to} is not in the model");
        let is_cross = |objects: &[ModelObject], id: u32| matches!(objects.get(id as usize), Some(ModelObject::Conveyor(conveyor)) if conveyor.cross().is_some());
        assert_eq!(is_cross(&self.objects, from), from_side.is_some(), "a side is given for the end of a connection that is a cross conveyor, and only for it");
        assert_eq!(is_cross(&self.objects, to), to_side.is_some(), "a side is given for the end of a connection that is a cross conveyor, and only for it");
        let taken = |cross: &Cross, side: Side| cross.exit_sides.contains(&side) || cross.entries.iter().any(|&(_, entry)| entry == side);
        if let Some(side) = from_side {
            let cross = self.cross_mut(from);
            assert!(!taken(cross, side), "the {} side of cross conveyor {from} has a connection already", side.name());
            cross.exit_sides.push(side);
        }
        if let Some(side) = to_side {
            let cross = self.cross_mut(to);
            assert!(!taken(cross, side), "the {} side of cross conveyor {to} has a connection already", side.name());
            cross.entries.push((from, side));
        }
        let successors = match self.objects.get_mut(from as usize) {
            Some(ModelObject::Source(source)) => &mut source.succ,
            Some(ModelObject::Station(station)) => &mut station.succ,
            Some(ModelObject::MultiStation(station)) => &mut station.succ,
            Some(ModelObject::Buffer(buffer)) => &mut buffer.succ,
            Some(ModelObject::Conveyor(conveyor)) => &mut conveyor.succ,
            Some(_) => panic!("object {from} can't have successors"),
            None => panic!("object {from} is not in the model"),
        };
        successors.push(to);
    }

    /// Builds a run of this model, in its default state. Names, positions and dimensions stay
    /// behind: the run has no use for them.
    pub fn build(&self, seed: u64) -> SimulationRun {
        self.build_with(seed, false)
    }

    /// A run with no randomness, for verification (ADR 0030): no object fails, and every time
    /// drawn from a distribution would be its mean, which today means only the failures, since
    /// process times and intervals are fixed. User code's random numbers stay as they are: they
    /// come from the seed, so the run is still the same every time for a seed.
    pub fn build_deterministic(&self, seed: u64) -> SimulationRun {
        self.build_with(seed, true)
    }

    fn build_with(&self, seed: u64, deterministic: bool) -> SimulationRun {
        let mut objects = self.objects.clone();
        if deterministic {
            for object in &mut objects {
                object.set_failure_profiles(Vec::new());
            }
        }
        let routing = if self.destinations.is_empty() && self.plans.iter().all(Vec::is_empty) { Routing::default() } else { Routing::new(&self.plans, &self.destinations) };
        // Each conveyor reads MUs' sizes from its own copy of the kinds, so a run shares nothing,
        // and each cross conveyor works out which of its exits take which destination.
        for object in &mut objects {
            if let ModelObject::Conveyor(conveyor) = object {
                conveyor.kinds = self.mu_kinds.clone();
                if let Some(cross) = conveyor.cross_mut() {
                    cross.build_takes(&routing);
                }
            }
        }
        let mut run = SimulationRun::new(objects, seed, self.streams.clone(), self.distributions.clone(), std::sync::Arc::new(routing));
        if !self.hook_table.is_empty() {
            let hooks = self.hooks.clone().expect("a model with hooks has user code: set it with set_hooks");
            run.attach_hooks(self.hook_table.clone(), hooks);
        }
        run
    }

    /// Gives the model user code (ADR 0029), which the hooks set on objects and on the model call.
    /// Every run built gets its own copy.
    pub fn set_hooks(&mut self, hooks: Box<dyn crate::hooks::Hooks>) {
        self.hooks = Some(hooks);
    }

    /// Sets the object's hooks, replacing any it had. A Drain has only a hook on entry, and a cross
    /// conveyor's distribution runs as it lets an MU in, a moment that can't move.
    pub fn set_object_hooks(&mut self, id: u32, hooks: crate::hooks::ObjectHooks) {
        if hooks.distribution_on_entry.is_some() && hooks.distribution.is_none() {
            panic!("a distribution's moment moves only with a distribution hook");
        }
        match self.objects.get(id as usize) {
            Some(ModelObject::Drain(_)) if hooks.on_exit.is_some() || hooks.distribution.is_some() => panic!("a Drain has only a hook on entry"),
            Some(ModelObject::Conveyor(conveyor)) if conveyor.cross().is_some() && hooks.distribution_on_entry.is_some() => {
                panic!("a cross conveyor's distribution runs as it lets an MU in, a moment that can't move")
            }
            Some(_) => self.hook_table.set_object(id, hooks),
            None => panic!("object {id} is not in the model"),
        }
    }

    /// Sets the model's hook at a moment of a run's life.
    pub fn set_lifecycle_hook(&mut self, moment: crate::hooks::Lifecycle, function: Option<crate::hooks::HookFunction>) {
        self.hook_table.set_lifecycle(moment, function);
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
        self.distribution(id).weighted().map(|rule| crate::distribution::ranking(&self.objects, id, rule))
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
            ModelObject::Conveyor(conveyor) if conveyor.cross().is_some() => ObjectKind::CrossConveyor,
            ModelObject::Conveyor(_) => ObjectKind::Conveyor,
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
            ModelObject::Conveyor(conveyor) => &conveyor.succ,
            ModelObject::Drain(_) => &[],
            #[cfg(test)]
            ModelObject::RecordsThenFails(_) => &[],
        }
    }

    /// The sides a connection from `from` to `to` leaves and arrives by, each given only where
    /// that end is a cross conveyor, for views to draw it from and to the side's middle.
    pub fn connection_sides(&self, from: u32, to: u32) -> (Option<Side>, Option<Side>) {
        let cross = |id: u32| match self.objects.get(id as usize) {
            Some(ModelObject::Conveyor(conveyor)) => conveyor.cross(),
            _ => None,
        };
        let leaves = cross(from).and_then(|cross| self.successors(from).iter().position(|&successor| successor == to).and_then(|exit| cross.exit_sides.get(exit).copied()));
        let arrives = cross(to).and_then(|cross| cross.entry_side(from));
        (leaves, arrives)
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
