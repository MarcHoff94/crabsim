use crate::error::{InitError, MaterialflowError, RunTimeError};
use crate::event::{EventContext, EventTemplate, EventType, ExitTicket};
use crate::report::{Capacity, ObjectKind, ObjectState, OccurrenceKind};
use crate::statistics::{MachineState, StatisticsKeeper};
use crate::time::SimTime;
use crate::materialflow_objects::OtherObjects;
use crate::flows::{
	EventInterface, MaterialFlow, MoveableUnit, MuKindId, ProcessState, SuccessorContainer, Distributing, retry_move_to_successor, try_to_move_to_successor,
};
use super::flows::{BaseObject, FailureProfile};
use super::interruptions::{Interruptible, Interruptions};

pub fn handle_source_mu_exit(
	source: &mut Source,
	context: &mut EventContext,
	others: &mut OtherObjects<'_>,
) -> Result<(), RunTimeError> {
	// Its interval has ended, so its MU is ready to leave from now.
	source.ready_since = context.time;
	source.pass_on(context, others)
}

impl Source {
	/// Offers its MU to its successors, and starts the next interval once it has gone: its next
	/// MU's exit is scheduled before the successor's event.
	fn pass_on(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		let moved = try_to_move_to_successor(self, context, others).map_err(|_| RunTimeError {})?;
		if self.content.is_none() {
			let next_mu_event = self.create_mu(context).map_err(|_| RunTimeError {})?;
			context.schedule(next_mu_event);
		} else if moved.is_none() && !self.distribution.awaits(self.content.as_ref().map(|mu| mu.id)) {
			self.become_blocked(context);
		}
		if let Some(entered) = moved {
			context.schedule(entered);
		}
		Ok(())
	}
}

#[derive(Clone)]
pub struct Source {
	pub base: BaseObject,
	pub content: Option<MoveableUnit>,
	pub proc_time: SimTime,
	pub completion_time: Option<SimTime>,
	/// When its MU's interval ended. Read only while that MU waits to leave.
	ready_since: SimTime,
	pub succ: Vec<u32>,
	pub succ_pointer: usize,
	/// How it picks the successor for its MU. Set by the run, which ranks the successors.
	pub distribution: Distributing,
	pub state: ProcessState,
	/// Its failures and pauses (ADR 0020). Interrupted, it creates no MUs and releases none.
	pub interruptions: Interruptions,
	/// Bumped when an interruption freezes its interval, so the exit queued for it is stale.
	stop_count: u32,
	/// What was left of its interval when an interruption froze it.
	frozen: Option<SimTime>,
	pub statistics: StatisticsKeeper,
	/// The kind of every MU it creates (ADR 0021): 0, the built-in MU, unless set.
	pub mu_kind: MuKindId,
}
impl Source {
	pub fn new(
		object_id: u32,
		proc_time: SimTime,
		succ: Vec<u32>,
	) -> Source {
		Source {
			base: BaseObject { object_id, random_seed: 0 },
			content: None,
			proc_time,
			completion_time: None,
			ready_since: SimTime::ZERO,
			succ,
			succ_pointer: 0,
			distribution: Distributing::RoundRobin,
			state: ProcessState::Idle,
			interruptions: Interruptions::new(None),
			stop_count: 0,
			frozen: None,
			statistics: StatisticsKeeper::new(0),
			mu_kind: 0,
		}
	}
	/// Gives it failures, or none with `None`.
	pub fn set_failures(&mut self, failures: Option<FailureProfile>) {
		self.interruptions.set_failures(failures);
	}
	fn exit_event(&self, mu_id: u32, after: SimTime) -> EventTemplate {
		EventTemplate { eventtype: EventType::MUExit(ExitTicket { mu_id, stop_count: self.stop_count }), exec_time: after, object_id: self.base.object_id }
	}
	pub fn create_mu(&mut self, context: &mut EventContext) -> Result<EventTemplate, MaterialflowError> {
		if self.content.is_some() {
			return Err(MaterialflowError::NoMU);
		}
		let mu_id = context.new_mu_id();
		let mut mu = MoveableUnit::new(mu_id, context.time, self.mu_kind);
		// Its work plan comes with its kind (ADR 0026), and it starts at the plan's first step.
		mu.step = context.routing.first_step(self.mu_kind);
		self.content = Some(mu);
		self.statistics.count_mu_in();
		log::info!("Source {} created MU {}", self.base.object_id, mu_id);
		// Its MU coming into being is its entry (ADR 0029).
		if let Some(mu) = &self.content {
			context.created(mu, self.base.object_id, self.proc_time);
		}
		if self.state == ProcessState::Blocked {
			context.record(OccurrenceKind::Unblocked { object_id: self.base.object_id });
		}
		self.state = ProcessState::Processing;
		self.completion_time = Some(context.time + self.proc_time);
		Ok(self.exit_event(mu_id, self.proc_time))
	}
	/// Whether its MU's interval is still running, or frozen, so it isn't ready to leave.
	fn in_interval(&self, now: SimTime) -> bool {
		self.frozen.is_some() || self.completion_time.is_some_and(|completion| completion > now)
	}
	/// Takes the MU out for user code (ADR 0029). Taken during its interval, the next MU comes when
	/// the interval ends, as if this one had left then. Taken once ready, the next interval starts
	/// now, as when an MU leaves.
	pub(crate) fn take_out(&mut self, mu_id: u32, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<MoveableUnit, crate::hooks::Refusal> {
		if self.content.as_ref().map(|mu| mu.id) != Some(mu_id) {
			return Err(crate::hooks::Refusal::new(format!("object {} doesn't hold MU {mu_id}", self.base.object_id)));
		}
		let mut mu = self.content.take().expect("checked above");
		mu.set_left_source(context.time);
		self.statistics.count_mu_out();
		if !self.in_interval(context.time) {
			for &successor in &self.succ {
				others.get_mut(successor).relist_blocked(self.base.object_id, None);
			}
			if let Ok(next) = self.create_mu(context) {
				context.schedule(next);
				if self.interruptions.is_interrupted() {
					self.stop_began(context);
				}
			}
		}
		self.update_statistics(context.time);
		Ok(mu)
	}
	fn handle_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<(), RunTimeError> {
		match eventtype {
			// An exit queued before an interruption froze the interval is stale.
			&EventType::MUExit(ticket) if self.interruptions.is_interrupted() || ticket.stop_count != self.stop_count => Ok(()),
			&EventType::MUExit(_) => handle_source_mu_exit(self, context, others),
			// User code answered where its MU goes (ADR 0029). Answered as the MU was made, where the
			// distribution's moment has moved there, it waits for its interval.
			EventType::Decided if self.interruptions.is_interrupted() || self.content.is_none() || self.in_interval(context.time) => Ok(()),
			EventType::Decided => self.pass_on(context, others),
			&EventType::BlockedRetry { from } => {
				// Interrupted while its MU waits, it keeps its place in the line, as if refused
				// (spec issue 122).
				if self.interruptions.is_interrupted() && self.content.is_some() && self.state != ProcessState::Processing {
					others.get_mut(from).add_blocked(self.base.object_id, self.ready_since, true);
					return Ok(());
				}
				// successor notified that it became available; attempt to move
				if !self.is_exit_allowed() {
					return Ok(());
				}
				let moved = retry_move_to_successor(self, context, others, from).map_err(|_| RunTimeError {})?;
				// Once its MU has gone, its next MU's interval starts, scheduled before the
				// successor's event.
				if self.content.is_none() {
					if let Ok(next_ev) = self.create_mu(context) {
						context.schedule(next_ev);
					}
				}
				if let Some(entered) = moved {
					context.schedule(entered);
				}
				Ok(())
			}
			_ => self.handle_interruption(eventtype, context, others).unwrap_or(Err(RunTimeError {})),
		}
	}
	/// Tells the statistics what the Source ended up doing. It has no MU-level statistics.
	fn update_statistics(&mut self, now: SimTime) {
		self.statistics.observe(now, MachineState::of(self.interruptions.most_serious(), &self.state), 0);
	}
	fn become_blocked(&mut self, context: &mut EventContext) {
		if self.state != ProcessState::Blocked {
			context.record(OccurrenceKind::Blocked { object_id: self.base.object_id });
		}
		self.state = ProcessState::Blocked;
		self.completion_time = None;
	}
}

impl EventInterface for Source {
	fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> {
		let mut events: Vec<EventTemplate> = self.interruptions.first_failure(self.base.object_id).into_iter().collect();
		let outcome = match self.create_mu(context) {
			Ok(e) => {
				events.push(e);
				Ok(events)
			}
			Err(_) => Err(InitError {}),
		};
		self.update_statistics(context.time);
		outcome
	}
	fn execute_event(
		&mut self,
		context: &mut EventContext,
		eventtype: &EventType,
		others: &mut OtherObjects<'_>,
	) -> Result<(), RunTimeError> {
		let outcome = self.handle_event(context, eventtype, others);
		self.update_statistics(context.time);
		outcome
	}
	fn reset(&mut self) {
		self.content = None;
		self.completion_time = None;
		self.ready_since = SimTime::ZERO;
		self.succ_pointer = 0;
		self.state = ProcessState::Idle;
		self.interruptions.reset();
		self.stop_count = 0;
		self.frozen = None;
		self.statistics.reset();
	}
	fn clear_statistics(&mut self, now: SimTime) {
		self.interruptions.clear_statistics(now, &self.statistics);
		self.statistics.clear(now);
	}
	fn report(&self, now: SimTime) -> ObjectState {
		ObjectState {
			id: self.base.object_id,
			kind: ObjectKind::Source,
			contents: self.content.iter().map(|mu| mu.id).collect(),
			content_kinds: self.content.iter().map(|mu| mu.kind()).collect(),
			process: self.state.clone(),
			interrupted: self.interruptions.most_serious(),
			interruptions: self.interruptions.list(),
			failure_profiles: self.interruptions.profile_states(now, &self.statistics),
			remaining: self.frozen.or(self.completion_time.map(|completion| completion.saturating_sub(now))),
			work: Vec::new(),
			blocked_predecessors: Vec::new(),
			successors: self.succ.clone(),
			capacity: Capacity::Limited(self.statistics.capacity()),
			process_time: None,
			configured_process_time: None,
			statistics: self.statistics.read(now),
			plan: None,
			fill: None,
			belt: None,
			positions: Vec::new(),
			cross: None,
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
		self.interruptions.reseed(seed);
	}
}
impl MaterialFlow for Source {
	/// Only an MU whose interval has ended may leave. A retry can arrive during the interval,
	/// from a successor that listed the Source before its last MU went elsewhere.
	fn is_exit_allowed(&self) -> bool {
		self.state != ProcessState::Processing && !self.interruptions.is_interrupted()
	}
	fn is_entrance_allowed(&self) -> bool {
		false
	}
	fn process_mu_entrance(
		&mut self,
		_mu: MoveableUnit,
		_arrival: crate::routing::Arrival,
		_context: &mut EventContext,
	) -> Result<Option<EventTemplate>, MaterialflowError> {
		panic!("Source should not be processing MU entrance")
	}
	fn add_blocked(&mut self, _id_blocked: u32, _since: SimTime, _answering_retry: bool) {
		panic!("Source cannot receive blocked objects")
	}
	fn relist_blocked(&mut self, _id_blocked: u32, _next: Option<SimTime>) {}
	fn notify_blocked_predecessors(&mut self, _context: &mut EventContext) -> Result<(), MaterialflowError> {
		//source has no predecessors
		Ok(())
	}
}

impl SuccessorContainer for Source {
	fn distribution_mut(&mut self) -> &mut Distributing {
		&mut self.distribution
	}
	fn successors(&self) -> &[u32] {
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
		self.state = ProcessState::Idle
	}
	/// An MU created at the start of an interval leaves at the end of it, and its lead time
	/// starts here. Time spent blocked in the Source is the Source's blocked time.
	fn take_content(&mut self, now: SimTime) -> Option<MoveableUnit> {
		let mut mu = self.content.take()?;
		mu.set_left_source(now);
		Some(mu)
	}
	fn offered_since(&self, _now: SimTime) -> Option<SimTime> {
		self.content.as_ref().map(|_| self.ready_since)
	}
	fn offered_kind(&self, _now: SimTime) -> Option<MuKindId> {
		self.content.as_ref().map(|mu| mu.kind())
	}
	fn offered_mu(&self, _now: SimTime) -> Option<&MoveableUnit> {
		self.content.as_ref()
	}
	fn statistics_mut(&mut self) -> &mut StatisticsKeeper {
		&mut self.statistics
	}
	fn statistics_and_kind(&self) -> (&StatisticsKeeper, crate::ObjectKind) {
		(&self.statistics, crate::ObjectKind::Source)
	}
}

impl Interruptible for Source {
	fn interruption_parts(&mut self) -> (&mut Interruptions, &mut StatisticsKeeper, u32) {
		(&mut self.interruptions, &mut self.statistics, self.base.object_id)
	}
	/// Its interval freezes, so its next creation moves later by the interruption's length.
	fn stop_began(&mut self, context: &mut EventContext) {
		if self.state == ProcessState::Processing {
			self.frozen = self.completion_time.map(|completion| completion.saturating_sub(context.time));
			self.completion_time = None;
			self.stop_count += 1;
		}
	}
	/// Its interval carries on where it froze, or its waiting MU is offered again, keeping the
	/// ready time it had.
	fn stop_ended(&mut self, context: &mut EventContext, others: &mut OtherObjects<'_>) -> Result<(), RunTimeError> {
		if let Some(left) = self.frozen.take() {
			self.completion_time = Some(context.time + left);
			// Its MU may have been taken by user code meanwhile: the interval still ends then.
			let mu_id = self.content.as_ref().map_or(0, |mu| mu.id);
			context.schedule(self.exit_event(mu_id, left));
		} else if self.content.is_some() {
			self.pass_on(context, others)?;
		}
		Ok(())
	}
}
