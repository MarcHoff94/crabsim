
use std::time::Duration;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::observation::{ObjectState, OccurrenceKind, StopCause};
use crate::random_numbers::{Distribution, DistributionSampler};
use std::{cell::RefCell, rc::Rc};

// Moveable unit
#[derive(Debug, Clone)]
pub struct MoveableUnit {
	pub id: u32,
	pub location_id: u32,
}

#[derive(Debug, Clone)]
pub struct BaseObject{
	pub object_id: u32,
	pub random_seed: u64,
}
#[derive(Clone)]
pub struct FailureProfile {
	pub active: bool,
	pub availablity: f32,
	pub mtbf: Duration,
	pub dist_sampler_mttr: DistributionSampler,
	pub dist_sampler_mtbf: DistributionSampler,
	pub seed: u64,
}
impl FailureProfile {
	pub fn new(availablity: f32, distribution_mttr: Distribution, seed: u64) -> FailureProfile {
		let placeholder = distribution_mttr.with_mean(0.0);
		let mut result = FailureProfile {
			active: false,
			availablity,
			mtbf: Duration::from_millis(0),
			dist_sampler_mttr: DistributionSampler::new(distribution_mttr, seed),
			dist_sampler_mtbf: DistributionSampler::new(placeholder, seed.wrapping_add(1)),
			seed,
		};
		result.set_availability(availablity);
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
		if availablity == 1.0 {
			self.active = false;
		} else {
			self.active = true;
			self.calculate_mtbf();
		};
	}

	pub fn reseed(&mut self, seed: u64) {
		self.seed = seed;
		self.dist_sampler_mttr = DistributionSampler::new(self.dist_sampler_mttr.params.clone(), seed);
		self.dist_sampler_mtbf = DistributionSampler::new(self.dist_sampler_mtbf.params.clone(), seed.wrapping_add(1));
	}

	fn calculate_mtbf(&mut self) {
		if self.availablity <= 0.0 || self.availablity >= 1.0 {
			panic!("FailureProfile: availablity must be in (0.0,1.0) range");
		}
		let mttr_mean = self.dist_sampler_mttr.params.mean();
		let mtbf_mean = mttr_mean / (1.0 / self.availablity as f64 - 1.0);
		self.mtbf = Duration::from_millis(mtbf_mean as u64);
		let mtbf_dist = self.dist_sampler_mttr.params.with_mean(mtbf_mean);
		self.dist_sampler_mtbf = DistributionSampler::new(mtbf_dist, self.seed.wrapping_add(1));
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
impl InterruptionState {
	pub fn cause(&self) -> Option<StopCause> {
		match self {
			InterruptionState::None => None,
			InterruptionState::Failed => Some(StopCause::Failure),
			InterruptionState::Paused => Some(StopCause::Pause),
		}
	}
}

pub trait EventInterface {
	/// Prepares the object for a run and returns its first events. Called from the default state.
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError>;
	fn execute_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError>;
	/// Returns the object to its default state: every MU from the run is gone, and the
	/// configured settings and starting MUs are back.
	fn reset(&mut self);
	/// Reports the object's current state at simulation time `now`.
	fn report(&self, now: Duration) -> ObjectState;
	fn get_object_id(&self) -> &u32;
	fn set_object_id(&mut self, id: u32);
	fn set_random_seed(&mut self, seed: u64);
	fn as_any(&self) -> &dyn std::any::Any;
}
pub trait StopCtrl {
	fn set_stop(&mut self, failure_start: &Duration, reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError>;
	fn remove_stop(&mut self) -> Result<Option<EventTemplate>, MaterialflowError>;
}

pub trait MaterialFlow: EventInterface + StopCtrl {
	fn is_entrance_allowed(&self) -> bool;
	fn is_exit_allowed(&self) -> bool;
	fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError>;
	fn add_blocked(&mut self, id_blocked: u32);
	fn remove_blocked(&mut self, id_blocked: u32);
	fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError>;
}

/// Lists a predecessor as waiting. One already listed keeps its place, since it has been
/// waiting from then on.
pub fn list_blocked(blockedlist: &mut Vec<u32>, id_blocked: u32) {
	if !blockedlist.contains(&id_blocked) {
		blockedlist.push(id_blocked);
	}
}

/// Empties the list into one retry for each predecessor, longest-waiting first. A retry that
/// is refused lists its predecessor again, so those still waiting keep their order.
pub fn retries_for_blocked(blockedlist: &mut Vec<u32>) -> Option<Vec<EventTemplate>> {
	if blockedlist.is_empty() {
		return None;
	}
	Some(
		blockedlist
			.drain(..)
			.map(|blocked_id| EventTemplate {
				eventtype: EventType::BlockedRetry,
				exec_time: Duration::from_millis(0),
				object_id: blocked_id,
				seq: 0,
			})
			.collect(),
	)
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
>(obj: &mut T, context: &mut EventContext, strategy: &S) -> Result<Option<EventTemplate>, MaterialflowError> {
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
				let mu_id = mu.id;
				let seq = context.seq;
				// It is no longer waiting, so it gives up its place wherever it was refused.
				for refused_by in &succ {
					refused_by.borrow_mut().remove_blocked(pred_id);
				}
				let mut succ_mut = succ_obj.borrow_mut();
				context.record(OccurrenceKind::MuMoved { mu_id, from: pred_id, to: succ_id });
				log::info!("seq={} MU {} moved from {} to {}", seq, mu_id, pred_id, succ_id);
				match succ_mut.process_mu_entrance(mu, context) {
					Ok(Some(event_template)) => {
						// advance pointer to next after chosen index on success
						*obj.succ_pointer_mut() = (idx + 1) % n;
						return Ok(Some(event_template));
					}
					Ok(None) => {
						*obj.succ_pointer_mut() = (idx + 1) % n;
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
				context.seq,
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
	context: &mut EventContext,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	let strategy = RoundRobinStrategy;
	try_to_move_to_successor_with_strategy(obj, context, &strategy)
}

