//! The interventions the user made during a run, as marks on the time display. The session keeps
//! its own log, which these mirror: going back drops the interventions after the time gone back
//! to, and Reset drops them all.

use crabsim::{Occurrence, OccurrenceKind, SimTime, StopCause};

/// Something the user does to an object during a run, at shown time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Intervention {
    Pause,
    Resume,
    ForceFailure,
    EndForcedFailure,
    SetProcessTime(SimTime),
}

impl Intervention {
    /// The occurrence that says when it took effect, if it records one.
    fn shows_as(self) -> Option<(bool, StopCause)> {
        match self {
            Intervention::Pause => Some((true, StopCause::Pause)),
            Intervention::Resume => Some((false, StopCause::Pause)),
            Intervention::ForceFailure => Some((true, StopCause::ForcedFailure)),
            Intervention::EndForcedFailure => Some((false, StopCause::ForcedFailure)),
            Intervention::SetProcessTime(_) => None,
        }
    }
}

/// One intervention on the time display.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// When it took effect: shown time when it was sent, until its occurrence says exactly.
    pub at: SimTime,
    pub object: u32,
    pub intervention: Intervention,
    /// Whether an occurrence has confirmed `at`.
    settled: bool,
}

/// How many interventions stepping back would drop. Stepping back goes to the instant before the
/// one on screen, which the view knows only if it saw that instant arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepBackLoss {
    Exactly(usize),
    /// The instant stepping back goes to isn't known, so this counts every mark there is.
    UpTo(usize),
}

impl StepBackLoss {
    pub fn count(self) -> usize {
        match self {
            StepBackLoss::Exactly(count) | StepBackLoss::UpTo(count) => count,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Marks {
    marks: Vec<Mark>,
    /// The last two instants seen, latest first. An instant is a time with occurrences, as the
    /// session counts them for stepping.
    instants: [Option<SimTime>; 2],
}

impl Marks {
    pub(crate) fn all(&self) -> &[Mark] {
        &self.marks
    }

    pub(crate) fn add(&mut self, at: SimTime, object: u32, intervention: Intervention) {
        let settled = intervention.shows_as().is_none();
        self.marks.push(Mark { at, object, intervention, settled });
    }

    pub(crate) fn saw_occurrences(&mut self, occurrences: &[Occurrence]) {
        for occurrence in occurrences {
            if self.instants[0] != Some(occurrence.time) {
                self.instants = [Some(occurrence.time), self.instants[0]];
            }
            let (object, shown) = match occurrence.kind {
                OccurrenceKind::InterruptionBegan { object_id, cause } => (object_id, (true, cause)),
                OccurrenceKind::InterruptionEnded { object_id, cause } => (object_id, (false, cause)),
                _ => continue,
            };
            // The view sends a command a frame or so after the instant it shows, so at speed the
            // session's shown time can be later. The occurrence says when it really happened.
            if let Some(mark) = self
                .marks
                .iter_mut()
                .find(|mark| !mark.settled && mark.object == object && mark.intervention.shows_as() == Some(shown))
            {
                mark.at = occurrence.time;
                mark.settled = true;
            }
        }
    }

    /// A full state at `time`: after Init, Reset, going back, or asked for at shown time.
    pub(crate) fn saw_state(&mut self, time: SimTime, initialised: bool) {
        if !initialised {
            *self = Marks::default();
            return;
        }
        self.marks.retain(|mark| mark.at <= time);
        self.instants = match self.instants {
            [Some(last), before] if last <= time => [Some(last), before],
            [_, Some(before)] if before <= time => [Some(before), None],
            _ => [None, None],
        };
    }

    pub(crate) fn lost_going_back(&self, to: SimTime) -> usize {
        self.marks.iter().filter(|mark| mark.at > to).count()
    }

    pub(crate) fn lost_stepping_back(&self) -> StepBackLoss {
        match self.instants[1] {
            Some(before) => StepBackLoss::Exactly(self.lost_going_back(before)),
            None => StepBackLoss::UpTo(self.marks.len()),
        }
    }
}
