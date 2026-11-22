//! A conveyor (spec issue 133, ADR 0021 to ADR 0024): straight and flat, at a constant speed,
//! holding as many MUs as fit along it, accumulating at its end. How it works is described in
//! `docs/design/conveyor.md`.
//!
//! An MU's place is where its front is, in metres from the start. Every MU moves at the
//! conveyor's speed or stands, so the standing MUs are a block at the front, and a conveyor keeps
//! at most a few events queued: its front MU reaching the end or handing over, the first moving
//! MU closing up to the block, and room at the start for those waiting.

use std::collections::VecDeque;

use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType};
use crate::flows::{
	list_blocked, relist_blocked, retries_for_blocked, try_to_reserve_successor, waiting_ids, BaseObject, Distributing, EventInterface, MaterialFlow, MoveableUnit,
	MuKindId, ProcessState, SuccessorContainer, Waiting,
};
use crate::materialflow_objects::OtherObjects;
use crate::model::MuKind;
use crate::report::{BeltPosition, Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;

use super::flows::FailureProfile;
use super::interruptions::{Interruptible, Interruptions};

/// Lengths closer than this, in metres, are the same: a tenth of a micrometre.
const EPSILON: f64 = 1e-7;

/// What a conveyor's own event is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConveyorStep {
	/// The front MU's front reaches the end.
	FrontAtEnd,
	/// The front MU's booking point crosses the end, so it goes to the successor that took it.
	Booked,
	/// The first moving MU closes up to the standing block ahead of it.
	CloseUp,
	/// There is room at the start for the first waiting predecessor whose MU can fit.
	Room,
	/// Something changed from outside, such as an MU entering: work out the events again.
	Wake,
}

/// Names a conveyor's event and the plan it belongs to. Anything that changes the conveyor makes
/// a new plan, so an event of an older one is stale when it fires, as `MUExit` is by `stop_count`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConveyorTicket {
	/// A `ConveyorStep` as a plain byte, which has no unused values: an enum's would let the
	/// compiler keep `EventType`'s own tag in it, and every event would pay to work that out.
	step: u8,
	pub plan: u32,
}

impl ConveyorTicket {
	pub fn step(self) -> ConveyorStep {
		match self.step {
			0 => ConveyorStep::FrontAtEnd,
			1 => ConveyorStep::Booked,
			2 => ConveyorStep::CloseUp,
			3 => ConveyorStep::Room,
			_ => ConveyorStep::Wake,
		}
	}
}

/// An MU on the belt, and how it moves.
#[derive(Debug, Clone)]
struct OnBelt {
	mu: MoveableUnit,
	/// Where its front was at `since`, in metres from the start.
	front: f64,
	since: SimTime,
	/// Moving at the conveyor's speed, or standing.
	moving: bool,
}

#[derive(Clone)]
pub struct Conveyor {
	pub base: BaseObject,
	/// Its path's length, in metres.
	pub length: f64,
	/// In metres: an MU wider than this doesn't fit.
	pub width: f64,
	/// In metres per second, above 0.
	pub speed: f64,
	/// The least space between two MUs, in metres.
	pub gap: f64,
	/// Front first.
	belt: VecDeque<OnBelt>,
	/// The front MU has reached the end and is offered, or waits to be taken.
	front_at_end: bool,
	/// The successor that has taken the front MU, which the conveyor hands it to once its booking
	/// point crosses the end.
	handing_to: Option<u32>,
	/// When the front MU reached the end, which it waits in lines by.
	ready_since: SimTime,
	/// The plan its queued events belong to.
	plan: u32,
	/// A wake-up is queued.
	wake_pending: bool,
	/// Those waiting have had a retry for the room there is now, so they don't get another until
	/// something changes, and an MU that can never fit can't make a zero-time loop.
	retried: bool,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	/// How it picks the successor for its MU. Set by the run, which ranks the successors.
	pub distribution: Distributing,
	pub blockedlist: Vec<Waiting>,
	state: ProcessState,
	/// Its failures and pauses (ADR 0020). Interrupted, it stops every MU and takes and passes on
	/// none.
	pub interruptions: Interruptions,
	pub statistics: StatisticsKeeper,
	/// The model's kinds of MU, by `MuKindId`, given when a run is built.
	pub(crate) kinds: Vec<MuKind>,
}

impl Conveyor {
	pub fn new(object_id: u32, length: f64, width: f64, speed: f64) -> Conveyor {
		assert!(speed > 0.0, "a conveyor's speed is above 0");
		Conveyor {
			base: BaseObject { object_id, random_seed: 0 },
			length,
			width,
			speed,
			gap: 0.0,
			belt: VecDeque::new(),
			front_at_end: false,
			handing_to: None,
			ready_since: SimTime::ZERO,
			plan: 0,
			wake_pending: false,
			retried: false,
			succ: Vec::new(),
			succ_pointer: 0,
			distribution: Distributing::RoundRobin,
			blockedlist: Vec::new(),
			state: ProcessState::Idle,
			interruptions: Interruptions::new(None),
			statistics: StatisticsKeeper::new(0),
			kinds: vec![MuKind::default()],
		}
	}

	/// Gives it failures, or none with `None`.
	pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
		self.interruptions.set_failures(failures);
	}

	fn kind(&self, kind: MuKindId) -> MuKind {
		self.kinds.get(usize::from(kind)).copied().unwrap_or_default()
	}

	/// Where the MU at `at` has its front at `now`.
	fn front(&self, at: usize, now: SimTime) -> f64 {
		let on = &self.belt[at];
		if on.moving { on.front + self.speed * now.saturating_sub(on.since).as_millis() as f64 / 1000.0 } else { on.front }
	}

	fn rear(&self, at: usize, now: SimTime) -> f64 {
		self.front(at, now) - self.kind(self.belt[at].mu.kind).length
	}

	/// How far its front is ahead of its booking point.
	fn booking_offset(&self, kind: MuKindId) -> f64 {
		let kind = self.kind(kind);
		kind.booking_point * kind.length
	}

	/// Sets the MU at `at` at `front` from `now` on, moving or standing, and says so.
	fn plan_mu(&mut self, at: usize, front: f64, moving: bool, context: &mut EventContext) {
		let on = &mut self.belt[at];
		on.front = front;
		on.since = context.time;
		on.moving = moving;
		let (mu_id, kind) = (on.mu.id, on.mu.kind);
		let speed = if moving { self.speed } else { 0.0 };
		context.record(OccurrenceKind::MotionPlanned { mu_id, at: self.base.object_id, front: front as f32, speed: speed as f32, kind });
	}

	/// Whether an MU of this kind could ever be on it.
	fn fits(&self, kind: MuKindId) -> bool {
		let kind = self.kind(kind);
		kind.width <= self.width + EPSILON && kind.length <= self.length + EPSILON
	}

	/// How far the last MU's rear must be past the start for one of this kind to enter.
	fn room_needed(&self, kind: MuKindId) -> f64 {
		self.booking_offset(kind) + self.gap
	}

	/// How long it takes to move `distance` metres, rounded up to the next millisecond so a
	/// position is never short of where an event says it is.
	fn time_to(&self, distance: f64) -> SimTime {
		if distance <= EPSILON {
			return SimTime::ZERO;
		}
		SimTime::from_millis((distance / self.speed * 1000.0 - 1e-6).ceil().max(0.0) as u64)
	}

	fn event(&self, step: ConveyorStep, after: SimTime) -> EventTemplate {
		EventTemplate { eventtype: EventType::Conveyor(ConveyorTicket { step: step as u8, plan: self.plan }), exec_time: after, object_id: self.base.object_id }
	}

	/// The first MU behind the standing block at the front, which is moving. `None` if every MU
	/// moves, or every MU stands.
	fn first_behind_block(&self) -> Option<usize> {
		let block = self.belt.iter().take_while(|on| !on.moving).count();
		(block > 0 && block < self.belt.len()).then_some(block)
	}

	/// Works out its events again from how its MUs move now, voiding those queued.
	fn replan(&mut self, context: &mut EventContext, others: &OtherObjects<'_>) {
		self.plan = self.plan.wrapping_add(1);
		if self.interruptions.is_interrupted() {
			return;
		}
		let now = context.time;
		if self.belt.front().is_some_and(|on| on.moving) {
			let front = self.front(0, now);
			let event = match self.handing_to {
				Some(_) => self.event(ConveyorStep::Booked, self.time_to(self.length + self.booking_offset(self.belt[0].mu.kind) - front)),
				None => self.event(ConveyorStep::FrontAtEnd, self.time_to(self.length - front)),
			};
			context.schedule(event);
		}
		if let Some(at) = self.first_behind_block() {
			let stop_at = self.rear(at - 1, now) - self.gap;
			context.schedule(self.event(ConveyorStep::CloseUp, self.time_to(stop_at - self.front(at, now))));
		}
		if !self.retried {
			let waiting = waiting_ids(&self.blockedlist);
			let first = waiting.into_iter().find_map(|id| others.get(id).offered_kind(now).filter(|&kind| self.fits(kind)));
			if let Some(kind) = first {
				let need = self.room_needed(kind);
				let after = match self.belt.back() {
					None => Some(SimTime::ZERO),
					Some(_) => {
						let rear = self.rear(self.belt.len() - 1, now);
						if rear >= need - EPSILON {
							Some(SimTime::ZERO)
						} else if self.belt.back().is_some_and(|on| on.moving) {
							Some(self.time_to(need - rear))
						} else {
							None
						}
					}
				};
				if let Some(after) = after {
					context.schedule(self.event(ConveyorStep::Room, after));
				}
			}
		}
	}

	/// The standing block at the front moves again, now that the front MU may move.
	fn move_block(&mut self, context: &mut EventContext) {
		let now = context.time;
		for at in 0..self.belt.len() {
			if self.belt[at].moving {
				break;
			}
			let front = self.front(at, now);
			self.plan_mu(at, front, true, context);
		}
		self.retried = false;
	}

	/// Offers the front MU, at the end, to its successors, or only to `only` answering its retry.
	/// The one that takes it reserves a place, and the MU is handed over once its booking point
	/// crosses the end, at once with its booking point at the front.
	fn offer(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>, only: Option<u32>) -> Result<bool, RunTimeError> {
		let mu_id = self.belt[0].mu.id;
		let Some(taken_by) = try_to_reserve_successor(self, context, others, mu_id, only) else {
			return Ok(false);
		};
		self.front_at_end = false;
		self.move_block(context);
		if self.booking_offset(self.belt[0].mu.kind) <= EPSILON {
			self.hand_over(taken_by, context, others)?;
		} else {
			self.handing_to = Some(taken_by);
		}
		Ok(true)
	}

	/// The front MU's booking point has crossed the end: it belongs to the successor now, and
	/// its space on the conveyor is free.
	fn hand_over(&mut self, to: u32, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		let on = self.belt.pop_front().expect("handing over the front MU");
		self.handing_to = None;
		self.front_at_end = false;
		self.statistics.count_mu_out();
		let from = self.base.object_id;
		context.record(OccurrenceKind::MuMoved { mu_id: on.mu.id, from, to });
		log::info!("seq={} MU {} moved from conveyor {} to {}", context.seq, on.mu.id, from, to);
		if let Some(entered) = others.get_mut(to).process_mu_entrance(on.mu, context).map_err(|_| RunTimeError {})? {
			context.schedule(entered);
		}
		Ok(())
	}

	fn handle_event(&mut self, context: &mut EventContext, eventtype: &EventType, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		match eventtype {
			EventType::Conveyor(ticket) if ticket.step() == ConveyorStep::Wake => {
				self.wake_pending = false;
				self.replan(context, others);
				Ok(())
			}
			EventType::Conveyor(ticket) if ticket.plan != self.plan || self.interruptions.is_interrupted() => Ok(()),
			EventType::Conveyor(ticket) => {
				let now = context.time;
				match ticket.step() {
					ConveyorStep::FrontAtEnd => {
						self.plan_mu_quietly(0, self.length, now);
						self.front_at_end = true;
						self.ready_since = now;
						if !self.offer(context, others, None)? {
							self.plan_mu(0, self.length, false, context);
						}
					}
					ConveyorStep::Booked => {
						let to = self.handing_to.expect("a successor took the front MU");
						self.hand_over(to, context, others)?;
					}
					ConveyorStep::CloseUp => {
						if let Some(at) = self.first_behind_block() {
							let stop_at = self.rear(at - 1, now) - self.gap;
							self.plan_mu(at, stop_at, false, context);
						}
					}
					ConveyorStep::Room => {
						self.retried = true;
						retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
						return Ok(());
					}
					ConveyorStep::Wake => unreachable!("handled above"),
				}
				self.replan(context, others);
				Ok(())
			}
			&EventType::BlockedRetry { from } => {
				// Nothing waits at the end: a retry already queued when the front MU was taken.
				if !self.front_at_end || self.handing_to.is_some() || self.belt.is_empty() {
					return Ok(());
				}
				// Interrupted, it keeps its place in the line, as if refused (spec issue 122).
				if self.interruptions.is_interrupted() {
					others.get_mut(from).add_blocked(self.base.object_id, self.ready_since, true);
					return Ok(());
				}
				if self.offer(context, others, Some(from))? {
					self.replan(context, others);
				}
				Ok(())
			}
			_ => self.handle_interruption(eventtype, context, others).unwrap_or(Err(RunTimeError {})),
		}
	}

	/// Sets where the MU at `at` is, exactly, without saying so: its movement doesn't change.
	fn plan_mu_quietly(&mut self, at: usize, front: f64, now: SimTime) {
		let on = &mut self.belt[at];
		on.front = front;
		on.since = now;
	}

	/// Empty, moving MUs, or every MU standing.
	fn process_state(&self) -> ProcessState {
		if self.belt.is_empty() {
			ProcessState::Idle
		} else if self.belt.iter().any(|on| on.moving) {
			ProcessState::Processing
		} else {
			ProcessState::Blocked
		}
	}

	/// Reports when blocking begins or ends, and tells the statistics what it ended up doing.
	fn update_state(&mut self, context: &mut EventContext) {
		let state = self.process_state();
		let object_id = self.base.object_id;
		match (self.state == ProcessState::Blocked, state == ProcessState::Blocked) {
			(false, true) => context.record(OccurrenceKind::Blocked { object_id }),
			(true, false) => context.record(OccurrenceKind::Unblocked { object_id }),
			_ => {}
		}
		self.state = state;
		let machine = MachineState::of(self.interruptions.most_serious(), &self.state);
		self.statistics.observe(context.time, machine, self.belt.len() as u64);
	}
}

impl MaterialFlow for Conveyor {
	/// It takes an MU only once it knows which: see `accepts`.
	#[inline(never)]
	fn is_entrance_allowed(&self) -> bool {
		false
	}
	#[inline(never)]
	fn accepts(&self, kind: MuKindId, now: SimTime) -> bool {
		if self.interruptions.is_interrupted() || !self.fits(kind) {
			return false;
		}
		match self.belt.back() {
			None => true,
			Some(_) => self.rear(self.belt.len() - 1, now) >= self.room_needed(kind) - EPSILON,
		}
	}
	#[inline(never)]
	fn is_exit_allowed(&self) -> bool {
		self.front_at_end && self.handing_to.is_none() && !self.interruptions.is_interrupted()
	}
	/// The MU enters with its booking point at the start, the rest of it behind, still taking
	/// that space (ADR 0023), and moves on.
	#[inline(never)]
	fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered conveyor {}", mu.id, self.base.object_id);
		let front = self.booking_offset(mu.kind);
		self.belt.push_back(OnBelt { mu, front, since: context.time, moving: true });
		let at = self.belt.len() - 1;
		self.plan_mu(at, front, true, context);
		self.statistics.count_mu_in();
		self.retried = false;
		self.update_state(context);
		// Its events are worked out again once it is in, with the objects around it at hand.
		self.wake_pending = true;
		Ok(Some(self.event(ConveyorStep::Wake, SimTime::ZERO)))
	}
	#[inline(never)]
	fn wake_on_listing(&mut self) -> Option<EventTemplate> {
		self.retried = false;
		if self.wake_pending {
			return None;
		}
		self.wake_pending = true;
		Some(self.event(ConveyorStep::Wake, SimTime::ZERO))
	}
	#[inline(never)]
	fn add_blocked(&mut self, id_blocked: u32, since: SimTime, answering_retry: bool) {
		list_blocked(&mut self.blockedlist, id_blocked, since, answering_retry);
	}
	#[inline(never)]
	fn relist_blocked(&mut self, id_blocked: u32, next: Option<SimTime>) {
		relist_blocked(&mut self.blockedlist, id_blocked, next);
	}
	#[inline(never)]
	fn notify_blocked_predecessors(&mut self, context: &mut EventContext) -> Result<(), MaterialflowError> {
		retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
		Ok(())
	}
}

impl EventInterface for Conveyor {
	#[inline(never)]
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		self.update_state(context);
		Ok(self.interruptions.first_failure(self.base.object_id).into_iter().collect())
	}
	#[inline(never)]
	fn execute_event(&mut self, context: &mut EventContext, eventtype: &EventType, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		let outcome = self.handle_event(context, eventtype, others);
		self.update_state(context);
		outcome
	}
	#[inline(never)]
	fn reset(&mut self) {
		self.belt.clear();
		self.front_at_end = false;
		self.handing_to = None;
		self.ready_since = SimTime::ZERO;
		self.plan = 0;
		self.wake_pending = false;
		self.retried = false;
		self.succ_pointer = 0;
		self.blockedlist.clear();
		self.state = ProcessState::Idle;
		self.interruptions.reset();
		self.statistics.reset();
	}
	#[inline(never)]
	fn clear_statistics(&mut self, now: SimTime) {
		self.interruptions.clear_statistics(now, &self.statistics);
		self.statistics.clear(now);
	}
	#[inline(never)]
	fn report(&self, now: SimTime) -> ObjectState {
		let stopped = self.interruptions.is_interrupted();
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Conveyor,
			contents: self.belt.iter().map(|on| on.mu.id).collect(),
			process: self.state.clone(),
			interrupted: self.interruptions.most_serious(),
			interruptions: self.interruptions.list(),
			failure_profiles: self.interruptions.profile_states(now, &self.statistics),
			remaining: None,
			work: Vec::new(),
			blocked_predecessors: waiting_ids(&self.blockedlist),
			successors: self.succ.clone(),
			capacity: Capacity::Limited(0),
			process_time: None,
			configured_process_time: None,
			statistics: self.statistics.read(now),
			fill: None,
			positions: (0..self.belt.len())
				.map(|at| BeltPosition {
					mu_id: self.belt[at].mu.id,
					front: self.front(at, now),
					speed: if self.belt[at].moving && !stopped { self.speed } else { 0.0 },
					kind: self.belt[at].mu.kind,
				})
				.collect(),
		}
	}
	#[inline(never)]
	fn get_object_id(&self) -> &u32 {
		&self.base.object_id
	}
	#[inline(never)]
	fn set_object_id(&mut self, id: u32) {
		self.base.object_id = id;
	}
	#[inline(never)]
	fn set_random_seed(&mut self, seed: u64) {
		self.base.random_seed = seed;
		self.interruptions.reseed(seed);
	}
}

impl Interruptible for Conveyor {
	#[inline(never)]
	fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32) {
		(&mut self.interruptions, &mut self.statistics, self.base.object_id)
	}
	/// Every MU stops where it is, and the events queued are void.
	#[inline(never)]
	fn stop_began(&mut self, context: &mut EventContext) {
		let now = context.time;
		for at in 0..self.belt.len() {
			if self.belt[at].moving {
				let front = self.front(at, now);
				self.plan_mu(at, front, false, context);
			}
		}
		self.plan = self.plan.wrapping_add(1);
	}
	/// Its MUs move again where their way is clear: the front MU unless it waits at the end, and
	/// each behind it unless it has closed up to a standing one. The front MU at the end is
	/// offered again.
	#[inline(never)]
	fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		let now = context.time;
		let mut ahead_moves = false;
		for at in 0..self.belt.len() {
			let moves = if at == 0 {
				!(self.front_at_end && self.handing_to.is_none())
			} else {
				ahead_moves || self.rear(at - 1, now) - self.gap - self.front(at, now) > EPSILON
			};
			if moves {
				let front = self.front(at, now);
				self.plan_mu(at, front, true, context);
			}
			ahead_moves = moves;
		}
		self.retried = false;
		if self.front_at_end && self.handing_to.is_none() && !self.belt.is_empty() {
			self.offer(context, others, None)?;
		}
		self.replan(context, others);
		Ok(())
	}
}

impl SuccessorContainer for Conveyor {
	#[inline(never)]
	fn distribution_mut(&mut self) -> &mut Distributing {
		&mut self.distribution
	}
	#[inline(never)]
	fn successors(&self) -> &[u32] {
		&self.succ
	}
	#[inline(never)]
	fn succ_pointer(&self) -> usize {
		self.succ_pointer
	}
	#[inline(never)]
	fn succ_pointer_mut(&mut self) -> &mut usize {
		&mut self.succ_pointer
	}
	#[inline(never)]
	fn clear_content_and_idle(&mut self) {
		self.belt.clear();
		self.front_at_end = false;
		self.handing_to = None;
		self.state = ProcessState::Idle;
	}
	#[inline(never)]
	fn take_content(&mut self, _now: SimTime) -> Option<MoveableUnit> {
		self.front_at_end = false;
		self.belt.pop_front().map(|on| on.mu)
	}
	/// Only its front MU at the end is offered, from when it got there.
	#[inline(never)]
	fn offered_since(&self, _now: SimTime) -> Option<SimTime> {
		(self.front_at_end && self.handing_to.is_none() && !self.belt.is_empty()).then_some(self.ready_since)
	}
	#[inline(never)]
	fn offered_kind(&self, _now: SimTime) -> Option<MuKindId> {
		self.belt.front().filter(|_| self.front_at_end && self.handing_to.is_none()).map(|on| on.mu.kind)
	}
	#[inline(never)]
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
}
