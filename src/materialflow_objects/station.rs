use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::report::{Interruption, ObjectKind, ObjectState, OccurrenceKind, StopCause};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;
use super::flows::BaseObject;
use crate::materialflow_objects::OtherObjects;
use crate::flows::{
	EventInterface, StopCtrl, FailureProfile, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, Distributing, list_blocked, retries_for_blocked, retry_move_to_successor, try_to_move_to_successor
};

#[derive(Clone)]
pub struct SingleStation {
	pub base: BaseObject,
	pub proc_time: SimTime,
	/// The process time in the default state. Reset returns `proc_time` to it.
	pub configured_proc_time: SimTime,
	pub completion_time: Option<SimTime>,
	pub interrupted_remain_proc_time: Option<SimTime>,
	pub stop_count: u32,
	pub failure_profile: FailureProfile,
	pub starting_content: Option<MoveableUnit>,
	pub content: Option<MoveableUnit>,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	/// How it picks the successor for its MU. Set by the run, which ranks the successors.
	pub distribution: Distributing,
	pub blockedlist: Vec<u32>,
	pub entrancelocked: bool,
	pub exitlocked: bool,
	pub state: ProcessState,
	failed: bool,
	/// Forced to fail by the user. Kept apart from `failed`, so random failures carry on
	/// underneath: each random failure schedules the next only when it ends.
	forced: bool,
	paused: bool,
	/// When each kind of interruption last began, by `StopCause::index`. Read only while it is
	/// active.
	interrupted_since: [SimTime; 3],
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
			completion_time: None,
			interrupted_remain_proc_time: None,
			stop_count: 0,
			failure_profile,
			starting_content: content.clone(),
			content,
			succ,
			succ_pointer: 0,
			distribution: Distributing::RoundRobin,
			blockedlist: Vec::new(),
			entrancelocked: false,
			exitlocked: false,
			state: ProcessState::Idle,
			failed: false,
			forced: false,
			paused: false,
			interrupted_since: [SimTime::ZERO; 3],
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
		self.completion_time = Some(context.time + self.proc_time);
		self.interrupted_remain_proc_time = None;
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
				if self.is_stopped() || self.is_stale_exit(ticket) {
					return Ok(None);
				}
				self.completion_time = None;
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
				if !self.is_exit_allowed() || self.content.is_none() {
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
			&EventType::StartFailure => {
				if self.failed {
					return Ok(None);
				}
				self.begin_interruption(StopCause::Failure, context);
				self.failed = true;
				self.statistics.count_failure();
				let mttr = SimTime::from_millis_f64(self.failure_profile.dist_sampler_mttr.sample());
				Ok(Some(vec![EventTemplate {
					eventtype: EventType::EndFailure,
					exec_time: mttr,
					object_id: self.base.object_id,
				}]))
			}
			&EventType::EndFailure => {
				self.failed = false;
				context.record(OccurrenceKind::InterruptionEnded { object_id: self.base.object_id, cause: StopCause::Failure });
				let mut events: Vec<EventTemplate> = Vec::new();
				// Schedule next failure
				if let Ok(Some(ttf)) = self.failure_profile.get_time_to_next_failure() {
					events.push(EventTemplate {
						eventtype: EventType::StartFailure,
						exec_time: ttf,
						object_id: self.base.object_id,
					});
				}
				if !self.is_stopped() {
					self.end_stop(context, others, &mut events)?;
				}
				if events.is_empty() { Ok(None) } else { Ok(Some(events)) }
			}
			&EventType::ForceFailureOn => {
				if !self.forced {
					self.begin_interruption(StopCause::ForcedFailure, context);
					self.forced = true;
					self.statistics.count_failure();
				}
				Ok(None)
			}
			&EventType::ForceFailureOff => {
				if !self.forced {
					return Ok(None);
				}
				self.forced = false;
				context.record(OccurrenceKind::InterruptionEnded { object_id: self.base.object_id, cause: StopCause::ForcedFailure });
				let mut events: Vec<EventTemplate> = Vec::new();
				if !self.is_stopped() {
					self.end_stop(context, others, &mut events)?;
				}
				if events.is_empty() { Ok(None) } else { Ok(Some(events)) }
			}
			&EventType::PauseOn => {
				if !self.paused {
					self.begin_interruption(StopCause::Pause, context);
					self.paused = true;
					self.statistics.count_pause();
				}
				Ok(None)
			}
			&EventType::PauseOff => {
				if !self.paused {
					return Ok(None);
				}
				self.paused = false;
				context.record(OccurrenceKind::InterruptionEnded { object_id: self.base.object_id, cause: StopCause::Pause });
				let mut events: Vec<EventTemplate> = Vec::new();
				if !self.is_stopped() {
					self.end_stop(context, others, &mut events)?;
				}
				if events.is_empty() { Ok(None) } else { Ok(Some(events)) }
			}
			_ => Err(RunTimeError {}),
		}
	}
	/// Starts a failure or pause. Only the first interruption of a stop freezes the work, so
	/// time spent stopped counts once however many overlap.
	fn begin_interruption(&mut self, cause: StopCause, context: &mut EventContext) {
		context.record(OccurrenceKind::InterruptionBegan { object_id: self.base.object_id, cause });
		self.interrupted_since[cause.index()] = context.time;
		if self.is_stopped() {
			return;
		}
		self.statistics.count_stop();
		if matches!(self.state, ProcessState::Processing) {
			let remaining = self.completion_time.take().unwrap_or(SimTime::ZERO).saturating_sub(context.time);
			self.interrupted_remain_proc_time = Some(remaining);
			self.stop_count += 1;
		}
	}
	/// The last interruption has ended, so the station picks up from where it was stopped.
	fn end_stop(
		&mut self,
		context: &mut EventContext,
		others: &mut OtherObjects<'_>,
		events: &mut Vec<EventTemplate>,
	) -> Result<(), RunTimeError> {
		if let Some(remain) = self.interrupted_remain_proc_time.take() {
			// Resume interrupted processing
			self.completion_time = Some(context.time + remain);
			self.state = ProcessState::Processing;
			if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
				events.push(self.exit_event(mu_id, remain));
			}
		} else if self.content.is_some() {
			// Blocked when the stop began (MU done, waiting to exit): retry the move
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
	pub fn is_stopped(&self) -> bool {
		self.failed || self.forced || self.paused
	}
	pub fn is_paused(&self) -> bool {
		self.paused
	}
	/// What the station reports while stopped: the most serious active interruption, so a
	/// failure, random or forced, wins over a pause, as in the statistics.
	fn interruption(&self) -> InterruptionState {
		if self.failed {
			InterruptionState::Failed
		} else if self.forced {
			InterruptionState::ForcedFailed
		} else if self.paused {
			InterruptionState::Paused
		} else {
			InterruptionState::None
		}
	}
	/// Every active interruption, most serious first, with when it began.
	fn interruptions(&self) -> Vec<Interruption> {
		[(self.failed, StopCause::Failure), (self.forced, StopCause::ForcedFailure), (self.paused, StopCause::Pause)]
			.into_iter()
			.filter(|&(active, _)| active)
			.map(|(_, cause)| Interruption { cause, since: self.interrupted_since[cause.index()] })
			.collect()
	}
	/// Tells the statistics what the station ended up doing.
	fn update_statistics(&mut self, now: SimTime) {
		let state = MachineState::of(&self.interruption(), &self.state);
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
			ProcessState::Idle => !self.is_stopped(),
			ProcessState::Processing | ProcessState::Blocked => false,
		}
	}
	fn is_exit_allowed(&self) -> bool {
		if self.exitlocked {
			return false;
		}
		match self.state {
			// Idle: allow exit when not interrupted
			ProcessState::Idle => !self.is_stopped(),
			// Processing: actively working on an MU, do not allow exit
			ProcessState::Processing => false,
			// Blocked: allow exit when not interrupted
			ProcessState::Blocked => !self.is_stopped(),
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
	fn add_blocked(&mut self, id_blocked: u32) {
		list_blocked(&mut self.blockedlist, id_blocked);
	}
	fn remove_blocked(&mut self, id_blocked: u32) {
		self.blockedlist.retain(|&id| id != id_blocked);
	}
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		Ok(retries_for_blocked(&mut self.blockedlist, self.base.object_id))
	}
}

impl EventInterface for SingleStation {
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		let mut events = Vec::new();
		if self.failure_profile.active {
			if let Ok(Some(ttf)) = self.failure_profile.get_time_to_next_failure() {
				events.push(EventTemplate {
					eventtype: EventType::StartFailure,
					exec_time: ttf,
					object_id: self.base.object_id,
				});
			}
		}
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
		self.completion_time = None;
		self.interrupted_remain_proc_time = None;
		self.stop_count = 0;
		self.succ_pointer = 0;
		self.blockedlist.clear();
		self.entrancelocked = false;
		self.exitlocked = false;
		self.state = ProcessState::Idle;
		self.failed = false;
		self.forced = false;
		self.paused = false;
		self.interrupted_since = [SimTime::ZERO; 3];
		self.statistics.reset();
	}
	fn clear_statistics(&mut self, now: SimTime) {
		self.statistics.clear(now);
	}
	fn report(&self, now: SimTime) -> ObjectState {
		let remaining = self.interrupted_remain_proc_time
			.or_else(|| self.completion_time.map(|completion| completion.saturating_sub(now)));
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Station,
			contents: self.content.iter().map(|mu| mu.id).collect(),
			process: self.state.clone(),
			stop: self.interruption().cause(),
			interruptions: self.interruptions(),
			remaining,
			blocked_predecessors: self.blockedlist.clone(),
			successors: self.succ.clone(),
			capacity: self.statistics.capacity(),
			process_time: Some(self.proc_time),
			configured_process_time: Some(self.configured_proc_time),
			statistics: self.statistics.read(now),
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
		self.failure_profile.reseed(seed);
	}
}
impl StopCtrl for SingleStation{
	fn set_stop(&mut self, _failure_start: &SimTime, _reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
		todo!("not fully implemented yet")
	}
	fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
		todo!("not fully implemented yet")
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
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
}
