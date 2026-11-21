//! What every interruptible object keeps about its interruptions, and the trait tying it to each
//! type (ADR 0020). `Interruptions` does the bookkeeping once: which interruptions are active,
//! when each began, and the failure profile. It is handed each stop event and says what changed,
//! so it can be tested without an object around it. A type implements `Interruptible`, saying only
//! what being interrupted means for it.

use crate::error::RunTimeError;
use crate::event::{EventContext, EventTemplate, EventType, ProfileIndex};
use crate::materialflow_objects::OtherObjects;
use crate::report::{ActiveInterruption, FailureProfileState, Interruption, OccurrenceKind};
use crate::statistics::StatisticsKeeper;
use crate::time::SimTime;
use super::flows::{FailureProfile, TimeBasis};

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
    /// Its named failure profiles, independent of each other. One pointer, `None` for none: a
    /// profile holds two random number generators, and every object in a run takes the size of
    /// the largest, so the list lives behind a box. A failure event looks its profile up here,
    /// a few times per failure and never per MU.
    failures: Option<Box<Profiles>>,
}

/// The failure profiles, and what only they need: how long the object has been paused since Init,
/// not counting a pause in progress, which working time leaves out. It lives here, so an object
/// without profiles doesn't grow for it, and clearing the statistics doesn't touch it.
#[derive(Clone)]
struct Profiles {
    list: Vec<FailureProfile>,
    paused: SimTime,
}

impl std::ops::Deref for Profiles {
    type Target = Vec<FailureProfile>;
    fn deref(&self) -> &Vec<FailureProfile> {
        &self.list
    }
}

impl std::ops::DerefMut for Profiles {
    fn deref_mut(&mut self) -> &mut Vec<FailureProfile> {
        &mut self.list
    }
}

impl Interruptions {
    pub fn new(failures: Option<FailureProfile>) -> Interruptions {
        Interruptions::with_profiles(failures.into_iter().collect())
    }

    pub fn with_profiles(failures: Vec<FailureProfile>) -> Interruptions {
        Interruptions { active: [false; 3], since: [SimTime::ZERO; 3], failures: boxed(failures) }
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

    /// The failure profiles, in the order they were given.
    pub fn failure_profiles(&self) -> &[FailureProfile] {
        self.failures.as_deref().map_or(&[], |list| list.as_slice())
    }

    /// Gives the object one failure profile, or none with `None`. Part of its configuration, so
    /// it is set before a run, not during one.
    pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
        self.set_failure_profiles(failures.into_iter().collect());
    }

    /// Gives the object these failure profiles, replacing any it had.
    pub fn set_failure_profiles(&mut self, failures: Vec<FailureProfile>) {
        self.failures = boxed(failures);
    }

    /// Seeds every profile from the object's `seed` and the profile's stream, which is its own
    /// within the object, or else its place in the list. Stream 0 is the object's seed itself, so
    /// a single profile from before profiles had streams fails as it always has. Since the stream
    /// is the object's, a class's profiles fail apart in each of its instances.
    pub fn reseed(&mut self, seed: u64) {
        if let Some(failures) = &mut self.failures {
            for (index, profile) in failures.iter_mut().enumerate() {
                let stream = profile.stream.unwrap_or(index as u64);
                profile.reseed(seed ^ stream.wrapping_mul(0xD1B5_4A32_D192_ED03));
            }
        }
    }

    /// Each profile's figures at `now`, for the object's state, with its up time from the object's
    /// `statistics`.
    pub fn profile_states(&self, now: SimTime, statistics: &StatisticsKeeper) -> Vec<FailureProfileState> {
        if self.failure_profiles().iter().all(|profile| !profile.active) {
            return Vec::new();
        }
        let read = statistics.read(now);
        let measured = read.measured_time();
        self.failure_profiles()
            .iter()
            .enumerate()
            .filter(|(_, profile)| profile.active)
            .map(|(index, profile)| FailureProfileState {
                index: index as u8,
                name: profile.name.clone(),
                availability: profile.availablity,
                failures: profile.failures,
                time_failed: profile.failed_by(now),
                failing_since: profile.failing_since,
                time_basis: profile.time_basis,
                up: match profile.time_basis {
                    TimeBasis::Simulation => measured.saturating_sub(profile.failed_by(now)),
                    TimeBasis::Working => measured.saturating_sub(profile.failed_by(now)).saturating_sub(read.time_paused),
                    TimeBasis::Processing => read.time_busy,
                },
            })
            .collect()
    }

    /// The first random failure of each active profile, due at Init, when no time has counted on
    /// any basis.
    pub fn first_failure(&mut self, object_id: u32) -> Vec<EventTemplate> {
        let count = self.failure_profiles().len();
        (0..count).filter_map(|index| self.next_failure(index, object_id, SimTime::ZERO)).collect()
    }

    /// Schedules a profile's next failure, `counted` being the time counted on its basis now. It is
    /// due once its basis has counted the time sampled, which is never sooner than that time from
    /// now, so the event goes there and is checked when it fires (lazy checking).
    fn next_failure(&mut self, index: usize, object_id: u32, counted: SimTime) -> Option<EventTemplate> {
        let profile = self.failures.as_mut()?.get_mut(index).filter(|profile| profile.active)?;
        let after = profile.get_time_to_next_failure().ok().flatten()?;
        profile.due = counted + after;
        Some(EventTemplate { eventtype: EventType::StartFailure(index as ProfileIndex), exec_time: after, object_id })
    }

    /// The time counted on `basis` by `now`, the object's statistics giving its busy time.
    fn counted(&self, basis: TimeBasis, now: SimTime, statistics: &StatisticsKeeper) -> SimTime {
        match basis {
            TimeBasis::Simulation => now,
            TimeBasis::Working => now.saturating_sub(self.paused_by(now)),
            TimeBasis::Processing => statistics.read(now).time_busy,
        }
    }

    /// How long it has been paused by `now`, a pause in progress included.
    fn paused_by(&self, now: SimTime) -> SimTime {
        let in_progress = if self.is_active(Interruption::Paused) { now.saturating_sub(self.since[Interruption::Paused.index()]) } else { SimTime::ZERO };
        self.failures.as_ref().map_or(SimTime::ZERO, |profiles| profiles.paused) + in_progress
    }

    /// Back to the default state: nothing active and nothing counted. The profiles keep their
    /// random numbers' state, as a Station's always has.
    pub fn reset(&mut self) {
        self.active = [false; 3];
        self.since = [SimTime::ZERO; 3];
        if let Some(profiles) = self.failures.as_mut() {
            profiles.paused = SimTime::ZERO;
        }
        for profile in self.failures.iter_mut().flat_map(|list| list.iter_mut()) {
            profile.failing_since = None;
            profile.time_failed = SimTime::ZERO;
            profile.failures = 0;
        }
    }

    /// Starts each profile's figures afresh at `now`: a failure in progress counts from here. Called
    /// before the object's `statistics` are cleared, so that a profile counting busy time, which
    /// they hold, keeps what it has counted towards its next failure.
    pub fn clear_statistics(&mut self, now: SimTime, statistics: &StatisticsKeeper) {
        let busy = statistics.read(now).time_busy;
        for profile in self.failures.iter_mut().flat_map(|list| list.iter_mut()) {
            if profile.time_basis == TimeBasis::Processing {
                profile.due = profile.due.saturating_sub(busy);
            }
            profile.time_failed = SimTime::ZERO;
            profile.failures = 0;
            if profile.failing_since.is_some() {
                profile.failing_since = Some(now);
            }
        }
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
        // A random failure means nothing for an object with no such failure profile.
        if let EventType::StartFailure(index) | EventType::EndFailure(index) = *eventtype {
            if self.failure_profiles().len() <= index as usize {
                return None;
            }
        }
        let change = match *eventtype {
            EventType::StartFailure(index) => {
                let basis = self.failure_profiles()[index as usize].time_basis;
                let counted = if basis == TimeBasis::Simulation { None } else { Some(self.counted(basis, context.time, statistics)) };
                let profile = &mut self.failures.as_mut().expect("checked above")[index as usize];
                if profile.failing_since.is_some() {
                    Change::Unchanged
                } else if let Some(counted) = counted.filter(|&counted| counted < profile.due) {
                    // Its basis counted too little to get here: come back when it could have.
                    context.schedule(EventTemplate { eventtype: EventType::StartFailure(index), exec_time: profile.due.saturating_sub(counted), object_id });
                    Change::Unchanged
                } else {
                    profile.failing_since = Some(context.time);
                    profile.failures += 1;
                    let repair = SimTime::from_millis_f64(profile.dist_sampler_mttr.sample());
                    let change = self.begin(Interruption::Failed, Some(index), object_id, context, statistics);
                    statistics.count_failure();
                    context.schedule(EventTemplate { eventtype: EventType::EndFailure(index), exec_time: repair, object_id });
                    change
                }
            }
            EventType::EndFailure(index) => {
                let profile = &mut self.failures.as_mut().expect("checked above")[index as usize];
                if let Some(since) = profile.failing_since.take() {
                    profile.time_failed += context.time.saturating_sub(since);
                }
                context.record(OccurrenceKind::InterruptionEnded { object_id, cause: Interruption::Failed, profile: Some(index) });
                // The failure is over, but the object stays failed while another profile is.
                let still_failing = self.failure_profiles().iter().any(|profile| profile.failing_since.is_some());
                self.active[Interruption::Failed.index()] = still_failing;
                let basis = self.failure_profiles()[index as usize].time_basis;
                let counted = self.counted(basis, context.time, statistics);
                if let Some(next) = self.next_failure(index as usize, object_id, counted) {
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
    fn begin(&mut self, cause: Interruption, profile: Option<ProfileIndex>, object_id: u32, context: &mut EventContext, statistics: &mut StatisticsKeeper) -> Change {
        context.record(OccurrenceKind::InterruptionBegan { object_id, cause, profile });
        // A cause already active, such as a second failing profile, keeps its first time.
        if !self.is_active(cause) {
            self.since[cause.index()] = context.time;
        }
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
        let change = self.begin(cause, None, object_id, context, statistics);
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
        if cause == Interruption::Paused {
            if let Some(profiles) = self.failures.as_mut() {
                profiles.paused += context.time.saturating_sub(self.since[cause.index()]);
            }
        }
        self.active[cause.index()] = false;
        context.record(OccurrenceKind::InterruptionEnded { object_id, cause, profile: None });
        self.ended()
    }

    fn ended(&self) -> Change {
        if self.is_interrupted() { Change::Unchanged } else { Change::Ended }
    }
}

fn boxed(failures: Vec<FailureProfile>) -> Option<Box<Profiles>> {
    (!failures.is_empty()).then(|| Box::new(Profiles { list: failures, paused: SimTime::ZERO }))
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
        let first = fixture.interruptions.first_failure(ID).pop().expect("an active profile fails");
        assert!(matches!(first.eventtype, EventType::StartFailure(0)));
        let (change, events) = fixture.at(100, EventType::StartFailure(0));
        assert_eq!(change, Change::Began);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].eventtype, EventType::EndFailure(0)));
        assert_eq!(events[0].exec_time.as_millis(), 60_000, "the repair time");
        let (change, events) = fixture.at(60_100, EventType::EndFailure(0));
        assert_eq!(change, Change::Ended);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].eventtype, EventType::StartFailure(0)));
    }

    #[test]
    fn an_inactive_profile_never_fails() {
        let mut fixture = Fixture::new(Some(FailureProfile::new(1.0, Distribution::CONSTANT { value: 60_000.0 }, 7)));
        assert!(fixture.interruptions.first_failure(ID).is_empty());
        assert!(Fixture::new(None).interruptions.first_failure(ID).is_empty());
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

    fn two_profiles() -> Interruptions {
        Interruptions::with_profiles(vec![
            FailureProfile::new(0.9, Distribution::CONSTANT { value: 60_000.0 }, 7).named("Tool wear"),
            FailureProfile::new(0.8, Distribution::CONSTANT { value: 10_000.0 }, 8).named("Jam"),
        ])
    }

    #[test]
    fn each_profile_schedules_its_own_failures_and_names_itself_in_its_events() {
        let mut interruptions = two_profiles();
        let first = interruptions.first_failure(ID);
        assert_eq!(first.len(), 2);
        assert!(matches!(first[0].eventtype, EventType::StartFailure(0)));
        assert!(matches!(first[1].eventtype, EventType::StartFailure(1)));
    }

    #[test]
    fn overlapping_failures_of_two_profiles_are_one_stop_and_each_counts_its_own_time() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = two_profiles();
        assert_eq!(fixture.change(100, EventType::StartFailure(0)), Change::Began);
        assert_eq!(fixture.change(200, EventType::StartFailure(1)), Change::Unchanged, "already interrupted");
        // The Jam ends first, so the object stays failed for Tool wear.
        assert_eq!(fixture.change(10_200, EventType::EndFailure(1)), Change::Unchanged);
        assert!(fixture.interruptions.is_active(Interruption::Failed));
        assert_eq!(fixture.change(60_100, EventType::EndFailure(0)), Change::Ended);
        assert!(!fixture.interruptions.is_interrupted());

        let states = fixture.interruptions.profile_states(SimTime::from_millis(70_000), &fixture.statistics);
        assert_eq!(states.iter().map(|state| (state.name.as_str(), state.failures, state.time_failed.as_millis())).collect::<Vec<_>>(), vec![("Tool wear", 1, 60_000), ("Jam", 1, 10_000)]);
        let statistics = fixture.statistics.read(SimTime::from_millis(70_000));
        assert_eq!((statistics.stops, statistics.failures), (1, 2), "one stop, two failures");

        let occurrences = fixture.context.take_occurrences();
        let profiles: Vec<Option<u8>> = occurrences
            .iter()
            .filter_map(|o| match o.kind {
                OccurrenceKind::InterruptionBegan { profile, .. } => Some(profile),
                _ => None,
            })
            .collect();
        assert_eq!(profiles, vec![Some(0), Some(1)], "each failure names its profile");
    }

    #[test]
    fn the_failed_state_keeps_the_time_its_first_failure_began() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = two_profiles();
        fixture.at(100, EventType::StartFailure(0));
        fixture.at(200, EventType::StartFailure(1));
        assert_eq!(fixture.interruptions.list()[0].since.as_millis(), 100);
    }

    #[test]
    fn a_failure_in_progress_counts_in_the_profiles_time_so_far() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = two_profiles();
        fixture.at(1_000, EventType::StartFailure(1));
        let jam = &fixture.interruptions.profile_states(SimTime::from_millis(4_000), &fixture.statistics)[1];
        assert_eq!((jam.time_failed.as_millis(), jam.failing_since), (3_000, Some(SimTime::from_millis(1_000))));
        fixture.interruptions.clear_statistics(SimTime::from_millis(4_000), &fixture.statistics);
        let jam = &fixture.interruptions.profile_states(SimTime::from_millis(5_000), &fixture.statistics)[1];
        assert_eq!((jam.time_failed.as_millis(), jam.failures), (1_000, 0), "counts afresh from 4 s, and the failure that began earlier isn't counted again");
    }

    #[test]
    fn an_event_for_a_profile_the_object_lacks_is_not_its_business() {
        let mut fixture = Fixture::new(Some(failing()));
        assert!(fixture.interruptions.handle(&EventType::StartFailure(1), ID, &mut fixture.context, &mut fixture.statistics).is_none());
    }

    #[test]
    fn reset_clears_every_profiles_figures() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = two_profiles();
        fixture.at(100, EventType::StartFailure(0));
        fixture.interruptions.reset();
        let states = fixture.interruptions.profile_states(SimTime::from_millis(500), &fixture.statistics);
        assert!(states.iter().all(|state| state.failures == 0 && state.time_failed == SimTime::ZERO && state.failing_since.is_none()));
    }

    #[test]
    fn profiles_with_no_stream_of_their_own_fail_independently_of_each_other() {
        let mut interruptions = Interruptions::with_profiles(vec![
            FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0).named("A"),
            FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0).named("B"),
        ]);
        interruptions.reseed(1234);
        let firsts = interruptions.first_failure(ID);
        assert_ne!(firsts[0].exec_time, firsts[1].exec_time, "the same seed would give the same failures");
    }

    #[test]
    fn a_profiles_stream_keeps_its_failures_wherever_it_is_in_the_list() {
        let profile = |name: &str, stream: u64| FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0).named(name).with_stream(stream);
        let mut alone = Interruptions::with_profiles(vec![profile("Jam", 3)]);
        let mut second = Interruptions::with_profiles(vec![profile("Wear", 1), profile("Jam", 3)]);
        alone.reseed(42);
        second.reseed(42);
        assert_eq!(alone.first_failure(ID)[0].exec_time, second.first_failure(ID)[1].exec_time);
        let mut unstreamed = Interruptions::with_profiles(vec![FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0)]);
        let mut stream_0 = Interruptions::with_profiles(vec![profile("Failures", 0)]);
        unstreamed.reseed(42);
        stream_0.reseed(42);
        assert_eq!(unstreamed.first_failure(ID)[0].exec_time, stream_0.first_failure(ID)[0].exec_time, "no stream is its place, and the first's is the object's own");
    }

    #[test]
    fn on_working_time_a_failure_comes_later_by_the_time_paused_checked_when_it_fires() {
        let mut fixture = Fixture::new(None);
        // 50 % available with a constant minute's repair: a minute between failures.
        fixture.interruptions = Interruptions::with_profiles(vec![FailureProfile::new(0.5, Distribution::CONSTANT { value: 60_000.0 }, 0).with_time_basis(TimeBasis::Working)]);
        let first = fixture.interruptions.first_failure(ID);
        assert_eq!(first[0].exec_time.as_millis(), 60_000, "as soon as it could come, as if never paused");
        fixture.at(10_000, EventType::PauseOn);
        fixture.at(30_000, EventType::PauseOff);
        let (change, events) = fixture.at(60_000, EventType::StartFailure(0));
        assert_eq!(change, Change::Unchanged, "only 40 s of working time so far");
        assert!(matches!(events[0].eventtype, EventType::StartFailure(0)));
        assert_eq!(events[0].exec_time.as_millis(), 20_000, "the 20 s paused");
        assert!(fixture.context.take_occurrences().iter().all(|o| !matches!(o.kind, OccurrenceKind::InterruptionBegan { cause: Interruption::Failed, .. })));
        let (change, _) = fixture.at(80_000, EventType::StartFailure(0));
        assert_eq!(change, Change::Began);
    }

    #[test]
    fn a_pause_in_progress_holds_a_working_time_failure_back() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = Interruptions::with_profiles(vec![FailureProfile::new(0.5, Distribution::CONSTANT { value: 60_000.0 }, 0).with_time_basis(TimeBasis::Working)]);
        fixture.interruptions.first_failure(ID);
        fixture.at(50_000, EventType::PauseOn);
        let (change, events) = fixture.at(60_000, EventType::StartFailure(0));
        assert_eq!(change, Change::Unchanged);
        assert_eq!(events[0].exec_time.as_millis(), 10_000, "10 s of it paused so far, and more may come");
    }

    #[test]
    fn on_simulation_time_nothing_is_checked() {
        let mut fixture = Fixture::new(None);
        fixture.interruptions = Interruptions::with_profiles(vec![FailureProfile::new(0.5, Distribution::CONSTANT { value: 60_000.0 }, 0)]);
        fixture.interruptions.first_failure(ID);
        fixture.at(10_000, EventType::PauseOn);
        fixture.at(30_000, EventType::PauseOff);
        assert_eq!(fixture.change(60_000, EventType::StartFailure(0)), Change::Began);
    }

    #[test]
    fn other_events_are_not_its_business() {
        let mut fixture = Fixture::new(None);
        assert!(fixture.interruptions.handle(&EventType::MUEntrance, ID, &mut fixture.context, &mut fixture.statistics).is_none());
    }
}
