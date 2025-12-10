use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventTemplate, EventType};
use std::time::Duration;
use super::flows::BaseObject;
use crate::flows::{
	EventInterface, InterruptionState, MaterialFlow, MoveableUnit, ProcessState, SuccessorContainer, try_to_move_to_successor
};
use crate::simulator::Simulator;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub struct SingleStation {
	pub base: BaseObject,
	pub proc_time: Duration,
	pub remain_proc_time: Duration,
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
		proc_time: Duration,
		content: Option<MoveableUnit>,
		succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
		pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
	) -> SingleStation {
		SingleStation {
			base: BaseObject { object_id, random_seed: 0 },
			proc_time,
			remain_proc_time: Duration::from_millis(0),
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
				self.base.object_id,
				blocked_id
			);
			self.blockedlist.push(blocked_id);
		} else {
			log::debug!(
				"{}: predecessor {} already in blockedlist",
				self.base.object_id,
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
			// Blocked: allow exit when not interrupted
			ProcessState::Blocked => matches!(self.interruption, InterruptionState::None),
		}
	}
	fn process_mu_entrance(
		&mut self,
		_mu: MoveableUnit,
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered SingleStation {}", _mu.id, self.base.object_id);
		self.content = Some(_mu);
		self.remain_proc_time = self.proc_time;
		self.entrancelocked = true;
		self.state = ProcessState::Processing;
		Ok(Some(EventTemplate {
			eventtype: EventType::MUExit,
			exec_time: self.proc_time,
			object_id: self.base.object_id,
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
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
		let mut blocked_events = Vec::new();
		while let Some(blocked_id) = self.blockedlist.pop() {
			blocked_events.push(EventTemplate {
				eventtype: EventType::BlockedRetry,
				exec_time: Duration::from_millis(0),
				object_id: blocked_id,
				seq: 0,
			});
		}
		if blocked_events.is_empty() {
			Ok(None)
		} else {
			Ok(Some(blocked_events))
		}
	}
}

impl EventInterface for SingleStation {
	fn init(&mut self) -> Result<Vec<EventTemplate>, InitError> {
		Ok(Vec::new())
	}
	fn execute_event(
		&mut self,
		_simulator: &mut Simulator,
		eventtype: &EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
		match eventtype {
			&EventType::MUExit => match try_to_move_to_successor(self, _simulator) {
				Ok(Some(event_template)) => {
					// Move succeeded and produced an event template.
					self.entrancelocked = false;
					self.state = ProcessState::Idle;
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
					// Ok(None) can mean either: (A) the MU was moved but no
					// immediate event was returned by the successor, or (B)
					// no successor could accept the MU and it remains here.
					if self.content.is_none() {
						// MU was moved (no event). Station becomes idle and
						// notify any blocked predecessors.
						self.entrancelocked = false;
						self.state = ProcessState::Idle;
						let blocked_events = self.notify_blocked_predecessors();
						match blocked_events {
							Ok(Some(be)) => Ok(Some(be)),
							Ok(None) => Ok(None),
							Err(_e) => Err(RunTimeError {}),
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
			&EventType::BlockedRetry => {
				// BlockedRetry is a notification from a successor that the
				// successor has become available and requests the predecessor
				// to attempt to move its MU. 
				if !self.is_exit_allowed() {
					return Ok(None);
				}
				match try_to_move_to_successor(self, _simulator) {
					Ok(Some(event_template)) => {
						// If the move succeeded (content taken) we may need to unlock entrance
						// and flush blocked predecessors similar to MUExit path.
						if self.content.is_none() {
							self.entrancelocked = false;
							self.state = ProcessState::Idle;
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
						// Either moved with no immediate event or still blocked.
						if self.content.is_none() {
							self.entrancelocked = false;
							self.state = ProcessState::Idle;
							let blocked_events = self.notify_blocked_predecessors();
							match blocked_events {
								Ok(Some(be)) => Ok(Some(be)),
								Ok(None) => Ok(None),
								Err(_e) => Err(RunTimeError {}),
							}
						} else {
							// Still holding MU: remain blocked.
							self.state = ProcessState::Blocked;
							Ok(None)
						}
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

