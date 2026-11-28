//! Which MUs an object takes (ADR 0030): a limit an expectation puts on the classes or current
//! steps of the MUs entering an object, such as "Mill only works MUs whose current step is
//! Welding or Deburr". A check, never a constraint: the run counts the MUs that broke it and
//! keeps the first, and routing stays as it is.
//!
//! A run is told which objects to watch (`SimulationRun::watch`), and only runs that judge
//! expectations are: stage checks, experiments and the CLI's headless run, not the window's live
//! session. The check is folded into the one every move already makes for recording and user
//! code (`EventContext::moved`), so a run that watches nothing pays nothing.

use crate::flows::{DestinationId, MoveableUnit, MuKindId, NO_DESTINATION};
use crate::time::SimTime;
use std::sync::Arc;

/// What a watch checks of each MU entering its object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watched {
    /// The MU's class, by its kind: every kind a listed class stands for.
    Class(Vec<MuKindId>),
    /// The MU's current step, its next destination as it enters: every destination a listed class
    /// or instance stands for. An MU without a step breaks it, whether `inside` or not.
    Step(Vec<DestinationId>),
}

/// One limit on the MUs entering one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    pub object: u32,
    pub watched: Watched,
    /// Each MU must be one of them (`in`), or with `false` none of them (`not in`).
    pub inside: bool,
}

impl Watch {
    /// Whether the MU, whose current step is `step`, meets it.
    fn met_by(&self, mu: &MoveableUnit, step: DestinationId) -> bool {
        match &self.watched {
            Watched::Class(kinds) => kinds.contains(&mu.kind()) == self.inside,
            Watched::Step(destinations) => step != NO_DESTINATION && destinations.contains(&step) == self.inside,
        }
    }
}

/// The first MU that broke a watch, as it entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Break {
    pub time: SimTime,
    pub object: u32,
    pub mu: u32,
    pub kind: MuKindId,
    /// Its current step, or `NO_DESTINATION` without one.
    pub step: DestinationId,
}

/// What one watch has seen since statistics began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WatchState {
    /// How many MUs broke it.
    pub breaks: u64,
    pub first: Option<Break>,
}

/// A run's watches as its context keeps them: the list, where each object's are, and what each
/// has seen. A checkpoint clones it with the context.
#[derive(Debug, Clone)]
pub(crate) struct Watching {
    watches: Arc<[Watch]>,
    /// Each watch's place in the list, sorted by its object.
    by_object: Arc<[(u32, usize)]>,
    states: Vec<WatchState>,
}

impl Watching {
    pub(crate) fn new(watches: Vec<Watch>) -> Watching {
        let mut by_object: Vec<(u32, usize)> = watches.iter().enumerate().map(|(at, watch)| (watch.object, at)).collect();
        by_object.sort_unstable();
        let states = vec![WatchState::default(); watches.len()];
        Watching { watches: watches.into(), by_object: by_object.into(), states }
    }

    /// The same watches, having seen nothing yet.
    pub(crate) fn fresh(&self) -> Watching {
        Watching { watches: Arc::clone(&self.watches), by_object: Arc::clone(&self.by_object), states: vec![WatchState::default(); self.watches.len()] }
    }

    /// Checks the MU entering `object` at `time` against the object's watches.
    pub(crate) fn check(&mut self, mu: &MoveableUnit, object: u32, step: DestinationId, time: SimTime) {
        let from = self.by_object.partition_point(|&(watched, _)| watched < object);
        for &(watched, at) in &self.by_object[from..] {
            if watched != object {
                break;
            }
            if !self.watches[at].met_by(mu, step) {
                let state = &mut self.states[at];
                state.breaks += 1;
                state.first.get_or_insert(Break { time, object, mu: mu.id, kind: mu.kind(), step });
            }
        }
    }

    /// Forgets what the watches saw, as statistics start afresh after a warm-up.
    pub(crate) fn clear(&mut self) {
        self.states.iter_mut().for_each(|state| *state = WatchState::default());
    }

    pub(crate) fn states(&self) -> &[WatchState] {
        &self.states
    }
}
