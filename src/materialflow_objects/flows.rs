
use crate::time::SimTime;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::{ObjectState, OccurrenceKind, StopCause};
use crate::materialflow_objects::OtherObjects;
use crate::random_numbers::{Distribution, DistributionSampler};
use crate::statistics::StatisticsKeeper;

// Moveable unit
#[derive(Debug, Clone)]
pub struct MoveableUnit {
	pub id: u32,
	pub location_id: u32,
	/// When it left its Source, which is where its lead time starts. A starting MU counts from
	/// Init, and a Source sets this as the MU leaves.
	pub left_source: SimTime,
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
	pub mtbf: SimTime,
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
			mtbf: SimTime::ZERO,
			dist_sampler_mttr: DistributionSampler::new(distribution_mttr, seed),
			dist_sampler_mtbf: DistributionSampler::new(placeholder, seed.wrapping_add(1)),
			seed,
		};
		result.set_availability(availablity);
		result
	}
	pub fn get_time_to_next_failure(&mut self) -> Result<Option<SimTime>,MaterialflowError> {
		if self.active == false {
			return Ok(None);
		}
		Ok(Some(SimTime::from_millis_f64(self.dist_sampler_mtbf.sample())))
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
		self.mtbf = SimTime::from_millis_f64(mtbf_mean);
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
	ForcedFailed,
	Paused,
}
impl InterruptionState {
	pub fn cause(&self) -> Option<StopCause> {
		match self {
			InterruptionState::None => None,
			InterruptionState::Failed => Some(StopCause::Failure),
			InterruptionState::ForcedFailed => Some(StopCause::ForcedFailure),
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
		others: &mut OtherObjects<'_>,
	) -> Result<Option<Vec<EventTemplate>>, RunTimeError>;
	/// Returns the object to its default state: every MU from the run is gone, and the
	/// configured settings and starting MUs are back.
	fn reset(&mut self);
	/// Reports the object's current state at simulation time `now`.
	fn report(&self, now: SimTime) -> ObjectState;
	/// Starts the object's statistics afresh from `now`, changing nothing else.
	fn clear_statistics(&mut self, now: SimTime);
	fn get_object_id(&self) -> &u32;
	fn set_object_id(&mut self, id: u32);
	fn set_random_seed(&mut self, seed: u64);
}
pub trait StopCtrl {
	fn set_stop(&mut self, failure_start: &SimTime, reason: InterruptionState) -> Result<Option<EventTemplate>, MaterialflowError>;
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
pub fn retries_for_blocked(blockedlist: &mut Vec<u32>, from: u32) -> Option<Vec<EventTemplate>> {
	if blockedlist.is_empty() {
		return None;
	}
	Some(
		blockedlist
			.drain(..)
			.map(|blocked_id| EventTemplate {
				eventtype: EventType::BlockedRetry { from },
				exec_time: SimTime::ZERO,
				object_id: blocked_id,
			})
			.collect(),
	)
}

pub trait SuccessorContainer {
	fn successors(&self) -> &[u32];
	fn succ_pointer(&self) -> usize;
	fn succ_pointer_mut(&mut self) -> &mut usize;
	fn clear_content_and_idle(&mut self);
	/// Hands over the next MU to leave, at time `now`.
	fn take_content(&mut self, now: SimTime) -> Option<MoveableUnit>;
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper;
}

// Note: Do not re-export concrete sibling types here to avoid
// circular dependencies with `station.rs`/`source.rs` importing
// items from this module.

// try_to_move_to_successor helper (keeps generic movement logic near traits)
pub trait DistributionStrategy {
	/// Return the successor indices to try, in order, given the number of
	/// successors `n` and a starting pointer `start`. This runs on every move
	/// attempt, so it returns an iterator to avoid allocating.
	fn order(&self, n: usize, start: usize) -> impl Iterator<Item = usize>;
}

pub struct RoundRobinStrategy;
impl DistributionStrategy for RoundRobinStrategy {
	fn order(&self, n: usize, start: usize) -> impl Iterator<Item = usize> {
		(0..n).map(move |i| (start + i) % n)
	}
}

pub struct LowestIndexStrategy;
impl DistributionStrategy for LowestIndexStrategy {
	fn order(&self, n: usize, _start: usize) -> impl Iterator<Item = usize> {
		0..n
	}
}

/// New implementation that accepts a distribution strategy.
pub fn try_to_move_to_successor_with_strategy<
	T: SuccessorContainer + EventInterface,
	S: DistributionStrategy,
>(obj: &mut T, context: &mut EventContext, others: &mut OtherObjects<'_>, strategy: &S) -> Result<Option<EventTemplate>, MaterialflowError> {
	let n = obj.successors().len();
	if n == 0 {
		return Ok(None);
	}
	let pred_id = *obj.get_object_id();
	for idx in strategy.order(n, obj.succ_pointer()) {
		let succ_id = obj.successors()[idx];
		if others.get(succ_id).is_entrance_allowed() {
			// We selected a successor that appears able to accept an MU.
			// Now take the MU from the predecessor and attempt the transfer.
			if let Some(mu) = obj.take_content(context.time) {
				obj.statistics_mut().count_mu_out();
				let mu_id = mu.id;
				let seq = context.seq;
				// It is no longer waiting, so it gives up its place wherever it was refused.
				for &refused_by in obj.successors() {
					others.get_mut(refused_by).remove_blocked(pred_id);
				}
				context.record(OccurrenceKind::MuMoved { mu_id, from: pred_id, to: succ_id });
				log::info!("seq={} MU {} moved from {} to {}", seq, mu_id, pred_id, succ_id);
				match others.get_mut(succ_id).process_mu_entrance(mu, context) {
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
			log::debug!(
				"seq={} Successor {} currently denies entrance; adding blocked predecessor {}",
				context.seq,
				succ_id,
				pred_id
			);
			others.get_mut(succ_id).add_blocked(pred_id);
		}
	}
	Ok(None)
}

/// Tries only one successor, whichever the other rules would have picked.
struct OnlySuccessor(usize);
impl DistributionStrategy for OnlySuccessor {
	fn order(&self, _n: usize, _start: usize) -> impl Iterator<Item = usize> {
		std::iter::once(self.0)
	}
}

/// Answers a retry by trying only the successor that sent it. Any other successor that has
/// room has sent retries of its own, to whoever has waited longest for it.
pub fn retry_move_to_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
	from: u32,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	let sender = obj.successors().iter().position(|&succ| succ == from);
	match sender {
		Some(index) => try_to_move_to_successor_with_strategy(obj, context, others, &OnlySuccessor(index)),
		None => Ok(None),
	}
}

/// Backwards-compatible default that uses round-robin ordering.
pub fn try_to_move_to_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	let strategy = RoundRobinStrategy;
	try_to_move_to_successor_with_strategy(obj, context, others, &strategy)
}

