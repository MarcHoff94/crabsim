use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::report::{ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;
use crate::materialflow_objects::OtherObjects;
use crate::flows::{
	EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, StopCtrl, SuccessorContainer, retry_move_to_successor, try_to_move_to_successor,
};
use super::flows::BaseObject;

pub fn handle_source_mu_exit(
	source: &mut Source,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
	let event_template_result = try_to_move_to_successor(source, context, others);
	match event_template_result {
		Ok(opt_evt) => {
			if source.content.is_none() {
				let next_mu_event = match source.create_mu(context) {
					Ok(v) => v,
					Err(_) => return Err(RunTimeError {}),
				};
				if let Some(ev) = opt_evt {
					return Ok(Some(vec![next_mu_event, ev]));
				} else {
					return Ok(Some(vec![next_mu_event]));
				}
			}
			if let Some(ev) = opt_evt {
				Ok(Some(vec![ev]))
			} else {
				source.become_blocked(context);
				Ok(None)
			}
		}
		Err(_) => Err(RunTimeError {}),
	}
}

#[derive(Clone)]
pub struct Source {
	pub base: BaseObject,
	pub content: Option<MoveableUnit>,
	pub proc_time: SimTime,
	pub completion_time: Option<SimTime>,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	pub state: ProcessState,
	pub interruption: InterruptionState,
	pub statistics: StatisticsKeeper,
}
impl Source {
	pub fn new(
		object_id: u32,
		proc_time: SimTime,
		succ: Vec<u32>,
	) -> Source {
		Source {
			base: BaseObject { object_id, random_seed: 0 },
			content: None,
			proc_time,
			completion_time: None,
			succ,
			succ_pointer: 0,
			state: ProcessState::Idle,
			interruption: InterruptionState::None,
			statistics: StatisticsKeeper::new(0),
		}
	}
	pub fn create_mu(&mut self, context: &mut EventContext) -> Result<EventTemplate, MaterialflowError> {
		if self.content.is_some() {
			return Err(MaterialflowError::NoMU);
		}
		let mu_id = context.new_mu_id();
		self.content = Some(MoveableUnit {
			id: mu_id,
			location_id: self.base.object_id,
			left_source: context.time,
		});
		self.statistics.count_mu_in();
		log::info!("Source {} created MU {}", self.base.object_id, mu_id);
		context.record(OccurrenceKind::MuCreated { mu_id, at: self.base.object_id, duration: self.proc_time });
		if self.state == ProcessState::Blocked {
			context.record(OccurrenceKind::Unblocked { object_id: self.base.object_id });
		}
		self.state = ProcessState::Processing;
		self.completion_time = Some(context.time + self.proc_time);
		Ok(EventTemplate {
			eventtype: EventType::MUExit(ExitTicket::new(mu_id)),
			exec_time: self.proc_time,
			object_id: self.base.object_id,
		})
	}
	fn handle_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			&EventType::MUExit(_) => handle_source_mu_exit(self, context, others),
			&EventType::BlockedRetry { from } => {
				// successor notified that it became available; attempt to move
				if !self.is_exit_allowed() {
					return Ok(None);
				}
				match retry_move_to_successor(self, context, others, from) {
					Ok(Some(ev)) => {
						// If the MU was moved and the source is now empty, create the next MU
						if self.content.is_none() {
							match self.create_mu(context) {
								Ok(next_ev) => return Ok(Some(vec![next_ev, ev])),
								Err(_) => return Ok(Some(vec![ev])),
							}
						}
						Ok(Some(vec![ev]))
					}
					Ok(None) => {
						// No immediate event from successor. If the source became empty, create next MU
						if self.content.is_none() {
							match self.create_mu(context) {
								Ok(next_ev) => return Ok(Some(vec![next_ev])),
								Err(_) => return Ok(None),
							}
						}
						Ok(None)
					}
					Err(_) => Err(RunTimeError {}),
				}
			}
			_ => Err(RunTimeError {}),
		}
	}
	/// Tells the statistics what the Source ended up doing. It has no MU-level statistics.
	fn update_statistics(&mut self, now: SimTime) {
		self.statistics.observe(now, MachineState::of(&self.interruption, &self.state), 0);
	}
	fn become_blocked(&mut self, context: &mut EventContext) {
		if self.state != ProcessState::Blocked {
			context.record(OccurrenceKind::Blocked { object_id: self.base.object_id });
		}
		self.state = ProcessState::Blocked;
		self.completion_time = None;
	}
}

impl EventInterface for Source {
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		let outcome = match self.create_mu(context) {
			Ok(e) => Ok(vec![e]),
			Err(_) => Err(InitError {}),
		};
		self.update_statistics(context.time);
		outcome
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
		self.content = None;
		self.completion_time = None;
		self.succ_pointer = 0;
		self.state = ProcessState::Idle;
		self.interruption = InterruptionState::None;
		self.statistics.reset();
	}
	fn clear_statistics(&mut self, now: SimTime) {
		self.statistics.clear(now);
	}
	fn report(&self, now: SimTime) -> ObjectState {
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Source,
			contents: self.content.iter().map(|mu| mu.id).collect(),
			process: self.state.clone(),
			stop: self.interruption.cause(),
			// A Source can't be interrupted yet: its stops aren't built.
			interruptions: Vec::new(),
			remaining: self.completion_time.map(|completion| completion.saturating_sub(now)),
			blocked_predecessors: Vec::new(),
			successors: self.succ.clone(),
			capacity: self.statistics.capacity(),
			process_time: None,
			configured_process_time: None,
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
	}
}
impl StopCtrl for Source {
	fn set_stop(&mut self, _failure_start: &SimTime, _reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError> {
		todo!("StopCtrl is not implemented for Source yet")
	}
	fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
		todo!("StopCtrl is not implemented for Source yet")
	}
}

impl MaterialFlow for Source {
	fn is_exit_allowed(&self) -> bool {
		true
	}
	fn is_entrance_allowed(&self) -> bool {
		false
	}
	fn process_mu_entrance(
		&mut self,
		_mu: MoveableUnit,
		_context: &mut EventContext,
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		panic!("Source should not be processing MU entrance")
	}
	fn add_blocked(&mut self, _id_blocked: u32) {
		panic!("Source cannot receive blocked objects")
	}
	fn remove_blocked(&mut self, _id_blocked: u32) {}
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		//source has no predecessors
		Ok(None)
	}
}

impl SuccessorContainer for Source {
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
		self.state = ProcessState::Idle
	}
	/// An MU created at the start of an interval leaves at the end of it, and its lead time
	/// starts here. Time spent blocked in the Source is the Source's blocked time.
	fn take_content(&mut self, now: SimTime) -> Option<MoveableUnit> {
		let mut mu = self.content.take()?;
		mu.left_source = now;
		Some(mu)
	}
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
}
