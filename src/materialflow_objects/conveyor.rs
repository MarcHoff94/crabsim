//! A conveyor (spec issue 133, ADR 0021 to ADR 0024): straight and flat, at a constant speed,
//! holding as many MUs as fit along it, accumulating at its end. How it works is described in
//! `docs/design/conveyor.md`.
//!
//! An MU's place is where its front is, in metres from the start. Every MU moves at the
//! conveyor's speed or stands, except one straddling this conveyor and the next, which moves at the
//! slower of their speeds (ADR 0023). A conveyor keeps at most a few events queued: its front MU
//! reaching the end or handing over, the first MU closing up to the one ahead, the MU it handed to
//! a conveyor clearing its start, and room at the start for those waiting.

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
	/// The first MU that is faster than the one ahead of it closes up to it.
	CloseUp,
	/// There is room at the start for the first waiting predecessor whose MU can fit.
	Room,
	/// Something changed from outside, such as an MU entering: work out the events again.
	Wake,
	/// An MU straddling this conveyor and the one before it has passed a point that changes how it
	/// moves or what is behind it: work out the events again.
	Straddle,
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
			5 => ConveyorStep::Straddle,
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
	/// In metres per second, 0 standing: the conveyor's, or the slower speed of an MU on two.
	speed: f64,
}

/// The MU a conveyor handed to the next one, which still has its rear on this one: where its rear
/// is, in this conveyor's metres from its start, and how fast it moves.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lead {
	rear: f64,
	speed: f64,
}

/// A conveyor that reserved a place for an MU whose predecessor isn't known yet.
const NOBODY: u32 = u32::MAX;

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
	/// Whether MUs behind a stopped one close up and stop while the rest still move (the default),
	/// or every MU stops as a whole when the first one does and they all start together.
	pub accumulating: bool,
	/// Front first.
	belt: VecDeque<OnBelt>,
	/// The front MU has reached the end and is offered, or waits to be taken.
	front_at_end: bool,
	/// The successor that has taken the front MU, which the conveyor hands it to once its booking
	/// point crosses the end.
	handing_to: Option<u32>,
	/// The conveyor it has handed an MU to that still has its rear on this one, which the MUs
	/// behind must not run into (ADR 0023).
	led_by: Option<u32>,
	/// The conveyor whose MU is on both, with its rear still on that one, being this conveyor's
	/// last MU.
	straddle_from: Option<u32>,
	/// The conveyor that has reserved the space under its front MU for its start: nothing else may
	/// enter until it has handed that MU over.
	reserved_by: Option<u32>,
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
			accumulating: true,
			belt: VecDeque::new(),
			front_at_end: false,
			handing_to: None,
			led_by: None,
			straddle_from: None,
			reserved_by: None,
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
		on.front + on.speed * now.saturating_sub(on.since).as_millis() as f64 / 1000.0
	}

	fn rear(&self, at: usize, now: SimTime) -> f64 {
		self.front(at, now) - self.kind(self.belt[at].mu.kind).length
	}

	/// How far its front is ahead of its booking point.
	fn booking_offset(&self, kind: MuKindId) -> f64 {
		let kind = self.kind(kind);
		kind.booking_point * kind.length
	}

	/// Sets the MU at `at` at `front` from `now` on, moving at `speed` or standing, and says so.
	fn plan_mu(&mut self, at: usize, front: f64, speed: f64, context: &mut EventContext) {
		let on = &mut self.belt[at];
		on.front = front;
		on.since = context.time;
		on.speed = speed;
		let (mu_id, kind) = (on.mu.id, on.mu.kind);
		context.record(OccurrenceKind::MotionPlanned { mu_id, at: self.base.object_id, front: front as f32, speed: speed as f32, kind });
	}

	/// Whether an MU of this kind could ever be on it.
	fn fits(&self, kind: MuKindId) -> bool {
		let kind = self.kind(kind);
		kind.width <= self.width + EPSILON && kind.length <= self.length + EPSILON
	}

	/// How far the last MU's rear must be past the start for one of this kind to enter. From
	/// another conveyor it enters with its front at the start, so only the gap is needed. From
	/// anything else it enters with its booking point there.
	fn room_needed(&self, kind: MuKindId, from_belt: bool) -> f64 {
		if from_belt { self.gap } else { self.booking_offset(kind) + self.gap }
	}

	/// How long it takes to move `distance` metres at `speed`, rounded up to the next millisecond
	/// so a position is never short of where an event says it is.
	fn time_for(distance: f64, speed: f64) -> SimTime {
		if distance <= EPSILON {
			return SimTime::ZERO;
		}
		SimTime::from_millis((distance / speed * 1000.0 - 1e-6).ceil().max(0.0) as u64)
	}

	fn event(&self, step: ConveyorStep, after: SimTime) -> EventTemplate {
		EventTemplate { eventtype: EventType::Conveyor(ConveyorTicket { step: step as u8, plan: self.plan }), exec_time: after, object_id: self.base.object_id }
	}

	/// Its speed if it isn't interrupted, which another conveyor's MU on both of them is limited by.
	pub(crate) fn belt_speed(&self) -> Option<f64> {
		(!self.interruptions.is_interrupted()).then_some(self.speed)
	}

	/// The last MU's rear and speed, for the conveyor `from` that has reserved a place for its MU
	/// here, or handed one over that still has its rear on it: neither may run into it.
	pub(crate) fn tail_lead(&self, from: u32, now: SimTime) -> Option<Lead> {
		if (self.straddle_from != Some(from) && self.reserved_by != Some(from)) || self.belt.is_empty() {
			return None;
		}
		let last = self.belt.len() - 1;
		Some(Lead { rear: self.rear(last, now), speed: self.belt[last].speed })
	}

	/// The wake-up that has it work out its events again, unless one is queued already.
	pub(crate) fn take_wake(&mut self) -> Option<EventTemplate> {
		if self.wake_pending {
			return None;
		}
		self.wake_pending = true;
		Some(self.event(ConveyorStep::Wake, SimTime::ZERO))
	}

	/// Has the conveyor `id` work out its events again.
	fn wake(id: u32, context: &mut EventContext, others: &mut OtherObjects<'_>) {
		if let Some(wake) = others.get_mut(id).as_conveyor_mut().and_then(Conveyor::take_wake) {
			context.schedule(wake);
		}
	}

	/// Has every conveyor whose MUs depend on how this one is moving work out its events again.
	fn wake_neighbours(&self, context: &mut EventContext, others: &mut OtherObjects<'_>) {
		for id in [self.led_by, self.straddle_from, self.reserved_by.filter(|&id| id != NOBODY)].into_iter().flatten() {
			Self::wake(id, context, others);
		}
	}

	/// What its front MU must not run into on the next conveyor, in this conveyor's metres: the last
	/// MU there, while it has handed it an MU whose rear is still on this one, or has reserved a
	/// place there for the front MU, which has its front on it.
	fn lead(&mut self, others: &OtherObjects<'_>, now: SimTime) -> Option<Lead> {
		let next = self.handing_to.or(self.led_by)?;
		let lead = others.get(next).as_conveyor().and_then(|next| next.tail_lead(self.base.object_id, now));
		if lead.is_none() && self.handing_to.is_none() {
			self.led_by = None;
		}
		lead.map(|lead| Lead { rear: self.length + lead.rear, speed: lead.speed })
	}

	/// How fast the MU at `at` moves unless it is held up by the one ahead of it.
	fn own_speed(&self, at: usize, others: &OtherObjects<'_>, now: SimTime) -> f64 {
		if self.interruptions.is_interrupted() || (at == 0 && self.front_at_end && self.handing_to.is_none()) {
			return 0.0;
		}
		let mut speed = self.speed;
		// An MU on this conveyor and the next moves at the slower of the two, but while one is
		// stopped it moves at the speed of the one it belongs to (ADR 0023).
		if at == 0 {
			if let Some(next) = self.handing_to {
				speed = others.get(next).as_conveyor().and_then(Conveyor::belt_speed).map_or(speed, |next| speed.min(next));
			}
		}
		if let (Some(from), true) = (self.straddle_from, at + 1 == self.belt.len()) {
			if self.rear(at, now) < -EPSILON {
				speed = others.get(from).as_conveyor().and_then(Conveyor::belt_speed).map_or(speed, |from| speed.min(from));
			}
		}
		speed
	}

	/// Sets the MU at `at` where it is now at `front`, moving at `speed`, and says so if either
	/// differs from what it was planned to do.
	fn apply(&mut self, at: usize, was: f64, front: f64, speed: f64, context: &mut EventContext) {
		if self.belt[at].speed != speed || (front - was).abs() > EPSILON {
			self.plan_mu(at, front, speed, context);
		}
	}

	/// Gives every MU the speed it has from now on, which depends on the one ahead of it and, for
	/// the front one, on what it handed to the next conveyor. Returns in how many seconds the first
	/// MU that is faster than the one ahead closes up to it, if one does.
	fn settle(&mut self, lead: Option<Lead>, others: &OtherObjects<'_>, context: &mut EventContext) -> Option<f64> {
		let now = context.time;
		let mut closes_up: Option<f64> = None;
		let mut note = |time: f64| closes_up = Some(closes_up.map_or(time, |least| least.min(time)));
		if self.accumulating {
			let mut ahead = lead.map(|lead| (lead.rear, lead.speed));
			for at in 0..self.belt.len() {
				let was = self.front(at, now);
				let mut front = was;
				let mut speed = self.own_speed(at, others, now);
				if let Some((rear, ahead_speed)) = ahead {
					let room = rear - self.gap - front;
					if room <= EPSILON {
						speed = speed.min(ahead_speed);
						// Rounded up to a millisecond, it may have gone a little past.
						if -room > EPSILON {
							front += room;
						}
					} else if speed > ahead_speed {
						note(room / (speed - ahead_speed));
					}
				}
				self.apply(at, was, front, speed, context);
				ahead = Some((front - self.kind(self.belt[at].mu.kind).length, speed));
			}
		} else if !self.belt.is_empty() {
			// Every MU rides the belt, which stops when the first can't move on, and goes at the
			// slowest speed any of them is held to.
			let mut group = (0..self.belt.len()).map(|at| self.own_speed(at, others, now)).fold(f64::INFINITY, f64::min);
			if let Some(lead) = lead {
				let room = lead.rear - self.gap - self.front(0, now);
				if room <= EPSILON {
					group = group.min(lead.speed);
				} else if group > lead.speed {
					note(room / (group - lead.speed));
				}
			}
			for at in 0..self.belt.len() {
				let front = self.front(at, now);
				self.apply(at, front, front, group, context);
			}
		}
		closes_up
	}

	/// Works out its events again from how its MUs move now, voiding those queued.
	fn replan(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) {
		self.plan = self.plan.wrapping_add(1);
		if self.interruptions.is_interrupted() {
			return;
		}
		let now = context.time;
		let lead = self.lead(others, now);
		let closes_up = self.settle(lead, others, context);
		if let Some(first) = self.belt.front() {
			if first.speed > 0.0 {
				let front = self.front(0, now);
				let event = match self.handing_to {
					Some(_) => Some((ConveyorStep::Booked, self.length + self.booking_offset(first.mu.kind) - front)),
					None if !self.front_at_end => Some((ConveyorStep::FrontAtEnd, self.length - front)),
					None => None,
				};
				if let Some((step, distance)) = event {
					context.schedule(self.event(step, Self::time_for(distance, first.speed)));
				}
			}
		}
		if let Some(seconds) = closes_up {
			// At least a millisecond, so one that is nearly there can't make a loop of events at
			// one time.
			let after = SimTime::from_millis(((seconds * 1000.0 - 1e-6).ceil().max(1.0)) as u64);
			context.schedule(self.event(ConveyorStep::CloseUp, after));
		}
		if let Some(from) = self.straddle_from {
			self.watch_straddler(from, context, others);
		} else if let Some(from) = self.reserved_by.filter(|&from| from != NOBODY) {
			// Its last MU may be what the MU that is coming in must not run into.
			Self::wake(from, context, others);
		}
		if !self.retried {
			let first = waiting_ids(&self.blockedlist).into_iter().find_map(|id| {
				let waiting = others.get(id);
				waiting.offered_kind(now).filter(|&kind| self.fits(kind)).map(|kind| (kind, waiting.reads_mu_kind()))
			});
			if let Some((kind, from_belt)) = first {
				let need = self.room_needed(kind, from_belt);
				let after = match self.belt.back() {
					None => Some(SimTime::ZERO),
					Some(last) => {
						let rear = self.rear(self.belt.len() - 1, now);
						if rear >= need - EPSILON {
							Some(SimTime::ZERO)
						} else if last.speed > 0.0 {
							Some(Self::time_for(need - rear, last.speed))
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

	/// The last MU still has its rear on the conveyor `from` that handed it over, or is less than
	/// the gap past its start. `from` is told so its MUs behind don't run into it, and it is told
	/// again when it has cleared.
	fn watch_straddler(&mut self, from: u32, context: &mut EventContext, others: &mut OtherObjects<'_>) {
		let now = context.time;
		let last = self.belt.len().checked_sub(1);
		let rear = last.map(|last| self.rear(last, now));
		match (last, rear) {
			(Some(last), Some(rear)) if rear < self.gap - EPSILON => {
				let speed = self.belt[last].speed;
				if speed > 0.0 {
					// First the point where `from` stops holding it back, then where it is clear.
					let target = if rear < -EPSILON { 0.0 } else { self.gap };
					context.schedule(self.event(ConveyorStep::Straddle, Self::time_for(target - rear, speed)));
				}
			}
			_ => self.straddle_from = None,
		}
		Self::wake(from, context, others);
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
		self.retried = false;
		if let Some(next) = others.get_mut(taken_by).as_conveyor_mut() {
			next.reserved_by = Some(self.base.object_id);
		}
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
		if others.get(to).as_conveyor().is_some() {
			// Its rear may still be on this one, which the MUs behind must not run into.
			self.led_by = Some(to);
		}
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
						// Held up on the way, it isn't there: the plan was already void.
						if self.belt.is_empty() || self.front(0, now) < self.length - 1e-3 {
							self.replan(context, others);
							return Ok(());
						}
						self.plan_mu_quietly(0, self.length, now);
						self.front_at_end = true;
						self.ready_since = now;
						self.offer(context, others, None)?;
					}
					ConveyorStep::Booked => {
						let to = self.handing_to.expect("a successor took the front MU");
						self.hand_over(to, context, others)?;
					}
					ConveyorStep::CloseUp | ConveyorStep::Straddle => {}
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
			_ => {
				let was = self.interruptions.is_interrupted();
				let outcome = self.handle_interruption(eventtype, context, others).unwrap_or(Err(RunTimeError {}));
				if was != self.interruptions.is_interrupted() {
					self.wake_neighbours(context, others);
				}
				outcome
			}
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
		} else if self.belt.iter().any(|on| on.speed > 0.0) {
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
	fn accepts(&self, kind: MuKindId, now: SimTime, from_belt: bool) -> bool {
		if self.interruptions.is_interrupted() || self.reserved_by.is_some() || !self.fits(kind) {
			return false;
		}
		match self.belt.back() {
			None => true,
			Some(_) => self.rear(self.belt.len() - 1, now) >= self.room_needed(kind, from_belt) - EPSILON,
		}
	}
	/// Another conveyor's MU has its front at the start: nothing else may enter until it has been
	/// handed over (ADR 0023).
	#[inline(never)]
	fn reserve(&mut self, _mu_id: u32, _context: &mut EventContext) {
		self.reserved_by = Some(NOBODY);
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
		// Handed over by a conveyor, it may still have its rear on that one.
		if let Some(from) = self.reserved_by.take() {
			self.straddle_from = Some(from).filter(|&from| from != NOBODY);
		}
		// A belt that is stopped as a whole takes it standing, and one that moves, moving: the
		// events that follow work out the rest.
		let speed = if self.interruptions.is_interrupted() {
			0.0
		} else if self.accumulating {
			self.speed
		} else {
			self.belt.front().map_or(self.speed, |first| first.speed)
		};
		self.belt.push_back(OnBelt { mu, front, since: context.time, speed });
		let at = self.belt.len() - 1;
		self.plan_mu(at, front, speed, context);
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
		self.led_by = None;
		self.straddle_from = None;
		self.reserved_by = None;
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
			content_kinds: self.belt.iter().map(|on| on.mu.kind).collect(),
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
					speed: if stopped { 0.0 } else { self.belt[at].speed },
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
			if self.belt[at].speed > 0.0 {
				let front = self.front(at, now);
				self.plan_mu(at, front, 0.0, context);
			}
		}
		self.plan = self.plan.wrapping_add(1);
	}
	/// Its MUs move again where their way is clear, as `replan` works out, and the front MU at the
	/// end is offered again.
	#[inline(never)]
	fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
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
		self.led_by = None;
		self.straddle_from = None;
		self.reserved_by = None;
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
	fn on_belt(&self) -> bool {
		true
	}
	#[inline(never)]
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
}
