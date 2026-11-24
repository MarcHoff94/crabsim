
use crate::time::SimTime;
use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::report::{ObjectState, OccurrenceKind};
use crate::materialflow_objects::OtherObjects;
use crate::random_numbers::{Distribution, DistributionSampler};
use crate::statistics::StatisticsKeeper;
use crate::routing::Arrival;

/// An MU. Every object holding MUs pays for each, so it packs its departure time and kind into one
/// word (ADR 0027) and stays 16 bytes with its place in its work plan and its detour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveableUnit {
	pub id: u32,
	/// Its place in the run's table of work plan steps (ADR 0026): the step it is on its way to,
	/// or `NO_STEP` once its plan is done or it has none.
	pub(crate) step: u16,
	/// The destination it goes to before carrying on with its plan, set by a Station's detour and
	/// cleared when reached, or `NO_DESTINATION`.
	pub(crate) detour: DestinationId,
	/// When it left its Source in the low 48 bits, as milliseconds, and its kind in the high 16.
	packed: u64,
}

/// An MU kind's place in a model's table of them. A small number, so every MU pays little for it.
pub type MuKindId = u16;

/// A destination's place in a model's table of them (ADR 0025): an instance, or a class standing
/// for any of its instances.
pub type DestinationId = u16;

/// No destination: an MU without a detour, or none left in its plan.
pub const NO_DESTINATION: DestinationId = DestinationId::MAX;

/// No step: an MU with no work plan, or whose plan is done.
pub const NO_STEP: u16 = u16::MAX;

/// The bits of an MU's packed word that hold its departure time.
const DEPARTURE_BITS: u64 = (1 << 48) - 1;

/// The latest simulated time an MU can record as its departure, about 8,900 years (ADR 0027). A
/// run refuses to go beyond it.
pub const TIME_LIMIT: SimTime = SimTime::from_millis(DEPARTURE_BITS);

impl MoveableUnit {
	/// An MU of `kind` that left its Source at `left_source`, with no plan and no detour.
	#[inline]
	pub fn new(id: u32, left_source: SimTime, kind: MuKindId) -> MoveableUnit {
		debug_assert!(left_source <= TIME_LIMIT, "a run refuses to go beyond the time an MU can hold");
		MoveableUnit { id, step: NO_STEP, detour: NO_DESTINATION, packed: (left_source.as_millis() & DEPARTURE_BITS) | (u64::from(kind) << 48) }
	}

	/// A starting MU: the built-in kind, counting from Init, with no plan.
	pub fn starting(id: u32) -> MoveableUnit {
		MoveableUnit::new(id, SimTime::ZERO, 0)
	}

	/// When it left its Source, which is where its lead time starts. A starting MU counts from
	/// Init, and a Source sets this as the MU leaves.
	#[inline]
	pub fn left_source(&self) -> SimTime {
		SimTime::from_millis(self.packed & DEPARTURE_BITS)
	}

	#[inline]
	pub fn set_left_source(&mut self, at: SimTime) {
		debug_assert!(at <= TIME_LIMIT, "a run refuses to go beyond the time an MU can hold");
		self.packed = (self.packed & !DEPARTURE_BITS) | (at.as_millis() & DEPARTURE_BITS);
	}

	/// Its MU kind, its place in the model's table of them (ADR 0021): what its MU class and its
	/// Source make it, such as its footprint and work plan. 0, the built-in MU, for starting MUs.
	#[inline]
	pub fn kind(&self) -> MuKindId {
		(self.packed >> 48) as MuKindId
	}

	/// Whether it has somewhere left to go: a detour or a step of its plan. Checked on every
	/// entry, so models without work plans pay one comparison.
	#[inline]
	pub fn is_routed(&self) -> bool {
		self.step != NO_STEP || self.detour != NO_DESTINATION
	}
}

#[derive(Debug, Clone)]
pub struct BaseObject{
	pub object_id: u32,
	pub random_seed: u64,
}
/// The name of an object's only failure profile when none is given: what a model file's single
/// `failures` object is called.
pub const DEFAULT_FAILURE_NAME: &str = "Failures";

/// The kind of time that counts down to a failure profile's next failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeBasis {
	/// All time counts, whatever the object is doing.
	#[default]
	Simulation,
	/// All time but paused, idle and blocked included.
	Working,
	/// Only time busy, once however many MUs are in work. A Source is busy while it waits out its
	/// interval. Only Stations and Sources offer it.
	Processing,
}

#[derive(Clone)]
pub struct FailureProfile {
	/// Unique within its object.
	pub name: String,
	/// The profile's random-number stream within its object (ADR 0012): with the object's seed it
	/// gives the profile's failures, so adding or removing another profile changes nothing here.
	/// `None` is its place in the list, and stream 0 is the object's own seed, which is what a single
	/// profile from an older file had, so its failures don't change.
	pub stream: Option<u64>,
	/// What counts down to its next failure.
	pub time_basis: TimeBasis,
	/// The time counted on its basis at which its next failure is due. On simulation time this is
	/// never read: the failure simply comes when its event does.
	pub due: SimTime,
	pub active: bool,
	pub availablity: f32,
	pub mtbf: SimTime,
	pub dist_sampler_mttr: DistributionSampler,
	pub dist_sampler_mtbf: DistributionSampler,
	pub seed: u64,
	/// Since when this profile has been failing, if it is.
	pub failing_since: Option<SimTime>,
	/// How long it has failed since statistics began, not counting a failure in progress.
	pub time_failed: SimTime,
	/// How many failures began since statistics began.
	pub failures: u64,
}
impl FailureProfile {
	pub fn new(availablity: f32, distribution_mttr: Distribution, seed: u64) -> FailureProfile {
		let placeholder = distribution_mttr.with_mean(0.0);
		let mut result = FailureProfile {
			name: DEFAULT_FAILURE_NAME.to_string(),
			stream: None,
			time_basis: TimeBasis::Simulation,
			due: SimTime::ZERO,
			active: false,
			availablity,
			mtbf: SimTime::ZERO,
			dist_sampler_mttr: DistributionSampler::new(distribution_mttr, seed),
			dist_sampler_mtbf: DistributionSampler::new(placeholder, seed.wrapping_add(1)),
			seed,
			failing_since: None,
			time_failed: SimTime::ZERO,
			failures: 0,
		};
		result.set_availability(availablity);
		result
	}
	pub fn named(mut self, name: impl Into<String>) -> FailureProfile {
		self.name = name.into();
		self
	}
	pub fn with_stream(mut self, stream: u64) -> FailureProfile {
		self.stream = Some(stream);
		self
	}
	pub fn with_time_basis(mut self, time_basis: TimeBasis) -> FailureProfile {
		self.time_basis = time_basis;
		self
	}
	/// How long it has failed by `now`, a failure in progress included.
	pub fn failed_by(&self, now: SimTime) -> SimTime {
		self.time_failed + self.failing_since.map_or(SimTime::ZERO, |since| now.saturating_sub(since))
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
	/// A place is reserved for an MU a conveyor is handing over, whose booking point hasn't
	/// arrived yet (ADR 0023), and the object is doing nothing else.
	Receiving,
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
	/// Whether it can take an MU of this kind at `now`. Only an object whose geometry counts asks
	/// about the MU (ADR 0021): the rest take any MU they have room for. `from_belt` says it comes
	/// from another conveyor, with its front at the start rather than its booking point.
	fn accepts(&self, _kind: MuKindId, _now: SimTime, _from_belt: bool) -> bool {
		self.is_entrance_allowed()
	}
	/// Reserves a place for the MU `mu_id` a conveyor is handing over, which it takes through
	/// `process_mu_entrance` once its booking point arrives (ADR 0023). Asked only after `accepts`
	/// said yes. A transfer once begun always finishes, even if the object is interrupted.
	fn reserve(&mut self, _mu_id: u32, _context: &mut EventContext) {}
	/// The event that wakes it, if it needs one, after a predecessor it refused has joined its
	/// line: a conveyor works out when there will be room for it.
	fn wake_on_listing(&mut self) -> Option<EventTemplate> {
		None
	}
	fn is_exit_allowed(&self) -> bool;
	/// Takes the MU, which has just arrived as `arrival` says: reaching its next destination has
	/// already moved it on (ADR 0026). Only a Station reads it, for its process time.
	fn process_mu_entrance(&mut self, mu: MoveableUnit, arrival: Arrival, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError>;
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
	/// The kind of the MU it would hand over at `now`, which a successor whose geometry counts
	/// needs to say whether it fits. `None` if it has none to hand over.
	fn offered_kind(&self, now: SimTime) -> Option<MuKindId>;
	/// The MU it would hand over at `now`, whose destination an object routing by destination
	/// reads (ADR 0025). `None` if it has none to hand over.
	fn offered_mu(&self, now: SimTime) -> Option<&MoveableUnit>;
	/// Whether it is a conveyor, whose MUs hand over to another conveyor with their front first.
	fn on_belt(&self) -> bool {
		false
	}
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
	/// Among the successors from which the MU's next destination can be reached (ADR 0025), by
	/// round robin or a ranking. Boxed, since only objects routing by destination pay for it.
	ByDestination(Box<ByDestination>),
}

/// An object routing by destination: which of its exits reach each destination, worked out when
/// the run is built, and what chooses among them.
#[derive(Debug, Clone)]
pub struct ByDestination {
	/// Round robin or weighted, never by destination again.
	pub then: Distributing,
	/// By destination, which of its exits reach it, or `None` where none does, and the MU is
	/// offered to every one.
	reaching: crate::distribution::Reaching,
}

impl ByDestination {
	/// Choosing by round robin until the run ranks the successors, from which exits reach each
	/// destination, by destination and then by exit.
	pub fn new(reaching: crate::distribution::Reaching) -> ByDestination {
		ByDestination { then: Distributing::RoundRobin, reaching }
	}
}

/// Offers an MU only to the exits that reach its next destination, in the order of the rule
/// that chooses among them, or to all when none does.
struct Toward<'a, S> {
	allowed: Option<&'a [bool]>,
	inner: S,
}

impl<S: DistributionStrategy> DistributionStrategy for Toward<'_, S> {
	fn order(&self, n: usize, start: usize) -> impl Iterator<Item = usize> {
		let allowed = self.allowed;
		self.inner.order(n, start).filter(move |&idx| allowed.is_none_or(|allowed| allowed[idx]))
	}
	fn moved(&mut self, idx: usize, n: usize, pointer: &mut usize) {
		self.inner.moved(idx, n, pointer);
	}
	fn answers_retry(&self) -> bool {
		self.inner.answers_retry()
	}
}

/// The rule that chooses among the exits reaching an MU's destination.
enum Then<'a> {
	RoundRobin,
	Ranked(&'a mut Ranked),
}

/// One of two iterators, so `Then` needs no allocation.
enum Either<A, B> {
	A(A),
	B(B),
}

impl<A: Iterator<Item = usize>, B: Iterator<Item = usize>> Iterator for Either<A, B> {
	type Item = usize;
	fn next(&mut self) -> Option<usize> {
		match self {
			Either::A(a) => a.next(),
			Either::B(b) => b.next(),
		}
	}
}

impl DistributionStrategy for Then<'_> {
	fn order(&self, n: usize, start: usize) -> impl Iterator<Item = usize> {
		match self {
			Then::RoundRobin => Either::A(RoundRobinStrategy.order(n, start)),
			Then::Ranked(ranked) => Either::B(ranked.order(n, start)),
		}
	}
	fn moved(&mut self, idx: usize, n: usize, pointer: &mut usize) {
		match self {
			Then::RoundRobin => RoundRobinStrategy.moved(idx, n, pointer),
			Then::Ranked(ranked) => ranked.moved(idx, n, pointer),
		}
	}
}

/// Where the MU `obj` would hand over at `now` goes next, or `NO_DESTINATION`.
fn offered_destination<T: SuccessorContainer>(obj: &T, context: &EventContext) -> DestinationId {
	obj.offered_mu(context.time).map_or(NO_DESTINATION, |mu| context.routing.next_destination(mu))
}

/// Runs `move_it` with the object's by-destination state taken out of it for the while, given the
/// exits that reach `destination` and the rule that chooses among them. Taking it out moves no
/// memory.
fn with_toward<T: SuccessorContainer, R>(obj: &mut T, destination: DestinationId, move_it: impl FnOnce(&mut T, Option<&[bool]>, Then<'_>) -> R) -> R {
	let mut distribution = std::mem::take(obj.distribution_mut());
	let Distributing::ByDestination(by) = &mut distribution else {
		unreachable!("only an object routing by destination is moved toward one")
	};
	let ByDestination { then, reaching } = &mut **by;
	let allowed = reaching.get(usize::from(destination)).and_then(|exits| exits.as_deref());
	let then = match then {
		Distributing::RoundRobin => Then::RoundRobin,
		Distributing::Weighted(ranked) => Then::Ranked(ranked),
		Distributing::ByDestination(_) => unreachable!("by destination chooses by round robin or weighted"),
	};
	let result = move_it(obj, allowed, then);
	*obj.distribution_mut() = distribution;
	result
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
		// Only a successor whose geometry counts needs to know which MU (ADR 0021), so the rest
		// keep the cheaper question on this, the busiest path of every run.
		let successor = others.get(succ_id);
		let allowed = if successor.reads_mu_kind() { accepts_offered(obj, successor, context.time) } else { successor.is_entrance_allowed() };
		if allowed {
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
				match others.get_mut(succ_id).enter(mu, context) {
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
				let successor = others.get_mut(succ_id);
				successor.add_blocked(pred_id, since, strategy.answers_retry());
				if !strategy.answers_retry() {
					if let Some(wake) = successor.wake_on_listing() {
						context.schedule(wake);
					}
				}
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

/// Whether a successor whose geometry counts takes the MU `obj` offers. Kept out of the move loop,
/// which every run goes through for every move, so models without conveyors pay nothing for it.
#[cold]
#[inline(never)]
fn accepts_offered<T: SuccessorContainer>(obj: &T, successor: &crate::materialflow_objects::ModelObject, now: SimTime) -> bool {
	obj.offered_kind(now).is_some_and(|kind| successor.accepts(kind, now, obj.on_belt()))
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
	match obj.distribution_mut() {
		Distributing::RoundRobin => try_to_move_to_successor_with_strategy(obj, context, others, &mut OnlySuccessor(index, &mut RoundRobinStrategy)),
		Distributing::Weighted(_) => with_ranked(obj, |obj, ranked| try_to_move_to_successor_with_strategy(obj, context, others, &mut OnlySuccessor(index, ranked))),
		Distributing::ByDestination(_) => with_toward(obj, offered_destination(obj, context), |obj, allowed, mut then| {
			try_to_move_to_successor_with_strategy(obj, context, others, &mut Toward { allowed, inner: OnlySuccessor(index, &mut then) })
		}),
	}
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

/// Offers the MU it would hand over to its successors in the order of its distribution strategy,
/// as a conveyor does when its front MU reaches its end, and has the first that can take it
/// reserve a place for it, rather than moving it now (ADR 0023). Those that can't take it list it
/// as waiting. With `only`, it offers it to that successor alone, answering its retry. Returns the
/// successor that reserved a place.
pub fn try_to_reserve_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
	mu_id: u32,
	only: Option<u32>,
) -> Option<u32> {
	let index = match only {
		Some(from) => Some(obj.successors().iter().position(|&succ| succ == from)?),
		None => None,
	};
	match obj.distribution_mut() {
		Distributing::RoundRobin => match index {
			Some(index) => reserve_with_strategy(obj, context, others, mu_id, &mut OnlySuccessor(index, &mut RoundRobinStrategy)),
			None => reserve_with_strategy(obj, context, others, mu_id, &mut RoundRobinStrategy),
		},
		Distributing::Weighted(_) => with_ranked(obj, |obj, ranked| match index {
			Some(index) => reserve_with_strategy(obj, context, others, mu_id, &mut OnlySuccessor(index, ranked)),
			None => reserve_with_strategy(obj, context, others, mu_id, ranked),
		}),
		Distributing::ByDestination(_) => with_toward(obj, offered_destination(obj, context), |obj, allowed, mut then| match index {
			Some(index) => reserve_with_strategy(obj, context, others, mu_id, &mut Toward { allowed, inner: OnlySuccessor(index, &mut then) }),
			None => reserve_with_strategy(obj, context, others, mu_id, &mut Toward { allowed, inner: then }),
		}),
	}
}

fn reserve_with_strategy<T: SuccessorContainer + EventInterface, S: DistributionStrategy>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
	mu_id: u32,
	strategy: &mut S,
) -> Option<u32> {
	let n = obj.successors().len();
	let kind = obj.offered_kind(context.time)?;
	let pred_id = *obj.get_object_id();
	let answering = strategy.answers_retry();
	let mut chosen = None;
	for idx in strategy.order(n, obj.succ_pointer()) {
		let succ_id = obj.successors()[idx];
		if others.get(succ_id).accepts(kind, context.time, obj.on_belt()) {
			chosen = Some((idx, succ_id));
			break;
		}
		if let Some(since) = obj.offered_since(context.time) {
			let successor = others.get_mut(succ_id);
			successor.add_blocked(pred_id, since, answering);
			if !answering {
				if let Some(wake) = successor.wake_on_listing() {
					context.schedule(wake);
				}
			}
		}
	}
	let (idx, succ_id) = chosen?;
	others.get_mut(succ_id).reserve(mu_id, context);
	// Its MU no longer waits anywhere.
	for &refused_by in obj.successors() {
		others.get_mut(refused_by).relist_blocked(pred_id, None);
	}
	strategy.moved(idx, n, obj.succ_pointer_mut());
	Some(succ_id)
}

/// Backwards-compatible default that uses round-robin ordering.
pub fn try_to_move_to_successor<T: SuccessorContainer + EventInterface>(
	obj: &mut T,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
) -> Result<Option<EventTemplate>, MaterialflowError> {
	match obj.distribution_mut() {
		Distributing::RoundRobin => try_to_move_to_successor_with_strategy(obj, context, others, &mut RoundRobinStrategy),
		Distributing::Weighted(_) => with_ranked(obj, |obj, ranked| try_to_move_to_successor_with_strategy(obj, context, others, ranked)),
		Distributing::ByDestination(_) => with_toward(obj, offered_destination(obj, context), |obj, allowed, then| {
			try_to_move_to_successor_with_strategy(obj, context, others, &mut Toward { allowed, inner: then })
		}),
	}
}


#[cfg(test)]
mod size_tests {
	use super::{MoveableUnit, TIME_LIMIT};
	use crate::time::SimTime;

	/// Every object holding MUs pays for each, so an MU's kind (ADR 0021), its step and its detour
	/// (ADR 0026) cost no room: 16 bytes, as before it had any (ADR 0027).
	#[test]
	fn an_mu_is_16_bytes() {
		assert_eq!(size_of::<MoveableUnit>(), 16);
	}

	#[test]
	fn an_mu_keeps_its_departure_time_and_kind_apart_up_to_the_limit() {
		let mut mu = MoveableUnit::new(7, TIME_LIMIT, u16::MAX);
		assert_eq!((mu.left_source(), mu.kind()), (TIME_LIMIT, u16::MAX));
		mu.set_left_source(SimTime::from_millis(1));
		assert_eq!((mu.left_source(), mu.kind()), (SimTime::from_millis(1), u16::MAX));
		let mu = MoveableUnit::new(8, TIME_LIMIT, 0);
		assert_eq!((mu.left_source(), mu.kind()), (TIME_LIMIT, 0));
		assert!(!mu.is_routed(), "a new MU has no plan and no detour");
	}

	#[test]
	fn the_limit_is_about_8900_years() {
		let years = TIME_LIMIT.as_secs_f64() / (365.25 * 86_400.0);
		assert!((8_900.0..8_930.0).contains(&years), "{years}");
	}
}
