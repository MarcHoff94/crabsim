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

#[derive(Debug, Default)]
pub(crate) struct Marks {
    marks: Vec<Mark>,
    /// The last two instants seen, latest first. An instant is a time with occurrences, as the
    /// session counts them for stepping.
    instants: [Option<SimTime>; 2],
    /// The session's last answer to where stepping back goes: from that shown time, to that
    /// instant, or nowhere.
    located: Option<(SimTime, Option<SimTime>)>,
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
        // New occurrences can make shown time an instant, such as a Pause where nothing else
        // happened, which moves where stepping back goes.
        if !occurrences.is_empty() {
            self.located = None;
        }
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

    /// The session's answer to `locate_step_back`.
    pub(crate) fn saw_step_back_target(&mut self, shown: SimTime, to: Option<SimTime>) {
        self.located = Some((shown, to));
    }

    /// Where stepping back from shown time `shown` goes, if the view knows: the instant before
    /// the one on screen, or `Some(None)` from the first instant. The view knows it if it saw that
    /// instant arrive, or if the session said so for this shown time.
    pub(crate) fn step_back_target(&self, shown: SimTime) -> Option<Option<SimTime>> {
        match (self.located, self.instants[1]) {
            (Some((at, to)), _) if at == shown => Some(to),
            (_, Some(before)) => Some(Some(before)),
            _ => None,
        }
    }
}
