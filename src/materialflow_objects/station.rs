use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EcTime, EventTemplate, EventType};
use crate::flows::{
	EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer,
};
use crate::simulator::Simulator;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct SingleStation {
	pub object_id: u32,
	pub proc_time: EcTime,
	pub remain_proc_time: EcTime,
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
	pub fn new(
		object_id: u32,
		proc_time: EcTime,
		content: Option<MoveableUnit>,
		succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
		pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	) -> SingleStation {
		SingleStation {
			object_id,
			proc_time,
			remain_proc_time: 0.into(),
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
	pub fn add_blocked(&mut self, blocked_id: u32) {
		if !self.blockedlist.contains(&blocked_id) {
			log::debug!(
				"{}: adding blocked predecessor {}",
				self.object_id,
				blocked_id
			);
			self.blockedlist.push(blocked_id);
		} else {
			log::debug!(
				"{}: predecessor {} already in blockedlist",
				self.object_id,
				blocked_id
			);
		}
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
			// Blocked: previous attempt to exit after processing found no successor; allow
			// subsequent exit attempts (e.g., on BlockedRetry notifications).
			ProcessState::Blocked => matches!(self.interruption, InterruptionState::None),
		}
	}
	fn process_mu_entrance(
		&mut self,
		_mu: MoveableUnit,
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered SingleStation {}", _mu.id, self.object_id);
		self.content = Some(_mu);
		self.remain_proc_time = self.proc_time;
		self.entrancelocked = true;
		self.state = ProcessState::Processing;
		Ok(Some(EventTemplate {
			eventtype: EventType::MUExit,
			exec_time: self.proc_time,
			object_id: self.object_id,
			seq: 0,
		}))
	}
	fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError> {
		match self.interruption {
			InterruptionState::None => Ok(None),
			InterruptionState::Failed | InterruptionState::Paused => {
				self.state = ProcessState::Blocked;
				Ok(None)
			}
		}
	}
	fn add_blocked(&mut self, id_blocked: u32) {
		self.blockedlist.push(id_blocked);
	}
	fn as_any(&mut self) -> &mut dyn std::any::Any {
		self
	}
}

impl EventInterface for SingleStation {
	fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
		Ok(Vec::new())
	}
	fn execute_event(
		&mut self,
		_simulator: &mut Simulator,
		eventtype: EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			EventType::MUExit => match crate::flows::try_to_move_to_successor(self, _simulator) {
				Ok(Some(event_template)) => {
					// Move succeeded and produced an event template.
					self.entrancelocked = false;
					self.state = ProcessState::Idle;
					let mut blocked_events = Vec::new();
					while let Some(blocked_id) = self.blockedlist.pop() {
						blocked_events.push(EventTemplate {
							eventtype: EventType::BlockedRetry,
							exec_time: 0.into(),
							object_id: blocked_id,
							seq: 0,
						});
					}
					blocked_events.push(event_template);
					Ok(Some(blocked_events))
				}
				Ok(None) => {
					// Ok(None) can mean either: (A) the MU was moved but no
					// immediate event was returned by the successor, or (B)
					// no successor could accept the MU and it remains here.
					if self.content.is_none() {
						// MU was moved (no event). Station becomes idle and
						// notify any blocked predecessors.
						self.entrancelocked = false;
						self.state = ProcessState::Idle;
						let mut blocked_events = Vec::new();
						while let Some(blocked_id) = self.blockedlist.pop() {
							blocked_events.push(EventTemplate {
								eventtype: EventType::BlockedRetry,
								exec_time: 0.into(),
								object_id: blocked_id,
								seq: 0,
							});
						}
						if !blocked_events.is_empty() {
							Ok(Some(blocked_events))
						} else {
							Ok(None)
						}
					} else {
						// MU could not be moved to any successor: remain blocked.
						self.state = ProcessState::Blocked;
						// ensure entrance stays locked to prevent additional MU
						// entrances while this MU waits for successors to free up.
						self.entrancelocked = true;
						Ok(None)
					}
				}
				Err(_) => Err(RunTimeError {}),
			},
			EventType::BlockedRetry => {
				// BlockedRetry is a notification from a successor that the
				// successor has become available and requests the predecessor
				// to attempt to move its MU. Use `is_exit_allowed()` to decide
				// whether an exit/transfer may be attempted. Note: by design
				// `is_exit_allowed` should not check `self.state`.
				if !self.is_exit_allowed() {
					return Ok(None);
				}
				match crate::flows::try_to_move_to_successor(self, _simulator) {
					Ok(Some(ev)) => Ok(Some(vec![ev])),
					Ok(None) => Ok(None),
					Err(_) => Err(RunTimeError {}),
				}
			}
			_ => Err(RunTimeError {}),
		}
	}
	fn get_object_id(&self) -> &u32 {
		&self.object_id
	}
	fn set_object_id(&mut self, id: u32) {
		self.object_id = id;
	}
	fn as_any(&self) -> &dyn std::any::Any {
		self
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

