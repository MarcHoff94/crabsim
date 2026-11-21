
use crate::time::SimTime;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::{ObjectState, OccurrenceKind};
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
pub trait EventInterface {
	/// Prepares the object for a run and returns its first events. Called from the default state.
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError>;
	/// Handles an event, scheduling what follows through `context.schedule`, in the order the
	/// queue should take them: events at one time run in the order they were scheduled.
	fn execute_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<(), RunTimeError>;
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

pub trait MaterialFlow: EventInterface {
	fn is_entrance_allowed(&self) -> bool;
	fn is_exit_allowed(&self) -> bool;
	fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError>;
	/// Lists a predecessor it refused, whose MU has been ready to leave since `since`.
	/// `answering_retry` says the predecessor was answering this object's retry.
	fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool);
	/// A listed predecessor's MU has moved. It waits on by its next MU, ready since `next`, or
	/// gives up its place if it has none.
	fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>);
	/// Schedules a retry for each predecessor waiting for it, longest-waiting first.
	fn notify_blocked_predecessors(&mut self, context: &mut EventContext) -> Result<(), MaterialflowError>;
}

/// A predecessor's place in the line for a successor to have room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waiting {
	/// When the MU it offered became ready to leave, which the line is ordered by.
	pub since: SimTime,
	pub id: u32,
	/// The successor has had room and sent it a retry. Until that retry is refused it isn't
	/// waiting: if its MU goes in or the retry finds it stopped, the place is gone.
	retried: bool,
}

/// Lists a predecessor as waiting with an MU ready to leave since `since`. The line is ordered by
/// that time, longest ready first, and those ready at the same instant in the order they were
/// listed (ADR 0018). One already listed keeps its place, and so does one whose retry from this
/// successor was just refused (`answering_retry`), so those still waiting keep their order.
#[inline]
pub fn list_blocked(blockedlist: &mut Vec<Waiting>, id_blocked: u32, since: SimTime, answering_retry: bool) {
	if let Some(at) = blockedlist.iter().position(|waiting| waiting.id == id_blocked) {
		let listed = &mut blockedlist[at];
		if !listed.retried {
			return;
		}
		if answering_retry {
			listed.retried = false;
			return;
		}
		// Its last retry came to nothing, so it lost that place and joins the line afresh.
		blockedlist.remove(at);
	}
	let waiting = Waiting { since, id: id_blocked, retried: false };
	match blockedlist.last() {
		// Nearly always the back: an MU refused now became ready now, or was listed before.
		Some(last) if last.since > since => {
			let at = blockedlist.iter().rposition(|listed| listed.since <= since).map_or(0, |last| last + 1);
			blockedlist.insert(at, waiting);
		}
		_ => blockedlist.push(waiting),
	}
}

/// A predecessor's MU has moved, so it gives up its place. If it has another MU ready to leave,
/// since `next`, it waits on by that one, wherever it was listed.
#[inline]
pub fn relist_blocked(blockedlist: &mut Vec<Waiting>, id_blocked: u32, next: Option<SimTime>) {
	let Some(at) = blockedlist.iter().position(|waiting| waiting.id == id_blocked) else {
		return;
	};
	blockedlist.remove(at);
	if let Some(since) = next {
		list_blocked(blockedlist, id_blocked, since, false);
	}
}

/// Schedules one retry for each predecessor waiting, longest-waiting first. Each keeps its entry
/// until its retry is answered: refused, it is back in its place, and otherwise the place is
/// gone. Those whose last retry came to nothing aren't waiting, and go in the same pass.
pub fn retries_for_blocked(blockedlist: &mut Vec<Waiting>, from: u32, context: &mut EventContext) {
	blockedlist.retain_mut(|waiting| {
		if waiting.retried {
			return false;
		}
		waiting.retried = true;
		context.schedule(EventTemplate { eventtype: EventType::BlockedRetry { from }, exec_time: SimTime::ZERO, object_id: waiting.id });
		true
	});
}

/// The predecessors waiting in a line, longest-waiting first, for a state report.
pub fn waiting_ids(blockedlist: &[Waiting]) -> Vec<u32> {
	blockedlist.iter().filter(|waiting| !waiting.retried).map(|waiting| waiting.id).collect()
}

pub trait SuccessorContainer {
	fn successors(&self) -> &[u32];
	/// How the object picks a successor, kept with it so a weighted object remembers whose turn
	/// it is among successors that rank equal.
	fn distribution_mut(&mut self) -> &mut Distributing;
	fn succ_pointer(&self) -> usize;
	fn succ_pointer_mut(&mut self) -> &mut usize;
	fn clear_content_and_idle(&mut self);
	/// Hands over the next MU to leave, at time `now`. Which one that is, is the object's own
	/// business: its exit strategy.
	fn take_content(&mut self, now: SimTime) -> Option<MoveableUnit>;
	/// When the MU it would hand over at `now` became ready to leave, or `None` if it has none
	/// that may leave.
	fn offered_since(&self, now: SimTime) -> Option<SimTime>;
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
	/// Notes that the MU went to the successor at `idx`, so the next one starts after it.
	fn moved(&mut self, idx: usize, n: usize, pointer: &mut usize) {
		*pointer = (idx + 1) % n;
	}
	/// Whether this offers the MU in answer to a successor's retry.
	fn answers_retry(&self) -> bool {
		false
	}
}

/// How an object picks the successor that receives its MU, at run time (ADR 0016).
#[derive(Debug, Clone, Default)]
pub enum Distributing {
	/// Each successor in turn, from the one after the last that received an MU.
	#[default]
	RoundRobin,
	/// By a ranking worked out when the run was built or a weight changed.
	Weighted(Ranked),
}

/// A weighted object's successors, best first, those that rank equal taking turns.
#[derive(Debug, Clone)]
pub struct Ranked {
	/// Places among the exits, in groups that rank equal, the best group first.
	groups: Vec<Vec<usize>>,
	/// Within each group, the place in it to offer first.
	pointers: Vec<usize>,
	/// Each exit's group and place in it.
	place: Vec<(usize, usize)>,
}

impl Ranked {
	/// From groups of exit places, best first, that between them name every exit once.
	pub fn new(groups: Vec<Vec<usize>>) -> Ranked {
		let exits = groups.iter().map(Vec::len).sum();
		let mut place = vec![(0, 0); exits];
		for (group, members) in groups.iter().enumerate() {
			for (at, &exit) in members.iter().enumerate() {
				place[exit] = (group, at);
			}
		}
		let pointers = vec![0; groups.len()];
		Ranked { groups, pointers, place }
	}
}

impl DistributionStrategy for Ranked {
	fn order(&self, _n: usize, _start: usize) -> impl Iterator<Item = usize> {
		self.groups.iter().zip(&self.pointers).flat_map(|(group, &first)| (0..group.len()).map(move |i| group[(first + i) % group.len()]))
	}
	fn moved(&mut self, idx: usize, _n: usize, _pointer: &mut usize) {
		let (group, at) = self.place[idx];
		self.pointers[group] = (at + 1) % self.groups[group].len();
	}
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
>(obj: &mut T, context: &mut EventContext, others: &mut OtherObjects<'_>, strategy: &mut S) -> Result<Option<EventTemplate>, MaterialflowError> {
	let n = obj.successors().len();
	if n == 0 {
		return Ok(None);
	}
	let pred_id = *obj.get_object_id();
	let mut chosen = None;
	for idx in strategy.order(n, obj.succ_pointer()) {
		let succ_id = obj.successors()[idx];
		if others.get(succ_id).is_entrance_allowed() {
			// We selected a successor that appears able to accept an MU.
			// Now take the MU from the predecessor and attempt the transfer.
			if let Some(mu) = obj.take_content(context.time) {
				obj.statistics_mut().count_mu_out();
				let mu_id = mu.id;
				let seq = context.seq;
				// That MU is no longer waiting, so the object gives up its place wherever it was
				// refused, and waits on there by its next MU if it has one ready.
				let next = obj.offered_since(context.time);
				for &refused_by in obj.successors() {
					others.get_mut(refused_by).relist_blocked(pred_id, next);
				}
				context.record(OccurrenceKind::MuMoved { mu_id, from: pred_id, to: succ_id });
				log::info!("seq={} MU {} moved from {} to {}", seq, mu_id, pred_id, succ_id);
				match others.get_mut(succ_id).process_mu_entrance(mu, context) {
					Ok(entered) => {
						chosen = Some((idx, entered));
						break;
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
			if let Some(since) = obj.offered_since(context.time) {
				others.get_mut(succ_id).add_blocked(pred_id, since, strategy.answers_retry());
			}
		}
	}
	match chosen {
		Some((idx, entered)) => {
			// The next MU starts after the chosen successor.
			strategy.moved(idx, n, obj.succ_pointer_mut());
			Ok(entered)
		}
		None => Ok(None),
	}
}

/// Tries only one successor, whichever the other rules would have picked, and tells the object's
/// own rule where the MU went.
struct OnlySuccessor<'s, S>(usize, &'s mut S);
impl<S: DistributionStrategy> DistributionStrategy for OnlySuccessor<'_, S> {
	fn order(&self, _n: usize, _start: usize) -> impl Iterator<Item = usize> {
		std::iter::once(self.0)
	}
	fn moved(&mut self, idx: usize, n: usize, pointer: &mut usize) {
		self.1.moved(idx, n, pointer);
	}
	fn answers_retry(&self) -> bool {
		true
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
	let Some(index) = obj.successors().iter().position(|&succ| succ == from) else {
		return Ok(None);
	};
	if let Distributing::RoundRobin = obj.distribution_mut() {
		return try_to_move_to_successor_with_strategy(obj, context, others, &mut OnlySuccessor(index, &mut RoundRobinStrategy));
	}
	with_ranked(obj, |obj, ranked| try_to_move_to_successor_with_strategy(obj, context, others, &mut OnlySuccessor(index, ranked)))
}

/// Runs `move_it` with the object's ranking taken out of it for the while, so both can be
/// borrowed at once. Taking it out moves no memory.
fn with_ranked<T: SuccessorContainer, R>(obj: &mut T, move_it: impl FnOnce(&mut T, &mut Ranked) -> R) -> R {
	let mut distribution = std::mem::take(obj.distribution_mut());
	let Distributing::Weighted(ranked) = &mut distribution else {
		unreachable!("only a weighted object is moved by its ranking")
	};
	let result = move_it(obj, ranked);
	*obj.distribution_mut() = distribution;
	result
}

/// Backwards-compatible default that uses round-robin ordering.
pub fn try_to_move_to_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	if let Distributing::RoundRobin = obj.distribution_mut() {
		return try_to_move_to_successor_with_strategy(obj, context, others, &mut RoundRobinStrategy);
	}
	with_ranked(obj, |obj, ranked| try_to_move_to_successor_with_strategy(obj, context, others, ranked))
}

