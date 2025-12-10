use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventTemplate, EventType};
use std::time::Duration;
use crate::flows::{
	EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, try_to_move_to_successor,
};
use crate::simulator::Simulator;
use std::{cell::RefCell, rc::Rc};
use super::flows::BaseObject;

pub fn handle_source_mu_exit(
	source: &mut Source,
	_simulator: &Simulator,
) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
	let event_template_result = try_to_move_to_successor(source, _simulator);
	match event_template_result {
		Ok(opt_evt) => {
			if source.content.is_none() {
				let next_mu_event = match source.create_mu() {
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
				source.state = ProcessState::Blocked;
				Ok(None)
			}
		}
		Err(_) => Err(RunTimeError {}),
	}
}

pub struct Source {
	pub base: BaseObject,
	pub content: Option<MoveableUnit>,
	pub proc_time: Duration,
	pub remain_proc_time: Duration,
	pub succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	pub succ_pointer: usize,
	pub state: ProcessState,
	pub interruption: InterruptionState,
	pub mu_id: u32,
}
impl Source {
	pub fn new(
		object_id: u32,
		proc_time: Duration,
		succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	) -> Source {
		Source {
			base: BaseObject { object_id, random_seed: 0 },
			content: None,
			proc_time,
			remain_proc_time: Duration::from_millis(0),
			succ,
			succ_pointer: 0,
			state: ProcessState::Idle,
			interruption: InterruptionState::None,
			mu_id: 0,
		}
	}
	pub fn create_mu(&mut self) -> Result<EventTemplate, MaterialflowError> {
		self.mu_id += 1;
		if self.content.is_none() {
			self.content = Some(MoveableUnit {
				id: self.mu_id,
				location_id: self.base.object_id,
			});
			log::info!("Source {} created MU {}", self.base.object_id, self.mu_id);
			self.state = ProcessState::Processing;
			// set remaining processing time when a new MU is created
			self.remain_proc_time = self.proc_time;
			return Ok(EventTemplate {
				eventtype: EventType::MUExit,
				exec_time: self.proc_time,
				object_id: self.base.object_id,
				seq: 0,
			});
		}
		Err(MaterialflowError::NoMU)
	}
}

impl EventInterface for Source {
	fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
		match self.create_mu() {
			Ok(e) => Ok(vec![e]),
			Err(_) => Err(InitError {}),
		}
	}
	fn execute_event(
		&mut self,
		_simulator: &mut Simulator,
		eventtype: &EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			&EventType::MUExit => handle_source_mu_exit(self, _simulator),
			&EventType::BlockedRetry => {
				// successor notified that it became available; attempt to move
				if !self.is_exit_allowed() {
					return Ok(None);
				}
				match try_to_move_to_successor(self, _simulator) {
					Ok(Some(ev)) => {
						// If the MU was moved and the source is now empty, create the next MU
						if self.content.is_none() {
							match self.create_mu() {
								Ok(next_ev) => return Ok(Some(vec![next_ev, ev])),
								Err(_) => return Ok(Some(vec![ev])),
							}
						}
						Ok(Some(vec![ev]))
					}
					Ok(None) => {
						// No immediate event from successor. If the source became empty, create next MU
						if self.content.is_none() {
							match self.create_mu() {
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
	fn get_object_id(&self) -> &u32 {
		&self.base.object_id
	}
	fn set_object_id(&mut self, id: u32) {
		self.base.object_id = id;
	}
	fn set_random_seed(&mut self, seed: u64) {
		self.base.random_seed = seed;
	}
	fn as_any(&self) -> &dyn std::any::Any {
		self
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
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		panic!("Source should not be processing MU entrance")
	}
	fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
		match self.interruption {
			InterruptionState::None => Ok(None),
			_ => {
				self.state = ProcessState::Blocked;
				Ok(None)
			}
		}
	}
	fn add_blocked(&mut self, _id_blocked: u32) {
		panic!("Source cannot receive blocked objects")
	}
	fn as_any(&mut self) -> &mut dyn std::any::Any {
		self
	}
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		//source has no predecessors
		Ok(None)
	}
}

impl SuccessorContainer for Source {
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
		self.state = ProcessState::Idle
	}
	fn take_content(&mut self) -> Option<MoveableUnit> {
		self.content.take()
	}
}
