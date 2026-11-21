//! What every interruptible object keeps about its interruptions, and the trait tying it to each
//! type (ADR 0020). `Interruptions` does the bookkeeping once: which interruptions are active,
//! when each began, and the failure profile. It is handed each stop event and says what changed,
//! so it can be tested without an object around it. A type implements `Interruptible`, saying only
//! what being interrupted means for it.

use crate::error::RunTimeError;
use crate::event::{EventContext, EventTemplate, EventType};
use crate::materialflow_objects::OtherObjects;
use crate::report::{ActiveInterruption, Interruption, OccurrenceKind};
use crate::statistics::StatisticsKeeper;
use crate::time::SimTime;
use super::flows::FailureProfile;

/// What one stop event did to an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The object wasn't interrupted and now is.
    Began,
    /// Its last interruption ended.
    Ended,
    /// It stays as it was: interrupted or not, perhaps with another cause added or ended.
    Unchanged,
}

/// An object's interruptions: which are active, since when, and its failure profile.
#[derive(Clone)]
pub struct Interruptions {
    /// Which causes are active, by `Interruption::index`.
    active: [bool; 3],
    /// When each cause last began, by `Interruption::index`. Read only while it is active.
    since: [SimTime; 3],
    /// Boxed, so an object pays for it only through a pointer: a profile holds two random number
    /// generators, and every object in a run takes the size of the largest.
    failures: Option<Box<FailureProfile>>,
}

impl Interruptions {
    pub fn new(failures: Option<FailureProfile>) -> Interruptions {
        Interruptions { active: [false; 3], since: [SimTime::ZERO; 3], failures: failures.map(Box::new) }
    }

    /// Whether any interruption is active.
    pub fn is_interrupted(&self) -> bool {
        self.active.iter().any(|&active| active)
    }

    pub fn is_active(&self, cause: Interruption) -> bool {
        self.active[cause.index()]
    }

    /// The most serious active cause, which is what the object reports and the statistics count.
    pub fn most_serious(&self) -> Option<Interruption> {
        Interruption::ALL.into_iter().find(|&cause| self.is_active(cause))
    }

    /// Every active interruption, most serious first, with when it began.
    pub fn list(&self) -> Vec<ActiveInterruption> {
        Interruption::ALL
            .into_iter()
            .filter(|&cause| self.is_active(cause))
            .map(|cause| ActiveInterruption { cause, since: self.since[cause.index()] })
            .collect()
    }

    pub fn failure_profile(&self) -> Option<&FailureProfile> {
        self.failures.as_deref()
    }

    /// Gives the object failures, or none with `None`. Part of its configuration, so it is set
    /// before a run, not during one.
    pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
        self.failures = failures.map(Box::new);
    }

    pub fn reseed(&mut self, seed: u64) {
        if let Some(failures) = &mut self.failures {
            failures.reseed(seed);
        }
    }

    /// The first random failure, due at Init, if the object has an active profile.
    pub fn first_failure(&mut self, object_id: u32) -> Option<EventTemplate> {
        self.next_failure(object_id)
    }

    fn next_failure(&mut self, object_id: u32) -> Option<EventTemplate> {
        let failures = self.failures.as_mut().filter(|failures| failures.active)?;
        let after = failures.get_time_to_next_failure().ok().flatten()?;
        Some(EventTemplate { eventtype: EventType::StartFailure, exec_time: after, object_id })
    }

    /// Back to the default state: nothing active. The profile keeps its random numbers' state, as
    /// a Station's always has.
    pub fn reset(&mut self) {
        self.active = [false; 3];
        self.since = [SimTime::ZERO; 3];
    }

    /// Handles a stop event, recording what began or ended, counting it in `statistics`,
    /// scheduling what follows through `context` (a failure's end, or the next failure), and
    /// returning what changed. `None` for an event that isn't about interruptions.
    pub fn handle(
        &mut self,
        eventtype: &EventType,
        object_id: u32,
        context: &mut EventContext,
        statistics: &mut StatisticsKeeper,
    ) -> Option<Change> {
        // A random failure means nothing for an object with no failure profile.
        if matches!(eventtype, EventType::StartFailure | EventType::EndFailure) && self.failures.is_none() {
            return None;
        }
        let change = match *eventtype {
            EventType::StartFailure => {
                if self.is_active(Interruption::Failed) {
                    Change::Unchanged
                } else {
                    let change = self.begin(Interruption::Failed, object_id, context, statistics);
                    statistics.count_failure();
                    let failures = self.failures.as_mut().expect("checked above");
                    let repair = SimTime::from_millis_f64(failures.dist_sampler_mttr.sample());
                    context.schedule(EventTemplate { eventtype: EventType::EndFailure, exec_time: repair, object_id });
                    change
                }
            }
            EventType::EndFailure => {
                // The failure's own end, so it is always active here.
                self.active[Interruption::Failed.index()] = false;
                context.record(OccurrenceKind::InterruptionEnded { object_id, cause: Interruption::Failed });
                if let Some(next) = self.next_failure(object_id) {
                    context.schedule(next);
                }
                self.ended()
            }
            EventType::StopOn => self.begin_by_hand(Interruption::Stopped, object_id, context, statistics),
            EventType::PauseOn => self.begin_by_hand(Interruption::Paused, object_id, context, statistics),
            EventType::StopOff => self.end_by_hand(Interruption::Stopped, object_id, context),
            EventType::PauseOff => self.end_by_hand(Interruption::Paused, object_id, context),
            _ => return None,
        };
        Some(change)
    }

    /// Starts an interruption. Only the first of overlapping ones begins the stop, so time spent
    /// interrupted counts once.
    fn begin(&mut self, cause: Interruption, object_id: u32, context: &mut EventContext, statistics: &mut StatisticsKeeper) -> Change {
        context.record(OccurrenceKind::InterruptionBegan { object_id, cause });
        self.since[cause.index()] = context.time;
        let was_interrupted = self.is_interrupted();
        self.active[cause.index()] = true;
        if was_interrupted {
            Change::Unchanged
        } else {
            statistics.count_stop();
            Change::Began
        }
    }

    fn begin_by_hand(&mut self, cause: Interruption, object_id: u32, context: &mut EventContext, statistics: &mut StatisticsKeeper) -> Change {
        if self.is_active(cause) {
            return Change::Unchanged;
        }
        let change = self.begin(cause, object_id, context, statistics);
        match cause {
            Interruption::Paused => statistics.count_pause(),
            _ => statistics.count_failure(),
        }
        change
    }

    fn end_by_hand(&mut self, cause: Interruption, object_id: u32, context: &mut EventContext) -> Change {
        if !self.is_active(cause) {
            return Change::Unchanged;
        }
        self.active[cause.index()] = false;
        context.record(OccurrenceKind::InterruptionEnded { object_id, cause });
        self.ended()
    }

    fn ended(&self) -> Change {
        if self.is_interrupted() { Change::Unchanged } else { Change::Ended }
    }
}

/// An object that can be interrupted. It keeps `Interruptions` and says what an interruption means
/// for it: `stop_began` when the first begins, `stop_ended` when the last ends.
pub trait Interruptible {
    /// Its interruptions, its statistics and its id, borrowed together.
    fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32);

    /// The object has just been interrupted.
    fn stop_began(&mut self, context: &mut EventContext);

    /// The object's last interruption has just ended. What it schedules comes after what the
    /// interruption itself scheduled, such as the next failure.
    fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError>;

    /// Handles a stop event for the object: `None` for any other event.
    fn handle_interruption(
        &mut self,
        eventtype: &EventType,
        context: &mut EventContext,
        others: &mut OtherObjects<'_>,
    ) -> Option<Result<(), RunTimeError>> {
        let (interruptions, statistics, object_id) = self.interruption_parts();
        let change = interruptions.handle(eventtype, object_id, context, statistics)?;
        Some(match change {
            Change::Began => {
                self.stop_began(context);
                Ok(())
            }
            Change::Ended => self.stop_ended(context, others),
            Change::Unchanged => Ok(()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random_numbers::Distribution;
    use crate::report::Recording;

    const ID: u32 = 4;

    struct Fixture {
        interruptions: Interruptions,
        context: EventContext,
        statistics: StatisticsKeeper,
    }

    impl Fixture {
        fn new(failures: Option<FailureProfile>) -> Fixture {
            Fixture { interruptions: Interruptions::new(failures), context: EventContext::new(0, Recording::ALL), statistics: StatisticsKeeper::new(1) }
        }
        fn at(&mut self, millis: u64, event: EventType) -> (Change, Vec<EventTemplate>) {
            self.context.time = SimTime::from_millis(millis);
            let change = self.interruptions.handle(&event, ID, &mut self.context, &mut self.statistics).expect("a stop event");
            (change, self.context.take_scheduled())
        }
        fn change(&mut self, millis: u64, event: EventType) -> Change {
            self.at(millis, event).0
        }
    }

    fn failing() -> FailureProfile {
        FailureProfile::new(0.9, Distribution::CONSTANT { value: 60_000.0 }, 7)
    }

    #[test]
    fn overlapping_interruptions_begin_and_end_one_stop() {
        let mut fixture = Fixture::new(None);
        assert_eq!(fixture.change(10, EventType::PauseOn), Change::Began);
        assert_eq!(fixture.change(20, EventType::StopOn), Change::Unchanged, "already interrupted");
        assert_eq!(fixture.interruptions.most_serious(), Some(Interruption::Stopped), "the more serious cause wins");
        assert_eq!(fixture.change(30, EventType::PauseOff), Change::Unchanged, "still stopped by hand");
        assert_eq!(fixture.change(40, EventType::StopOff), Change::Ended);
        assert!(!fixture.interruptions.is_interrupted());
        let statistics = fixture.statistics.read(SimTime::from_millis(40));
        assert_eq!((statistics.stops, statistics.pauses, statistics.failures), (1, 1, 1), "one stop of two interruptions");
    }

    #[test]
    fn repeating_or_ending_what_isnt_active_changes_nothing() {
        let mut fixture = Fixture::new(None);
        assert_eq!(fixture.change(5, EventType::PauseOff), Change::Unchanged);
        assert_eq!(fixture.change(10, EventType::PauseOn), Change::Began);
        assert_eq!(fixture.change(15, EventType::PauseOn), Change::Unchanged);
        let occurrences = fixture.context.take_occurrences();
        assert_eq!(occurrences.len(), 1, "only the first pause began: {occurrences:?}");
        assert_eq!(fixture.statistics.read(SimTime::from_millis(15)).pauses, 1);
    }

    #[test]
    fn the_list_says_each_active_cause_and_when_it_began_most_serious_first() {
        let mut fixture = Fixture::new(None);
        fixture.at(10, EventType::PauseOn);
        fixture.at(25, EventType::StopOn);
        let list = fixture.interruptions.list();
        assert_eq!(list.iter().map(|interruption| (interruption.cause, interruption.since.as_millis())).collect::<Vec<_>>(), vec![(Interruption::Stopped, 25), (Interruption::Paused, 10)]);
    }

    #[test]
    fn a_random_failure_schedules_its_repair_and_then_the_next_failure() {
        let mut fixture = Fixture::new(Some(failing()));
        let first = fixture.interruptions.first_failure(ID).expect("an active profile fails");
        assert!(matches!(first.eventtype, EventType::StartFailure));
        let (change, events) = fixture.at(100, EventType::StartFailure);
        assert_eq!(change, Change::Began);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].eventtype, EventType::EndFailure));
        assert_eq!(events[0].exec_time.as_millis(), 60_000, "the repair time");
        let (change, events) = fixture.at(60_100, EventType::EndFailure);
        assert_eq!(change, Change::Ended);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].eventtype, EventType::StartFailure));
    }

    #[test]
    fn an_inactive_profile_never_fails() {
        let mut fixture = Fixture::new(Some(FailureProfile::new(1.0, Distribution::CONSTANT { value: 60_000.0 }, 7)));
        assert!(fixture.interruptions.first_failure(ID).is_none());
        assert!(Fixture::new(None).interruptions.first_failure(ID).is_none());
    }

    #[test]
    fn reset_ends_every_interruption() {
        let mut fixture = Fixture::new(None);
        fixture.at(10, EventType::PauseOn);
        fixture.at(20, EventType::StopOn);
        fixture.interruptions.reset();
        assert!(!fixture.interruptions.is_interrupted());
        assert!(fixture.interruptions.list().is_empty());
        assert_eq!(fixture.change(30, EventType::PauseOn), Change::Began);
    }

    #[test]
    fn other_events_are_not_its_business() {
        let mut fixture = Fixture::new(None);
        assert!(fixture.interruptions.handle(&EventType::MUEntrance, ID, &mut fixture.context, &mut fixture.statistics).is_none());
    }
}
