//! The picture: what the user currently knows about a simulation run at shown time, rebuilt from
//! a live session's observations. It says what is where and since when, and nothing about how
//! that is drawn, so a 2D view, a 3D view and tests all read the same thing.
//!
//! It keeps what occurrences can keep current: each object's contents, process state, active
//! interruptions and remaining time. Statistics and waiting predecessors change without an
//! occurrence, so they come only from a full state, which a UI asks the session for.

use crate::flows::ProcessState;
use crate::live_session::Observation;
use crate::observation::{Interruption, ObjectKind, ObjectState, OccurrenceKind, RunState, StopCause};
use crate::time::SimTime;

/// What the user knows about a run at shown time. Feed it every observation a session sends, in
/// order, through `observe`.
#[derive(Debug, Clone, Default)]
pub struct Picture {
    shown: SimTime,
    has_state: bool,
    objects: Vec<ObjectPicture>,
}

/// What the picture knows about one material-flow object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectPicture {
    id: u32,
    kind: ObjectKind,
    successors: Vec<u32>,
    contents: Vec<u32>,
    process: ProcessState,
    /// When each active interruption began, by `StopCause::index`.
    interrupted_since: [Option<SimTime>; 3],
    /// When the work in progress completes, while not stopped.
    completion: Option<SimTime>,
    /// The work left, frozen while stopped.
    frozen: Option<SimTime>,
}

impl Picture {
    /// An empty picture, before a session has reported any state.
    pub fn new() -> Picture {
        Picture::default()
    }

    /// Takes in one observation. A full state replaces the whole picture, as after Init, Reset
    /// or going back. Occurrences update it, but only once it has a state: Init's occurrences
    /// arrive before Init's state, which already includes them.
    pub fn observe(&mut self, observation: &Observation) {
        match observation {
            Observation::State(state) => self.take_state(state),
            Observation::Occurrences(occurrences) if self.has_state => {
                for occurrence in occurrences {
                    self.apply(&occurrence.kind, occurrence.time);
                    self.shown = self.shown.max(occurrence.time);
                }
            }
            Observation::ShownTime(time) => self.shown = *time,
            Observation::Occurrences(_)
            | Observation::Status(_)
            | Observation::Error(_)
            | Observation::StepBackTarget { .. } => {}
        }
    }

    /// The simulation time the picture shows.
    pub fn shown(&self) -> SimTime {
        self.shown
    }

    /// Whether a full state has arrived. Until then the picture has no objects.
    pub fn has_state(&self) -> bool {
        self.has_state
    }

    /// Every object, indexed by id.
    pub fn objects(&self) -> &[ObjectPicture] {
        &self.objects
    }

    pub fn object(&self, id: u32) -> Option<&ObjectPicture> {
        self.objects.get(id as usize)
    }

    fn take_state(&mut self, state: &RunState) {
        self.shown = state.time;
        self.has_state = true;
        self.objects = state.objects.iter().map(|object| ObjectPicture::from_state(object, state.time)).collect();
    }

    fn apply(&mut self, kind: &OccurrenceKind, now: SimTime) {
        let objects = &mut self.objects;
        match *kind {
            OccurrenceKind::RunInitialised => {}
            OccurrenceKind::MuCreated { mu_id, at, duration } => {
                let object = &mut objects[at as usize];
                object.contents = vec![mu_id];
                object.process = ProcessState::Processing;
                object.completion = Some(now + duration);
            }
            OccurrenceKind::MuMoved { mu_id, from, to } => {
                let object = &mut objects[from as usize];
                object.contents.retain(|&id| id != mu_id);
                object.process = ProcessState::Idle;
                object.completion = None;
                if let Some(object) = objects.get_mut(to as usize) {
                    object.contents.push(mu_id);
                }
            }
            OccurrenceKind::MuLeft { mu_id, at, .. } => objects[at as usize].contents.retain(|&id| id != mu_id),
            OccurrenceKind::WorkStarted { at, duration, .. } => {
                let object = &mut objects[at as usize];
                object.process = ProcessState::Processing;
                object.completion = Some(now + duration);
            }
            OccurrenceKind::WorkFinished { at, .. } => objects[at as usize].completion = None,
            OccurrenceKind::Blocked { object_id } => {
                let object = &mut objects[object_id as usize];
                object.process = ProcessState::Blocked;
                object.completion = None;
            }
            OccurrenceKind::Unblocked { object_id } => {
                // A Source unblocks by creating its next MU, which has already set what it is doing.
                let object = &mut objects[object_id as usize];
                if object.process == ProcessState::Blocked {
                    object.process = ProcessState::Idle;
                }
            }
            OccurrenceKind::InterruptionBegan { object_id, cause } => {
                let object = &mut objects[object_id as usize];
                // Only the first interruption of a stop freezes the work.
                if !object.is_stopped() {
                    object.frozen = object.completion.take().map(|completion| completion.saturating_sub(now));
                }
                object.interrupted_since[cause.index()] = Some(now);
            }
            OccurrenceKind::InterruptionEnded { object_id, cause } => {
                let object = &mut objects[object_id as usize];
                object.interrupted_since[cause.index()] = None;
                // Only the last interruption of a stop to end resumes the work.
                if !object.is_stopped() {
                    object.completion = object.frozen.take().map(|left| now + left);
                }
            }
        }
    }
}

impl ObjectPicture {
    fn from_state(state: &ObjectState, now: SimTime) -> ObjectPicture {
        let stopped = state.stop.is_some();
        ObjectPicture {
            id: state.id,
            kind: state.kind,
            successors: state.successors.clone(),
            contents: state.contents.clone(),
            process: state.process.clone(),
            interrupted_since: StopCause::ALL.map(|cause| {
                state.interruptions.iter().find(|interruption| interruption.cause == cause).map(|i| i.since)
            }),
            completion: state.remaining.filter(|_| !stopped).map(|left| now + left),
            frozen: state.remaining.filter(|_| stopped),
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub fn successors(&self) -> &[u32] {
        &self.successors
    }

    /// The MUs the object holds.
    pub fn contents(&self) -> &[u32] {
        &self.contents
    }

    pub fn process(&self) -> ProcessState {
        self.process.clone()
    }

    pub fn is_stopped(&self) -> bool {
        self.interrupted_since.iter().any(Option::is_some)
    }

    /// Every active interruption, most serious first, with when it began.
    pub fn interruptions(&self) -> Vec<Interruption> {
        StopCause::ALL
            .into_iter()
            .filter_map(|cause| Some(Interruption { cause, since: self.interrupted_since[cause.index()]? }))
            .collect()
    }

    /// Why the object is stopped: its most serious active interruption, as a run's state reports.
    pub fn stop(&self) -> Option<StopCause> {
        StopCause::ALL.into_iter().find(|cause| self.interrupted_since[cause.index()].is_some())
    }

    /// Time left until the object's current work completes at `now`, frozen during a stop.
    pub fn remaining(&self, now: SimTime) -> Option<SimTime> {
        self.frozen.or_else(|| self.completion.map(|completion| completion.saturating_sub(now)))
    }
}
