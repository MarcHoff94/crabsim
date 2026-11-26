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
	MuKindId, Offer, ProcessState, SuccessorContainer, Waiting,
};
use crate::materialflow_objects::OtherObjects;
use crate::model::MuKind;
use crate::report::{BeltPosition, Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{BeltKeeper, MachineState, ObjectStatistics, StatisticsKeeper};
use crate::routing::Arrival;
use crate::time::SimTime;

use super::cross::{Cross, Stage, Timer};
use super::flows::{choose_exit, try_to_reserve_exit, FailureProfile};
use super::interruptions::{Interruptible, Interruptions};

/// How many times conveyors worked out their events again and how many MUs those passes walked, for
/// the benchmark's `conveyor-line` scenario (ADR 0022 says a replan's cost must be measured). Only
/// with the `counters` feature, so no other build pays for counting.
#[cfg(feature = "counters")]
pub mod counters {
	use std::sync::atomic::AtomicU64;
	pub static REPLANS: AtomicU64 = AtomicU64::new(0);
	pub static MUS_WALKED: AtomicU64 = AtomicU64::new(0);
}

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
	/// On a cross conveyor, the MU turning there reaches its centre.
	Centre,
	/// On a cross conveyor, the switch time is over and the turning MU leaves sideways.
	Switched,
	/// On a cross conveyor, the release time after a turned MU is over.
	Released,
	/// On a cross conveyor, it has reserved room for a conveyor's MU: choose that MU's exit, and
	/// nothing else.
	Decide,
	/// The rear of its last MU crosses its start, which user code's hooks at the rear wait for.
	RearIn,
	/// The rear of the MU it handed to an object without geometry crosses its end, at its speed,
	/// which user code's hooks at the rear wait for. Not part of a plan: the MU has gone.
	RearOff,
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
			6 => ConveyorStep::Centre,
			7 => ConveyorStep::Switched,
			8 => ConveyorStep::Released,
			9 => ConveyorStep::Decide,
			10 => ConveyorStep::RearIn,
			11 => ConveyorStep::RearOff,
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
	/// When its booking point crossed the start, for its crossing time.
	entered: SimTime,
	/// It lies across its way, having turned at a cross conveyor: its width lies along the belt.
	crosswise: bool,
	/// How long it is along the belt, its length or lying crosswise its width, kept so the walk
	/// over the MUs on every replan reads no MU kind.
	along: f64,
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

/// User code's calls at an MU's rear (spec issue 156, PR 1b), kept by the conveyor its rear is on
/// until it crosses: the hooks on exit of the object it came from and on entry of the one it went
/// to. Only kept where a hook asks for the rear.
#[derive(Debug, Clone)]
pub(crate) struct RearCalls {
	mu_id: u32,
	/// When the rear crosses, for an MU handed to an object without geometry, gone from the conveyor.
	due: SimTime,
	calls: VecDeque<crate::hooks::HookCall>,
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
	/// Its fill, the length its MUs took and their crossing times, boxed so the conveyor stays no
	/// larger than a Buffer.
	belt_statistics: Box<BeltKeeper>,
	/// The whole length of the MUs it holds, in metres.
	taken: f64,
	/// When the start comes free while it is busy and full, with nothing else happening: the last
	/// MU's rear clearing the gap past the start. Its statistics split the stretch of busy and full
	/// there, so it needs no event of its own.
	clears_at: Option<SimTime>,
	/// Until when the last MU's rear is short of the gap past the start, so that no MU could enter:
	/// `SimTime::MAX` while it stands there, `None` while it isn't. Worked out where its position is
	/// already at hand, when it enters and when the conveyor replans, so telling busy and full from
	/// busy with room reads no position on any other event.
	short_until: Option<SimTime>,
	/// Who hands over the MU about to enter, and whether it lies crosswise, told just before it
	/// enters.
	arriving: Option<(u32, bool)>,
	/// What makes it a cross conveyor (spec issue 148), if it is one: then `length` and `width`
	/// are those of the way the MU on it goes now. Boxed, so other conveyors pay a pointer.
	pub(crate) cross: Option<Box<Cross>>,
	/// The model's kinds of MU, by `MuKindId`, given when a run is built.
	pub(crate) kinds: Vec<MuKind>,
	/// User code's calls at the rear of its last MU, until that rear crosses its start.
	rear_in: Option<Box<RearCalls>>,
	/// User code's calls at the rear of the MU it handed to an object without geometry, until that
	/// rear would cross its end.
	rear_off: Option<Box<RearCalls>>,
	/// A replan stopped to ask user code where the MU it lets in goes, so the answer replans.
	replan_when_decided: bool,
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
			belt_statistics: Box::new(BeltKeeper::new(length)),
			taken: 0.0,
			clears_at: None,
			short_until: None,
			arriving: None,
			cross: None,
			kinds: vec![MuKind::default()],
			rear_in: None,
			rear_off: None,
			replan_when_decided: false,
		}
	}

	/// A cross conveyor of `length` along its x axis, front to back, and `width` across, moving
	/// its MUs at `speed` metres per second, one at a time.
	pub fn new_cross(object_id: u32, length: f64, width: f64, speed: f64) -> Conveyor {
		let mut conveyor = Conveyor::new(object_id, length, width, speed);
		conveyor.cross = Some(Box::new(Cross::new(length, width)));
		conveyor
	}

	/// Its cross conveyor part, if it is one.
	pub fn cross(&self) -> Option<&Cross> {
		self.cross.as_deref()
	}

	pub(crate) fn cross_mut(&mut self) -> Option<&mut Cross> {
		self.cross.as_deref_mut()
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
		self.front(at, now) - self.along(&self.belt[at])
	}

	/// How long an MU of this kind is along its way, and how wide across it: its length and width,
	/// or the other way round while it lies crosswise.
	fn extent(&self, kind: MuKindId, crosswise: bool) -> (f64, f64) {
		let kind = self.kind(kind);
		if crosswise { (kind.width, kind.length) } else { (kind.length, kind.width) }
	}

	/// How long the MU is along the belt.
	#[inline]
	fn along(&self, on: &OnBelt) -> f64 {
		on.along
	}

	/// How far its front is ahead of its booking point. Lying crosswise, its booking point is at
	/// its middle along its way.
	fn booking_offset(&self, kind: MuKindId, crosswise: bool) -> f64 {
		let kind = self.kind(kind);
		if crosswise { kind.width / 2.0 } else { kind.booking_point * kind.length }
	}

	fn booking_offset_of(&self, on: &OnBelt) -> f64 {
		self.booking_offset(on.mu.kind(), on.crosswise)
	}

	/// Sets the MU at `at` at `front` from `now` on, moving at `speed` or standing, and says so.
	fn plan_mu(&mut self, at: usize, front: f64, speed: f64, context: &mut EventContext) {
		let on = &mut self.belt[at];
		on.front = front;
		on.since = context.time;
		on.speed = speed;
		let (mu_id, kind, crosswise) = (on.mu.id, on.mu.kind(), on.crosswise);
		let from_side = self.cross.as_ref().map(|cross| cross.leg_from);
		context.record(OccurrenceKind::MotionPlanned { mu_id, at: self.base.object_id, front: front as f32, speed: speed as f32, kind, crosswise, from_side });
	}

	/// Whether the MU offered could ever be on it, lying as it does.
	fn fits(&self, offer: &Offer) -> bool {
		let (along, across) = self.extent(offer.kind, offer.crosswise);
		if let Some(cross) = &self.cross {
			// Its way across depends on the side it comes in by.
			return cross.entry_side(offer.from).is_some_and(|side| cross.fits_from(side, along, across, EPSILON));
		}
		across <= self.width + EPSILON && along <= self.length + EPSILON
	}

	/// How far the last MU's rear must be past the start for the MU offered to enter. From
	/// another conveyor it enters with its front at the start, so only the gap is needed. From
	/// anything else it enters with its booking point there.
	fn room_needed(&self, offer: &Offer) -> f64 {
		if offer.from_belt { self.gap } else { self.booking_offset(offer.kind, offer.crosswise) + self.gap }
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
		if self.cross.as_ref().is_some_and(|cross| cross.stage == Stage::Switching) {
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
	fn settle(&mut self, lead: Option<Lead>, others: &OtherObjects<'_>, context: &mut EventContext) -> (Option<f64>, Option<(f64, f64)>) {
		let now = context.time;
		#[cfg(feature = "counters")]
		{
			use std::sync::atomic::Ordering::Relaxed;
			counters::REPLANS.fetch_add(1, Relaxed);
			counters::MUS_WALKED.fetch_add(self.belt.len() as u64, Relaxed);
		}
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
				ahead = Some((front - self.along(&self.belt[at]), speed));
			}
			// The last MU's rear and speed, unless the belt is empty and this is what was ahead.
			let tail = if self.belt.is_empty() { None } else { ahead };
			return (closes_up, tail);
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
			let mut last_front = 0.0;
			for at in 0..self.belt.len() {
				let front = self.front(at, now);
				self.apply(at, front, front, group, context);
				last_front = front;
			}
			let last = self.belt.len() - 1;
			return (closes_up, Some((last_front - self.along(&self.belt[last]), group)));
		}
		(closes_up, None)
	}

	/// Until when an MU with its rear at `rear`, moving at `speed`, is short of the gap past the
	/// start, as an event for when it clears would be queued.
	fn short_until(&self, rear: f64, speed: f64, now: SimTime) -> Option<SimTime> {
		if rear >= self.gap - EPSILON {
			None
		} else if speed > 0.0 {
			Some(now + Self::time_for(self.gap - rear, speed))
		} else {
			Some(SimTime::MAX)
		}
	}

	/// Works out its events again from how its MUs move now, voiding those queued.
	fn replan(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) {
		self.plan = self.plan.wrapping_add(1);
		self.replan_when_decided = false;
		if self.interruptions.is_interrupted() {
			return;
		}
		let now = context.time;
		if self.cross.as_ref().is_some_and(|cross| !cross.decided) {
			self.decide_exit(context, others);
			// User code is asked where the MU goes and answers at this instant: the events are
			// worked out then, once (ADR 0029).
			if self.cross.as_ref().is_some_and(|cross| !cross.decided) && self.distribution.is_code() && self.awaits_exit(others) {
				self.replan_when_decided = true;
				return;
			}
		}
		let had_lead = self.led_by.is_some();
		let mut lead = self.lead(others, now);
		if self.cross.is_some() {
			lead = self.cross_lead(had_lead, lead, now);
		}
		let (closes_up, tail) = self.settle(lead, others, context);
		self.short_until = tail.and_then(|(rear, speed)| self.short_until(rear, speed, now));
		// Before its front MU's events, so an MU as long as the conveyor has its rear in before it
		// leaves.
		if self.rear_in.is_some() {
			self.plan_rear_in(context);
		}
		if let Some(first) = self.belt.front() {
			if first.speed > 0.0 {
				let front = self.front(0, now);
				let event = match self.handing_to {
					Some(_) => Some((ConveyorStep::Booked, self.length + self.booking_offset_of(first) - front)),
					// A turning MU on a cross conveyor stops at its centre first.
					None if !self.front_at_end => match self.centre_ahead(first) {
						Some(centre) => Some((ConveyorStep::Centre, centre - front)),
						None => Some((ConveyorStep::FrontAtEnd, self.length - front)),
					},
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
		if let Some(cross) = &self.cross {
			if let Some(Timer::Until(at)) = cross.switch.filter(|_| cross.stage == Stage::Switching) {
				context.schedule(self.event(ConveyorStep::Switched, at.saturating_sub(now)));
			}
			if let Some(Timer::Until(at)) = cross.release {
				context.schedule(self.event(ConveyorStep::Released, at.saturating_sub(now)));
			}
		}
		if !self.retried && self.free_for_another() {
			let first = waiting_ids(&self.blockedlist).into_iter().find_map(|id| others.get(id).offer(now).filter(|offer| self.fits(offer)));
			if let Some(offer) = first {
				let need = self.room_needed(&offer);
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

	/// The rear of its last MU, whose calls at the rear it keeps, crosses the start now, or is planned
	/// to as it moves.
	#[cold]
	#[inline(never)]
	fn plan_rear_in(&mut self, context: &mut EventContext) {
		let now = context.time;
		let Some(last) = self.belt.len().checked_sub(1) else {
			self.rear_in = None;
			return;
		};
		let rear = self.rear(last, now);
		if rear >= -EPSILON {
			self.flush_rear_in(context);
		} else if self.belt[last].speed > 0.0 {
			context.schedule(self.event(ConveyorStep::RearIn, Self::time_for(-rear, self.belt[last].speed)));
		}
	}

	/// The calls at the rear of its last MU are made now.
	fn flush_rear_in(&mut self, context: &mut EventContext) {
		if let Some(rear) = self.rear_in.take() {
			context.note_calls(rear.calls);
		}
	}

	/// The calls at the rear of the MU it handed to an object without geometry are made now, but
	/// for a distribution asked there of an object that has passed the MU on already.
	fn flush_rear_off(&mut self, context: &mut EventContext, others: &OtherObjects<'_>) {
		if let Some(mut rear) = self.rear_off.take() {
			let now = context.time;
			let mu_id = rear.mu_id;
			rear.calls.retain(|call| match call.kind {
				crate::hooks::CallKind::Distribution { object, .. } => others.get(object).report(now).contents.contains(&mu_id),
				_ => true,
			});
			context.note_calls(rear.calls);
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
		let taken_by = match self.cross.as_ref() {
			// A cross conveyor offers its MU only to the exit it leaves by, if it has one.
			Some(cross) => match cross.exit {
				Some(exit) if only.is_none_or(|from| from == self.succ[exit]) => try_to_reserve_exit(self, context, others, mu_id, exit, only.is_some()),
				_ => None,
			},
			None => try_to_reserve_successor(self, context, others, mu_id, only),
		};
		let Some(taken_by) = taken_by else {
			return Ok(false);
		};
		self.front_at_end = false;
		self.retried = false;
		if let Some(next) = others.get_mut(taken_by).as_conveyor_mut() {
			next.reserved_by = Some(self.base.object_id);
		}
		if self.booking_offset_of(&self.belt[0]) <= EPSILON {
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
		if self.belt.is_empty() {
			self.short_until = None;
		}
		self.handing_to = None;
		self.front_at_end = false;
		self.statistics.count_mu_out();
		self.belt_statistics.crossed(context.time.saturating_sub(on.entered));
		self.taken -= self.along(&on);
		let from = self.base.object_id;
		context.moved(&on.mu, from, to);
		log::info!("seq={} MU {} moved from conveyor {} to {}", context.seq, on.mu.id, from, to);
		let to_belt = others.get(to).as_conveyor().is_some();
		// Its answer from user code goes with it, if it has one.
		if let Distributing::Code(coded) = &mut self.distribution {
			coded.forget_mu(on.mu.id);
		}
		// Into an object without geometry, its rear crosses the end later at this conveyor's speed.
		// Onto a conveyor, that one keeps the calls at its rear.
		if !to_belt && context.has_hooks() {
			let rears = context.take_rears();
			if !rears.is_empty() {
				let rest = self.along(&on) - self.booking_offset_of(&on);
				self.keep_rear_off(on.mu.id, rears, Self::time_for(rest, self.speed), context);
			}
		}
		if to_belt {
			// Its rear may still be on this one, which the MUs behind must not run into.
			self.led_by = Some(to);
		}
		if let Some(cross) = self.cross.as_deref_mut() {
			if let Some(exit) = cross.exit {
				cross.statistics.out(cross.exit_sides[exit]);
			}
			cross.turned_last = cross.turning;
			cross.stage = Stage::Approaching;
			cross.turning = false;
			cross.exit = None;
			cross.decided = false;
			// Into an object without geometry it is gone whole at once. Onto a conveyor, once its
			// rear has left (`cross_lead`).
			if !to_belt {
				Self::start_release(cross, context.time);
			}
		}
		others.get_mut(to).arriving(from, on.crosswise);
		if let Some(entered) = others.get_mut(to).enter(on.mu, context).map_err(|_| RunTimeError {})? {
			context.schedule(entered);
		}
		Ok(())
	}

	/// Keeps the calls at the rear of the MU it handed to an object without geometry until `after`
	/// from now, making any kept before first: that MU's rear crossed by now.
	#[cold]
	#[inline(never)]
	fn keep_rear_off(&mut self, mu_id: u32, calls: VecDeque<crate::hooks::HookCall>, after: SimTime, context: &mut EventContext) {
		if let Some(earlier) = self.rear_off.take() {
			context.note_calls(earlier.calls);
		}
		if after == SimTime::ZERO {
			context.note_calls(calls);
			return;
		}
		self.rear_off = Some(Box::new(RearCalls { mu_id, due: context.time + after, calls }));
		context.schedule(EventTemplate { eventtype: EventType::Conveyor(ConveyorTicket { step: ConveyorStep::RearOff as u8, plan: mu_id }), exec_time: after, object_id: self.base.object_id });
	}

	/// User code answered where its MU goes (ADR 0029): a cross conveyor chooses the exit of the MU
	/// it lets in, and a conveyor offers its front MU at the end.
	#[cold]
	#[inline(never)]
	fn decided(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		if self.interruptions.is_interrupted() {
			return Ok(());
		}
		if let Some(cross) = &self.cross {
			if !cross.decided {
				self.decide_exit(context, others);
			}
			if std::mem::take(&mut self.replan_when_decided) {
				self.replan(context, others);
			}
		} else if self.front_at_end && self.handing_to.is_none() && !self.belt.is_empty() {
			self.offer(context, others, None)?;
			self.replan(context, others);
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
			// The MU has gone whole, so neither a new plan nor an interruption holds its rear back.
			EventType::Conveyor(ticket) if ticket.step() == ConveyorStep::RearOff => {
				if self.rear_off.as_ref().is_some_and(|rear| rear.mu_id == ticket.plan && rear.due == context.time) {
					self.flush_rear_off(context, others);
				}
				Ok(())
			}
			EventType::Decided => self.decided(context, others),
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
						// User code answers where it goes at this instant, and its MUs move on as they
						// were until then (ADR 0029).
						if self.distribution.awaits(self.belt.front().map(|on| on.mu.id)) {
							return Ok(());
						}
					}
					ConveyorStep::Booked => {
						let to = self.handing_to.expect("a successor took the front MU");
						self.hand_over(to, context, others)?;
					}
					ConveyorStep::CloseUp | ConveyorStep::Straddle => {}
				ConveyorStep::Centre => {
					let Some(centre) = self.belt.front().and_then(|first| self.centre_ahead(first)) else {
						self.replan(context, others);
						return Ok(());
					};
					// Held up on the way, it isn't there: the plan was already void.
					if self.front(0, now) < centre - 1e-3 {
						self.replan(context, others);
						return Ok(());
					}
					self.turn_at_centre(centre, context);
				}
				ConveyorStep::Switched => self.leave_sideways(context),
				ConveyorStep::Released => {
					if let Some(cross) = self.cross.as_deref_mut() {
						cross.release = None;
					}
				}
					// Nothing on it moves differently, so nothing is worked out again.
					ConveyorStep::Decide => {
						if self.cross.as_ref().is_some_and(|cross| !cross.decided) {
							self.decide_exit(context, others);
						}
						return Ok(());
					}
					ConveyorStep::Room => {
						self.retried = true;
						retries_for_blocked(&mut self.blockedlist, self.base.object_id, context);
						return Ok(());
					}
					// Nothing on it moves differently.
					ConveyorStep::RearIn => {
						if self.rear_in.is_some() {
							self.plan_rear_in(context);
						}
						return Ok(());
					}
					ConveyorStep::Wake | ConveyorStep::RearOff => unreachable!("handled above"),
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

	/// Whether another MU could come in now, as far as what is already on it goes: on a cross
	/// conveyor, only when the last MU's rear has left, nothing is reserved and the release time
	/// is over. A conveyor takes one whenever its start has room, which `accepts` works out.
	fn free_for_another(&self) -> bool {
		match &self.cross {
			None => true,
			Some(cross) => self.belt.is_empty() && self.led_by.is_none() && self.reserved_by.is_none() && cross.release.is_none(),
		}
	}

	/// On a cross conveyor, where the front of `first` stops if it turns at the centre: with the MU's
	/// centre over the cross conveyor's.
	fn centre_ahead(&self, first: &OnBelt) -> Option<f64> {
		let cross = self.cross.as_ref()?;
		(cross.turning && cross.stage == Stage::Approaching).then(|| (self.length + self.along(first)) / 2.0)
	}

	/// On a cross conveyor, what its MUs must not run into on the next conveyor. The MU it handed
	/// over has left once its rear is off it, which the next conveyor tells it by waking it: then
	/// nothing on it is held back any more, and a release time begins after a turned MU.
	fn cross_lead(&mut self, had_lead: bool, lead: Option<Lead>, now: SimTime) -> Option<Lead> {
		let mut lead = lead;
		if self.led_by.is_some() && lead.is_some_and(|lead| lead.rear >= self.length - EPSILON) {
			self.led_by = None;
			lead = None;
		}
		if had_lead && self.led_by.is_none() {
			if let Some(cross) = self.cross.as_deref_mut() {
				Self::start_release(cross, now);
			}
		}
		lead
	}

	/// The MU handed over has left whole: after one that turned, the release time begins.
	fn start_release(cross: &mut Cross, now: SimTime) {
		if std::mem::take(&mut cross.turned_last) && cross.release_time > SimTime::ZERO {
			cross.release = Some(Timer::Until(now + cross.release_time));
		}
	}

	/// The MU turning on a cross conveyor has its centre over the cross conveyor's: it stands there
	/// for the switch time, and its exit is chosen now, among those that take where it goes, by the
	/// cross conveyor's own strategy.
	fn turn_at_centre(&mut self, centre: f64, context: &mut EventContext) {
		let now = context.time;
		self.plan_mu(0, centre, 0.0, context);
		let cross = self.cross.as_deref_mut().expect("only a cross conveyor turns MUs");
		cross.stage = Stage::Switching;
		cross.switch = Some(Timer::Until(now + cross.switch_time));
	}

	/// Chooses the exit of the MU on it, or of the one a conveyor before it is handing over, for
	/// which it has reserved room: as the MU is allowed in. Among the exits whose lists take its
	/// next destination, straight across included, its own strategy picks the first whose
	/// successor would take the MU now, otherwise the first in its order, which the MU then waits
	/// for. An MU no exit lists goes straight across.
	fn decide_exit(&mut self, context: &mut EventContext, others: &OtherObjects<'_>) {
		let Some(cross) = self.cross.as_deref() else { return };
		let (mu, crosswise, side) = match self.belt.front() {
			Some(on) => (&on.mu, on.crosswise, cross.leg_from),
			None => {
				let Some(giver) = self.reserved_by.filter(|&from| from != NOBODY) else { return };
				let Some((mu, crosswise)) = others.get(giver).as_conveyor().and_then(Conveyor::handed_mu) else { return };
				let Some(side) = cross.entry_side(giver) else { return };
				(mu, crosswise, side)
			}
		};
		let (destination, kind) = (context.routing.next_destination(mu), mu.kind());
		if self.distribution.is_code() {
			let mu = mu.clone();
			match self.exit_by_code(&mu, context) {
				CodeExit::Decided(exit) => {
					self.cross.as_deref_mut().expect("a cross conveyor").decide(side, exit);
					return;
				}
				CodeExit::Asked => return,
				CodeExit::Builtin => {}
			}
		}
		let Some(cross) = self.cross.as_deref() else { return };
		let takes = cross.exits_taking(destination);
		let exit = if takes == 0 {
			cross.exit_at(side.opposite())
		} else {
			// Leaving sideways it lies the other way round from how it came in.
			let mut turns = [false; 8];
			for (exit, &at) in cross.exit_sides.iter().enumerate().take(8) {
				turns[exit] = at != side.opposite();
			}
			let from = self.base.object_id;
			choose_exit(self, others, takes, &|exit| Offer { kind, from, from_belt: true, crosswise: crosswise != turns[exit] }, context.time)
		};
		self.cross.as_deref_mut().expect("a cross conveyor").decide(side, exit);
	}

	/// User code's answer for where the MU it lets in goes: an exit, none for hold, so it stays at
	/// the end it reaches until code moves it, or its own strategy. Asks once if there is no answer
	/// yet, which comes at this instant.
	#[cold]
	#[inline(never)]
	fn exit_by_code(&mut self, mu: &MoveableUnit, context: &mut EventContext) -> CodeExit {
		let id = self.base.object_id;
		let Distributing::Code(coded) = &mut self.distribution else { return CodeExit::Builtin };
		let decision = coded.decision(mu.id);
		if decision.is_some() {
			coded.forget_mu(mu.id);
		}
		match decision {
			Some(crate::hooks::Decision::Successor(to)) => CodeExit::Decided(self.succ.iter().position(|&succ| succ == to)),
			Some(crate::hooks::Decision::Hold) => CodeExit::Decided(None),
			Some(crate::hooks::Decision::Default) => CodeExit::Builtin,
			None => {
				crate::flows::ask_once(coded, context, id, mu.clone());
				CodeExit::Asked
			}
		}
	}

	/// Whether user code has been asked where the MU it lets in goes, and hasn't answered yet.
	fn awaits_exit(&self, others: &OtherObjects<'_>) -> bool {
		let mu = match self.belt.front() {
			Some(on) => Some(on.mu.id),
			None => self.reserved_by.filter(|&from| from != NOBODY).and_then(|giver| others.get(giver).as_conveyor()).and_then(Conveyor::handed_mu).map(|(mu, _)| mu.id),
		};
		self.distribution.awaits(mu)
	}

	/// The MU it is handing over to the next object, and whether it lies crosswise.
	fn handed_mu(&self) -> Option<(&MoveableUnit, bool)> {
		self.handing_to.and(self.belt.front()).map(|on| (&on.mu, on.crosswise))
	}

	/// The switch time is over: the turning MU leaves sideways from the centre, keeping its
	/// orientation, so it lies crosswise to its new way, or lengthwise again if it came in
	/// crosswise.
	fn leave_sideways(&mut self, context: &mut EventContext) {
		// Its rear crossed the side it came in by on its way to the centre.
		self.flush_rear_in(context);
		let cross = self.cross.as_deref_mut().expect("only a cross conveyor turns MUs");
		cross.switch = None;
		cross.stage = Stage::Leaving;
		let side = cross.exit.map_or(cross.leg_from.opposite(), |exit| cross.exit_sides[exit]);
		cross.leg_from = side.opposite();
		(self.length, self.width) = cross.dimensions_from(side);
		self.belt[0].crosswise = !self.belt[0].crosswise;
		self.belt[0].along = self.extent(self.belt[0].mu.kind(), self.belt[0].crosswise).0;
		let front = (self.length + self.along(&self.belt[0])) / 2.0;
		self.plan_mu(0, front, self.speed, context);
	}

	/// Takes the MU off for user code (ADR 0029), wherever it is, unless it is being handed over: to
	/// a successor that has taken it, or from the conveyor before, which still holds its rear (ADR
	/// 0023). The MUs behind it move on, and those waiting are offered the room.
	pub(crate) fn take_out(&mut self, mu_id: u32, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<MoveableUnit, crate::hooks::Refusal> {
		let id = self.base.object_id;
		let now = context.time;
		let Some(at) = self.belt.iter().position(|on| on.mu.id == mu_id) else {
			return Err(crate::hooks::Refusal::new(format!("object {id} doesn't hold MU {mu_id}")));
		};
		let last = self.belt.len() - 1;
		if let (0, Some(to)) = (at, self.handing_to) {
			return Err(crate::hooks::Refusal::new(format!("MU {mu_id} is being handed over to object {to}, and a transfer once begun always finishes")));
		}
		if let (true, Some(from)) = (at == last, self.straddle_from) {
			if self.rear(at, now) < -EPSILON {
				return Err(crate::hooks::Refusal::new(format!("MU {mu_id} still has its rear on object {from}, and a transfer once begun always finishes")));
			}
			self.straddle_from = None;
			Self::wake(from, context, others);
		}
		let on = self.belt.remove(at).expect("found above");
		if at == 0 && self.front_at_end {
			self.front_at_end = false;
			for &successor in &self.succ {
				others.get_mut(successor).relist_blocked(id, None);
			}
		}
		if self.rear_in.as_ref().is_some_and(|rear| rear.mu_id == mu_id) {
			self.rear_in = None;
		}
		self.taken -= self.along(&on);
		if self.belt.is_empty() {
			self.short_until = None;
		}
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.stage = Stage::Approaching;
			cross.turning = false;
			cross.exit = None;
			cross.decided = false;
			cross.switch = None;
			(self.length, self.width) = cross.dimensions_from(cross.leg_from);
		}
		self.statistics.count_mu_out();
		self.retried = false;
		self.replan(context, others);
		self.update_state(context);
		Ok(on.mu)
	}

	/// Sets where the MU at `at` is, exactly, without saying so: its movement doesn't change.
	fn plan_mu_quietly(&mut self, at: usize, front: f64, now: SimTime) {
		let on = &mut self.belt[at];
		on.front = front;
		on.since = now;
	}

	/// Empty, moving MUs, or every MU standing.
	fn process_state(&self) -> ProcessState {
		// Switching, or releasing after a turned MU, a cross conveyor is busy with nothing moving,
		// and so it is while an MU moves over it that belongs to a conveyor beside it: one coming in
		// that it has reserved the space for, or one it handed over whose rear is still on it.
		if self.cross.as_ref().is_some_and(|cross| cross.switching()) || (self.cross.is_some() && self.belt.is_empty() && (self.reserved_by.is_some() || self.led_by.is_some())) {
			return ProcessState::Processing;
		}
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
		let now = context.time;
		// The start came free since, with nothing else happening, which ended busy and full there.
		if let Some(at) = self.clears_at.take_if(|at| *at < now) {
			if let Some((held, time)) = self.statistics.split(at, MachineState::BusyNotFull) {
				self.belt_statistics.stretch(held, time);
			}
		}
		let machine = self.machine_state(now);
		if let Some((held, time)) = self.statistics.observe_stretch(now, machine, self.belt.len() as u64) {
			self.belt_statistics.stretch(held, time);
		}
		let interrupted = self.interruptions.is_interrupted();
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.statistics.observe(cross.switching() && !interrupted, now);
		}
		// Busy and full only because of the last MU's rear, it comes free when that moves clear. A
		// cross conveyor busy is always full: it holds one MU at a time.
		self.clears_at = if machine == MachineState::Busy && self.reserved_by.is_none() && self.cross.is_none() { self.short_until.filter(|&at| at != SimTime::MAX) } else { None };
		if self.belt.is_empty() {
			// So rounding never leaves a little behind.
			self.taken = 0.0;
		}
		self.belt_statistics.set_taken(self.taken);
	}

	/// Its statistics as of `now`, with busy and full ending where the start came free since.
	fn statistics_at(&self, now: SimTime) -> ObjectStatistics {
		match self.clears_at.filter(|&at| at < now) {
			Some(at) => {
				let mut keeper = self.statistics.clone();
				keeper.split(at, MachineState::BusyNotFull);
				keeper.read(now)
			}
			None => self.statistics.read(now),
		}
	}

	/// What its statistics call it: interrupted, empty, standing, or moving MUs, which is busy and
	/// full while its start is taken so that no MU of any size could enter, busy with room otherwise,
	/// and partly blocked while the MU at its end waits for a successor and others still move.
	fn machine_state(&self, now: SimTime) -> MachineState {
		let machine = match self.state {
			// Holding one MU at a time, a cross conveyor that is busy has no room for another.
			ProcessState::Processing if self.cross.is_some() => MachineState::Busy,
			ProcessState::Processing if self.front_at_end && self.handing_to.is_none() => MachineState::BusyPartlyBlocked,
			ProcessState::Processing => {
				let taken = self.reserved_by.is_some() || self.short_until.is_some_and(|until| now < until);
				if taken { MachineState::Busy } else { MachineState::BusyNotFull }
			}
			ProcessState::Blocked => MachineState::Blocked,
			ProcessState::Idle => MachineState::Idle,
			ProcessState::Receiving => MachineState::Receiving,
		};
		match self.interruptions.most_serious() {
			None => machine,
			interruption => MachineState::of(interruption, &ProcessState::Idle),
		}
	}
}

impl Conveyor {
	/// Keeps user code's calls at the rear of the MU entering now until its rear crosses the start,
	/// making those of the MU before it first: that one's rear crossed by now.
	#[cold]
	#[inline(never)]
	fn keep_rear_in(&mut self, mu_id: u32, context: &mut EventContext) {
		self.flush_rear_in(context);
		let calls = context.take_rears();
		if !calls.is_empty() {
			self.rear_in = Some(Box::new(RearCalls { mu_id, due: SimTime::MAX, calls }));
		}
	}
}

/// What user code answered about where the MU a cross conveyor lets in goes.
enum CodeExit {
	/// This exit, or none.
	Decided(Option<usize>),
	/// A call is asking, and its answer comes at this instant.
	Asked,
	/// The cross conveyor's own strategy chooses.
	Builtin,
}

impl MaterialFlow for Conveyor {
	/// It takes an MU only once it knows which: see `accepts`.
	#[inline(never)]
	fn is_entrance_allowed(&self) -> bool {
		false
	}
	#[inline(never)]
	fn accepts(&self, offer: &Offer, now: SimTime) -> bool {
		if self.interruptions.is_interrupted() || self.reserved_by.is_some() || !self.fits(offer) {
			return false;
		}
		if self.cross.is_some() {
			return self.free_for_another();
		}
		match self.belt.back() {
			None => true,
			Some(_) => self.rear(self.belt.len() - 1, now) >= self.room_needed(offer) - EPSILON,
		}
	}
	#[inline(never)]
	fn arriving(&mut self, from: u32, crosswise: bool) {
		self.arriving = Some((from, crosswise));
	}
	/// Another conveyor's MU has its front at the start: nothing else may enter until it has been
	/// handed over (ADR 0023).
	#[inline(never)]
	fn reserve(&mut self, _mu_id: u32, context: &mut EventContext) {
		self.reserved_by = Some(NOBODY);
		// Its start is taken from now on.
		self.update_state(context);
		// A cross conveyor chooses the exit of the MU it has let in now, once it knows whose it is.
		if self.cross.is_some() {
			context.schedule(self.event(ConveyorStep::Decide, SimTime::ZERO));
		}
	}
	#[inline(never)]
	fn is_exit_allowed(&self) -> bool {
		self.front_at_end && self.handing_to.is_none() && !self.interruptions.is_interrupted()
	}
	/// The MU enters with its booking point at the start, the rest of it behind, still taking
	/// that space (ADR 0023), and moves on.
	#[inline(never)]
	fn process_mu_entrance(&mut self, mu: MoveableUnit, _arrival: Arrival, context: &mut EventContext) -> Result<Option<EventTemplate>, MaterialflowError> {
		log::info!("MU {} entered conveyor {}", mu.id, self.base.object_id);
		let arriving = self.arriving.take();
		let crosswise = arriving.is_some_and(|(_, crosswise)| crosswise);
		if let Some(cross) = self.cross.as_deref_mut() {
			let from = arriving.map(|(from, _)| from).or(self.reserved_by.filter(|&from| from != NOBODY));
			let side = from.and_then(|from| cross.entry_side(from)).ok_or(MaterialflowError::EntranceDenied)?;
			cross.enter(side);
			(self.length, self.width) = cross.dimensions_from(side);
		}
		let front = self.booking_offset(mu.kind(), crosswise);
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
		let along = self.extent(mu.kind(), crosswise).0;
		self.taken += along;
		self.short_until = self.short_until(front - along, speed, context.time);
		if context.has_hooks() {
			self.keep_rear_in(mu.id, context);
		}
		self.belt.push_back(OnBelt { mu, front, since: context.time, speed, entered: context.time, crosswise, along });
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
		self.belt_statistics.reset();
		self.taken = 0.0;
		self.clears_at = None;
		self.short_until = None;
		self.arriving = None;
		self.rear_in = None;
		self.rear_off = None;
		self.replan_when_decided = false;
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.reset();
			(self.length, self.width) = (cross.length, cross.width);
		}
	}
	#[inline(never)]
	fn clear_statistics(&mut self, now: SimTime) {
		if let Some(at) = self.clears_at.take_if(|at| *at < now) {
			self.statistics.split(at, MachineState::BusyNotFull);
		}
		self.interruptions.clear_statistics(now, &self.statistics);
		self.statistics.clear(now);
		self.belt_statistics.clear();
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.statistics.clear(now);
		}
	}
	#[inline(never)]
	fn report(&self, now: SimTime) -> ObjectState {
		let stopped = self.interruptions.is_interrupted();
		let (fill, belt) = self.belt_statistics.read(self.statistics.in_progress(now), self.belt.front().map(|first| now.saturating_sub(first.entered)));
		ObjectState {
			id: self.base.object_id,
			kind: if self.cross.is_some() { ObjectKind::CrossConveyor } else { ObjectKind::Conveyor },
			contents: self.belt.iter().map(|on| on.mu.id).collect(),
			content_kinds: self.belt.iter().map(|on| on.mu.kind()).collect(),
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
			statistics: self.statistics_at(now),
			plan: None,
			fill: Some(Box::new(fill)),
			belt: Some(Box::new(belt)),
			cross: self.cross.as_ref().map(|cross| Box::new(cross.statistics.read(now))),
			positions: (0..self.belt.len())
				.map(|at| BeltPosition {
					mu_id: self.belt[at].mu.id,
					front: self.front(at, now),
					speed: if stopped { 0.0 } else { self.belt[at].speed },
					kind: self.belt[at].mu.kind(),
					crosswise: self.belt[at].crosswise,
					from_side: self.cross.as_ref().map(|cross| cross.leg_from),
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
		// A cross conveyor's switch time and release time freeze.
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.switch = cross.switch.map(|timer| timer.freeze(now));
			cross.release = cross.release.map(|timer| timer.freeze(now));
		}
	}
	/// Its MUs move again where their way is clear, as `replan` works out, and the front MU at the
	/// end is offered again.
	#[inline(never)]
	fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		self.retried = false;
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.switch = cross.switch.map(|timer| timer.thaw(context.time));
			cross.release = cross.release.map(|timer| timer.thaw(context.time));
		}
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
		if let Some(cross) = self.cross.as_deref_mut() {
			cross.reset();
		}
		self.short_until = None;
		self.taken = 0.0;
		self.front_at_end = false;
		self.handing_to = None;
		self.led_by = None;
		self.straddle_from = None;
		self.reserved_by = None;
		self.rear_in = None;
		self.rear_off = None;
		self.state = ProcessState::Idle;
	}
	#[inline(never)]
	fn take_content(&mut self, _now: SimTime) -> Option<MoveableUnit> {
		self.front_at_end = false;
		let on = self.belt.pop_front()?;
		if self.belt.is_empty() {
			self.short_until = None;
		}
		self.taken -= self.along(&on);
		Some(on.mu)
	}
	/// Only its front MU at the end is offered, from when it got there.
	#[inline(never)]
	fn offered_since(&self, _now: SimTime) -> Option<SimTime> {
		(self.front_at_end && self.handing_to.is_none() && !self.belt.is_empty()).then_some(self.ready_since)
	}
	#[inline(never)]
	fn offered_crosswise(&self) -> bool {
		self.belt.front().is_some_and(|on| on.crosswise)
	}
	#[inline(never)]
	fn offered_kind(&self, _now: SimTime) -> Option<MuKindId> {
		self.belt.front().filter(|_| self.front_at_end && self.handing_to.is_none()).map(|on| on.mu.kind())
	}
	fn offered_mu(&self, _now: SimTime) -> Option<&MoveableUnit> {
		self.belt.front().filter(|_| self.front_at_end && self.handing_to.is_none()).map(|on| &on.mu)
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
