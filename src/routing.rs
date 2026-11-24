//! Where MUs go (spec issue 148): destinations (ADR 0025) and work plans (ADR 0026), as a run
//! holds them.
//!
//! A destination is an instance, or a class standing for any of its instances, so the run knows
//! it as the objects it stands for. Every MU kind has a work plan, a list of steps, each a
//! destination with an optional process time. An MU carries only its place in one table of every
//! plan's steps and its detour (ADR 0027), so it stays 16 bytes, and an object reads the rest
//! here. Models without plans or detours build an empty table, and every entry costs them one
//! comparison.

use crate::flows::{DestinationId, MoveableUnit, MuKindId, NO_DESTINATION, NO_STEP};
use crate::time::SimTime;

/// One step of a work plan, as a model gives it: where the MU goes next, and how long a Station
/// there works on it. Without a time, the Station's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanStep {
    pub at: DestinationId,
    pub process_time: Option<SimTime>,
}

/// What an MU entering an object found there, worked out as it entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// It has no plan left and no detour.
    Unrouted,
    /// The object is its plan's step, which it has now moved on from. A Station works on it for
    /// the step's time, if it has one, and otherwise its own.
    OnPlan(Option<SimTime>),
    /// The object is its detour, now cleared. A Station works on it for its own time.
    Detour,
    /// The object isn't where it goes next, which it keeps.
    OffPlan,
}

/// How many MUs a Station worked on for their plan's step or their detour, and how many it
/// didn't match (spec issue 148), so routing mistakes show. MUs with nowhere to go count in
/// neither. Kept by the Station itself rather than its statistics keeper, which every object has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlanCounts {
    pub on_plan: u64,
    pub off_plan: u64,
}

impl PlanCounts {
    #[inline]
    pub fn count(&mut self, arrival: Arrival) {
        match arrival {
            Arrival::Unrouted => {}
            Arrival::OnPlan(_) | Arrival::Detour => self.on_plan += 1,
            Arrival::OffPlan => self.off_plan += 1,
        }
    }
}

impl Arrival {
    /// How long a Station whose own process time is `own` works on the MU.
    #[inline]
    pub fn process_time(self, own: SimTime) -> SimTime {
        match self {
            Arrival::OnPlan(Some(time)) => time,
            _ => own,
        }
    }
}

/// One step in the run's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Step {
    at: DestinationId,
    /// The last step of its plan: after it, the MU has no destination left.
    last: bool,
    process_time: Option<SimTime>,
}

/// Every work plan and destination of a model, for a run. Shared by everything that reads it and
/// never changed during a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routing {
    /// Every plan's steps, each plan's together and in order.
    steps: Vec<Step>,
    /// Each MU kind's first step, by kind, or `NO_STEP` for a kind without a plan.
    first_steps: Vec<u16>,
    /// The objects each destination stands for, by destination, sorted.
    members: Vec<Box<[u32]>>,
}

impl Routing {
    /// From each MU kind's plan, by kind, and the objects each destination stands for, by
    /// destination.
    ///
    /// # Panics
    /// Past 65,535 steps or destinations, which an MU can't count: a model file is checked for it.
    pub fn new(plans: &[Vec<PlanStep>], destinations: &[Vec<u32>]) -> Routing {
        assert!(destinations.len() <= usize::from(NO_DESTINATION), "a model can have at most 65,535 destinations");
        let total: usize = plans.iter().map(Vec::len).sum();
        assert!(total <= usize::from(NO_STEP), "a model's work plans can have at most 65,535 steps between them");
        let mut steps = Vec::with_capacity(total);
        let mut first_steps = Vec::with_capacity(plans.len());
        for plan in plans {
            first_steps.push(if plan.is_empty() { NO_STEP } else { steps.len() as u16 });
            for (at, step) in plan.iter().enumerate() {
                steps.push(Step { at: step.at, last: at + 1 == plan.len(), process_time: step.process_time });
            }
        }
        let members = destinations
            .iter()
            .map(|objects| {
                let mut objects = objects.clone();
                objects.sort_unstable();
                objects.dedup();
                objects.into_boxed_slice()
            })
            .collect();
        Routing { steps, first_steps, members }
    }

    /// Whether any MU can have somewhere to go: a model without plans or destinations has none.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.members.is_empty()
    }

    /// How many destinations the model has.
    pub fn destinations(&self) -> usize {
        self.members.len()
    }

    /// The objects a destination stands for, sorted.
    pub fn members(&self, destination: DestinationId) -> &[u32] {
        self.members.get(usize::from(destination)).map_or(&[], |members| members)
    }

    /// Where an MU of `kind` starts its plan, or `NO_STEP` for a kind without one.
    #[inline]
    pub fn first_step(&self, kind: MuKindId) -> u16 {
        self.first_steps.get(usize::from(kind)).copied().unwrap_or(NO_STEP)
    }

    /// Where the MU goes next: its detour if it has one, otherwise its plan's step, and
    /// `NO_DESTINATION` once it has neither.
    #[inline]
    pub fn next_destination(&self, mu: &MoveableUnit) -> DestinationId {
        if mu.detour != NO_DESTINATION {
            return mu.detour;
        }
        self.steps.get(usize::from(mu.step)).map_or(NO_DESTINATION, |step| step.at)
    }

    /// Whether `object` is one of the objects `destination` stands for.
    pub fn names(&self, destination: DestinationId, object: u32) -> bool {
        self.members.get(usize::from(destination)).is_some_and(|members| members.binary_search(&object).is_ok())
    }

    /// What it means for the MU to enter `object`: reaching its next destination moves it on,
    /// clearing a detour or moving on to its plan's next step. Only its next destination counts,
    /// so a Station its plan names further on, or while it has a detour, is off plan.
    #[inline]
    pub fn arrive(&self, mu: &mut MoveableUnit, object: u32) -> Arrival {
        if !mu.is_routed() {
            return Arrival::Unrouted;
        }
        self.arrive_routed(mu, object)
    }

    #[cold]
    #[inline(never)]
    fn arrive_routed(&self, mu: &mut MoveableUnit, object: u32) -> Arrival {
        if mu.detour != NO_DESTINATION {
            if self.names(mu.detour, object) {
                mu.detour = NO_DESTINATION;
                return Arrival::Detour;
            }
            return Arrival::OffPlan;
        }
        let Some(step) = self.steps.get(usize::from(mu.step)) else {
            return Arrival::Unrouted;
        };
        if !self.names(step.at, object) {
            return Arrival::OffPlan;
        }
        mu.step = if step.last { NO_STEP } else { mu.step + 1 };
        Arrival::OnPlan(step.process_time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(at: DestinationId, secs: Option<u64>) -> PlanStep {
        PlanStep { at, process_time: secs.map(SimTime::from_secs) }
    }

    /// Kind 0 without a plan, kind 1 going to destination 0 (objects 2 and 3) for 40 s, then
    /// destination 1 (object 5).
    fn routing() -> Routing {
        Routing::new(&[vec![], vec![step(0, Some(40)), step(1, None)]], &[vec![3, 2], vec![5]])
    }

    fn mu_of(routing: &Routing, kind: MuKindId) -> MoveableUnit {
        let mut mu = MoveableUnit::new(1, SimTime::ZERO, kind);
        mu.step = routing.first_step(kind);
        mu
    }

    #[test]
    fn an_mu_follows_its_plan_and_then_has_nowhere_to_go() {
        let routing = routing();
        let mut mu = mu_of(&routing, 1);
        assert_eq!(routing.next_destination(&mu), 0);
        assert_eq!(routing.arrive(&mut mu, 4), Arrival::OffPlan, "object 4 isn't destination 0");
        assert_eq!(routing.arrive(&mut mu, 3), Arrival::OnPlan(Some(SimTime::from_secs(40))), "a class's other instance");
        assert_eq!(routing.next_destination(&mu), 1);
        assert_eq!(routing.arrive(&mut mu, 5), Arrival::OnPlan(None));
        assert_eq!(routing.next_destination(&mu), NO_DESTINATION);
        assert!(!mu.is_routed());
        assert_eq!(routing.arrive(&mut mu, 5), Arrival::Unrouted);
    }

    #[test]
    fn a_detour_comes_first_and_the_plan_carries_on_after_it() {
        let routing = routing();
        let mut mu = mu_of(&routing, 1);
        mu.detour = 1;
        assert_eq!(routing.next_destination(&mu), 1);
        assert_eq!(routing.arrive(&mut mu, 2), Arrival::OffPlan, "its step's Station, but the detour comes first");
        assert_eq!(routing.arrive(&mut mu, 5), Arrival::Detour);
        assert_eq!(routing.next_destination(&mu), 0, "back to its plan, still at its first step");
    }

    #[test]
    fn a_kind_without_a_plan_has_no_step() {
        let routing = routing();
        assert_eq!(routing.first_step(0), NO_STEP);
        assert_eq!(routing.first_step(9), NO_STEP, "a kind the table doesn't know");
        assert!(Routing::default().is_empty());
    }
}
