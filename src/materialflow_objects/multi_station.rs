//! A Station with a capacity above 1: one machine working on several MUs at once (ADR 0001). A
//! Station of capacity 1 is a `SingleStation`, which stays the cheaper path, so a model without
//! capacity runs exactly as it did. Both report as a Station.

use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::flows::{
	list_blocked, relist_blocked, retries_for_blocked, retry_move_to_successor, try_to_move_to_successor, waiting_ids, BaseObject, Distributing,
	EventInterface, FailureProfile, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, Waiting,
};
use super::interruptions::{Interruptible, Interruptions};
use crate::materialflow_objects::OtherObjects;
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind, WorkLeft};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;

/// When a Station with capacity begins work on its MUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartMode {
	/// Each MU starts on entry, with its own process time.
	#[default]
	Individual,
	/// Work begins when the Station is full, and its entrance stays closed until it is
	/// completely empty.
	Batch,
}

/// Where one MU in a Station is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
	/// Waiting for a batch to fill.
	Waiting,
	/// In work, done at this time.
	At(SimTime),
	/// In work, stopped with this much left.
	Interrupted(SimTime),
	/// Finished, ready to leave since this time, when its work finished. An interruption doesn't
	/// change it, so the Station keeps its place in line (spec issue 122).
	ReadySince(SimTime),
}

#[derive(Debug, Clone)]
struct Place {
	mu: MoveableUnit,
	work: Work,
}

#[derive(Clone)]
pub struct MultiStation {
	pub base: BaseObject,
	pub proc_time: SimTime,
	/// The process time in the default state. Reset returns `proc_time` to it.
	pub configured_proc_time: SimTime,
	pub capacity: u64,
	pub start: StartMode,
	/// Its failures and pauses (ADR 0020), whose failure profile is boxed.
	pub interruptions: Interruptions,
	/// The MUs it holds in its default state, in the order they count as entered.
	pub starting_mus: Vec<u32>,
	/// In the order they came in, which breaks ties between MUs that finished at once.
	places: Vec<Place>,
	/// Bumped by every stop, so an exit scheduled before it is known to be stale.
	stop_count: u32,
	/// A batch is in work or leaving: closed until the Station is empty.
	batch_closed: bool,
	/// A finished MU was refused by every successor, so the Station waits in their lines.
	waiting: bool,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	pub distribution: Distributing,
	pub blockedlist: Vec<Waiting>,
	pub statistics: StatisticsKeeper,
}

impl MultiStation {
	pub fn new(object_id: u32, proc_time: SimTime, failure_profile: FailureProfile, capacity: u64, start: StartMode) -> MultiStation {
		MultiStation {
			base: BaseObject { object_id, random_seed: 0 },
			proc_time,
			configured_proc_time: proc_time,
			capacity,
			start,
			interruptions: Interruptions::new(Some(failure_profile)),
			starting_mus: Vec::new(),
			places: Vec::new(),
			stop_count: 0,
			batch_closed: false,
			waiting: false,
			succ: Vec::new(),
			succ_pointer: 0,
			distribution: Distributing::RoundRobin,
			blockedlist: Vec::new(),
			statistics: StatisticsKeeper::new(capacity),
		}
	}

	/// Sets the MUs it holds in its default state, by id, in the order they count as entered. More
	/// than its capacity is the caller's mistake: a model file is checked for it.
	pub fn set_starting_mus(&mut self, mus: Vec<u32>) {
		let object_id = self.base.object_id;
		self.places = mus
			.iter()
			.map(|&id| Place { mu: MoveableUnit { id, location_id: object_id, left_source: SimTime::ZERO }, work: Work::Waiting })
			.collect();
		self.starting_mus = mus;
	}

	pub fn is_interrupted(&self) -> bool {
		self.interruptions.is_interrupted()
	}

	fn held(&self) -> u64 {
		self.places.len() as u64
	}

	fn in_work(&self) -> usize {
		self.places.iter().filter(|place| matches!(place.work, Work::At(_) | Work::Interrupted(_))).count()
	}

	/// Work outranks blocking: blocked only when nothing is in work and a finished MU can't leave.
	fn process_state(&self) -> ProcessState {
		if self.in_work() > 0 {
			ProcessState::Processing
		} else if self.waiting {
			ProcessState::Blocked
		} else {
			ProcessState::Idle
		}
	}

	/// What the statistics count: stopped, then the three parts of busy, partly blocked first.
	fn machine_state(&self) -> MachineState {
		let in_work = self.in_work();
		match self.interruptions.most_serious() {
			None if in_work == 0 => MachineState::of(None, &self.process_state()),
			None if self.waiting => MachineState::BusyPartlyBlocked,
			None if in_work as u64 >= self.capacity => MachineState::Busy,
			None => MachineState::BusyNotFull,
			stopped => MachineState::of(stopped, &ProcessState::Idle),
		}
	}

	fn exit_event(&self, mu_id: u32, after: SimTime) -> EventTemplate {
		EventTemplate { eventtype: EventType::MUExit(ExitTicket { mu_id, stop_count: self.stop_count }), exec_time: after, object_id: self.base.object_id }
	}

	/// Begins work on the place at `at`, and returns its exit.
	fn start_work(&mut self, at: usize, context: &mut EventContext) -> EventTemplate {
		let place = &mut self.places[at];
		place.work = Work::At(context.time + self.proc_time);
		let mu_id = place.mu.id;
		context.record(OccurrenceKind::WorkStarted { mu_id, at: self.base.object_id, duration: self.proc_time });
		self.exit_event(mu_id, self.proc_time)
	}

	/// With batch start and every place taken, begins work on all of them, with one exit, since
	/// they finish together. Otherwise nothing.
	fn start_batch_if_full(&mut self, context: &mut EventContext) -> Option<EventTemplate> {
		if self.start != StartMode::Batch || self.held() < self.capacity || self.batch_closed {
			return None;
		}
		self.batch_closed = true;
		(0..self.places.len()).map(|at| self.start_work(at, context)).reduce(|first, _| first)
	}

	/// Notes whether a finished MU is waiting, reporting the Station's own change into or out of
	/// Blocked, which work outranks.
	fn set_waiting(&mut self, waiting: bool, context: &mut EventContext) {
		let before = self.process_state() == ProcessState::Blocked;
		self.waiting = waiting;
		self.report_blocking(before, context);
	}

	fn report_blocking(&self, was_blocked: bool, context: &mut EventContext) {
		let object_id = self.base.object_id;
		match (was_blocked, self.process_state() == ProcessState::Blocked) {
			(false, true) => context.record(OccurrenceKind::Blocked { object_id }),
			(true, false) => context.record(OccurrenceKind::Unblocked { object_id }),
			_ => {}
		}
	}

	fn handle_event(&mut self, context: &mut EventContext, eventtype: &EventType, others: &mut OtherObjects<'_>) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match *eventtype {
			EventType::MUExit(ticket) => {
				if self.is_interrupted() || ticket.stop_count != self.stop_count {
					return Ok(None);
				}
				let was_blocked = self.process_state() == ProcessState::Blocked;
				let mut finished = false;
				for place in &mut self.places {
					if let Work::At(done) = place.work {
						if done <= context.time {
							place.work = Work::ReadySince(context.time);
							context.record(OccurrenceKind::WorkFinished { mu_id: place.mu.id, at: self.base.object_id });
							finished = true;
						}
					}
				}
				self.report_blocking(was_blocked, context);
				// A Station already waiting in line keeps its place with an earlier MU.
				if !finished || self.waiting {
					return Ok(None);
				}
				self.pass_on(context, others, None)
			}
			EventType::BlockedRetry { from } => {
				if !self.waiting {
					return Ok(None);
				}
				// Interrupted, it can't take the room now, but keeps its place in the line, as if
				// refused (spec issue 122).
				if self.is_interrupted() {
					if let Some(since) = self.offered_since(context.time) {
						others.get_mut(from).add_blocked(self.base.object_id, since, true);
					}
					return Ok(None);
				}
				self.pass_on(context, others, Some(from))
			}
			_ => match self.handle_interruption(eventtype, context, others) {
				Some(outcome) => outcome,
				None => Err(RunTimeError {}),
			},
		}
	}

	/// Passes on finished MUs, earliest finished first, until one is refused: to any successor, or
	/// in answer to a retry only to the one that sent it. Then lets waiting predecessors know if
	/// there is room.
	fn pass_on(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, only: Option<u32>) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		let mut events = Vec::new();
		let mut moved = false;
		let mut refused = false;
		while self.offered().is_some() {
			let held = self.places.len();
			let entered = match only {
				Some(from) => retry_move_to_successor(self, context, others, from),
				None => try_to_move_to_successor(self, context, others),
			}
			.map_err(|_| RunTimeError {})?;
			if self.places.len() == held {
				refused = true;
				break;
			}
			moved = true;
			events.extend(entered);
		}
		// Once, at the end, so passing on one MU and having the next refused isn't reported as
		// blocking ending and beginning again at one instant.
		self.set_waiting(refused, context);
		if moved {
			if self.places.is_empty() {
				self.batch_closed = false;
			}
			if self.is_entrance_allowed() {
				events.extend(retries_for_blocked(&mut self.blockedlist, self.base.object_id).unwrap_or_default());
			}
		}
		Ok((!events.is_empty()).then_some(events))
	}

	/// The finished MU it passes on next: earliest finished first, ties by entry order.
	fn offered(&self) -> Option<usize> {
		let mut best: Option<(usize, SimTime)> = None;
		for (at, place) in self.places.iter().enumerate() {
			if let Work::ReadySince(since) = place.work {
				if best.is_none_or(|(_, earliest)| since < earliest) {
					best = Some((at, since));
				}
			}
		}
		best.map(|(at, _)| at)
	}

	/// The first interruption of a stop freezes the work, of every MU in work at once.
	fn freeze_work(&mut self, context: &mut EventContext) {
		let mut froze = false;
		for place in &mut self.places {
			if let Work::At(done) = place.work {
				place.work = Work::Interrupted(done.saturating_sub(context.time));
				froze = true;
			}
		}
		if froze {
			self.stop_count += 1;
		}
	}

	/// The last interruption has ended: work resumes where it stopped, finished MUs are ready to
	/// leave again from now, and waiting predecessors hear of any room.
	fn resume(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, events: &mut Vec<EventTemplate>) -> Result<(), RunTimeError> {
		let mut exits: Vec<(SimTime, u32)> = Vec::new();
		let mut finished = false;
		for place in &mut self.places {
			match place.work {
				Work::Interrupted(left) => {
					place.work = Work::At(context.time + left);
					if !exits.iter().any(|&(time, _)| time == left) {
						exits.push((left, place.mu.id));
					}
				}
				// Ready to leave since its work finished, which the stop doesn't change.
				Work::ReadySince(_) => finished = true,
				Work::At(_) | Work::Waiting => {}
			}
		}
		events.extend(exits.into_iter().map(|(left, mu_id)| self.exit_event(mu_id, left)));
		if finished {
			// An interrupted Station released nothing, so it tries afresh, as a SingleStation does,
			// keeping its place wherever it is listed.
			if let Some(passed) = self.pass_on(context, others, None)? {
				events.extend(passed);
			}
		} else if self.is_entrance_allowed() {
			events.extend(retries_for_blocked(&mut self.blockedlist, self.base.object_id).unwrap_or_default());
		}
		Ok(())
	}

	fn update_statistics(&mut self, now: SimTime) {
		let state = self.machine_state();
		self.statistics.observe(now, state, self.held());
	}
}

impl MaterialFlow for MultiStation {
	fn is_entrance_allowed(&self) -> bool {
		!self.is_interrupted() && !self.batch_closed && self.held() < self.capacity
	}
	fn is_exit_allowed(&self) -> bool {
		self.waiting && !self.is_interrupted()
	}
	fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered Station {}", mu.id, self.base.object_id);
		let was_blocked = self.process_state() == ProcessState::Blocked;
		self.places.push(Place { mu, work: Work::Waiting });
		self.statistics.count_mu_in();
		let exit = match self.start {
			StartMode::Individual => Some(self.start_work(self.places.len() - 1, context)),
			StartMode::Batch => self.start_batch_if_full(context),
		};
		// Work outranks blocking, so new work ends a Station's blocking.
		self.report_blocking(was_blocked, context);
		self.update_statistics(context.time);
		Ok(exit)
	}
	fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool) {
		list_blocked(&mut self.blockedlist, id_blocked, since, answering_retry);
	}
	fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>) {
		relist_blocked(&mut self.blockedlist, id_blocked, next);
	}
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		Ok(retries_for_blocked(&mut self.blockedlist, self.base.object_id))
	}
}

impl EventInterface for MultiStation {
	/// Its starting MUs count as entered now. With individual start each begins work, and a full
	/// starting batch begins work, while a part of one waits for the rest.
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		let mut events: Vec<EventTemplate> = self.interruptions.first_failure(self.base.object_id).into_iter().collect();
		for _ in &self.places {
			self.statistics.count_mu_in();
		}
		match self.start {
			StartMode::Individual => {
				for at in 0..self.places.len() {
					events.push(self.start_work(at, context));
				}
			}
			StartMode::Batch => events.extend(self.start_batch_if_full(context)),
		}
		self.update_statistics(context.time);
		Ok(events)
	}
	fn execute_event(&mut self, context: &mut EventContext, eventtype: &EventType, others: &mut OtherObjects<'_>) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		let outcome = self.handle_event(context, eventtype, others);
		self.update_statistics(context.time);
		outcome
	}
	fn reset(&mut self) {
		self.proc_time = self.configured_proc_time;
		self.set_starting_mus(self.starting_mus.clone());
		self.stop_count = 0;
		self.batch_closed = false;
		self.waiting = false;
		self.interruptions.reset();
		self.succ_pointer = 0;
		self.blockedlist.clear();
		self.statistics.reset();
	}
	fn clear_statistics(&mut self, now: SimTime) {
		self.statistics.clear(now);
	}
	fn report(&self, now: SimTime) -> ObjectState {
		let work: Vec<WorkLeft> = self
			.places
			.iter()
			.filter_map(|place| match place.work {
				Work::At(done) => Some(WorkLeft { mu_id: place.mu.id, left: done.saturating_sub(now) }),
				Work::Interrupted(left) => Some(WorkLeft { mu_id: place.mu.id, left }),
				_ => None,
			})
			.collect();
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Station,
			contents: self.places.iter().map(|place| place.mu.id).collect(),
			process: self.process_state(),
			interrupted: self.interruptions.most_serious(),
			interruptions: self.interruptions.list(),
			remaining: work.iter().map(|work| work.left).min(),
			work,
			blocked_predecessors: waiting_ids(&self.blockedlist),
			successors: self.succ.clone(),
			capacity: Capacity::Limited(self.capacity),
			process_time: Some(self.proc_time),
			configured_process_time: Some(self.configured_proc_time),
			statistics: self.statistics.read(now),
			fill: None,
		}
	}
	fn get_object_id(&self) -> &u32 {
		&self.base.object_id
	}
	fn set_object_id(&mut self, id: u32) {
		self.base.object_id = id;
		for place in &mut self.places {
			place.mu.location_id = id;
		}
	}
	fn set_random_seed(&mut self, seed: u64) {
		self.base.random_seed = seed;
		self.interruptions.reseed(seed);
	}
}

impl Interruptible for MultiStation {
	fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32) {
		(&mut self.interruptions, &mut self.statistics, self.base.object_id)
	}
	fn stop_began(&mut self, context: &mut EventContext) {
		self.freeze_work(context);
	}
	fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, events: &mut Vec<EventTemplate>) -> Result<(), RunTimeError> {
		self.resume(context, others, events)
	}
}

impl SuccessorContainer for MultiStation {
	fn distribution_mut(&mut self) -> &mut Distributing {
		&mut self.distribution
	}
	fn successors(&self) -> &[u32] {
		&self.succ
	}
	fn succ_pointer(&self) -> usize {
		self.succ_pointer
	}
	fn succ_pointer_mut(&mut self) -> &mut usize {
		&mut self.succ_pointer
	}
	fn clear_content_and_idle(&mut self) {
		self.places.clear();
		self.waiting = false;
		self.batch_closed = false;
	}
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
	fn take_content(&mut self, _now: SimTime) -> Option<MoveableUnit> {
		let at = self.offered()?;
		Some(self.places.remove(at).mu)
	}
	fn offered_since(&self, _now: SimTime) -> Option<SimTime> {
		self.offered().map(|at| match self.places[at].work {
			Work::ReadySince(since) => since,
			_ => SimTime::ZERO,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::model::ModelDescription;
	use crate::report::Recording;
	use crate::{demo, ModelObject, SingleStation};

	/// Runs `model` with every occurrence recorded, interrupting Stations 1 and 2 on the way,
	/// and returns what it recorded and its state at the end.
	fn run(model: &ModelDescription, seed: u64) -> (Vec<crate::report::Occurrence>, crate::report::RunState) {
		let mut run = model.build(seed);
		run.set_recording(Recording::ALL);
		let mut occurrences = run.init().unwrap();
		let at = |secs| SimTime::from_secs(secs);
		for (time, event, id) in [
			(at(3_600), EventType::PauseOn, 1),
			(at(3_700), EventType::StopOn, 1),
			(at(3_800), EventType::PauseOff, 1),
			(at(4_000), EventType::StopOff, 1),
			(at(7_000), EventType::PauseOn, 2),
			(at(7_000), EventType::PauseOff, 2),
			(at(9_000), EventType::StopOn, 9),
			(at(12_000), EventType::StopOff, 9),
		] {
			occurrences.extend(run.advance_to(time).map_err(|failure| failure.error).unwrap());
			run.schedule_event(time, event, id).unwrap();
		}
		occurrences.extend(run.advance_to(SimTime::from_secs(5 * 86_400)).map_err(|failure| failure.error).unwrap());
		(occurrences, run.state())
	}

	#[test]
	fn with_capacity_1_it_does_exactly_what_a_single_station_does() {
		let model = demo::model();
		let general = model.with_general_stations();
		assert!(general.build(0).object(1).is_some_and(|object| matches!(object, ModelObject::MultiStation(_))));
		for seed in [0, 7, 123] {
			let (expected_occurrences, expected_state) = run(&model, seed);
			let (occurrences, state) = run(&general, seed);
			assert_eq!(occurrences.len(), expected_occurrences.len(), "seed {seed}");
			assert!(occurrences == expected_occurrences, "seed {seed}: the occurrences differ");
			assert_eq!(state, expected_state, "seed {seed}");
		}
	}

	#[test]
	fn a_station_with_capacity_is_no_larger_than_a_single_station() {
		// Every object in a run takes the size of the largest, so a Station with capacity mustn't
		// grow every object, and the statistics' new fields mustn't grow a SingleStation.
		assert!(size_of::<MultiStation>() <= size_of::<SingleStation>(), "{} bytes", size_of::<MultiStation>());
		// 1,280 until a SingleStation's failure profile moved into its boxed `Interruptions`, and
		// 1,080 until a Buffer's fill keeper was boxed: a Buffer is the largest object.
		assert_eq!(size_of::<ModelObject>(), 584);
	}
}
