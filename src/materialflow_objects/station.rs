use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::observation::{ObjectKind, ObjectState, OccurrenceKind, StopCause};
use std::time::Duration;
use super::flows::BaseObject;
use crate::flows::{
	EventInterface, StopCtrl, FailureProfile, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, list_blocked, retries_for_blocked, try_to_move_to_successor
};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct SingleStation {
	pub base: BaseObject,
	pub proc_time: Duration,
	pub completion_time: Option<Duration>,
	pub interrupted_remain_proc_time: Option<Duration>,
	pub stop_count: u32,
	pub failure_profile: FailureProfile,
	pub starting_content: Option<MoveableUnit>,
	pub content: Option<MoveableUnit>,
	pub succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	pub succ_pointer: usize,
	pub pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	pub blockedlist: Vec<u32>,
	pub entrancelocked: bool,
	pub exitlocked: bool,
	pub state: ProcessState,
	pub interruption: InterruptionState,
}

impl SingleStation {
	/// `content` is the station's starting MU: part of its default state, worked on from Init.
	pub fn new(
		object_id: u32,
		proc_time: Duration,
		failure_profile: FailureProfile,
		content: Option<MoveableUnit>,
		succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
		pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	) -> SingleStation {
		SingleStation {
			base: BaseObject { object_id, random_seed: 0 },
			proc_time,
			completion_time: None,
			interrupted_remain_proc_time: None,
			stop_count: 0,
			failure_profile,
			starting_content: content.clone(),
			content,
			succ,
			succ_pointer: 0,
			pred,
			blockedlist: Vec::new(),
			entrancelocked: false,
			exitlocked: false,
			state: ProcessState::Idle,
			interruption: InterruptionState::None,
		}
	}
	fn exit_event(&self, mu_id: u32, after: Duration) -> EventTemplate {
		EventTemplate {
			eventtype: EventType::MUExit(ExitTicket { mu_id, stop_count: self.stop_count }),
			exec_time: after,
			object_id: self.base.object_id,
			seq: 0,
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
		context.record(OccurrenceKind::WorkStarted { mu_id, at: self.base.object_id });
		self.exit_event(mu_id, self.proc_time)
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
			ProcessState::Idle => matches!(self.interruption, InterruptionState::None),
			ProcessState::Processing | ProcessState::Blocked => false,
		}
	}
	fn is_exit_allowed(&self) -> bool {
		if self.exitlocked {
			return false;
		}
		match self.state {
			// Idle: allow exit when not interrupted
			ProcessState::Idle => matches!(self.interruption, InterruptionState::None),
			// Processing: actively working on an MU, do not allow exit
			ProcessState::Processing => false,
			// Blocked: allow exit when not interrupted
			ProcessState::Blocked => matches!(self.interruption, InterruptionState::None),
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
		Ok(Some(self.start_work(mu_id, context)))
	}
	fn add_blocked(&mut self, id_blocked: u32) {
		list_blocked(&mut self.blockedlist, id_blocked);
	}
	fn remove_blocked(&mut self, id_blocked: u32) {
		self.blockedlist.retain(|&id| id != id_blocked);
	}
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		Ok(retries_for_blocked(&mut self.blockedlist))
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
					seq: 0,
				});
			}
		}
		if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
			events.push(self.start_work(mu_id, context));
		}
		Ok(events)
	}
	fn execute_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			&EventType::MUExit(ticket) => {
				if !matches!(self.interruption, InterruptionState::None) || self.is_stale_exit(ticket) {
					return Ok(None);
				}
				self.completion_time = None;
				if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
					context.record(OccurrenceKind::WorkFinished { mu_id, at: self.base.object_id });
				}
				match try_to_move_to_successor(self, context) {
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
			&EventType::BlockedRetry => {
				// An empty station can still get a retry already queued when its MU left. It
				// has nothing to move, so it must not list itself as waiting.
				if !self.is_exit_allowed() || self.content.is_none() {
					return Ok(None);
				}
				match try_to_move_to_successor(self, context) {
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
				if matches!(self.interruption, InterruptionState::Failed) {
					return Ok(None);
				}
				self.interruption = InterruptionState::Failed;
				context.record(OccurrenceKind::StopBegan { object_id: self.base.object_id, cause: StopCause::Failure });
				if matches!(self.state, ProcessState::Processing) {
					let remaining = self.completion_time.take().unwrap_or(Duration::ZERO).saturating_sub(context.time);
					self.interrupted_remain_proc_time = Some(remaining);
					self.stop_count += 1;
				}
				let mttr = Duration::from_millis(
					self.failure_profile.dist_sampler_mttr.sample() as u64,
				);
				Ok(Some(vec![EventTemplate {
					eventtype: EventType::EndFailure,
					exec_time: mttr,
					object_id: self.base.object_id,
					seq: 0,
				}]))
			}
			&EventType::EndFailure => {
				self.interruption = InterruptionState::None;
				context.record(OccurrenceKind::StopEnded { object_id: self.base.object_id, cause: StopCause::Failure });
				let mut events: Vec<EventTemplate> = Vec::new();
				// Schedule next failure
				if let Ok(Some(ttf)) = self.failure_profile.get_time_to_next_failure() {
					events.push(EventTemplate {
						eventtype: EventType::StartFailure,
						exec_time: ttf,
						object_id: self.base.object_id,
						seq: 0,
					});
				}
				if let Some(remain) = self.interrupted_remain_proc_time.take() {
					// Resume interrupted processing
					self.completion_time = Some(context.time + remain);
					self.state = ProcessState::Processing;
					if let Some(mu_id) = self.content.as_ref().map(|mu| mu.id) {
						events.push(self.exit_event(mu_id, remain));
					}
				} else if self.content.is_some() {
					// Blocked when failure hit (MU done, waiting to exit) — retry move
					match try_to_move_to_successor(self, context) {
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
					// Idle when failure hit — notify predecessors we're available again
					self.set_state(ProcessState::Idle, context);
					if let Ok(Some(be)) = self.notify_blocked_predecessors() {
						events.extend(be);
					}
				}
				if events.is_empty() { Ok(None) } else { Ok(Some(events)) }
			}
			_ => Err(RunTimeError {}),
		}
	}
	fn reset(&mut self) {
		self.content = self.starting_content.clone();
		self.completion_time = None;
		self.interrupted_remain_proc_time = None;
		self.stop_count = 0;
		self.succ_pointer = 0;
		self.blockedlist.clear();
		self.entrancelocked = false;
		self.exitlocked = false;
		self.state = ProcessState::Idle;
		self.interruption = InterruptionState::None;
	}
	fn report(&self, now: Duration) -> ObjectState {
		let remaining = self.interrupted_remain_proc_time
			.or_else(|| self.completion_time.map(|completion| completion.saturating_sub(now)));
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Station,
			contents: self.content.iter().map(|mu| mu.id).collect(),
			process: self.state.clone(),
			stop: self.interruption.cause(),
			remaining,
			blocked_predecessors: self.blockedlist.clone(),
			successors: self.succ.iter().map(|s| *s.borrow().get_object_id()).collect(),
			mus_left: 0,
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
	fn as_any(&self) -> &dyn std::any::Any {
		self
	}
}
impl StopCtrl for SingleStation{
	fn set_stop(&mut self, _failure_start: &Duration, reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
		self.interruption = reason;

		todo!("not fully implemented yet")
	}
	fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
		todo!("not fully implemented yet")
	}
}
impl SuccessorContainer for SingleStation {
	fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>> {
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
	fn take_content(&mut self) -> Option<MoveableUnit> {
		self.content.take()
	}
}
