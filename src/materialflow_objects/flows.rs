
use std::time::Duration;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventTemplate, EventType};
use crate::simulator::Simulator;
use crate::random_numbers::{Distribution, DistributionSampler};
use std::{cell::RefCell, rc::Rc};

// Moveable unit
#[derive(Debug, Clone)]
pub struct MoveableUnit {
	pub id: u32,
	pub location_id: u32,
}

#[derive(Debug, Clone)]
pub struct BaseObject {
	pub object_id: u32,
	pub random_seed: u64,
}
#[derive(Clone)]
pub struct FailureProfile {
	pub active: bool,
	pub availablity: f32,
	pub mttr: Duration,
	pub mtbf: Duration,
	pub dist_sampler_mttr: DistributionSampler,
	pub dist_sampler_mtbf: DistributionSampler,
	pub seed: u64,
}
impl FailureProfile {
	pub fn new(availablity: f32, mttr: Duration, distribution_mttr: Distribution, distribution_mtbf: Distribution, seed: u64) -> FailureProfile {
		let mut result = FailureProfile {
			active: false,
			availablity,
			mttr,
			mtbf: Duration::from_millis(0),
			dist_sampler_mttr: DistributionSampler::new(distribution_mttr, seed),
			dist_sampler_mtbf: DistributionSampler::new(distribution_mtbf, seed), 
			seed,
		};
		result.calculate_mtbf();
		
		result
	}
	pub fn get_time_to_next_failure(&mut self) -> Result<Option<Duration>,MaterialflowError> {
		if self.active == false {
			return Ok(None);
		}
		Ok(Some(Duration::from_millis(self.dist_sampler_mtbf.sample() as u64)))
	}
	pub fn set_availability(&mut self, availablity: f32) {
		self.availablity = availablity;
		self.calculate_mtbf();
	}


	fn calculate_mtbf(&mut self) {
		if self.availablity <= 0.0 || self.availablity >= 1.0 {
			panic!("FailureProfile: availablity must be in (0.0,1.0) range");
		}
		self.mtbf = Duration::from_millis((self.mttr.as_millis() /(1.0 / self.availablity - 1.0).round() as u128) as u64);
	}
}
// Errors and states
#[derive(Debug, Clone)]
#[derive(PartialEq)]
pub enum ProcessState {
	Processing,
	Blocked,
	Idle,
}
#[derive(Debug,Clone)]
pub enum InterruptionState {
	None,
	Failed,
	Paused,
}

// Traits
pub trait EventInterface {
	fn init(&mut self) -> Result<Vec<EventTemplate>, InitError>;
	fn execute_event(
		&mut self,
		simulator: &mut Simulator,
		eventtype: &EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError>;
	fn get_object_id(&self) -> &u32;
	fn set_object_id(&mut self, id: u32);
	fn set_random_seed(&mut self, seed: u64);
	fn as_any(&self) -> &dyn std::any::Any;
}

pub trait MaterialFlow: EventInterface {
	fn is_entrance_allowed(&self) -> bool;
	fn is_exit_allowed(&self) -> bool;
	fn process_mu_entrance(&mut self,_mu: MoveableUnit) -> Result<Option<EventTemplate>, MaterialflowError>;
	fn process_mu_exit(&mut self) -> Result<Option<EventTemplate>, MaterialflowError>;
	fn add_blocked(&mut self, id_blocked: u32);
	fn as_any(&mut self) -> &mut dyn std::any::Any;
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError>;
}

pub trait SuccessorContainer {
	fn successors(&self) -> &Vec<Rc<RefCell<dyn MaterialFlow>>>;
	fn succ_pointer(&self) -> usize;
	fn succ_pointer_mut(&mut self) -> &mut usize;
	fn clear_content_and_idle(&mut self);
	fn take_content(&mut self) -> Option<MoveableUnit>;
}

// Note: Do not re-export concrete sibling types here to avoid
// circular dependencies with `station.rs`/`source.rs` importing
// items from this module.

// try_to_move_to_successor helper (keeps generic movement logic near traits)
pub trait DistributionStrategy {
	/// Return an ordered list of successor indices to try, given the
	/// number of successors `n` and a starting pointer `start`.
	fn order(&self, n: usize, start: usize) -> Vec<usize>;
}

pub struct RoundRobinStrategy;
impl DistributionStrategy for RoundRobinStrategy {
	fn order(&self, n: usize, start: usize) -> Vec<usize> {
		if n == 0 {
			return Vec::new();
		}
		let mut out = Vec::with_capacity(n);
		for i in 0..n {
			out.push((start + i) % n);
		}
		out
	}
}

pub struct LowestIndexStrategy;
impl DistributionStrategy for LowestIndexStrategy {
	fn order(&self, n: usize, _start: usize) -> Vec<usize> {
		(0..n).collect()
	}
}

/// New implementation that accepts a distribution strategy.
pub fn try_to_move_to_successor_with_strategy<
	T: SuccessorContainer + EventInterface,
	S: DistributionStrategy,
>(obj: &mut T, simulator: &Simulator, strategy: &S) -> Result<Option<EventTemplate>, MaterialflowError> {
	let succ = obj.successors().clone();
	if succ.is_empty() {
		return Ok(None);
	}
	let n = succ.len();
	let order = strategy.order(n, obj.succ_pointer());
	for &idx in &order {
		let succ_obj = &succ[idx];
		if succ_obj.borrow().is_entrance_allowed() {
			// We selected a successor that appears able to accept an MU.
			// Now take the MU from the predecessor and attempt the transfer.
			if let Some(mu) = obj.take_content() {
				let succ_id = *succ_obj.borrow().get_object_id();
				let pred_id = *obj.get_object_id();
				let mut succ_mut = succ_obj.borrow_mut();
				match succ_mut.process_mu_entrance(mu) {
					Ok(Some(event_template)) => {
						// advance pointer to next after chosen index on success
						*obj.succ_pointer_mut() = (idx + 1) % n;
						log::info!(
							"seq={} MU {} moved from {} to {}",
							simulator.current_seq,
							event_template.object_id,
							pred_id,
							succ_id
						);
						return Ok(Some(event_template));
					}
					Ok(None) => {
						*obj.succ_pointer_mut() = (idx + 1) % n;
						log::info!(
							"seq={} MU moved from {} to {} (no immediate event)",
							simulator.current_seq,
							pred_id,
							succ_id
						);
						return Ok(None);
					}
					Err(_) => {
						// According to the requested semantics, process_mu_entrance
						// should not fail for a successor that reported allowed;
						// panic if it does to surface programming errors early.
						panic!(
							"process_mu_entrance unexpectedly failed for successor {} from predecessor {}",
							succ_id, pred_id
						);
					}
				}
			} else {
				return Ok(None);
			}
		} else {
			let pred_id = *obj.get_object_id();
			let mut succ_mut = succ_obj.borrow_mut();
			log::debug!(
				"seq={} Successor {} currently denies entrance; adding blocked predecessor {}",
				simulator.current_seq,
				*succ_mut.get_object_id(),
				pred_id
			);
			succ_mut.add_blocked(pred_id);
		}
	}
	Ok(None)
}

/// Backwards-compatible default that uses round-robin ordering.
pub fn try_to_move_to_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	simulator: &Simulator,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	let strategy = RoundRobinStrategy;
	try_to_move_to_successor_with_strategy(obj, simulator, &strategy)
}

