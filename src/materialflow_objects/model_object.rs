//! How a run holds its objects: one `Vec` of this enum, indexed by object id, with successors
//! stored as ids. See ADR 0005.

use super::flows::{BaseObject, EventInterface, MaterialFlow, MoveableUnit};
#[cfg(test)]
use super::records_then_fails::RecordsThenFails;
use super::{Buffer, Conveyor, Drain, MultiStation, SingleStation, Source};
use super::flows::{MuKindId, SuccessorContainer};
use crate::flows::FailureProfile;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::ObjectState;
use crate::time::SimTime;

/// One material-flow object in a model.
#[derive(Clone)]
pub enum ModelObject {
    Source(Source),
    /// A Station of capacity 1.
    Station(SingleStation),
    /// A Station with a capacity above 1. Separate, so capacity 1 keeps its cheaper path.
    MultiStation(MultiStation),
    Buffer(Buffer),
    Drain(Drain),
    /// Boxed: it holds its MUs and the model's MU kinds, and every object in a run takes the size
    /// of the largest.
    Conveyor(Box<Conveyor>),
    /// No real object fails part way through an event, so only tests can exercise that.
    #[cfg(test)]
    RecordsThenFails(RecordsThenFails),
}

/// Runs the same code on whichever object this is.
macro_rules! on_the_object {
    ($self:expr, $object:ident => $body:expr) => {
        match $self {
            ModelObject::Source($object) => $body,
            ModelObject::Station($object) => $body,
            ModelObject::MultiStation($object) => $body,
            ModelObject::Buffer($object) => $body,
            ModelObject::Drain($object) => $body,
            ModelObject::Conveyor($object) => $body,
            #[cfg(test)]
            ModelObject::RecordsThenFails($object) => $body,
        }
    };
}

impl ModelObject {
    pub fn as_station(&self) -> Option<&SingleStation> {
        match self {
            ModelObject::Station(station) => Some(station),
            _ => None,
        }
    }

    /// Whether it is a Station, of any capacity.
    pub fn is_station(&self) -> bool {
        matches!(self, ModelObject::Station(_) | ModelObject::MultiStation(_))
    }

    /// A Station's process time now, whatever its capacity.
    pub fn process_time(&self) -> Option<SimTime> {
        match self {
            ModelObject::Station(station) => Some(station.proc_time),
            ModelObject::MultiStation(station) => Some(station.proc_time),
            _ => None,
        }
    }

    /// A Station's process time now and in its default state, to change.
    pub(crate) fn process_times_mut(&mut self) -> Option<(&mut SimTime, &mut SimTime)> {
        match self {
            ModelObject::Station(station) => Some((&mut station.proc_time, &mut station.configured_proc_time)),
            ModelObject::MultiStation(station) => Some((&mut station.proc_time, &mut station.configured_proc_time)),
            _ => None,
        }
    }

    /// The object's failure profiles, in order. Empty if it has none.
    pub fn failure_profiles(&self) -> &[FailureProfile] {
        match self {
            ModelObject::Station(station) => station.interruptions.failure_profiles(),
            ModelObject::MultiStation(station) => station.interruptions.failure_profiles(),
            ModelObject::Source(source) => source.interruptions.failure_profiles(),
            ModelObject::Buffer(buffer) => buffer.interruptions.failure_profiles(),
            ModelObject::Drain(drain) => drain.interruptions.failure_profiles(),
            ModelObject::Conveyor(conveyor) => conveyor.interruptions.failure_profiles(),
            #[cfg(test)]
            ModelObject::RecordsThenFails(_) => &[],
        }
    }

    /// Gives the object these failure profiles, replacing any it had.
    pub fn set_failure_profiles(&mut self, failures: Vec<FailureProfile>) {
        match self {
            ModelObject::Station(station) => station.interruptions.set_failure_profiles(failures),
            ModelObject::MultiStation(station) => station.interruptions.set_failure_profiles(failures),
            ModelObject::Source(source) => source.interruptions.set_failure_profiles(failures),
            ModelObject::Buffer(buffer) => buffer.interruptions.set_failure_profiles(failures),
            ModelObject::Drain(drain) => drain.interruptions.set_failure_profiles(failures),
            ModelObject::Conveyor(conveyor) => conveyor.interruptions.set_failure_profiles(failures),
            #[cfg(test)]
            ModelObject::RecordsThenFails(_) => {}
        }
    }

    pub fn as_drain(&self) -> Option<&Drain> {
        match self {
            ModelObject::Drain(drain) => Some(drain),
            _ => None,
        }
    }

    pub fn base(&self) -> &BaseObject {
        on_the_object!(self, object => &object.base)
    }

    /// Its successors' ids, in exit order. Empty for a Drain.
    pub fn successor_ids(&self) -> &[u32] {
        match self {
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

    /// Whether it asks which MU it is offered before taking one: an object whose geometry counts.
    #[inline]
    pub fn reads_mu_kind(&self) -> bool {
        matches!(self, ModelObject::Conveyor(_))
    }

    /// The kind of MU it would pass on at `now`, if it has one to pass on.
    pub fn offered_kind(&self, now: SimTime) -> Option<MuKindId> {
        match self {
            ModelObject::Source(source) => source.offered_kind(now),
            ModelObject::Station(station) => station.offered_kind(now),
            ModelObject::MultiStation(station) => station.offered_kind(now),
            ModelObject::Buffer(buffer) => buffer.offered_kind(now),
            ModelObject::Conveyor(conveyor) => conveyor.offered_kind(now),
            _ => None,
        }
    }

    /// How it picks a successor, if it has successors to pick from.
    pub fn distribution_mut(&mut self) -> Option<&mut crate::materialflow_objects::Distributing> {
        match self {
            ModelObject::Source(source) => Some(&mut source.distribution),
            ModelObject::Station(station) => Some(&mut station.distribution),
            ModelObject::MultiStation(station) => Some(&mut station.distribution),
            ModelObject::Buffer(buffer) => Some(&mut buffer.distribution),
            ModelObject::Conveyor(conveyor) => Some(&mut conveyor.distribution),
            _ => None,
        }
    }
}

impl EventInterface for ModelObject {
    fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
        on_the_object!(self, object => object.init(context))
    }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<(), RunTimeError> {
        on_the_object!(self, object => object.execute_event(context, eventtype, others))
    }
    fn reset(&mut self) {
        on_the_object!(self, object => object.reset())
    }
    fn report(&self, now: SimTime) -> ObjectState {
        on_the_object!(self, object => object.report(now))
    }
    fn clear_statistics(&mut self, now: SimTime) {
        on_the_object!(self, object => object.clear_statistics(now))
    }
    fn get_object_id(&self) -> &u32 {
        on_the_object!(self, object => object.get_object_id())
    }
    fn set_object_id(&mut self, id: u32) {
        on_the_object!(self, object => object.set_object_id(id))
    }
    fn set_random_seed(&mut self, seed: u64) {
        on_the_object!(self, object => object.set_random_seed(seed))
    }
}

impl MaterialFlow for ModelObject {
    fn is_entrance_allowed(&self) -> bool {
        on_the_object!(self, object => object.is_entrance_allowed())
    }
    fn is_exit_allowed(&self) -> bool {
        on_the_object!(self, object => object.is_exit_allowed())
    }
    fn accepts(&self, kind: MuKindId, now: SimTime) -> bool {
        on_the_object!(self, object => object.accepts(kind, now))
    }
    fn reserve(&mut self, mu_id: u32, context: &mut EventContext) {
        on_the_object!(self, object => object.reserve(mu_id, context))
    }
    fn wake_on_listing(&mut self) -> Option<EventTemplate> {
        on_the_object!(self, object => object.wake_on_listing())
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
        on_the_object!(self, object => object.process_mu_entrance(mu, context))
    }
    fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool) {
        on_the_object!(self, object => object.add_blocked(id_blocked, since, answering_retry))
    }
    fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>) {
        on_the_object!(self, object => object.relist_blocked(id_blocked, next))
    }
    fn notify_blocked_predecessors(&mut self, context: &mut EventContext) -> Result<(), MaterialflowError> {
        on_the_object!(self, object => object.notify_blocked_predecessors(context))
    }
}

/// Every object in the model except the one handling the current event, by id. The run splits
/// its `Vec` around that object, so an object can reach its successors without shared ownership
/// or `unsafe`. Asking for the object itself panics, which is what a connection from an object
/// to itself does.
pub struct OtherObjects<'a> {
    before: &'a mut [ModelObject],
    after: &'a mut [ModelObject],
}

impl<'a> OtherObjects<'a> {
    pub(crate) fn new(before: &'a mut [ModelObject], after: &'a mut [ModelObject]) -> OtherObjects<'a> {
        OtherObjects { before, after }
    }

    pub fn get(&self, id: u32) -> &ModelObject {
        let id = id as usize;
        let own = self.before.len();
        if id < own {
            &self.before[id]
        } else {
            &self.after[id.wrapping_sub(own + 1)]
        }
    }

    pub fn get_mut(&mut self, id: u32) -> &mut ModelObject {
        let id = id as usize;
        let own = self.before.len();
        if id < own {
            &mut self.before[id]
        } else {
            &mut self.after[id.wrapping_sub(own + 1)]
        }
    }
}
