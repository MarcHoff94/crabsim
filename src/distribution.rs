//! Which successor receives the MU an object passes on: its distribution strategy (ADR 0016).
//!
//! Round robin offers each MU to the successors in turn. Weighted ranks the successors by a
//! weight, best first, and offers the MU to the best one that can take it, those that rank equal
//! taking turns. The rankings are worked out from the objects' values when a run is built, and
//! again whenever a value they weigh changes, so moving an MU only reads them.

use crate::materialflow_objects::ModelObject;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};

/// The steps a search for the heaviest route through loops may take from one successor before
/// it settles for the heaviest found, so building a run can't take for ever.
const LOOP_SEARCH_STEPS: u64 = 2_000_000;

/// What a weighted object weighs the objects by. Times count in milliseconds, the others as
/// whole numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeighBy {
    /// A Station's process time.
    ProcessTime,
    /// A Source's interval.
    Interval,
    /// How many MUs a Station holds from Init.
    StartingMus,
    /// How many MUs a Buffer holds.
    Capacity,
}

/// Which totals rank first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Order {
    #[default]
    Smallest,
    Largest,
}

/// What a successor is ranked by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Rank {
    /// Its best route onward to a counted Drain, itself included: the lightest when ranking
    /// smallest first, the heaviest when largest first.
    #[default]
    Route,
    /// Its own weight only.
    Successor,
}

/// How a weighted object ranks its successors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeightedRule {
    pub by: WeighBy,
    pub order: Order,
    pub rank: Rank,
    /// The Drains a route may end in, by id. `None` counts every Drain. Ids that aren't Drains
    /// are ignored, so a list with none left ranks every successor the same.
    pub drains: Option<Vec<u32>>,
}

/// An object's distribution strategy, as a model gives it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum DistributionRule {
    #[default]
    RoundRobin,
    Weighted(WeightedRule),
}

/// How a weighted object ranks its successors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranking {
    /// The successors' places among the object's exits, counted from 0, in groups that rank
    /// equal, the best group first. Within a group, in exit order.
    pub groups: Vec<Vec<usize>>,
    /// Each successor's total, by its place among the exits: `None` for a route rank when it has
    /// no route to a counted Drain, which ranks last.
    pub totals: Vec<Option<u64>>,
}

/// What `object` weighs by `by`, or `None` if it has no such value: a Drain has none of them.
pub fn weight(object: &ModelObject, by: WeighBy) -> Option<u64> {
    match (object, by) {
        (ModelObject::Station(station), WeighBy::ProcessTime) => Some(station.proc_time.as_millis()),
        (ModelObject::Station(station), WeighBy::StartingMus) => Some(u64::from(station.starting_content.is_some())),
        (ModelObject::Source(source), WeighBy::Interval) => Some(source.proc_time.as_millis()),
        (ModelObject::Buffer(buffer), WeighBy::Capacity) => Some(buffer.capacity as u64),
        _ => None,
    }
}

/// The ranking of every object whose rule is weighted, by id, and `None` for the others.
pub fn rankings(objects: &[ModelObject], rules: &[DistributionRule]) -> Vec<Option<Ranking>> {
    let mut routes = Routes::new(objects);
    rules
        .iter()
        .enumerate()
        .map(|(id, rule)| match rule {
            DistributionRule::RoundRobin => None,
            DistributionRule::Weighted(rule) => Some(routes.rank(id, rule)),
        })
        .collect()
}

/// How `id` ranks its successors under `rule`, as a run would.
pub fn ranking(objects: &[ModelObject], id: u32, rule: &WeightedRule) -> Ranking {
    Routes::new(objects).rank(id as usize, rule)
}

/// What a route total depends on: what is weighed, which totals count as best, and the Drains
/// counted.
type RouteKey = (WeighBy, Order, Option<Vec<u32>>);

/// The route totals worked out so far, shared by the objects that weigh alike.
struct Routes<'o> {
    objects: &'o [ModelObject],
    predecessors: Vec<Vec<usize>>,
    /// Each object's best route total to a counted Drain, by the rule's `by`, `order` and
    /// `drains`, worked out once for every object that asks.
    totals: HashMap<RouteKey, Vec<Option<u64>>>,
}

impl<'o> Routes<'o> {
    fn new(objects: &'o [ModelObject]) -> Routes<'o> {
        let mut predecessors = vec![Vec::new(); objects.len()];
        for (from, object) in objects.iter().enumerate() {
            for &to in object.successor_ids() {
                predecessors[to as usize].push(from);
            }
        }
        Routes { objects, predecessors, totals: HashMap::new() }
    }

    fn rank(&mut self, id: usize, rule: &WeightedRule) -> Ranking {
        let successors: Vec<usize> = self.objects[id].successor_ids().iter().map(|&to| to as usize).collect();
        let totals: Vec<Option<u64>> = match rule.rank {
            Rank::Successor => successors.iter().map(|&to| Some(weight(&self.objects[to], rule.by).unwrap_or(0))).collect(),
            Rank::Route => {
                let key = (rule.by, rule.order, rule.drains.clone());
                if !self.totals.contains_key(&key) {
                    let totals = self.route_totals(rule);
                    self.totals.insert(key.clone(), totals);
                }
                let totals = &self.totals[&key];
                successors.iter().map(|&to| totals[to]).collect()
            }
        };
        // Best first, those without a route last, ties in exit order.
        let mut places: Vec<usize> = (0..successors.len()).collect();
        let sort_key = |place: &usize| match (totals[*place], rule.order) {
            (None, _) => (1, 0u64),
            (Some(total), Order::Smallest) => (0, total),
            (Some(total), Order::Largest) => (0, u64::MAX - total),
        };
        places.sort_by_key(sort_key);
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for place in places {
            match groups.last_mut() {
                Some(group) if totals[group[0]] == totals[place] => group.push(place),
                _ => groups.push(vec![place]),
            }
        }
        Ranking { groups, totals }
    }

    /// Every object's best route total to a Drain `rule` counts, itself and the Drain included.
    fn route_totals(&self, rule: &WeightedRule) -> Vec<Option<u64>> {
        let count = self.objects.len();
        let weights: Vec<u64> = self.objects.iter().map(|object| weight(object, rule.by).unwrap_or(0)).collect();
        let is_drain = |id: usize| matches!(self.objects[id], ModelObject::Drain(_));
        let mut counted = vec![false; count];
        match &rule.drains {
            None => (0..count).filter(|&id| is_drain(id)).for_each(|id| counted[id] = true),
            Some(drains) => drains.iter().map(|&id| id as usize).filter(|&id| id < count && is_drain(id)).for_each(|id| counted[id] = true),
        }
        match rule.order {
            Order::Smallest => self.lightest(&weights, &counted),
            Order::Largest => self.heaviest(&weights, &counted),
        }
    }

    /// Dijkstra's algorithm backwards from the counted Drains.
    fn lightest(&self, weights: &[u64], counted: &[bool]) -> Vec<Option<u64>> {
        let mut best: Vec<Option<u64>> = vec![None; weights.len()];
        let mut heap = BinaryHeap::new();
        for id in (0..weights.len()).filter(|&id| counted[id]) {
            best[id] = Some(weights[id]);
            heap.push(Reverse((weights[id], id)));
        }
        while let Some(Reverse((total, at))) = heap.pop() {
            if best[at] != Some(total) {
                continue;
            }
            for &before in &self.predecessors[at] {
                let reached = total.saturating_add(weights[before]);
                if best[before].is_none_or(|known| reached < known) {
                    best[before] = Some(reached);
                    heap.push(Reverse((reached, before)));
                }
            }
        }
        best
    }

    /// The heaviest route that passes no object twice: in reverse topological order when no loop
    /// lies on the way to the counted Drains, and otherwise by walking the routes from each object,
    /// up to a step limit.
    fn heaviest(&self, weights: &[u64], counted: &[bool]) -> Vec<Option<u64>> {
        let count = weights.len();
        // Those that can reach a counted Drain.
        let mut reaches = counted.to_vec();
        let mut queue: VecDeque<usize> = (0..count).filter(|&id| counted[id]).collect();
        while let Some(at) = queue.pop_front() {
            for &before in &self.predecessors[at] {
                if !reaches[before] {
                    reaches[before] = true;
                    queue.push_back(before);
                }
            }
        }
        let successors = |id: usize| self.objects[id].successor_ids().iter().map(|&to| to as usize).filter(|&to| reaches[to]);
        // Kahn's algorithm on those, to find whether a loop lies among them.
        let mut incoming = vec![0usize; count];
        for id in (0..count).filter(|&id| reaches[id]) {
            for to in successors(id) {
                incoming[to] += 1;
            }
        }
        let mut ready: VecDeque<usize> = (0..count).filter(|&id| reaches[id] && incoming[id] == 0).collect();
        let mut order = Vec::new();
        while let Some(at) = ready.pop_front() {
            order.push(at);
            for to in successors(at) {
                incoming[to] -= 1;
                if incoming[to] == 0 {
                    ready.push_back(to);
                }
            }
        }
        let mut best: Vec<Option<u64>> = vec![None; count];
        if order.len() == reaches.iter().filter(|&&r| r).count() {
            for &at in order.iter().rev() {
                let onward = successors(at).filter_map(|to| best[to]).max();
                best[at] = match (counted[at], onward) {
                    (true, onward) => Some(weights[at].saturating_add(onward.unwrap_or(0))),
                    (false, Some(onward)) => Some(weights[at].saturating_add(onward)),
                    (false, None) => None,
                };
            }
            return best;
        }
        for start in (0..count).filter(|&id| reaches[id]) {
            best[start] = self.heaviest_walk(start, weights, counted, &reaches);
        }
        best
    }

    /// The heaviest route from `start` to a counted Drain passing no object twice, found by
    /// walking them all, or the heaviest found within the step limit.
    fn heaviest_walk(&self, start: usize, weights: &[u64], counted: &[bool], reaches: &[bool]) -> Option<u64> {
        let mut visited = vec![false; weights.len()];
        visited[start] = true;
        let mut path = vec![start];
        let mut sum = weights[start];
        let mut best = counted[start].then_some(sum);
        let mut work: Vec<usize> = vec![0];
        let mut steps = 0u64;
        while let Some(child) = work.last_mut() {
            steps += 1;
            if steps > LOOP_SEARCH_STEPS {
                break;
            }
            let at = *path.last().expect("as deep as the work");
            if let Some(&next) = self.objects[at].successor_ids().get(*child) {
                *child += 1;
                let next = next as usize;
                if reaches[next] && !visited[next] {
                    visited[next] = true;
                    path.push(next);
                    sum = sum.saturating_add(weights[next]);
                    if counted[next] {
                        best = best.max(Some(sum));
                    }
                    work.push(0);
                }
            } else {
                work.pop();
                let left = path.pop().expect("as deep as the work");
                visited[left] = false;
                sum -= weights[left];
            }
        }
        best
    }
}
