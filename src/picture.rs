//! The picture: what the user currently knows about a simulation run at shown time, rebuilt from
//! a live session's outputs. It says what is where and since when, and nothing about how
//! that is drawn, so a 2D view, a 3D view and tests all read the same thing.
//!
//! It keeps what occurrences can keep current: each object's contents, process state, active
//! interruptions and remaining time. Statistics and waiting predecessors change without an
//! occurrence, so they come only from a full state, which a UI asks the session for.

use crate::flows::ProcessState;
use crate::live_session::SessionOutput;
use crate::report::{ActiveInterruption, ObjectKind, ObjectState, OccurrenceKind, RunState, Interruption};
use crate::time::SimTime;

/// What the user knows about a run at shown time. Feed it every output a session sends, in
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
    /// When each active interruption began, by `Interruption::index`.
    interrupted_since: [Option<SimTime>; 3],
    /// Which failure profiles are failing, so that the object stays failed until the last ends.
    failing: Vec<u8>,
    /// Each MU in work, in the order it came in, with when its work completes, or while
    /// stopped the work left on it, frozen. A Station with capacity has several, a Source the
    /// MU it is making.
    work: Vec<(u32, SimTime)>,
}

impl Picture {
    /// An empty picture, before a session has reported any state.
    pub fn new() -> Picture {
        Picture::default()
    }

    /// Takes in one output. A full state replaces the whole picture, as after Init, Reset
    /// or going back. Occurrences update it, but only once it has a state: Init's occurrences
    /// arrive before Init's state, which already includes them.
    pub fn observe(&mut self, output: &SessionOutput) {
        match output {
            SessionOutput::State(state) => self.take_state(state),
            SessionOutput::Occurrences(occurrences) if self.has_state => {
                for occurrence in occurrences {
                    self.apply(&occurrence.kind, occurrence.time);
                    self.shown = self.shown.max(occurrence.time);
                }
            }
            SessionOutput::ShownTime(time) => self.shown = *time,
            SessionOutput::Occurrences(_)
            | SessionOutput::Status(_)
            | SessionOutput::Error(_)
            | SessionOutput::StepBackTarget { .. } => {}
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
                object.work = vec![(mu_id, now + duration)];
            }
            OccurrenceKind::MuMoved { mu_id, from, to } => {
                let object = &mut objects[from as usize];
                object.contents.retain(|&id| id != mu_id);
                object.work.retain(|&(id, _)| id != mu_id);
                // A Station with capacity may still have MUs in work, or with none in work hold only
                // finished MUs that were refused, so it stays blocked: the run reports no change.
                object.process = if !object.work.is_empty() {
                    ProcessState::Processing
                } else if object.kind == ObjectKind::Station && object.process == ProcessState::Blocked && !object.contents.is_empty() {
                    ProcessState::Blocked
                } else {
                    ProcessState::Idle
                };
                object.hold();
                if let Some(object) = objects.get_mut(to as usize) {
                    object.contents.push(mu_id);
                    object.hold();
                }
            }
            OccurrenceKind::MuLeft { mu_id, at, .. } => objects[at as usize].contents.retain(|&id| id != mu_id),
            OccurrenceKind::WorkStarted { mu_id, at, duration } => {
                let object = &mut objects[at as usize];
                object.process = ProcessState::Processing;
                object.work.push((mu_id, now + duration));
            }
            OccurrenceKind::WorkFinished { mu_id, at } => objects[at as usize].work.retain(|&(id, _)| id != mu_id),
            OccurrenceKind::Blocked { object_id } => {
                // Work outranks blocking, so nothing is in work.
                let object = &mut objects[object_id as usize];
                object.process = ProcessState::Blocked;
                object.work.clear();
            }
            OccurrenceKind::Unblocked { object_id } => {
                // A Source unblocks by creating its next MU, which has already set what it is doing.
                let object = &mut objects[object_id as usize];
                if object.process == ProcessState::Blocked {
                    object.process = ProcessState::Idle;
                    object.hold();
                }
            }
            OccurrenceKind::InterruptionBegan { object_id, cause, profile } => {
                let object = &mut objects[object_id as usize];
                // Only the first interruption of a stop freezes the work.
                if !object.is_interrupted() {
                    for (_, time) in &mut object.work {
                        *time = time.saturating_sub(now);
                    }
                }
                if object.interrupted_since[cause.index()].is_none() {
                    object.interrupted_since[cause.index()] = Some(now);
                }
                if let Some(profile) = profile {
                    if !object.failing.contains(&profile) {
                        object.failing.push(profile);
                    }
                }
            }
            OccurrenceKind::InterruptionEnded { object_id, cause, profile } => {
                let object = &mut objects[object_id as usize];
                if let Some(profile) = profile {
                    object.failing.retain(|&failing| failing != profile);
                }
                // A failure ends the object's failed state only when no profile is still failing.
                if cause != Interruption::Failed || object.failing.is_empty() {
                    object.interrupted_since[cause.index()] = None;
                }
                // Only the last interruption of a stop to end resumes the work.
                if !object.is_interrupted() {
                    for (_, time) in &mut object.work {
                        *time = now + *time;
                    }
                }
            }
        }
    }
}

impl ObjectPicture {
    fn from_state(state: &ObjectState, now: SimTime) -> ObjectPicture {
        let stopped = state.interrupted.is_some();
        ObjectPicture {
            id: state.id,
            kind: state.kind,
            successors: state.successors.clone(),
            contents: state.contents.clone(),
            process: state.process.clone(),
            interrupted_since: Interruption::ALL.map(|cause| {
                state.interruptions.iter().find(|interruption| interruption.cause == cause).map(|i| i.since)
            }),
            failing: state.failure_profiles.iter().filter(|profile| profile.failing_since.is_some()).map(|profile| profile.index).collect(),
            work: if state.work.is_empty() {
                // A Source reports only its time left, on the MU it holds.
                state.remaining.map(|left| (state.contents.first().copied().unwrap_or(u32::MAX), left)).into_iter().collect::<Vec<_>>()
            } else {
                state.work.iter().map(|work| (work.mu_id, work.left)).collect::<Vec<_>>()
            }
            .into_iter()
            .map(|(mu_id, left)| (mu_id, if stopped { left } else { now + left }))
            .collect(),
        }
    }

    /// A Buffer that isn't blocked is holding while it has MUs, which a run reports as
    /// `Processing`, and otherwise empty. Nothing else changes for holding an MU.
    fn hold(&mut self) {
        if self.kind == ObjectKind::Buffer && self.process != ProcessState::Blocked {
            self.process = if self.contents.is_empty() { ProcessState::Idle } else { ProcessState::Processing };
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    /// The failure profiles failing now, by their place in the object's list.
    pub fn failing(&self) -> &[u8] {
        &self.failing
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

    pub fn is_interrupted(&self) -> bool {
        self.interrupted_since.iter().any(Option::is_some)
    }

    /// Every active interruption, most serious first, with when it began.
    pub fn interruptions(&self) -> Vec<ActiveInterruption> {
        Interruption::ALL
            .into_iter()
            .filter_map(|cause| Some(ActiveInterruption { cause, since: self.interrupted_since[cause.index()]? }))
            .collect()
    }

    /// Why the object is interrupted: its most serious active interruption, as a run's state reports.
    pub fn interrupted(&self) -> Option<Interruption> {
        Interruption::ALL.into_iter().find(|cause| self.interrupted_since[cause.index()].is_some())
    }

    /// Time left until the object's current work completes at `now`, frozen during a stop. For a
    /// Station with several MUs in work, the soonest.
    pub fn remaining(&self, now: SimTime) -> Option<SimTime> {
        let soonest = self.work.iter().map(|&(_, time)| time).min()?;
        Some(if self.is_interrupted() { soonest } else { soonest.saturating_sub(now) })
    }

    /// Each MU in work and the time left on it at `now`, in the order they came in.
    pub fn work(&self, now: SimTime) -> Vec<(u32, SimTime)> {
        let stopped = self.is_interrupted();
        self.work.iter().map(|&(mu_id, time)| (mu_id, if stopped { time } else { time.saturating_sub(now) })).collect()
    }
}
