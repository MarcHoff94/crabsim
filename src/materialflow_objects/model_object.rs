//! How a run holds its objects: one `Vec` of this enum, indexed by object id, with successors
//! stored as ids. See ADR 0005.

use super::flows::{BaseObject, EventInterface, InterruptionState, MaterialFlow, MoveableUnit, StopCtrl};
#[cfg(test)]
use super::records_then_fails::RecordsThenFails;
use super::{Buffer, Drain, SingleStation, Source};
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::observation::ObjectState;
use crate::time::SimTime;

/// One material-flow object in a model.
#[derive(Clone)]
pub enum ModelObject {
    Source(Source),
    Station(SingleStation),
    Buffer(Buffer),
    Drain(Drain),
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
            ModelObject::Buffer($object) => $body,
            ModelObject::Drain($object) => $body,
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

    pub fn as_drain(&self) -> Option<&Drain> {
        match self {
            ModelObject::Drain(drain) => Some(drain),
            _ => None,
        }
    }

    pub fn base(&self) -> &BaseObject {
        on_the_object!(self, object => &object.base)
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
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
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

impl StopCtrl for ModelObject {
    fn set_stop(&mut self, failure_start: &SimTime, reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
        on_the_object!(self, object => object.set_stop(failure_start, reason))
    }
    fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
        on_the_object!(self, object => object.remove_stop())
    }
}

impl MaterialFlow for ModelObject {
    fn is_entrance_allowed(&self) -> bool {
        on_the_object!(self, object => object.is_entrance_allowed())
    }
    fn is_exit_allowed(&self) -> bool {
        on_the_object!(self, object => object.is_exit_allowed())
    }
    fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
        on_the_object!(self, object => object.process_mu_entrance(mu, context))
    }
    fn add_blocked(&mut self, id_blocked: u32) {
        on_the_object!(self, object => object.add_blocked(id_blocked))
    }
    fn remove_blocked(&mut self, id_blocked: u32) {
        on_the_object!(self, object => object.remove_blocked(id_blocked))
    }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        on_the_object!(self, object => object.notify_blocked_predecessors())
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
