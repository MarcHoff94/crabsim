use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind, Interruption, WorkLeft};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;
use super::flows::BaseObject;
use crate::materialflow_objects::OtherObjects;
use super::interruptions::{Interruptible, Interruptions};
use crate::flows::{
	EventInterface, FailureProfile, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, Distributing, Waiting, list_blocked, relist_blocked, waiting_ids, retries_for_blocked, retry_move_to_successor, try_to_move_to_successor
};

/// When a Station's work ends, or ended, or how much is left of it. One field for all three, since
/// a Station is only ever in one of them, and a Station is the largest object: every object in a
/// run takes its size, so 16 more bytes cost the 6,006-object benchmark about 4% on a 12 MB cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
	/// Not working, and no finished MU waiting.
	Unknown,
	/// Working, done at this time.
	At(SimTime),
	/// Stopped mid-work, with this much work left.
	Interrupted(SimTime),
	/// Holding a finished MU, ready to leave since this time, when its work finished. An
	/// interruption doesn't change it, so the Station keeps its place in line (spec issue 122).
	ReadySince(SimTime),
}

#[derive(Clone)]
pub struct SingleStation {
	pub base: BaseObject,
	pub proc_time: SimTime,
	/// The process time in the default state. Reset returns `proc_time` to it.
	pub configured_proc_time: SimTime,
	finish: Finish,
	pub stop_count: u32,
	/// Its failures and pauses (ADR 0020).
	pub interruptions: Interruptions,
	pub starting_content: Option<MoveableUnit>,
	pub content: Option<MoveableUnit>,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	/// How it picks the successor for its MU. Set by the run, which ranks the successors.
	pub distribution: Distributing,
	pub blockedlist: Vec<Waiting>,
	pub entrancelocked: bool,
	pub exitlocked: bool,
	pub state: ProcessState,
	pub statistics: StatisticsKeeper,
}

impl SingleStation {
	/// `content` is the station's starting MU: part of its default state, worked on from Init.
	pub fn new(
		object_id: u32,
		proc_time: SimTime,
		failure_profile: FailureProfile,
		content: Option<MoveableUnit>,
		succ: Vec<u32>,
	) -> SingleStation {
		SingleStation {
			base: BaseObject { object_id, random_seed: 0 },
			proc_time,
			configured_proc_time: proc_time,
			finish: Finish::Unknown,
			stop_count: 0,
			interruptions: Interruptions::new(Some(failure_profile)),
			starting_content: content.clone(),
			content,
			succ,
			succ_pointer: 0,
			distribution: Distributing::RoundRobin,
			blockedlist: Vec::new(),
			entrancelocked: false,
			exitlocked: false,
			state: ProcessState::Idle,
			statistics: StatisticsKeeper::new(1),
		}
	}
	fn exit_event(&self, mu_id: u32, after: SimTime) -> EventTemplate {
		EventTemplate {
			eventtype: EventType::MUExit(ExitTicket { mu_id, stop_count: self.stop_count }),
			exec_time: after,
			object_id: self.base.object_id,
		}
	}
	/// An exit is stale if the station has been stopped since it was scheduled,
	/// because resuming schedules a new exit for the moved completion time.
	fn is_stale_exit(&self, ticket: ExitTicket) -> bool {
		ticket.stop_count != self.stop_count
			|| self.content.as_ref().is_some_and(|mu| mu.id != ticket.mu_id)
	}
	/// Begins work on the MU now in the station and returns the exit for its completion time.
	fn start_work(&mut self, mu_id: u32, context: &mut EventContext) -> EventTemplate {
		self.finish = Finish::At(context.time + self.proc_time);
		self.entrancelocked = true;
		self.state = ProcessState::Processing;
		context.record(OccurrenceKind::WorkStarted { mu_id, at: self.base.object_id, duration: self.proc_time });
		self.exit_event(mu_id, self.proc_time)
	}
	fn handle_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			&EventType::MUExit(ticket) => {
				if self.is_interrupted() || self.is_stale_exit(ticket) {
					return Ok(None);
				}
				self.finish = Finish::ReadySince(context.time);
				if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
					context.record(OccurrenceKind::WorkFinished { mu_id, at: self.base.object_id });
				}
				match try_to_move_to_successor(self, context, others) {
					Ok(Some(event_template)) => {
						self.entrancelocked = false;
						self.set_state(ProcessState::Idle, context);
						let blocked_events = self.notify_blocked_predecessors();
						match blocked_events {
							Ok(Some(mut be)) => {
								be.push(event_template);
								Ok(Some(be))
							}
							Ok(None) => Ok(Some(vec![event_template])),
							Err(_e) => Err(RunTimeError {}),
						}
					}
					Ok(None) => {
						if self.content.is_none() {
							self.entrancelocked = false;
							self.set_state(ProcessState::Idle, context);
							let blocked_events = self.notify_blocked_predecessors();
							match blocked_events {
								Ok(Some(be)) => Ok(Some(be)),
								Ok(None) => Ok(None),
								Err(_e) => Err(RunTimeError {}),
							}
						} else {
							self.set_state(ProcessState::Blocked, context);
							self.entrancelocked = true;
							Ok(None)
						}
					}
					Err(_) => Err(RunTimeError {}),
				}
			}
			&EventType::BlockedRetry { from } => {
				// An empty station can still get a retry already queued when its MU left. It
				// has nothing to move, so it must not list itself as waiting.
				if self.content.is_none() {
					return Ok(None);
				}
				// Interrupted with a finished MU, it can't take the room now, but it keeps its
				// place in the line, as if refused (spec issue 122).
				if let (true, Finish::ReadySince(since)) = (self.is_interrupted(), self.finish) {
					others.get_mut(from).add_blocked(self.base.object_id, since, true);
					return Ok(None);
				}
				if !self.is_exit_allowed() {
					return Ok(None);
				}
				match retry_move_to_successor(self, context, others, from) {
					Ok(Some(event_template)) => {
						if self.content.is_none() {
							self.entrancelocked = false;
							self.set_state(ProcessState::Idle, context);
							let blocked_events = self.notify_blocked_predecessors();
							match blocked_events {
								Ok(Some(mut be)) => {
									be.push(event_template);
									Ok(Some(be))
								}
								Ok(None) => Ok(Some(vec![event_template])),
								Err(_e) => Err(RunTimeError {}),
							}
						} else {
							Ok(Some(vec![event_template]))
						}
					}
					Ok(None) => {
						if self.content.is_none() {
							self.entrancelocked = false;
							self.set_state(ProcessState::Idle, context);
							let blocked_events = self.notify_blocked_predecessors();
							match blocked_events {
								Ok(Some(be)) => Ok(Some(be)),
								Ok(None) => Ok(None),
								Err(_e) => Err(RunTimeError {}),
							}
						} else {
							self.set_state(ProcessState::Blocked, context);
							Ok(None)
						}
					}
					Err(_) => Err(RunTimeError {}),
				}
			}
			_ => match self.handle_interruption(eventtype, context, others) {
				Some(outcome) => outcome,
				None => Err(RunTimeError {}),
			},
		}
	}
	/// The first interruption of a stop freezes the work, so time spent stopped counts once
	/// however many overlap.
	fn freeze_work(&mut self, context: &mut EventContext) {
		if matches!(self.state, ProcessState::Processing) {
			let completion = match self.finish {
				Finish::At(completion) => completion,
				_ => SimTime::ZERO,
			};
			self.finish = Finish::Interrupted(completion.saturating_sub(context.time));
			self.stop_count += 1;
		}
	}
	/// The last interruption has ended, so the station picks up from where it was stopped.
	fn resume(
		&mut self,
		context: &mut EventContext,
		others: &mut OtherObjects<'_>,
		events: &mut Vec<EventTemplate>,
	) -> Result<(), RunTimeError> {
		if let Finish::Interrupted(remain) = self.finish {
			// Resume interrupted processing
			self.finish = Finish::At(context.time + remain);
			self.state = ProcessState::Processing;
			if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
				events.push(self.exit_event(mu_id, remain));
			}
		} else if self.content.is_some() {
			// Blocked when the stop began (MU done, waiting to exit): retry the move. Its MU has
			// been ready to leave since its work finished, so it keeps its place in every line it
			// was in, and joins any other by that time too.
			match try_to_move_to_successor(self, context, others) {
				Ok(Some(evt)) => {
					self.entrancelocked = false;
					self.set_state(ProcessState::Idle, context);
					events.push(evt);
					if let Ok(Some(be)) = self.notify_blocked_predecessors() {
						events.extend(be);
					}
				}
				Ok(None) => {
					if self.content.is_none() {
						self.entrancelocked = false;
						self.set_state(ProcessState::Idle, context);
						if let Ok(Some(be)) = self.notify_blocked_predecessors() {
							events.extend(be);
						}
					} else {
						self.set_state(ProcessState::Blocked, context);
					}
				}
				Err(_) => return Err(RunTimeError {}),
			}
		} else {
			// Idle when the stop began: notify predecessors we're available again
			self.set_state(ProcessState::Idle, context);
			if let Ok(Some(be)) = self.notify_blocked_predecessors() {
				events.extend(be);
			}
		}
		Ok(())
	}
	pub fn is_interrupted(&self) -> bool {
		self.interruptions.is_interrupted()
	}
	pub fn is_paused(&self) -> bool {
		self.interruptions.is_active(Interruption::Paused)
	}
	/// Tells the statistics what the station ended up doing.
	fn update_statistics(&mut self, now: SimTime) {
		let state = MachineState::of(self.interruptions.most_serious(), &self.state);
		self.statistics.observe(now, state, self.content.is_some() as u64);
	}
	/// Changes the process state, reporting when blocking begins or ends.
	fn set_state(&mut self, state: ProcessState, context: &mut EventContext) {
		let object_id = self.base.object_id;
		match (self.state == ProcessState::Blocked, state == ProcessState::Blocked) {
			(false, true) => context.record(OccurrenceKind::Blocked { object_id }),
			(true, false) => context.record(OccurrenceKind::Unblocked { object_id }),
			_ => {}
		}
		self.state = state;
	}
}

impl MaterialFlow for SingleStation {
	fn is_entrance_allowed(&self) -> bool {
		if self.entrancelocked {
			return false;
		}
		match self.state {
			ProcessState::Idle => !self.is_interrupted(),
			ProcessState::Processing | ProcessState::Blocked => false,
		}
	}
	fn is_exit_allowed(&self) -> bool {
		if self.exitlocked {
			return false;
		}
		match self.state {
			// Idle: allow exit when not interrupted
			ProcessState::Idle => !self.is_interrupted(),
			// Processing: actively working on an MU, do not allow exit
			ProcessState::Processing => false,
			// Blocked: allow exit when not interrupted
			ProcessState::Blocked => !self.is_interrupted(),
		}
	}
	fn process_mu_entrance(
		&mut self,
		mu: MoveableUnit,
		context: &mut EventContext,
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered SingleStation {}", mu.id, self.base.object_id);
		let mu_id = mu.id;
		self.content = Some(mu);
		self.statistics.count_mu_in();
		let exit = self.start_work(mu_id, context);
		self.update_statistics(context.time);
		Ok(Some(exit))
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

impl EventInterface for SingleStation {
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		let mut events: Vec<EventTemplate> = self.interruptions.first_failure(self.base.object_id).into_iter().collect();
		if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
			self.statistics.count_mu_in();
			events.push(self.start_work(mu_id, context));
		}
		self.update_statistics(context.time);
		Ok(events)
	}
	fn execute_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		let outcome = self.handle_event(context, eventtype, others);
		self.update_statistics(context.time);
		outcome
	}
	fn reset(&mut self) {
		self.proc_time = self.configured_proc_time;
		self.content = self.starting_content.clone();
		self.finish = Finish::Unknown;
		self.stop_count = 0;
		self.succ_pointer = 0;
		self.blockedlist.clear();
		self.entrancelocked = false;
		self.exitlocked = false;
		self.state = ProcessState::Idle;
		self.interruptions.reset();
		self.statistics.reset();
	}
	fn clear_statistics(&mut self, now: SimTime) {
		self.statistics.clear(now);
	}
	fn report(&self, now: SimTime) -> ObjectState {
		let remaining = match self.finish {
			Finish::Interrupted(left) => Some(left),
			Finish::At(completion) => Some(completion.saturating_sub(now)),
			_ => None,
		};
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Station,
			contents: self.content.iter().map(|mu| mu.id).collect(),
			process: self.state.clone(),
			interrupted: self.interruptions.most_serious(),
			interruptions: self.interruptions.list(),
			remaining,
			work: self.content.iter().zip(remaining).map(|(mu, left)| WorkLeft { mu_id: mu.id, left }).collect(),
			blocked_predecessors: waiting_ids(&self.blockedlist),
			successors: self.succ.clone(),
			capacity: Capacity::Limited(self.statistics.capacity()),
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
	}
	fn set_random_seed(&mut self, seed: u64) {
		self.base.random_seed = seed;
		self.interruptions.reseed(seed);
	}
}
impl Interruptible for SingleStation {
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
impl SuccessorContainer for SingleStation {
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
		self.content = None;
		self.entrancelocked = false;
		self.state = ProcessState::Idle;
	}
	fn take_content(&mut self, _now: SimTime) -> Option<MoveableUnit> {
		self.content.take()
	}
	fn offered_since(&self, _now: SimTime) -> Option<SimTime> {
		// A Station offers its MU only once the work on it is finished.
		self.content.as_ref().map(|_| match self.finish {
			Finish::ReadySince(since) => since,
			_ => SimTime::ZERO,
		})
	}
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
}
