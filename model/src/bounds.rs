//! Bounds (ADR 0030): what crabsim can tell about an expectation without running the model, so
//! an agent learns before it runs anything that one can't be met, or contradicts another. They
//! are warnings, found only when asked for, as `validate_model`'s other warnings are, and never
//! stop a check: the check's verdict is what counts.
//!
//! A bound follows from what the model says and nothing else, so it is certain, with one
//! exception: a since-start throughput counts from the Drain's first MU, and is bounded as if
//! that came as early as it can.
//! - **What a figure can be:** a share from 0% to 100%, a count from 0.
//! - **What an object holds:** never more than its capacity.
//! - **What never happens in a check:** a failure at the verification stage, which runs without
//!   them, or of an object without a failure profile; a pause or a stop by hand; receiving where
//!   no conveyor leads in.
//! - **How many MUs can pass:** the most an object can pass on in a stretch of time, from a
//!   Source's interval, a Station's process time and capacity, a conveyor's speed and the MUs'
//!   spacing, carried along the connections as a maximum flow, with what objects can hold when
//!   the stretch begins. It bounds MUs in and out, a Drain's throughput and a Station's busy share.
//! - **How long an MU takes:** at least the work and dwelling on the shortest way from where it
//!   starts, which bounds lead times and a conveyor's crossing time from below.
//! - **What two expectations ask together:** the same figure of the same instance, read the same
//!   way in the same check, can't lie in two places, and an object's shares of time add up to
//!   100%.
//!
//! User code can move MUs, change process times, pause and stop objects (ADR 0029), so a model
//! with code gets only the bounds that hold whatever it does: a figure's range, capacities and
//! contradictions.

use crate::agree::Settings;
use crate::expect::{Aggregate, Bound, CheckStage, CheckSubject, Comparison, Target};
use crate::{Model, Problem};
use crabsim::figures::{Figure, Scope, Unit};
use crabsim::time::format_time_exact;
use crabsim::SimTime;
use std::cell::RefCell;
use std::collections::{BinaryHeap, HashMap, VecDeque};

/// No limit, in a count of MUs.
const NO_LIMIT: u64 = u64::MAX;

/// Allowed for rounding where values are worked out in floating point, such as a conveyor's MUs.
const ROUNDING: f64 = 1e-9;

/// What one instance lets through, holds and takes.
struct Object {
    name: String,
    built_in: &'static str,
    /// The most MUs it passes on at once, and the least time between two such batches in
    /// seconds: `None` for no limit, such as a Buffer's or a Source's with an interval of 0.
    pace: Option<(u64, f64)>,
    /// The most MUs it holds at once, `None` for no limit.
    holds: Option<u64>,
    /// The MUs it holds from Init.
    starting: u64,
    /// The least time an MU spends in it before it can be in the next object: a Station's
    /// shortest process time, a Buffer's dwelling time, a conveyor's crossing.
    least: f64,
    /// A Station's longest time working on one MU, its own or a work plan step's.
    work_most: Option<SimTime>,
    /// It has a failure profile, so it can fail in a run with randomness.
    can_fail: bool,
    /// A conveyor or cross conveyor leads into it, so it can be receiving.
    can_receive: bool,
    /// It can come back to itself along the connections, as in a rework loop, so the MUs it
    /// takes can be more than those that reach it from outside the loop.
    in_loop: bool,
    /// What limits it, in a few words, for saying why, such as "1 MU at a time, 1:00 each".
    why: String,
}

/// The stretch a figure is read over in a check: from `start` since Init, for `length`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Stretch {
    start: SimTime,
    length: SimTime,
}

/// Where a flow is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Sink {
    /// MUs entering this instance from its predecessors.
    Into(usize),
    /// MUs leaving this instance.
    OutOf(usize),
    /// MUs entering any Drain.
    Drains,
}

/// The most MUs a flow can carry, and the instances whose limits make it so.
#[derive(Debug, Clone)]
struct Flow {
    most: u64,
    limited_by: Vec<usize>,
}

/// The lowest or highest a figure can be, with why.
#[derive(Debug, Clone)]
struct Side {
    value: f64,
    why: String,
}

/// What a figure can be for one subject.
#[derive(Debug, Clone, Default)]
struct Range {
    low: Option<Side>,
    high: Option<Side>,
}

/// What an expectation reads, so two read the same value: an instance, or the model or a class's
/// mean as a whole, its figure and scope, in a check of a stage with its settings.
type Key = (Option<u32>, Option<String>, Figure, Scope, CheckStage, Settings);

/// The analysis of one model, worked out once for all its expectations.
struct Analysis<'a> {
    model: &'a Model,
    objects: Vec<Object>,
    successors: Vec<Vec<usize>>,
    predecessors: Vec<Vec<usize>>,
    /// The model has user code, which can do what nothing else can (ADR 0029).
    code: bool,
    flows: RefCell<HashMap<(Sink, Stretch), Flow>>,
    least_lead: RefCell<Option<Vec<f64>>>,
}

impl Model {
    /// Every expectation that can't be met or contradicts another, as a warning at its line,
    /// worked out from the model without running it. For a model without problems.
    pub fn expectation_bounds(&self) -> Vec<Problem> {
        if self.expectations.is_empty() {
            return Vec::new();
        }
        let analysis = Analysis::new(self);
        let bound = self.bound_expectations();
        let mut warnings = Vec::new();
        let mut warn = |index: usize, message: String| {
            warnings.push(Problem { pointer: format!("/expectations/{index}/text"), name: Some(self.expectations[index].id.clone()).filter(|id| !id.is_empty()), message });
        };
        for expectation in &bound {
            if let Some(message) = analysis.cannot_be_met(expectation) {
                warn(expectation.index, format!("it can't be met: {message}"));
            }
        }
        for (index, message) in analysis.contradictions(&bound) {
            warn(index, message);
        }
        warnings.sort_by_key(|warning| {
            let index: usize = warning.pointer.split('/').nth(2).and_then(|index| index.parse().ok()).unwrap_or(0);
            index
        });
        warnings
    }
}

impl<'a> Analysis<'a> {
    fn new(model: &'a Model) -> Analysis<'a> {
        let count = model.instances.len();
        let index: HashMap<&str, usize> = model.instances.iter().enumerate().map(|(i, instance)| (instance.name.as_str(), i)).collect();
        let mut successors = vec![Vec::new(); count];
        let mut predecessors = vec![Vec::new(); count];
        for connection in &model.connections {
            if let (Some(&from), Some(&to)) = (index.get(connection.from.as_str()), index.get(connection.to.as_str())) {
                successors[from].push(to);
                predecessors[to].push(from);
            }
        }
        // A work plan's step can give a Station its own time for an MU (ADR 0026).
        let mut step_times: Vec<Vec<SimTime>> = vec![Vec::new(); count];
        for source in 0..count {
            for step in model.work_plan_of(source).unwrap_or_default() {
                if let (Some(time), Some(members)) = (step.process_time, model.destination_members(&step.at)) {
                    for member in members {
                        step_times[member].push(time);
                    }
                }
            }
        }
        let spacing = mu_spacing(model);
        let objects = (0..count)
            .map(|i| {
                let resolved = model.resolve(i).expect("checked: every instance resolves");
                let has = &resolved.attributes;
                let built_in = resolved.built_in;
                let mut object = Object {
                    name: model.instances[i].name.clone(),
                    built_in,
                    pace: None,
                    holds: None,
                    starting: has.starting_mus.unwrap_or(0),
                    least: 0.0,
                    work_most: None,
                    can_fail: has.failures.as_ref().is_some_and(|list| !list.is_empty()),
                    can_receive: predecessors[i].iter().any(|&p| matches!(model.resolve(p).map(|r| r.built_in), Some("Conveyor" | "CrossConveyor"))),
                    in_loop: false,
                    why: String::new(),
                };
                match built_in {
                    "Source" => {
                        let interval = has.interval.unwrap_or(SimTime::ZERO);
                        if interval > SimTime::ZERO {
                            object.pace = Some((1, interval.as_secs_f64()));
                            object.why = format!("an MU every {}", short(interval));
                        }
                    }
                    "Station" => {
                        let own = has.process_time.unwrap_or(SimTime::ZERO);
                        let capacity = has.capacity.map_or(1, crabsim::Capacity::limit);
                        let shortest = step_times[i].iter().copied().chain([own]).min().expect("one at least");
                        let longest = step_times[i].iter().copied().chain([own]).max().expect("one at least");
                        object.holds = Some(capacity);
                        object.least = shortest.as_secs_f64();
                        object.work_most = Some(longest);
                        if shortest > SimTime::ZERO {
                            object.pace = Some((capacity, shortest.as_secs_f64()));
                            object.why = if capacity == 1 { format!("1 MU at a time, {} each", short(shortest)) } else { format!("{capacity} MUs at a time, {} each", short(shortest)) };
                        }
                    }
                    "Buffer" => {
                        object.holds = match has.capacity.unwrap_or(crabsim::Capacity::Unlimited) {
                            crabsim::Capacity::Limited(limit) => Some(limit),
                            crabsim::Capacity::Unlimited => None,
                        };
                        object.least = has.dwelling_time.unwrap_or(SimTime::ZERO).as_secs_f64();
                    }
                    "Drain" => object.holds = Some(0),
                    "Conveyor" => {
                        let length = crate::path_length(has.path.as_deref().unwrap_or(&crate::DEFAULT_PATH));
                        let speed = has.speed.unwrap_or(0.5);
                        let gap = has.gap.unwrap_or(0.0);
                        if speed > 0.0 {
                            object.least = length / speed;
                            // Booking points cross its end at least one MU's spacing apart.
                            let apart = gap + spacing;
                            if apart > 0.0 {
                                object.pace = Some((1, apart / speed));
                                object.holds = Some((length / apart + ROUNDING).floor() as u64 + 2);
                                object.why = format!("{speed} m/s with MUs at least {} m apart", crate::expect::plain(apart));
                            }
                        }
                    }
                    "CrossConveyor" => {
                        let leg = has.length.unwrap_or(0.6).min(has.width.unwrap_or(0.6));
                        let speed = has.speed.unwrap_or(0.5);
                        object.holds = Some(2);
                        if speed > 0.0 && leg > 0.0 {
                            // One MU at a time, its booking point across at least its shorter side.
                            object.least = leg / speed;
                            object.pace = Some((1, leg / speed));
                            object.why = format!("one MU at a time, at least {} across", short(SimTime::from_secs_f64(leg / speed)));
                        }
                    }
                    _ => {}
                }
                object
            })
            .collect::<Vec<Object>>();
        let mut analysis = Analysis { model, objects, successors, predecessors, code: model.has_code(), flows: RefCell::new(HashMap::new()), least_lead: RefCell::new(None) };
        for (i, in_loop) in in_loops(&analysis.successors).into_iter().enumerate() {
            analysis.objects[i].in_loop = in_loop;
        }
        analysis
    }

    /// The most MUs `object` can pass on in a stretch of `length`.
    fn own(&self, object: usize, length: SimTime) -> u64 {
        match self.objects[object].pace {
            None => NO_LIMIT,
            Some((batch, apart)) => batch.saturating_mul((length.as_secs_f64() / apart + ROUNDING).floor() as u64 + 1),
        }
    }

    /// The most MUs `object` can hold at `start`: at Init those it starts with.
    fn held_at(&self, object: usize, start: SimTime) -> u64 {
        let it = &self.objects[object];
        if start == SimTime::ZERO {
            return it.starting;
        }
        match (it.built_in, it.holds) {
            ("Source", _) => NO_LIMIT,
            (_, Some(holds)) => holds,
            // A Buffer without a limit holds what it started with and what came in since.
            _ if it.in_loop => NO_LIMIT,
            _ => it.starting.saturating_add(self.flow(Sink::Into(object), Stretch { start: SimTime::ZERO, length: start }).most),
        }
    }

    /// The most MUs that can reach `sink` in the stretch, as a maximum flow through the objects
    /// upstream of it, each passing on no more than its own limit, starting from Sources and
    /// what each object holds as the stretch begins.
    fn flow(&self, sink: Sink, stretch: Stretch) -> Flow {
        if let Some(flow) = self.flows.borrow().get(&(sink, stretch)) {
            return flow.clone();
        }
        let flow = self.work_out_flow(sink, stretch);
        self.flows.borrow_mut().insert((sink, stretch), flow.clone());
        flow
    }

    fn work_out_flow(&self, sink: Sink, stretch: Stretch) -> Flow {
        let ends: Vec<usize> = match sink {
            Sink::Into(x) | Sink::OutOf(x) => vec![x],
            Sink::Drains => (0..self.objects.len()).filter(|&i| self.objects[i].built_in == "Drain").collect(),
        };
        // Only what lies upstream of the sink can reach it, so a flow costs what lies there.
        let mut place: HashMap<usize, usize> = HashMap::new();
        let mut members: Vec<usize> = Vec::new();
        let mut queue: VecDeque<usize> = VecDeque::new();
        for &end in &ends {
            if place.insert(end, members.len()).is_none() {
                members.push(end);
                queue.push_back(end);
            }
        }
        while let Some(at) = queue.pop_front() {
            for &before in &self.predecessors[at] {
                if let std::collections::hash_map::Entry::Vacant(entry) = place.entry(before) {
                    entry.insert(members.len());
                    members.push(before);
                    queue.push_back(before);
                }
            }
        }
        let (source, target) = (2 * members.len(), 2 * members.len() + 1);
        let mut network = Network::new(2 * members.len() + 2);
        let is_end = |i: usize| ends.contains(&i);
        for (k, &i) in members.iter().enumerate() {
            let (entry, exit) = (2 * k, 2 * k + 1);
            let counted_on_entry = matches!(sink, Sink::Into(_) | Sink::Drains) && is_end(i);
            if counted_on_entry {
                network.add(entry, target, NO_LIMIT);
            } else {
                network.add(entry, exit, self.own(i, stretch.length));
                let supply = if self.objects[i].built_in == "Source" { NO_LIMIT } else { self.held_at(i, stretch.start) };
                if supply > 0 {
                    network.add(source, entry, supply);
                }
                if matches!(sink, Sink::OutOf(_)) && is_end(i) {
                    network.add(exit, target, NO_LIMIT);
                }
            }
            for &next in &self.successors[i] {
                if let Some(&to) = place.get(&next) {
                    network.add(exit, 2 * to, NO_LIMIT);
                }
            }
        }
        let most = network.max_flow(source, target);
        // The objects whose own limit the cut runs through say what limits the flow.
        let reached = network.reached_from(source);
        let mut limited_by: Vec<usize> = members.iter().enumerate().filter(|&(k, _)| reached[2 * k] && !reached[2 * k + 1]).map(|(_, &i)| i).collect();
        limited_by.sort_unstable();
        Flow { most, limited_by }
    }

    /// The least time an MU can take from where it starts, a Source or an object holding it at
    /// Init, to entering each instance, by instance: the work and dwelling on the shortest way.
    fn least_lead_times(&self) -> Vec<f64> {
        if let Some(times) = self.least_lead.borrow().as_ref() {
            return times.clone();
        }
        let count = self.objects.len();
        let mut reach = vec![f64::INFINITY; count];
        let mut queue: BinaryHeap<Reaching> = BinaryHeap::new();
        for i in 0..count {
            if self.objects[i].built_in == "Source" || self.objects[i].starting > 0 {
                // An MU starting in an object may be about to leave it.
                for &next in &self.successors[i] {
                    queue.push(Reaching(0.0, next));
                }
            }
        }
        // Dijkstra's, each instance reached at the least time an MU can enter it.
        while let Some(Reaching(time, object)) = queue.pop() {
            if time >= reach[object] {
                continue;
            }
            reach[object] = time;
            let leaving = time + self.objects[object].least;
            for &next in &self.successors[object] {
                if leaving < reach[next] {
                    queue.push(Reaching(leaving, next));
                }
            }
        }
        *self.least_lead.borrow_mut() = Some(reach.clone());
        reach
    }

    /// The earliest an MU can reach any of `drains` after the stretch begins, in seconds: its
    /// least lead time when the stretch begins at Init, and at once after a warm-up, when MUs may
    /// be on their way.
    fn earliest(&self, drains: &[usize], stretch: Stretch) -> f64 {
        if stretch.start > SimTime::ZERO {
            return 0.0;
        }
        let lead = self.least_lead_times();
        drains.iter().map(|&drain| lead[drain]).fold(f64::INFINITY, f64::min)
    }

    /// Names a few instances, such as "Station 1 (1 MU at a time, 1:00 each) and Station 2".
    fn naming(&self, objects: &[usize]) -> String {
        let named: Vec<String> = objects
            .iter()
            .take(3)
            .map(|&i| {
                let object = &self.objects[i];
                if object.why.is_empty() { object.name.clone() } else { format!("{} ({})", object.name, object.why) }
            })
            .collect();
        let more = objects.len().saturating_sub(3);
        match (named.as_slice(), more) {
            ([], _) => "what the objects start with".to_string(),
            ([one], _) => one.clone(),
            (_, 0) => format!("{} and {}", named[..named.len() - 1].join(", "), named[named.len() - 1]),
            (_, more) => format!("{} and {more} more", named.join(", ")),
        }
    }

    /// Why a flow is no larger: the objects whose limits make it so.
    fn limited(&self, flow: &Flow) -> String {
        if flow.limited_by.is_empty() {
            "what the objects hold as it begins".to_string()
        } else {
            format!("as {} let{} no more through", self.naming(&flow.limited_by), if flow.limited_by.len() == 1 { "s" } else { "" })
        }
    }

    // ---------- What a figure can be ----------

    /// The stretch a check reads a figure over.
    fn stretch(scope: Scope, settings: Settings) -> Option<Stretch> {
        match scope {
            Scope::SinceStart | Scope::Always => Some(Stretch { start: settings.warm_up, length: settings.length }),
            Scope::Last(length) if length <= settings.length => Some(Stretch { start: (settings.warm_up + settings.length).saturating_sub(length), length }),
            _ => None,
        }
    }

    /// What `figure` can be for the instance `object` in a check of `stage` with `settings`.
    fn range(&self, object: usize, figure: Figure, scope: Scope, stage: CheckStage, settings: Settings) -> Range {
        let it = &self.objects[object];
        let mut highs: Vec<Side> = Vec::new();
        let mut low: Option<Side> = None;
        let name = &it.name;
        let mut high = |value: f64, why: String| highs.push(Side { value, why });
        // Capacity, which code can't change either.
        if let Some(holds) = it.holds.filter(|_| it.built_in != "Drain")
            && matches!(figure, Figure::Mus | Figure::MaxMus | Figure::MinMus | Figure::MedianMus | Figure::MeanMus)
        {
            high(holds as f64, format!("{name} holds at most {holds} MU{}", if holds == 1 { "" } else { "s" }));
        }
        let done = |highs: Vec<Side>, low: Option<Side>| Range { low, high: highs.into_iter().min_by(|a, b| a.value.total_cmp(&b.value)) };
        if self.code {
            return done(highs, low);
        }
        // What never happens in a check.
        let never = |what: &str| format!("{name} {what}");
        let never_fails = if stage == CheckStage::Verification {
            Some(never("never fails at the verification stage, which runs without failures"))
        } else if !it.can_fail {
            Some(never("has no failure profile, so it never fails"))
        } else {
            None
        };
        match figure {
            Figure::FailedShare | Figure::Failures => {
                if let Some(why) = never_fails {
                    high(0.0, why);
                }
            }
            Figure::Availability => {
                if let Some(why) = never_fails {
                    low = Some(Side { value: 1.0, why });
                }
            }
            Figure::PausedShare => high(0.0, never("is never paused in a check: only a person or code pauses an object")),
            Figure::ReceivingShare if !it.can_receive => high(0.0, never("is never receiving: no conveyor leads into it")),
            _ => {}
        }
        let Some(stretch) = Self::stretch(scope, settings) else { return done(highs, low) };
        let measured = if matches!(scope, Scope::Last(_)) { format!("the last {}", short(stretch.length)) } else { format!("the {} measured", short(stretch.length)) };
        // How many MUs can pass.
        let entering = || -> (u64, String) {
            let from_outside = if it.in_loop {
                Flow { most: NO_LIMIT, limited_by: Vec::new() }
            } else {
                self.flow(Sink::Into(object), stretch)
            };
            // In a loop, what it takes is what it passes on and what it holds by the end.
            let own = self.own(object, stretch.length).saturating_add(it.holds.unwrap_or(NO_LIMIT));
            let starting = if stretch.start == SimTime::ZERO { it.starting } else { 0 };
            if own < from_outside.most {
                (own.saturating_add(starting), format!("{name} ({}) can take no more", it.why))
            } else {
                (from_outside.most.saturating_add(starting), self.limited(&from_outside))
            }
        };
        match figure {
            Figure::MusIn => {
                let (most, why) = if it.built_in == "Source" {
                    (self.own(object, stretch.length), format!("{name} makes {}", it.why))
                } else {
                    entering()
                };
                if most != NO_LIMIT {
                    high(most as f64, format!("at most {most} MUs can enter {name} in {measured}, {why}"));
                }
            }
            Figure::MusOut => {
                let flow = if it.in_loop { Flow { most: self.own(object, stretch.length), limited_by: vec![object] } } else { self.flow(Sink::OutOf(object), stretch) };
                if flow.most != NO_LIMIT {
                    high(flow.most as f64, format!("at most {} MUs can leave {name} in {measured}, {}", flow.most, self.limited(&flow)));
                }
            }
            Figure::ThroughputPerHour if it.built_in == "Drain" => {
                let flow = self.flow(Sink::Into(object), stretch);
                let first = self.earliest(&[object], stretch);
                if flow.most != NO_LIMIT
                    && let Some(rate) = throughput(flow.most, scope, stretch, first)
                {
                    high(rate, format!("at most {} MUs can enter {name} in {measured}, {}, {}", flow.most, crate::expect::written(Unit::PerHour, rate), self.limited(&flow)));
                }
            }
            Figure::WorkingShare if it.built_in == "Station" => {
                let work_most = it.work_most.unwrap_or(SimTime::ZERO);
                let (taken, why) = entering();
                // Those it holds as the stretch begins are worked on in it too.
                let worked = taken.saturating_add(if stretch.start == SimTime::ZERO { 0 } else { it.holds.unwrap_or(0) });
                let busy = (worked as f64 * work_most.as_secs_f64()) / stretch.length.as_secs_f64();
                if work_most == SimTime::ZERO {
                    high(0.0, format!("{name} works each MU for 0 s, so it is never busy"));
                } else if worked != NO_LIMIT && busy < 1.0 {
                    high(busy, format!("{name} can be busy at most {} of {measured}: at most {worked} MUs to work on, {why}, each for at most {}", crate::expect::written(Unit::Share, busy), short(work_most)));
                }
            }
            Figure::MeanLeadTime | Figure::LeadTime if it.built_in == "Drain" => {
                let least = self.least_lead_times()[object];
                if least.is_finite() && least > 0.0 {
                    low = Some(Side { value: least, why: format!("every MU takes at least {} to reach {name}, working and dwelling on the shortest way", short(SimTime::from_secs_f64(least))) });
                }
            }
            Figure::MeanCrossingTime | Figure::MaxCrossingTime if it.built_in == "Conveyor" && it.least > 0.0 => {
                low = Some(Side { value: it.least, why: format!("crossing {name} takes at least {} at its speed", short(SimTime::from_secs_f64(it.least))) });
            }
            _ => {}
        }
        done(highs, low)
    }

    /// What a model's figure can be: its Drains' together.
    fn model_range(&self, figure: Figure, scope: Scope, settings: Settings) -> Range {
        let mut range = Range::default();
        if self.code {
            return range;
        }
        let Some(stretch) = Self::stretch(scope, settings) else { return range };
        let drains: Vec<usize> = (0..self.objects.len()).filter(|&i| self.objects[i].built_in == "Drain").collect();
        match figure {
            Figure::ThroughputPerHour => {
                let flow = self.flow(Sink::Drains, stretch);
                let first = self.earliest(&drains, stretch);
                if flow.most != NO_LIMIT
                    && let Some(rate) = throughput(flow.most, scope, stretch, first)
                {
                    range.high = Some(Side { value: rate, why: format!("at most {} MUs can reach the Drains in {}, {}", flow.most, if matches!(scope, Scope::Last(_)) { format!("the last {}", short(stretch.length)) } else { format!("the {} measured", short(stretch.length)) }, self.limited(&flow)) });
                }
            }
            Figure::MeanLeadTime | Figure::LeadTime => {
                let lead = self.least_lead_times();
                let least = drains.iter().map(|&d| lead[d]).fold(f64::INFINITY, f64::min);
                if least.is_finite() && least > 0.0 {
                    range.low = Some(Side { value: least, why: format!("every MU takes at least {} to reach a Drain, working and dwelling on the shortest way", short(SimTime::from_secs_f64(least))) });
                }
            }
            _ => {}
        }
        range
    }

    // ---------- Expectations ----------

    /// Why `expectation` can't be met, if it can't.
    fn cannot_be_met(&self, expectation: &Bound) -> Option<String> {
        let check = &expectation.check;
        let index = expectation.index;
        let stage = self.model.expectations[index].stage;
        let settings = self.model.settings_of(index);
        let accepted = check.accepted()?;
        // A figure's own range first: a share is from 0% to 100%, a count never below 0.
        let unit = check.figure.unit();
        let mut nature = Range { low: Some(Side { value: 0.0, why: format!("`{}` is never below {}", check.figure.name(), crate::expect::written(unit, 0.0)) }), high: None };
        if unit == Unit::Share {
            nature.high = Some(Side { value: 1.0, why: "a share is at most 100%".into() });
        }
        if let Some(why) = outside(accepted, &nature) {
            return Some(why);
        }
        let ranges: Vec<(Option<u32>, Range)> = match (&check.subject, check.aggregate) {
            (CheckSubject::Model, _) => vec![(None, self.model_range(check.figure, check.scope, settings))],
            (_, Aggregate::Mean) => {
                // A mean is at most the mean of its instances' most, and at least that of their
                // least, when every one has a bound.
                let each: Vec<Range> = expectation.subjects.iter().map(|&id| self.range(id as usize, check.figure, check.scope, stage, settings)).collect();
                let mean = |side: fn(&Range) -> &Option<Side>, what: &str| -> Option<Side> {
                    let values: Option<Vec<f64>> = each.iter().map(|range| side(range).as_ref().map(|side| side.value)).collect();
                    let values = values.filter(|values| !values.is_empty())?;
                    let mean = values.iter().sum::<f64>() / values.len() as f64;
                    let first = side(&each[0]).as_ref().expect("every one has it").why.clone();
                    Some(Side { value: mean, why: format!("the {what} of the {} instances' is {}, such as {first}", values.len(), crate::expect::written(unit, mean)) })
                };
                vec![(None, Range { low: mean(|range| &range.low, "mean least"), high: mean(|range| &range.high, "mean most") })]
            }
            _ => expectation.subjects.iter().map(|&id| (Some(id), self.range(id as usize, check.figure, check.scope, stage, settings))).collect(),
        };
        let cannot: Vec<String> = ranges.iter().filter_map(|(_, range)| outside(accepted, range)).collect();
        match (check.aggregate, cannot.as_slice()) {
            (_, []) => None,
            (Aggregate::Every, [one]) if ranges.len() > 1 => Some(format!("1 of its {} instances can't: {one}", ranges.len())),
            (Aggregate::Every, [first, ..]) if ranges.len() > 1 => Some(format!("{} of its {} instances can't, such as: {first}", cannot.len(), ranges.len())),
            (_, [first, ..]) => Some(first.clone()),
        }
    }

    /// Expectations asking what no value can give together, each with the warning on the later
    /// one, by its place in the model.
    fn contradictions(&self, bound: &[Bound]) -> Vec<(usize, String)> {
        let mut found: Vec<(usize, String)> = Vec::new();
        let keys = |expectation: &Bound| -> Vec<Key> {
            let check = &expectation.check;
            let stage = self.model.expectations[expectation.index].stage;
            let settings = self.model.settings_of(expectation.index);
            // The most MUs held at any moment is the most an object held since start.
            let (figure, scope) = if (check.figure, check.scope) == (Figure::Mus, Scope::Always) { (Figure::MaxMus, Scope::SinceStart) } else { (check.figure, check.scope) };
            match (&check.subject, check.aggregate) {
                (CheckSubject::Model, _) => vec![(None, None, figure, scope, stage, settings)],
                (CheckSubject::Named(class), Aggregate::Mean) => vec![(None, Some(class.clone()), figure, scope, stage, settings)],
                _ => expectation.subjects.iter().map(|&id| (Some(id), None, figure, scope, stage, settings)).collect(),
            }
        };
        let subject_name = |key: &Key| match key {
            (Some(id), ..) => self.model.instances[*id as usize].name.clone(),
            (None, Some(class), ..) => format!("the mean over {class}"),
            (None, None, ..) => "the model".to_string(),
        };
        let mut seen: HashMap<Key, Vec<usize>> = HashMap::new();
        for (place, expectation) in bound.iter().enumerate() {
            let mine = keys(expectation);
            let mut against: Option<(usize, String)> = None;
            for key in &mine {
                for &at in seen.get(key).into_iter().flatten() {
                    if against.is_none() && !compatible(&expectation.check, &bound[at].check) {
                        against = Some((at, subject_name(key)));
                    }
                }
            }
            if let Some((at, subject)) = against {
                let other = &bound[at];
                let id = &self.model.expectations[other.index].id;
                let check = &expectation.check;
                found.push((
                    expectation.index,
                    format!(
                        "it contradicts {id}: {subject}'s {} {} can't be both {} and {}",
                        check.figure.name(),
                        check.scope_text(),
                        asks(&other.check),
                        asks(check)
                    ),
                ));
            }
            for key in mine {
                let places = seen.entry(key).or_default();
                if !places.contains(&place) {
                    places.push(place);
                }
            }
        }
        found.extend(self.shares_that_cant_add_up(bound, &keys));
        found
    }

    /// An object's shares of time add up to 100%: working, blocked, idle, receiving, failed and
    /// paused. Expectations asking for more than that together, or less, can't all be met.
    fn shares_that_cant_add_up(&self, bound: &[Bound], keys: &dyn Fn(&Bound) -> Vec<Key>) -> Vec<(usize, String)> {
        const PARTS: [Figure; 6] = [Figure::WorkingShare, Figure::BlockedShare, Figure::IdleShare, Figure::ReceivingShare, Figure::FailedShare, Figure::PausedShare];
        // By instance and check: each part's least and most, and which expectations ask it.
        type Group = (u32, Scope, CheckStage, Settings);
        let mut groups: HashMap<Group, ([f64; 6], [f64; 6], Vec<usize>)> = HashMap::new();
        for (place, expectation) in bound.iter().enumerate() {
            let check = &expectation.check;
            let Some(part) = PARTS.iter().position(|&part| part == check.figure) else { continue };
            if check.aggregate == Aggregate::Mean || check.tolerance.is_none() && check.comparison == Comparison::NotEqual {
                continue;
            }
            let Some((low, high)) = check.accepted() else { continue };
            for key in keys(expectation) {
                let (Some(id), _, _, scope, stage, settings) = key else { continue };
                let entry = groups.entry((id, scope, stage, settings)).or_insert_with(|| {
                    // A part that can't happen is 0 at most: those are known without asking.
                    let mut most = [1.0; 6];
                    for (k, &figure) in PARTS.iter().enumerate() {
                        if let Some(side) = self.range(id as usize, figure, scope, stage, settings).high {
                            most[k] = side.value.min(1.0);
                        }
                    }
                    ([0.0; 6], most, Vec::new())
                });
                if let Some((value, _)) = low {
                    entry.0[part] = entry.0[part].max(value);
                }
                if let Some((value, _)) = high {
                    entry.1[part] = entry.1[part].min(value);
                }
                if !entry.2.contains(&place) {
                    entry.2.push(place);
                }
            }
        }
        let mut found = Vec::new();
        let mut groups: Vec<(Group, ([f64; 6], [f64; 6], Vec<usize>))> = groups.into_iter().collect();
        groups.sort_by_key(|(group, (_, _, places))| (places.iter().max().copied(), group.0));
        for ((id, ..), (least, most, places)) in groups {
            let (low, high): (f64, f64) = (least.iter().sum(), most.iter().sum());
            if places.len() < 2 && low <= 1.0 + ROUNDING && high >= 1.0 - ROUNDING {
                continue;
            }
            let last = *places.iter().max().expect("one at least");
            let others: Vec<&str> = places.iter().filter(|&&place| place != last).map(|&place| self.model.expectations[bound[place].index].id.as_str()).collect();
            let name = &self.objects[id as usize].name;
            let with = if others.is_empty() { String::new() } else { format!(" with {}", others.join(", ")) };
            if low > 1.0 + ROUNDING {
                found.push((bound[last].index, format!("it contradicts itself{with}: {name}'s shares of time add up to 100%, and these ask for at least {} of them", crate::expect::written(Unit::Share, low))));
            } else if high < 1.0 - ROUNDING {
                found.push((bound[last].index, format!("it contradicts itself{with}: {name}'s shares of time add up to 100%, and these leave at most {} for them", crate::expect::written(Unit::Share, high))));
            }
        }
        found
    }
}

/// Which instances can come back to themselves along the connections: those in a strongly
/// connected component of more than one, or connected to themselves, by Tarjan's algorithm
/// without recursion, so a long line can't overflow the stack.
fn in_loops(successors: &[Vec<usize>]) -> Vec<bool> {
    let count = successors.len();
    let mut index = vec![usize::MAX; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack: Vec<usize> = Vec::new();
    let mut in_loop = vec![false; count];
    let mut next = 0;
    for root in 0..count {
        if index[root] != usize::MAX {
            continue;
        }
        // Each frame is an instance and how far through its successors it is.
        let mut frames: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (at, ref mut through)) = frames.last_mut() {
            if let Some(&to) = successors[at].get(*through) {
                *through += 1;
                if index[to] == usize::MAX {
                    index[to] = next;
                    low[to] = next;
                    next += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    frames.push((to, 0));
                } else if on_stack[to] {
                    low[at] = low[at].min(index[to]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent] = low[parent].min(low[at]);
            }
            if low[at] == index[at] {
                let mut members = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    members.push(member);
                    if member == at {
                        break;
                    }
                }
                let looping = members.len() > 1 || successors[at].contains(&at);
                for member in members {
                    in_loop[member] = looping;
                }
            }
        }
    }
    in_loop
}

/// An instance reached at a time, the earliest first out of a heap.
struct Reaching(f64, usize);

impl PartialEq for Reaching {
    fn eq(&self, other: &Reaching) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Reaching {}

impl PartialOrd for Reaching {
    fn partial_cmp(&self, other: &Reaching) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Reaching {
    fn cmp(&self, other: &Reaching) -> std::cmp::Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

/// A length of time as a line writes one, the shortest way: `1d`, `8h`, `30min`, `12s` or
/// `0.5s`, or as the UI writes time where none is whole.
fn short(time: SimTime) -> String {
    let millis = time.as_millis();
    for (unit, length) in [("d", 86_400_000), ("h", 3_600_000), ("min", 60_000), ("s", 1_000)] {
        if millis > 0 && millis % length == 0 {
            return format!("{}{unit}", millis / length);
        }
    }
    if millis < 60_000 {
        return format!("{}s", crate::expect::plain(time.as_secs_f64()));
    }
    format_time_exact(time)
}

/// A count over a stretch as a throughput. Over the last stretch, the MUs in it an hour. Since
/// start, the figure counts the MUs after the Drain's first, over the time from it, `first` at
/// the earliest after the stretch begins: bounded as if it came then, which is the one bound that
/// rests on how a run goes, since MUs coming together just before the end could make the figure
/// anything. `None` if no MU can come before the end.
fn throughput(most: u64, scope: Scope, stretch: Stretch, first: f64) -> Option<f64> {
    let hours = stretch.length.as_secs_f64() / 3600.0;
    match scope {
        Scope::Last(_) => Some(most as f64 / hours),
        _ => {
            let after_first = hours - first / 3600.0;
            (after_first > 0.0).then(|| most.saturating_sub(1) as f64 / after_first)
        }
    }
}

/// The least space along a conveyor between two MUs' booking points, without the gap: the part
/// of any MU behind its booking point and the part of any MU before it, lying lengthwise or, after
/// a cross conveyor turned it, crosswise, booked at its middle.
fn mu_spacing(model: &Model) -> f64 {
    let mut kinds = vec![model.resolve_class(crate::BUILT_IN_MU_CLASS).map(|class| crate::mu_kind(&class.attributes)).unwrap_or_default()];
    kinds.extend((0..model.instances.len()).filter_map(|i| model.mu_kind_of(i)));
    let ways: Vec<(f64, f64)> = kinds.iter().flat_map(|kind| [(kind.length, kind.booking_point), (kind.width, 0.5)]).collect();
    let behind = ways.iter().map(|&(length, booked)| (1.0 - booked) * length).fold(f64::INFINITY, f64::min);
    let before = ways.iter().map(|&(length, booked)| booked * length).fold(f64::INFINITY, f64::min);
    (behind + before).max(0.0)
}

/// Why no value `accepted` takes lies in `range`, if none does.
fn outside(accepted: (Option<(f64, bool)>, Option<(f64, bool)>), range: &Range) -> Option<String> {
    let (least, most) = accepted;
    if let (Some((wanted, inclusive)), Some(high)) = (least, &range.high)
        && (wanted > high.value + ROUNDING || !inclusive && wanted >= high.value - ROUNDING)
    {
        return Some(high.why.clone());
    }
    if let (Some((wanted, inclusive)), Some(low)) = (most, &range.low)
        && (wanted < low.value - ROUNDING || !inclusive && wanted <= low.value + ROUNDING)
    {
        return Some(low.why.clone());
    }
    None
}

/// Whether some value meets both lines, which read the same figure the same way.
fn compatible(a: &crate::expect::Check, b: &crate::expect::Check) -> bool {
    match (&a.target, &b.target) {
        (Target::State(one), Target::State(other)) => {
            let equal = |check: &crate::expect::Check| check.comparison != Comparison::NotEqual;
            match (equal(a), equal(b)) {
                (true, true) => one == other,
                (true, false) | (false, true) => one != other,
                (false, false) => true,
            }
        }
        (Target::Number { value: one, .. }, Target::Number { value: other, .. }) => {
            let point = |check: &crate::expect::Check| check.tolerance.is_none() && check.comparison == Comparison::NotEqual;
            match (a.accepted(), b.accepted()) {
                (Some(first), Some(second)) => {
                    let low = higher_low(first.0, second.0);
                    let high = lower_high(first.1, second.1);
                    match (low, high) {
                        (Some((low, low_in)), Some((high, high_in))) => low < high - ROUNDING || (low - high).abs() <= ROUNDING && low_in && high_in,
                        _ => true,
                    }
                }
                // `!=` against an exact value.
                (None, Some(other_accepted)) if point(a) => other_accepted.0.zip(other_accepted.1).is_none_or(|(low, high)| !(low.0 == high.0 && (low.0 - one).abs() <= ROUNDING)),
                (Some(own_accepted), None) if point(b) => own_accepted.0.zip(own_accepted.1).is_none_or(|(low, high)| !(low.0 == high.0 && (low.0 - other).abs() <= ROUNDING)),
                _ => true,
            }
        }
        _ => true,
    }
}

fn higher_low(a: Option<(f64, bool)>, b: Option<(f64, bool)>) -> Option<(f64, bool)> {
    match (a, b) {
        (Some(a), Some(b)) if (a.0 - b.0).abs() <= ROUNDING => Some((a.0, a.1 && b.1)),
        (Some(a), Some(b)) => Some(if a.0 > b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn lower_high(a: Option<(f64, bool)>, b: Option<(f64, bool)>) -> Option<(f64, bool)> {
    match (a, b) {
        (Some(a), Some(b)) if (a.0 - b.0).abs() <= ROUNDING => Some((a.0, a.1 && b.1)),
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// What a line asks of its figure, such as `>= 60%` or `= 4100/d ± 3%`.
fn asks(check: &crate::expect::Check) -> String {
    let tolerance = check.tolerance.as_ref().map_or_else(String::new, |tolerance| format!(" ± {}", tolerance.written()));
    format!("{} {}{tolerance}", check.comparison.written(), check.value_text())
}

// ---------- Maximum flow ----------

/// A network of whole numbers of MUs, for a maximum flow by Dinic's algorithm. `NO_LIMIT` is no
/// limit: a flow along limitless edges alone is limitless.
struct Network {
    edges: Vec<(usize, u64)>,
    out: Vec<Vec<usize>>,
}

impl Network {
    fn new(nodes: usize) -> Network {
        Network { edges: Vec::new(), out: vec![Vec::new(); nodes] }
    }

    /// An edge and, beside it, its reverse with nothing left on it.
    fn add(&mut self, from: usize, to: usize, capacity: u64) {
        self.out[from].push(self.edges.len());
        self.edges.push((to, capacity));
        self.out[to].push(self.edges.len());
        self.edges.push((from, 0));
    }

    fn max_flow(&mut self, source: usize, target: usize) -> u64 {
        // Along limitless edges alone, nothing limits it.
        let limitless = {
            let mut seen = vec![false; self.out.len()];
            let mut queue = VecDeque::from([source]);
            seen[source] = true;
            while let Some(at) = queue.pop_front() {
                for &edge in &self.out[at] {
                    let (to, capacity) = self.edges[edge];
                    if capacity == NO_LIMIT && !std::mem::replace(&mut seen[to], true) {
                        queue.push_back(to);
                    }
                }
            }
            seen[target]
        };
        if limitless {
            return NO_LIMIT;
        }
        // Otherwise the least cut is finite, under the sum of every finite capacity, so a
        // limitless edge can stand for that sum and change nothing.
        let finite: u128 = self.edges.iter().map(|&(_, capacity)| capacity).filter(|&capacity| capacity != NO_LIMIT).map(u128::from).sum();
        let stand_in = u64::try_from(finite + 1).unwrap_or(NO_LIMIT - 1);
        for edge in &mut self.edges {
            if edge.1 == NO_LIMIT {
                edge.1 = stand_in;
            }
        }
        let mut total: u64 = 0;
        while let Some(level) = self.levels(source, target) {
            total = total.saturating_add(self.blocking_flow(source, target, &level));
        }
        total
    }

    /// Pushes flow along shortest paths of the level graph until none is left, following each
    /// node's next edge, without recursion, so a long line can't overflow the stack.
    fn blocking_flow(&mut self, source: usize, target: usize, level: &[u32]) -> u64 {
        let mut next = vec![0; self.out.len()];
        let mut path: Vec<usize> = Vec::new();
        let mut at = source;
        let mut total: u64 = 0;
        loop {
            if at == target {
                let pushed = path.iter().map(|&edge| self.edges[edge].1).min().unwrap_or(0);
                for &edge in &path {
                    self.edges[edge].1 -= pushed;
                    self.edges[edge ^ 1].1 += pushed;
                }
                total = total.saturating_add(pushed);
                path.clear();
                at = source;
                continue;
            }
            let mut advanced = false;
            while next[at] < self.out[at].len() {
                let edge = self.out[at][next[at]];
                let (to, capacity) = self.edges[edge];
                if capacity > 0 && level[to] == level[at] + 1 {
                    path.push(edge);
                    at = to;
                    advanced = true;
                    break;
                }
                next[at] += 1;
            }
            if advanced {
                continue;
            }
            // A dead end: back to where the path came from, past the edge that led here.
            let Some(edge) = path.pop() else { return total };
            at = self.edges[edge ^ 1].0;
            next[at] += 1;
        }
    }

    fn levels(&self, source: usize, target: usize) -> Option<Vec<u32>> {
        let mut level = vec![u32::MAX; self.out.len()];
        level[source] = 0;
        let mut queue = VecDeque::from([source]);
        while let Some(at) = queue.pop_front() {
            for &edge in &self.out[at] {
                let (to, capacity) = self.edges[edge];
                if capacity > 0 && level[to] == u32::MAX {
                    level[to] = level[at] + 1;
                    queue.push_back(to);
                }
            }
        }
        (level[target] != u32::MAX).then_some(level)
    }

    /// The nodes the source still reaches through edges with capacity left: after a maximum flow,
    /// one side of a least cut.
    fn reached_from(&self, source: usize) -> Vec<bool> {
        let mut seen = vec![false; self.out.len()];
        seen[source] = true;
        let mut queue = VecDeque::from([source]);
        while let Some(at) = queue.pop_front() {
            for &edge in &self.out[at] {
                let (to, capacity) = self.edges[edge];
                if capacity > 0 && !std::mem::replace(&mut seen[to], true) {
                    queue.push_back(to);
                }
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_maximum_flow_is_its_least_cut() {
        // Two ways of 3 and 2 joined by one of 1 across.
        let mut network = Network::new(4);
        network.add(0, 1, 3);
        network.add(0, 2, 2);
        network.add(1, 2, 1);
        network.add(1, 3, 2);
        network.add(2, 3, 3);
        assert_eq!(network.max_flow(0, 3), 5);
        let mut limitless = Network::new(3);
        limitless.add(0, 1, NO_LIMIT);
        limitless.add(1, 2, NO_LIMIT);
        assert_eq!(limitless.max_flow(0, 2), NO_LIMIT);
        let mut one_limit = Network::new(3);
        one_limit.add(0, 1, NO_LIMIT);
        one_limit.add(1, 2, 7);
        assert_eq!(one_limit.max_flow(0, 2), 7);
        assert_eq!(one_limit.reached_from(0), vec![true, true, false]);
    }

    #[test]
    fn instances_in_a_loop_are_those_that_come_back_to_themselves() {
        // 0 -> 1 -> 2 -> 1, 2 -> 3, 4 -> 4.
        let successors = vec![vec![1], vec![2], vec![1, 3], vec![], vec![4]];
        assert_eq!(in_loops(&successors), [false, true, true, false, true]);
    }

    #[test]
    fn states_and_values_that_cant_both_hold() {
        let line = |text: &str| crate::expect::Check::parse(text).unwrap();
        assert!(!compatible(&line("A.working_share since start >= 60%"), &line("A.working_share since start <= 50%")));
        assert!(compatible(&line("A.working_share since start >= 50%"), &line("A.working_share since start <= 50%")));
        assert!(!compatible(&line("A.working_share since start > 50%"), &line("A.working_share since start <= 50%")));
        assert!(!compatible(&line("D.mus_in since start = 100 ± 5%"), &line("D.mus_in since start > 105")));
        assert!(compatible(&line("D.mus_in since start = 100 ± 5%"), &line("D.mus_in since start >= 105")));
        assert!(!compatible(&line("D.mus_in since start = 100"), &line("D.mus_in since start != 100")));
        assert!(compatible(&line("D.mus_in since start != 100"), &line("D.mus_in since start != 101")));
        assert!(!compatible(&line("A.state now = busy"), &line("A.state now = idle")));
        assert!(!compatible(&line("A.state now = busy"), &line("A.state now != busy")));
        assert!(compatible(&line("A.state now != busy"), &line("A.state now != idle")));
    }
}
