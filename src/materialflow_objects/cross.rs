//! What makes a conveyor a cross conveyor (spec issue 148): four sides, one MU at a time, exit
//! sides listing the destinations they take, a switch time for an MU turning at its centre and a
//! release time after it has left. The rest, how an MU moves, enters and is handed over, is the
//! conveyor's own, so a cross conveyor is a `Conveyor` with this part.
//!
//! Each MU's way across is a leg along one axis, from the side it starts at. Going straight
//! through, that is its entry side and the leg runs to the opposite side. Turning, it runs to the
//! centre, stands there for the switch time, and starts a second leg sideways from the centre,
//! keeping its orientation, so it lies crosswise to its new way.

use crate::flows::{DestinationId, NO_DESTINATION};
use crate::routing::Routing;
use crate::time::SimTime;

/// One of a cross conveyor's four sides. Front is where its own x axis points, left 90° counter
/// clockwise from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
	Front,
	Back,
	Left,
	Right,
}

impl Side {
	pub const ALL: [Side; 4] = [Side::Front, Side::Back, Side::Left, Side::Right];

	pub fn opposite(self) -> Side {
		match self {
			Side::Front => Side::Back,
			Side::Back => Side::Front,
			Side::Left => Side::Right,
			Side::Right => Side::Left,
		}
	}

	/// Whether a way to or from this side runs along the cross conveyor's x axis, its length.
	pub fn along_length(self) -> bool {
		matches!(self, Side::Front | Side::Back)
	}

	/// Its place in `Side::ALL`, which per-side figures are kept in.
	pub fn index(self) -> usize {
		self as usize
	}

	/// Its name in model files: `front`, `back`, `left` or `right`.
	pub fn name(self) -> &'static str {
		match self {
			Side::Front => "front",
			Side::Back => "back",
			Side::Left => "left",
			Side::Right => "right",
		}
	}

	pub fn from_name(name: &str) -> Option<Side> {
		Side::ALL.into_iter().find(|side| side.name() == name)
	}
}

/// Where the MU on a cross conveyor is in its way across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
	/// Going straight through, or to the centre to turn there.
	Approaching,
	/// Standing at the centre for the switch time.
	Switching,
	/// On its second leg, sideways from the centre to the side it leaves by.
	Leaving,
}

/// A wait that freezes while the cross conveyor is interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
	Until(SimTime),
	/// Frozen, with this much left.
	Left(SimTime),
}

impl Timer {
	pub(crate) fn freeze(self, now: SimTime) -> Timer {
		match self {
			Timer::Until(at) => Timer::Left(at.saturating_sub(now)),
			frozen => frozen,
		}
	}

	pub(crate) fn thaw(self, now: SimTime) -> Timer {
		match self {
			Timer::Left(left) => Timer::Until(now + left),
			running => running,
		}
	}
}

/// A cross conveyor's own part: its geometry, sides, exits and times, and what the MU on it is doing.
#[derive(Debug, Clone)]
pub struct Cross {
	/// Along its x axis, front to back, in metres.
	pub length: f64,
	/// Along its y axis, left to right, in metres.
	pub width: f64,
	/// How long a turning MU stands at the centre.
	pub switch_time: SimTime,
	/// How long it takes no MU after one that turned has left.
	pub release_time: SimTime,
	/// Each predecessor and the side it hands MUs over at.
	pub(crate) entries: Vec<(u32, Side)>,
	/// Each exit's side, by its place among the successors.
	pub(crate) exit_sides: Vec<Side>,
	/// The destinations each exit side lists, as given.
	pub(crate) lists: Vec<(Side, Vec<DestinationId>)>,
	/// By destination, the exits that take it, a bit per exit: worked out when the run is built.
	takes: Vec<u8>,
	/// The side the leg of the MU on it starts from.
	pub(crate) leg_from: Side,
	pub(crate) stage: Stage,
	/// The MU on it turns at its centre.
	pub(crate) turning: bool,
	/// The exit it leaves by, chosen as the MU is allowed in. `None` where no exit takes it.
	pub(crate) exit: Option<usize>,
	/// The exit has been chosen for the MU on it, or the one coming in that it reserved room for.
	pub(crate) decided: bool,
	pub(crate) switch: Option<Timer>,
	pub(crate) release: Option<Timer>,
	/// The MU handed over last turned, so the release time follows once it has left.
	pub(crate) turned_last: bool,
	pub(crate) statistics: CrossKeeper,
}

/// A cross conveyor's own figures (spec issue 148).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CrossStatistics {
	/// Time spent switching, an MU standing at the centre, and releasing after a turned one, not
	/// counting interruptions. Part of its busy time.
	pub time_switching: SimTime,
	/// MUs handed over by each side, by `Side::index`.
	pub mus_out: [u64; 4],
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CrossKeeper {
	figures: CrossStatistics,
	/// Since when it has been switching or releasing, while it is.
	switching_since: Option<SimTime>,
}

impl CrossKeeper {
	/// Notes whether it is switching now.
	pub(crate) fn observe(&mut self, switching: bool, now: SimTime) {
		match (self.switching_since, switching) {
			(None, true) => self.switching_since = Some(now),
			(Some(since), false) => {
				self.figures.time_switching = self.figures.time_switching + now.saturating_sub(since);
				self.switching_since = None;
			}
			_ => {}
		}
	}

	pub(crate) fn out(&mut self, side: Side) {
		self.figures.mus_out[side.index()] += 1;
	}

	pub(crate) fn read(&self, now: SimTime) -> CrossStatistics {
		let mut figures = self.figures;
		if let Some(since) = self.switching_since {
			figures.time_switching = figures.time_switching + now.saturating_sub(since);
		}
		figures
	}

	pub(crate) fn clear(&mut self, now: SimTime) {
		self.figures = CrossStatistics::default();
		self.switching_since = self.switching_since.map(|_| now);
	}
}

impl Cross {
	pub fn new(length: f64, width: f64) -> Cross {
		Cross {
			length,
			width,
			switch_time: SimTime::from_secs(2),
			release_time: SimTime::ZERO,
			entries: Vec::new(),
			exit_sides: Vec::new(),
			lists: Vec::new(),
			takes: Vec::new(),
			leg_from: Side::Back,
			stage: Stage::Approaching,
			turning: false,
			exit: None,
			decided: false,
			switch: None,
			release: None,
			turned_last: false,
			statistics: CrossKeeper::default(),
		}
	}

	/// How far a leg from `side` runs, and how wide the cross conveyor is across it.
	pub(crate) fn dimensions_from(&self, side: Side) -> (f64, f64) {
		if side.along_length() { (self.length, self.width) } else { (self.width, self.length) }
	}

	/// The side `from` hands MUs over at, if it is a predecessor.
	pub(crate) fn entry_side(&self, from: u32) -> Option<Side> {
		self.entries.iter().find(|(id, _)| *id == from).map(|&(_, side)| side)
	}

	/// The exit at `side`, if one leaves there.
	pub(crate) fn exit_at(&self, side: Side) -> Option<usize> {
		self.exit_sides.iter().position(|&exit| exit == side)
	}

	/// The exits that take MUs going to `destination`, a bit per exit.
	pub(crate) fn exits_taking(&self, destination: DestinationId) -> u8 {
		if destination == NO_DESTINATION {
			return 0;
		}
		self.takes.get(usize::from(destination)).copied().unwrap_or(0)
	}

	/// Works out which exits take which destination: an exit takes one if any object it stands for
	/// is one that the exit's listed destinations stand for.
	pub(crate) fn build_takes(&mut self, routing: &Routing) {
		let listed: Vec<Vec<u32>> = self
			.exit_sides
			.iter()
			.map(|side| {
				let mut objects: Vec<u32> = self.lists.iter().filter(|(listed, _)| listed == side).flat_map(|(_, destinations)| destinations.iter().flat_map(|&d| routing.members(d).iter().copied())).collect();
				objects.sort_unstable();
				objects.dedup();
				objects
			})
			.collect();
		self.takes = (0..routing.destinations())
			.map(|destination| {
				let members = routing.members(destination as DestinationId);
				listed.iter().enumerate().filter(|(_, objects)| members.iter().any(|object| objects.binary_search(object).is_ok())).fold(0u8, |bits, (exit, _)| bits | (1 << exit))
			})
			.collect();
	}

	/// An MU enters from `side`, its way starting there.
	pub(crate) fn enter(&mut self, side: Side) {
		self.leg_from = side;
		self.stage = Stage::Approaching;
		self.switch = None;
	}

	/// The MU coming in from `side` leaves by `exit`: turning at the centre unless that exit is
	/// straight across.
	pub(crate) fn decide(&mut self, side: Side, exit: Option<usize>) {
		self.leg_from = side;
		self.exit = exit;
		self.turning = exit.is_some_and(|exit| self.exit_sides[exit] != side.opposite());
		self.decided = true;
	}

	/// The run's state goes: nothing on it, and its figures afresh.
	pub(crate) fn reset(&mut self) {
		self.stage = Stage::Approaching;
		self.turning = false;
		self.exit = None;
		self.decided = false;
		self.switch = None;
		self.release = None;
		self.turned_last = false;
		self.statistics = CrossKeeper::default();
	}

	/// Whether it is switching or releasing, counting as busy with no MU moving.
	pub(crate) fn switching(&self) -> bool {
		self.stage == Stage::Switching || self.release.is_some()
	}

	/// Whether an MU of this extent, along and across the way it comes in, fits from `side`.
	pub(crate) fn fits_from(&self, side: Side, along: f64, across: f64, epsilon: f64) -> bool {
		let (way, breadth) = self.dimensions_from(side);
		along <= way + epsilon && across <= breadth + epsilon
	}
}
