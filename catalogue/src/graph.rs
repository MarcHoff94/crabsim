//! A model as a graph, for agents that can't take in a whole model: `describe_model` sums it up
//! in a fixed size, and `query_model` answers questions about its structure, such as what is
//! upstream of an instance or how MUs get from a Source to a Drain. Both work on the model in
//! memory, loaded once per call, from a file or from the store for a run (spec issue 56). There is
//! no graph database. See `docs/design/operation-catalogue.md`.
//!
//! Instances are numbered by their place in the file, and every list comes in that order unless it
//! says otherwise, so answers are the same every time.

use crate::error::{ModelProblem, OperationError};
use crate::jobs::run_number;
use crate::model_files::read_file;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::time::parse_time;
use crabsim::SimTime;
use crabsim_model::{Attribute, Model, BUILT_IN_CLASSES};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<DescribeModel>(), entry::<QueryModel>()]
}

/// How many names a summary lists of each kind.
const LISTED: usize = 10;
/// How many classes a summary lists.
const LISTED_CLASSES: usize = 50;
/// How many loops a summary shows.
const LISTED_LOOPS: usize = 5;
/// How many names a cycle shows.
const CYCLE_NAMES: usize = 20;
/// How many Sources a summary gives the flows of, and how many Drains for each.
const LISTED_FLOWS: usize = 10;
/// The most instances upstream, downstream or found.
const REACHED_LIMIT: usize = 1_000;
/// The most instances in a neighbourhood.
const SUBGRAPH_LIMIT: usize = 500;
/// The paths listed beside the shortest.
const OTHER_PATHS: usize = 20;
/// Paths are counted up to this many.
const PATH_COUNT_LIMIT: u64 = 10_000;
/// Steps a count of paths through loops may take before giving up, so it can't run for ever.
const PATH_COUNT_STEPS: u64 = 2_000_000;
/// The steps a neighbourhood goes either way, unless asked otherwise.
const SUBGRAPH_STEPS: usize = 2;

// --- Which model ---

/// A model file, or the model a run ran: exactly one.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WhichModel {
    /// A model file in the models folder, such as `line.json`.
    pub path: Option<String>,
    /// A run, such as `run-3`: the model it ran, as the results store keeps it, even if the file
    /// has changed since.
    pub run: Option<String>,
}

/// The model's text, and how to name it in a message.
pub(crate) fn model_text(catalogue: &Catalogue, path: Option<String>, run: Option<String>) -> Result<(String, String), OperationError> {
    match (path, run) {
        (Some(path), None) => {
            let (_, text) = read_file(catalogue, &path)?;
            Ok((format!("`{path}`"), text))
        }
        (None, Some(run)) => {
            let number = run_number(&run)?;
            let store = catalogue.store()?;
            let text = store
                .connection()
                .query_row("SELECT model_text FROM runs WHERE number = ?1", [number], |row| row.get::<_, String>(0))
                .optional()?
                .ok_or_else(|| OperationError::not_found(format!("there is no run `{run}` in the results store")))?;
            Ok((format!("the model `{run}` ran"), text))
        }
        _ => Err(OperationError::invalid_input("give either `path` or `run`, not both and not neither")),
    }
}

// --- The graph ---

/// A model's connections, by instance number, each instance's successors by exit and its
/// predecessors by entrance. Connections to unknown instances, to themselves and repeats are left
/// out, so a model with problems still gives a graph.
struct Graph<'m> {
    model: &'m Model,
    successors: Vec<Vec<usize>>,
    predecessors: Vec<Vec<usize>>,
    /// Each instance's built-in class, if its chain of classes can be followed.
    built_in: Vec<Option<&'static str>>,
    by_name: HashMap<&'m str, usize>,
}

impl<'m> Graph<'m> {
    fn new(model: &'m Model) -> Graph<'m> {
        let mut by_name = HashMap::new();
        for (index, instance) in model.instances.iter().enumerate() {
            by_name.entry(instance.name.as_str()).or_insert(index);
        }
        let count = model.instances.len();
        let mut successors = vec![Vec::new(); count];
        let mut predecessors = vec![Vec::new(); count];
        let ends = |connection: &crabsim_model::Connection| {
            let (&from, &to) = (by_name.get(connection.from.as_str())?, by_name.get(connection.to.as_str())?);
            (from != to).then_some((from, to))
        };
        for instance in &model.instances {
            for connection in model.connections_out(&instance.name) {
                if let Some((from, to)) = ends(connection)
                    && !successors[from].contains(&to)
                {
                    successors[from].push(to);
                }
            }
            for connection in model.connections_in(&instance.name) {
                if let Some((from, to)) = ends(connection)
                    && !predecessors[to].contains(&from)
                {
                    predecessors[to].push(from);
                }
            }
        }
        let built_in = model.instances.iter().map(|instance| model.chain(&instance.class).map(|(_, built_in)| built_in)).collect();
        Graph { model, successors, predecessors, built_in, by_name }
    }

    fn len(&self) -> usize {
        self.successors.len()
    }

    fn name(&self, index: usize) -> &'m str {
        &self.model.instances[index].name
    }

    fn names(&self, indices: impl IntoIterator<Item = usize>) -> Vec<String> {
        indices.into_iter().map(|index| self.name(index).to_string()).collect()
    }

    fn is(&self, index: usize, built_in: &str) -> bool {
        self.built_in[index] == Some(built_in)
    }

    /// The instance called `name`, or not found.
    fn find(&self, name: &str, what: &str) -> Result<usize, OperationError> {
        self.by_name.get(name).copied().ok_or_else(|| OperationError::not_found(format!("{what} has no instance called `{name}`")))
    }

    fn node(&self, index: usize) -> Node {
        Node {
            name: self.name(index).to_string(),
            class: self.model.instances[index].class.clone(),
            built_in_class: self.built_in[index].unwrap_or("unknown").to_string(),
        }
    }

    fn reached(&self, index: usize, distance: usize) -> Reached {
        let Node { name, class, built_in_class } = self.node(index);
        Reached { name, class, built_in_class, distance }
    }

    /// Each instance's distance from `starts`, following `next`, up to `steps`.
    fn distances(&self, starts: &[usize], next: impl Fn(usize) -> Vec<usize>, steps: Option<usize>) -> Vec<Option<usize>> {
        let mut distance = vec![None; self.len()];
        let mut queue = VecDeque::new();
        for &start in starts {
            distance[start] = Some(0);
            queue.push_back(start);
        }
        while let Some(at) = queue.pop_front() {
            let d = distance[at].expect("queued, so reached");
            if steps.is_some_and(|steps| d >= steps) {
                continue;
            }
            for to in next(at) {
                if distance[to].is_none() {
                    distance[to] = Some(d + 1);
                    queue.push_back(to);
                }
            }
        }
        distance
    }

    fn downstream_of(&self, starts: &[usize], steps: Option<usize>) -> Vec<Option<usize>> {
        self.distances(starts, |at| self.successors[at].clone(), steps)
    }

    fn upstream_of(&self, starts: &[usize], steps: Option<usize>) -> Vec<Option<usize>> {
        self.distances(starts, |at| self.predecessors[at].clone(), steps)
    }

    /// The strongly connected groups of instances, each by its members in file order, as a group
    /// number per instance and the groups' sizes. A group of more than one is a loop.
    fn groups(&self) -> (Vec<usize>, Vec<usize>) {
        // Tarjan's algorithm, without recursion, so a long line can't overflow the stack.
        let count = self.len();
        let mut index = vec![usize::MAX; count];
        let mut low = vec![0; count];
        let mut on_stack = vec![false; count];
        let mut stack = Vec::new();
        let mut group = vec![usize::MAX; count];
        let mut sizes = Vec::new();
        let mut next_index = 0;
        for root in 0..count {
            if index[root] != usize::MAX {
                continue;
            }
            let mut work = vec![(root, 0usize)];
            while let Some(&mut (at, ref mut child)) = work.last_mut() {
                if *child == 0 {
                    index[at] = next_index;
                    low[at] = next_index;
                    next_index += 1;
                    stack.push(at);
                    on_stack[at] = true;
                }
                if let Some(&to) = self.successors[at].get(*child) {
                    *child += 1;
                    if index[to] == usize::MAX {
                        work.push((to, 0));
                    } else if on_stack[to] {
                        low[at] = low[at].min(index[to]);
                    }
                    continue;
                }
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[at]);
                }
                if low[at] == index[at] {
                    let number = sizes.len();
                    let mut size = 0;
                    loop {
                        let member = stack.pop().expect("its group is on the stack");
                        on_stack[member] = false;
                        group[member] = number;
                        size += 1;
                        if member == at {
                            break;
                        }
                    }
                    sizes.push(size);
                }
            }
        }
        (group, sizes)
    }

    /// The shortest cycle through `start` that stays in its group, from `start` round to the
    /// instance before it.
    fn cycle_through(&self, start: usize, group: &[usize]) -> Vec<usize> {
        let mut parent = vec![usize::MAX; self.len()];
        let mut queue = VecDeque::from([start]);
        parent[start] = start;
        while let Some(at) = queue.pop_front() {
            for &to in &self.successors[at] {
                if to == start {
                    let mut cycle = vec![at];
                    while *cycle.last().expect("not empty") != start {
                        cycle.push(parent[*cycle.last().expect("not empty")]);
                    }
                    cycle.reverse();
                    return cycle;
                }
                if group[to] == group[start] && parent[to] == usize::MAX {
                    parent[to] = at;
                    queue.push_back(to);
                }
            }
        }
        vec![start]
    }

    /// The successors in file order, for searches whose ties go by file order.
    fn sorted_successors(&self) -> Vec<Vec<usize>> {
        self.successors
            .iter()
            .map(|list| {
                let mut list = list.clone();
                list.sort_unstable();
                list
            })
            .collect()
    }
}

/// An instance, with its class.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Node {
    pub name: String,
    pub class: String,
    pub built_in_class: String,
}

/// An instance, with its class and how many connections away it is.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Reached {
    pub name: String,
    pub class: String,
    pub built_in_class: String,
    pub distance: usize,
}

/// A connection from an instance to one of its successors, as the model file has it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Its place in the order `from` offers MUs on, from 1.
    pub exit: u64,
    /// Its place among the connections into `to`, from 1.
    pub entrance: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// --- describe_model ---

pub struct DescribeModel;

#[derive(Serialize, JsonSchema)]
pub struct DescribeModelOutput {
    /// No problems: the model can be run.
    pub valid: bool,
    /// The model's own description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// How many instances it has.
    pub instances: usize,
    /// How many instances are of each class itself, not counting classes derived from it: the
    /// built-in classes that have any, then the model's own in file order, up to 50.
    pub classes: Vec<ClassCount>,
    /// How many classes there are beyond those listed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_classes: usize,
    pub sources: Listed,
    pub drains: Listed,
    /// Instances with more than one successor, where the flow branches.
    pub branch_points: Widest,
    /// Instances with more than one predecessor, where the flow merges.
    pub merge_points: Widest,
    /// For each listed Source, the Drains it reaches.
    pub flows: Vec<Flow>,
    /// Groups of instances that MUs can go round in, such as a rework loop.
    pub loops: Loops,
    /// Instances, other than Sources, that no Source reaches.
    pub unreachable: Listed,
    /// Instances, other than Drains, with no successor, where an MU can't go on.
    pub dead_ends: Listed,
    pub problems: Problems,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// How many instances are of one class.
#[derive(Serialize, JsonSchema)]
pub struct ClassCount {
    pub class: String,
    /// Left out when the class's chain can't be followed, which a problem reports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_in_class: Option<&'static str>,
    pub instances: usize,
}

/// How many there are, and the first 10 by name, in file order.
#[derive(Serialize, JsonSchema)]
pub struct Listed {
    pub count: usize,
    pub names: Vec<String>,
}

/// How many there are, and the 10 widest, equally wide ones in file order.
#[derive(Serialize, JsonSchema)]
pub struct Widest {
    pub count: usize,
    pub widest: Vec<Width>,
}

/// One branch point, with its number of successors, or one merge point, with its number of
/// predecessors.
#[derive(Serialize, JsonSchema)]
pub struct Width {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub successors: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predecessors: Option<usize>,
}

/// Where the MUs of one Source can go.
#[derive(Serialize, JsonSchema)]
pub struct Flow {
    pub source: String,
    /// The Drains it reaches, in file order, up to 10.
    pub drains: Vec<Reach>,
    /// How many Drains it reaches beyond those listed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_drains: usize,
}

/// How a Source reaches one Drain.
#[derive(Serialize, JsonSchema)]
pub struct Reach {
    pub drain: String,
    /// The fewest objects an MU passes between the Source and the Drain.
    pub fewest_objects: usize,
    /// The most, when no route goes round a loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_objects: Option<usize>,
    /// A route goes round a loop, so an MU can pass any number of objects on the way.
    pub through_loop: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct Loops {
    /// How many groups of instances MUs can go round in.
    pub count: usize,
    /// Up to 5, in file order of their first instance.
    pub examples: Vec<Loop>,
}

#[derive(Serialize, JsonSchema)]
pub struct Loop {
    /// How many instances are in the group: an MU can get from each to every other.
    pub objects: usize,
    /// The shortest cycle through its first instance, from it round to the one before it, up to
    /// 20 names.
    pub cycle: Vec<String>,
    /// How many instances the cycle has.
    pub cycle_length: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct Problems {
    pub count: usize,
    /// The first 10, in file order. validate_model gives them all.
    pub first: Vec<ModelProblem>,
}

impl Operation for DescribeModel {
    const NAME: &'static str = "describe_model";
    const DESCRIPTION: &'static str = "Sums up a model, from a file in the models folder (`path`) or the one a run ran (`run`), in a summary whose size doesn't grow with the model: start here rather than reading a model file. It gives the model's description and number of instances, how many instances each class has, the Sources and Drains, where the flow branches and merges (the widest first), for each Source the Drains it reaches and the fewest and most objects an MU passes on the way, loops MUs can go round, instances no Source reaches, dead ends (non-Drains with no successor), and the model's problems: a model with problems is summed up as far as it can be read. Lists stop at 10 names, with the full count beside them. Then use query_model to look closer, and pass the names it gives to run_results or compare_runs as `objects`.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = WhichModel;
    type Output = DescribeModelOutput;

    fn run(catalogue: &Catalogue, input: WhichModel) -> Result<DescribeModelOutput, OperationError> {
        let (_, text) = model_text(catalogue, input.path, input.run)?;
        let (model, problems) = crabsim_model::load_what_can_be_read(&text);
        Ok(describe(&model, &problems))
    }
}

fn describe(model: &Model, problems: &[crabsim_model::Problem]) -> DescribeModelOutput {
    let graph = Graph::new(model);
    let count = graph.len();
    let some = |indices: Vec<usize>| Listed { count: indices.len(), names: graph.names(indices.into_iter().take(LISTED)) };

    let sources: Vec<usize> = (0..count).filter(|&i| graph.is(i, "Source")).collect();
    let drains: Vec<usize> = (0..count).filter(|&i| graph.is(i, "Drain")).collect();

    let (group, sizes) = graph.groups();
    let in_loop = |index: usize| sizes[group[index]] > 1;

    let flows = sources.iter().take(LISTED_FLOWS).map(|&source| flow(&graph, source, &drains, &in_loop)).collect();

    let mut loop_starts: Vec<usize> = Vec::new();
    let mut seen_groups = HashSet::new();
    for (index, &number) in group.iter().enumerate() {
        if sizes[number] > 1 && seen_groups.insert(number) {
            loop_starts.push(index);
        }
    }
    let examples = loop_starts
        .iter()
        .take(LISTED_LOOPS)
        .map(|&start| {
            let cycle = graph.cycle_through(start, &group);
            Loop { objects: sizes[group[start]], cycle_length: cycle.len(), cycle: graph.names(cycle.into_iter().take(CYCLE_NAMES)) }
        })
        .collect();

    let from_sources = graph.downstream_of(&sources, None);
    let unreachable = (0..count).filter(|&i| !graph.is(i, "Source") && from_sources[i].is_none()).collect();
    let dead_ends = (0..count).filter(|&i| !graph.is(i, "Drain") && graph.successors[i].is_empty()).collect();

    let (classes, more_classes) = class_counts(model, &graph);
    DescribeModelOutput {
        valid: problems.is_empty(),
        description: model.description.clone(),
        instances: count,
        classes,
        more_classes,
        sources: some(sources),
        drains: some(drains),
        branch_points: widest(&graph, |i| graph.successors[i].len(), true),
        merge_points: widest(&graph, |i| graph.predecessors[i].len(), false),
        flows,
        loops: Loops { count: loop_starts.len(), examples },
        unreachable: some(unreachable),
        dead_ends: some(dead_ends),
        problems: Problems { count: problems.len(), first: problems.iter().take(LISTED).map(ModelProblem::from).collect() },
    }
}

/// How many instances each class has: the built-in classes that have any, the model's own, then
/// unknown classes instances name, up to 50, and how many more.
fn class_counts(model: &Model, graph: &Graph) -> (Vec<ClassCount>, usize) {
    let mut names: Vec<&str> = BUILT_IN_CLASSES.iter().copied().filter(|built_in| model.instances.iter().any(|i| i.class == *built_in)).collect();
    names.extend(model.classes.iter().map(|class| class.name.as_str()));
    for instance in &model.instances {
        if !names.contains(&instance.class.as_str()) {
            names.push(&instance.class);
        }
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for instance in &model.instances {
        *counts.entry(&instance.class).or_default() += 1;
    }
    let more = names.len().saturating_sub(LISTED_CLASSES);
    let classes = names
        .into_iter()
        .take(LISTED_CLASSES)
        .map(|name| ClassCount {
            class: name.to_string(),
            built_in_class: graph.model.chain(name).map(|(_, built_in)| built_in),
            instances: counts.get(name).copied().unwrap_or(0),
        })
        .collect();
    (classes, more)
}

/// The instances with a `width` above 1, the widest first.
fn widest(graph: &Graph, width: impl Fn(usize) -> usize, successors: bool) -> Widest {
    let mut wide: Vec<usize> = (0..graph.len()).filter(|&i| width(i) > 1).collect();
    let count = wide.len();
    wide.sort_by_key(|&i| std::cmp::Reverse(width(i)));
    let widest = wide
        .into_iter()
        .take(LISTED)
        .map(|i| Width {
            name: graph.name(i).to_string(),
            successors: successors.then(|| width(i)),
            predecessors: (!successors).then(|| width(i)),
        })
        .collect();
    Widest { count, widest }
}

/// The Drains `source` reaches, with the fewest and most objects on the way.
fn flow(graph: &Graph, source: usize, drains: &[usize], in_loop: &impl Fn(usize) -> bool) -> Flow {
    let from_source = graph.downstream_of(&[source], None);
    let reached: Vec<usize> = drains.iter().copied().filter(|&drain| from_source[drain].is_some()).collect();
    let more_drains = reached.len().saturating_sub(LISTED_FLOWS);
    let drains = reached
        .into_iter()
        .take(LISTED_FLOWS)
        .map(|drain| {
            let to_drain = graph.upstream_of(&[drain], None);
            let between: Vec<bool> = (0..graph.len()).map(|i| from_source[i].is_some() && to_drain[i].is_some()).collect();
            let through_loop = (0..graph.len()).any(|i| between[i] && in_loop(i));
            let most_objects = (!through_loop).then(|| longest(graph, source, drain, &between) - 1);
            Reach {
                drain: graph.name(drain).to_string(),
                fewest_objects: from_source[drain].expect("reached") - 1,
                most_objects,
                through_loop,
            }
        })
        .collect();
    Flow { source: graph.name(source).to_string(), drains, more_drains }
}

/// The most connections on a route from `from` to `to` through the instances in `between`, which
/// hold no loop.
fn longest(graph: &Graph, from: usize, to: usize, between: &[bool]) -> usize {
    let order = topological(graph, between).expect("no loop between them");
    let mut most = vec![None::<usize>; graph.len()];
    most[from] = Some(0);
    for at in order {
        let Some(here) = most[at] else { continue };
        for &next in &graph.successors[at] {
            if between[next] {
                most[next] = Some(most[next].map_or(here + 1, |there| there.max(here + 1)));
            }
        }
    }
    most[to].expect("reached")
}

/// The instances in `within` in an order where every connection goes forwards, or none if they
/// hold a loop.
fn topological(graph: &Graph, within: &[bool]) -> Option<Vec<usize>> {
    let mut incoming = vec![0usize; graph.len()];
    for at in (0..graph.len()).filter(|&i| within[i]) {
        for &to in &graph.successors[at] {
            if within[to] {
                incoming[to] += 1;
            }
        }
    }
    let mut ready: VecDeque<usize> = (0..graph.len()).filter(|&i| within[i] && incoming[i] == 0).collect();
    let mut order = Vec::new();
    while let Some(at) = ready.pop_front() {
        order.push(at);
        for &to in &graph.successors[at] {
            if within[to] {
                incoming[to] -= 1;
                if incoming[to] == 0 {
                    ready.push_back(to);
                }
            }
        }
    }
    (order.len() == within.iter().filter(|&&w| w).count()).then_some(order)
}

// --- query_model ---

pub struct QueryModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryModelInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: Option<String>,
    /// A run, such as `run-3`: the model it ran, even if the file has changed since.
    pub run: Option<String>,
    /// The question, naming its kind in `kind`.
    pub query: Query,
}

/// A question about a model's structure. Names are instance names.
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Query {
    /// The instance's predecessors and successors.
    Neighbours { name: String },
    /// Every instance that can reach this one, with its distance in connections.
    Upstream {
        name: String,
        /// At most this many connections away. Unlimited if left out.
        steps: Option<usize>,
    },
    /// Every instance this one can reach, with its distance in connections.
    Downstream {
        name: String,
        /// At most this many connections away. Unlimited if left out.
        steps: Option<usize>,
    },
    /// The routes from one instance to another that pass no object twice, and with `weight`, the
    /// routes by their total weight.
    Paths {
        from: String,
        to: String,
        /// What each object on a route weighs, such as `{"attribute": "process_time"}`, for routes
        /// ordered by their total. Left out, routes are only ordered by how many connections they
        /// take.
        weight: Option<Weight>,
        /// With `weight`: `smallest` total first, the default, or `largest`, as for a critical path.
        order: Option<Order>,
    },
    /// The instances within `steps` connections of this one, following connections either way,
    /// and the connections between them.
    SubgraphAround {
        name: String,
        /// 2 if left out.
        steps: Option<usize>,
    },
    /// The instances that meet every criterion given.
    FindInstances {
        /// Part of the name, ignoring case.
        name_contains: Option<String>,
        /// Of this class or one derived from it, built-in classes too, such as `Station`.
        class: Option<String>,
        /// Part of the description, inherited or its own, ignoring case.
        description_contains: Option<String>,
        /// Conditions on attribute values, such as process_time > "0:30".
        conditions: Option<Vec<Condition>>,
        /// An attribute set on the instance itself rather than inherited, such as `process_time`.
        set_on_instance: Option<String>,
    },
    /// How an instance picks the successor for its MU: round robin, or weighted, with its
    /// successors in the order it ranks them, each with its total.
    Distribution { name: String },
    /// The classes from the built-in classes down, or below one class.
    ClassTree {
        /// Only this class and those derived from it.
        class: Option<String>,
        /// Also the names of each class's own instances.
        #[serde(default)]
        instances: bool,
    },
}

/// A condition on an attribute's value, as the instance has it, inherited or its own.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// Such as `process_time`, `interval`, `capacity` or `starting_mus`.
    pub attribute: String,
    pub op: Op,
    /// Written as in a model file, such as "0:30" for a time or 4 for a number.
    pub value: Value,
}

/// What an object on a route weighs.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Weight {
    /// A number or time the object has, resolved through inheritance, times in seconds:
    /// `process_time`, `interval`, `starting_mus` or `capacity`.
    Attribute(String),
    /// For the model a run ran (`run`): the object's mean over the run's observations of a figure
    /// the results store keeps, such as `working_share` or `mean_lead_time_s`.
    Figure(String),
}

/// Which routes come first by their total weight.
#[derive(Deserialize, Serialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    #[default]
    Smallest,
    Largest,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq)]
pub enum Op {
    #[serde(rename = "=")]
    Equal,
    #[serde(rename = "<")]
    Less,
    #[serde(rename = ">")]
    Greater,
    #[serde(rename = "<=")]
    AtMost,
    #[serde(rename = ">=")]
    AtLeast,
}

impl Op {
    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            Op::Equal => ordering == Equal,
            Op::Less => ordering == Less,
            Op::Greater => ordering == Greater,
            Op::AtMost => ordering != Greater,
            Op::AtLeast => ordering != Less,
        }
    }
}

/// The answer, naming its kind in `kind`, as the question did.
#[derive(Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
// Every answer is an object, which MCP requires of an output schema.
#[schemars(extend("type" = "object"))]
pub enum QueryModelOutput {
    Neighbours {
        name: String,
        predecessors: Vec<Node>,
        /// In the order they are tried.
        successors: Vec<Node>,
    },
    Upstream(Reachable),
    Downstream(Reachable),
    Paths {
        from: String,
        to: String,
        /// The shortest route, both ends included, or null if there is none.
        shortest: Option<Vec<String>>,
        /// Up to 20 other routes, shortest first.
        others: Vec<Vec<String>>,
        /// How many routes there are, counted up to 10,000.
        count: u64,
        /// There are more routes than `count`.
        more: bool,
        /// With `weight`: the routes by their total weight.
        #[serde(skip_serializing_if = "Option::is_none")]
        weighted: Option<Weighted>,
    },
    SubgraphAround {
        name: String,
        steps: usize,
        /// Up to 500 instances, nearest first, with their distance either way.
        nodes: Vec<Reached>,
        /// The connections between them.
        edges: Vec<Edge>,
        /// There were more than 500, and the farthest were left out.
        truncated: bool,
    },
    FindInstances {
        /// Up to 1,000, in file order.
        instances: Vec<Found>,
        /// How many there are.
        count: usize,
        truncated: bool,
    },
    Distribution {
        name: String,
        /// `round_robin` or `weighted`.
        strategy: String,
        /// For a weighted one: the strategy as a model file writes it.
        #[serde(skip_serializing_if = "Option::is_none")]
        weighted: Option<Value>,
        /// Its successors: in exit order for round robin, which takes them in turn, and in
        /// ranking order for weighted.
        successors: Vec<RankedSuccessor>,
    },
    ClassTree {
        /// Each class followed by those derived from it.
        classes: Vec<ClassNode>,
        /// More than 1,000 instance names were asked for, so the rest were left out.
        truncated: bool,
    },
}

/// What is upstream or downstream of an instance.
#[derive(Serialize, JsonSchema)]
pub struct Reachable {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    /// Up to 1,000, nearest first, then in file order. Never the instance itself, even in a loop.
    pub instances: Vec<Reached>,
    /// How many there are.
    pub count: usize,
    pub truncated: bool,
}

/// Routes by their total weight.
#[derive(Serialize, JsonSchema)]
pub struct Weighted {
    /// What each object weighed, such as "process_time, in seconds".
    pub by: String,
    pub order: Order,
    /// Up to 21 routes, the best total first, each passing no object twice.
    pub routes: Vec<WeightedRoute>,
    /// The objects on these routes without the value, which weigh 0, such as a Drain for
    /// `process_time` or an object with no figure in the run.
    pub weightless: Vec<String>,
    /// False when loops on the way stopped crabsim trying every route for the largest totals,
    /// so a heavier route may exist. The smallest totals are always exact.
    pub exact: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct WeightedRoute {
    /// Both ends included.
    pub instances: Vec<String>,
    /// The sum of its objects' weights, both ends included, to 4 significant digits.
    pub total: f64,
}

/// A successor as a distribution strategy ranks it.
#[derive(Serialize, JsonSchema)]
pub struct RankedSuccessor {
    pub name: String,
    /// Its place among the instance's exits, from 1.
    pub exit: usize,
    /// Its place in the ranking, from 1, the best first. Successors that rank equal share a
    /// place and take MUs in turn.
    pub place: usize,
    /// What its route or itself weighs, in seconds for a time. Left out when it has no route to
    /// a counted Drain, which ranks last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct Found {
    pub name: String,
    pub class: String,
    pub built_in_class: String,
    /// The values of the attributes the criteria asked about, as a model file writes them.
    pub values: Map<String, Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct ClassNode {
    pub name: String,
    /// Left out for a built-in class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// How far below the top of the tree.
    pub depth: usize,
    /// The attributes it sets, as a model file writes them.
    pub sets: Value,
    /// The classes derived from it directly.
    pub derived: Vec<String>,
    /// How many instances are of it.
    pub instances_direct: usize,
    /// How many are of it or a class derived from it.
    pub instances_total: usize,
    /// With `instances`: the names of its own instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances: Option<Vec<String>>,
}

impl Operation for QueryModel {
    const NAME: &'static str = "query_model";
    const DESCRIPTION: &'static str = "Answers a question about a model's structure, from a file in the models folder (`path`) or the one a run ran (`run`), without reading the whole model. `query` names its kind in `kind`: neighbours {name} gives predecessors and successors; upstream or downstream {name, steps?} every instance that can reach it or that it reaches, with distances; paths {from, to, weight?, order?} the shortest route, up to 20 others shortest first, and how many routes there are, counted up to 10,000, none passing an object twice, and with weight {\"attribute\": \"process_time\"} (also interval, starting_mus or capacity, resolved through inheritance, times in seconds) or, for a run's model, {\"figure\": \"working_share\"} (any statistic the results store keeps, the mean over the run's observations) up to 21 routes by the sum of their objects' weights, smallest first or with order \"largest\" largest first, as for a critical path, each with its total, naming the objects on them that have no value and so weigh 0; subgraph_around {name, steps?} the instances within steps (2 by default) following connections either way, and the connections between them, each with its exit, entrance and description; find_instances {name_contains?, class?, description_contains?, conditions?, set_on_instance?} the instances meeting every criterion, such as conditions [{\"attribute\": \"process_time\", \"op\": \">\", \"value\": \"0:30\"}] with op =, <, >, <= or >=, where class includes derived classes; class_tree {class?, instances?} the classes from the built-ins down with what each sets and how many instances follow it; distribution {name} how a Source or Station picks the successor for its MU, round_robin or weighted (ADR 0016), with its successors in the order it ranks them, each with its place and total, those sharing a place taking MUs in turn. Lists are capped (1,000 instances, 500 in a neighbourhood) and say so. A model with problems is refused with them: describe_model sums it up anyway. Pass the names it gives to run_results or compare_runs as `objects`.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = QueryModelInput;
    type Output = QueryModelOutput;

    fn run(catalogue: &Catalogue, input: QueryModelInput) -> Result<QueryModelOutput, OperationError> {
        let run = input.run.clone();
        let (what, text) = model_text(catalogue, input.path, input.run)?;
        let model = crabsim_model::load(&text).map_err(|problems| {
            let mut error = OperationError::invalid_model(&what, &problems);
            error.message += ", so its structure can't be queried: describe_model sums it up anyway";
            error
        })?;
        let graph = Graph::new(&model);
        match input.query {
            Query::Neighbours { name } => {
                let at = graph.find(&name, &what)?;
                Ok(QueryModelOutput::Neighbours {
                    name,
                    predecessors: graph.predecessors[at].iter().map(|&i| graph.node(i)).collect(),
                    successors: graph.successors[at].iter().map(|&i| graph.node(i)).collect(),
                })
            }
            Query::Upstream { name, steps } => {
                let at = graph.find(&name, &what)?;
                Ok(QueryModelOutput::Upstream(reachable(&graph, at, name, steps, graph.upstream_of(&[at], steps))))
            }
            Query::Downstream { name, steps } => {
                let at = graph.find(&name, &what)?;
                Ok(QueryModelOutput::Downstream(reachable(&graph, at, name, steps, graph.downstream_of(&[at], steps))))
            }
            Query::Paths { from, to, weight, order } => {
                let start = graph.find(&from, &what)?;
                let end = graph.find(&to, &what)?;
                if start == end {
                    return Err(OperationError::invalid_input("`from` and `to` are the same instance: give two different ones"));
                }
                let weighted = match weight {
                    Some(weight) => {
                        let (by, weights) = weights(catalogue, &graph, weight, run.as_deref())?;
                        Some(weighted_paths(&graph, start, end, &weights, by, order.unwrap_or_default()))
                    }
                    None if order.is_some() => return Err(OperationError::invalid_input("`order` orders routes by their total weight: give a `weight` too")),
                    None => None,
                };
                let (shortest, others) = shortest_paths(&graph, start, end);
                let (count, more) = count_paths(&graph, start, end);
                Ok(QueryModelOutput::Paths {
                    from,
                    to,
                    shortest: shortest.map(|path| graph.names(path)),
                    others: others.into_iter().map(|path| graph.names(path)).collect(),
                    count,
                    more,
                    weighted,
                })
            }
            Query::SubgraphAround { name, steps } => {
                let at = graph.find(&name, &what)?;
                let steps = steps.unwrap_or(SUBGRAPH_STEPS);
                let either_way = |i: usize| graph.successors[i].iter().chain(&graph.predecessors[i]).copied().collect();
                let distance = graph.distances(&[at], either_way, Some(steps));
                let mut nodes: Vec<usize> = (0..graph.len()).filter(|&i| distance[i].is_some()).collect();
                nodes.sort_by_key(|&i| (distance[i], i));
                let truncated = nodes.len() > SUBGRAPH_LIMIT;
                nodes.truncate(SUBGRAPH_LIMIT);
                let inside: HashSet<usize> = nodes.iter().copied().collect();
                let mut froms: Vec<usize> = nodes.clone();
                froms.sort_unstable();
                let edges = froms
                    .iter()
                    .flat_map(|&from| graph.successors[from].iter().filter(|to| inside.contains(to)).map(move |&to| (from, to)))
                    .filter_map(|(from, to)| {
                        let connection = graph.model.connections.iter().find(|c| c.from == graph.name(from) && c.to == graph.name(to))?;
                        Some(Edge {
                            from: connection.from.clone(),
                            to: connection.to.clone(),
                            exit: connection.exit,
                            entrance: connection.entrance,
                            description: connection.description.clone(),
                        })
                    })
                    .collect();
                let nodes = nodes.into_iter().map(|i| graph.reached(i, distance[i].expect("inside"))).collect();
                Ok(QueryModelOutput::SubgraphAround { name, steps, nodes, edges, truncated })
            }
            Query::FindInstances { name_contains, class, description_contains, conditions, set_on_instance } => {
                find_instances(&graph, &what, name_contains, class, description_contains, conditions.unwrap_or_default(), set_on_instance)
            }
            Query::Distribution { name } => {
                let at = graph.find(&name, &what)?;
                distribution(&model, at, name)
            }
            Query::ClassTree { class, instances } => class_tree(&model, &what, class, instances),
        }
    }
}

fn reachable(graph: &Graph, at: usize, name: String, steps: Option<usize>, distance: Vec<Option<usize>>) -> Reachable {
    let mut found: Vec<usize> = (0..graph.len()).filter(|&i| i != at && distance[i].is_some()).collect();
    found.sort_by_key(|&i| (distance[i], i));
    let count = found.len();
    let instances = found.into_iter().take(REACHED_LIMIT).map(|i| graph.reached(i, distance[i].expect("found"))).collect();
    Reachable { name, steps, instances, count, truncated: count > REACHED_LIMIT }
}

/// The shortest route through instances not `blocked`, avoiding the connections in `cut`. Of
/// equally short ones, the first in file order, compared instance by instance.
fn shortest_avoiding(successors: &[Vec<usize>], from: usize, to: usize, blocked: &[bool], cut: &HashSet<(usize, usize)>) -> Option<Vec<usize>> {
    // Breadth first with successors in file order gives each instance the parent that comes
    // first, so the route found is the first of the shortest in file order.
    let mut parent = vec![usize::MAX; successors.len()];
    parent[from] = from;
    let mut queue = VecDeque::from([from]);
    while let Some(at) = queue.pop_front() {
        if at == to {
            let mut path = vec![to];
            while *path.last().expect("not empty") != from {
                path.push(parent[*path.last().expect("not empty")]);
            }
            path.reverse();
            return Some(path);
        }
        for &next in &successors[at] {
            if parent[next] == usize::MAX && !blocked[next] && !cut.contains(&(at, next)) {
                parent[next] = at;
                queue.push_back(next);
            }
        }
    }
    None
}

/// The shortest route and up to 20 others, shortest first, none passing an object twice: Yen's
/// algorithm, with ties in file order.
fn shortest_paths(graph: &Graph, from: usize, to: usize) -> (Option<Vec<usize>>, Vec<Vec<usize>>) {
    let successors = graph.sorted_successors();
    let found = best_routes(graph.len(), from, to, |from, to, blocked, cut| shortest_avoiding(&successors, from, to, blocked, cut), |path| path.len());
    let mut found = found.into_iter();
    let shortest = found.next();
    (shortest, found.collect())
}

/// The best route and up to 20 others, best first by `key`, none passing an object twice: Yen's
/// algorithm, with `best` giving the best route avoiding `blocked` instances and `cut`
/// connections, and ties in file order.
fn best_routes<K: Ord>(
    count: usize,
    from: usize,
    to: usize,
    mut best: impl FnMut(usize, usize, &[bool], &HashSet<(usize, usize)>) -> Option<Vec<usize>>,
    key: impl Fn(&[usize]) -> K,
) -> Vec<Vec<usize>> {
    let none_blocked = vec![false; count];
    let Some(first) = best(from, to, &none_blocked, &HashSet::new()) else {
        return Vec::new();
    };
    let mut found = vec![first];
    let mut candidates: BTreeSet<(K, Vec<usize>)> = BTreeSet::new();
    while found.len() <= OTHER_PATHS {
        let last = found.last().expect("not empty").clone();
        for spur in 0..last.len() - 1 {
            let root = &last[..=spur];
            let mut cut = HashSet::new();
            for path in &found {
                if path.len() > spur + 1 && path[..=spur] == *root {
                    cut.insert((path[spur], path[spur + 1]));
                }
            }
            let mut blocked = vec![false; count];
            for &i in &root[..spur] {
                blocked[i] = true;
            }
            if let Some(tail) = best(root[spur], to, &blocked, &cut) {
                let mut path = root[..spur].to_vec();
                path.extend(tail);
                if !found.contains(&path) {
                    candidates.insert((key(&path), path));
                }
            }
        }
        let Some((_, next)) = candidates.pop_first() else { break };
        found.push(next);
    }
    found
}

/// A total weight, ordered so routes can be sorted by it. Weights are never NaN.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Total(f64);

impl Eq for Total {}

impl PartialOrd for Total {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Total {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// What each instance weighs, `None` where it has no value, and what weighs them.
fn weights(catalogue: &Catalogue, graph: &Graph, weight: Weight, run: Option<&str>) -> Result<(String, Vec<Option<f64>>), OperationError> {
    let model = graph.model;
    match weight {
        Weight::Attribute(name) => {
            let numeric = [Attribute::ProcessTime, Attribute::Interval, Attribute::StartingMus, Attribute::Capacity];
            let attribute = numeric.into_iter().find(|attribute| attribute.name() == name).ok_or_else(|| {
                OperationError::invalid_input(format!(
                    "routes can be weighed by process_time, interval, starting_mus or capacity, not `{name}`, or by a run's figure with {{\"figure\": ...}}"
                ))
            })?;
            let weights = (0..graph.len())
                .map(|index| {
                    let resolved = model.resolve(index)?;
                    if !attribute.applies_to(resolved.built_in) {
                        return None;
                    }
                    let has = &resolved.attributes;
                    match attribute {
                        Attribute::ProcessTime => has.process_time.map(|time| time.as_millis() as f64 / 1000.0),
                        Attribute::Interval => has.interval.map(|time| time.as_millis() as f64 / 1000.0),
                        Attribute::StartingMus => has.starting_mus.map(|number| number as f64),
                        _ => has.capacity.map(|number| number as f64),
                    }
                })
                .collect();
            let unit = if matches!(attribute, Attribute::ProcessTime | Attribute::Interval) { ", in seconds" } else { "" };
            Ok((format!("{name}{unit}"), weights))
        }
        Weight::Figure(name) => {
            let run = run.ok_or_else(|| OperationError::invalid_input("a figure is a run's: give `run` rather than `path` to weigh routes by one"))?;
            if !crate::results::STATISTICS.contains(&name.as_str()) {
                return Err(OperationError::invalid_input(format!(
                    "the results store keeps no figure `{name}`: there are {}",
                    crate::results::STATISTICS.join(", ")
                )));
            }
            let store = catalogue.store()?;
            let connection = store.connection();
            let mut statement = connection.prepare("SELECT object, avg(value) FROM statistics WHERE run = ?1 AND statistic = ?2 GROUP BY object")?;
            let rows = statement.query_map([run, name.as_str()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))?;
            let mut means: HashMap<String, f64> = HashMap::new();
            for row in rows {
                let (object, mean) = row?;
                means.insert(object, mean);
            }
            let weights = (0..graph.len()).map(|index| means.get(graph.name(index)).copied()).collect();
            Ok((format!("{name}, the mean over the run's observations"), weights))
        }
    }
}

/// Up to 21 routes from `from` to `to` by the sum of their instances' weights, those without
/// one weighing 0, best first as `order` says.
fn weighted_paths(graph: &Graph, from: usize, to: usize, weights: &[Option<f64>], by: String, order: Order) -> Weighted {
    let weight: Vec<f64> = weights.iter().map(|weight| weight.unwrap_or(0.0).max(0.0)).collect();
    let total = |path: &[usize]| path.iter().map(|&i| weight[i]).sum::<f64>();
    let successors = graph.sorted_successors();
    let (routes, exact) = match order {
        Order::Smallest => {
            let routes = best_routes(
                graph.len(),
                from,
                to,
                |from, to, blocked, cut| lightest_avoiding(&successors, &weight, from, to, blocked, cut),
                |path| (Total(total(path)), path.len()),
            );
            (routes, true)
        }
        Order::Largest => {
            let after = graph.downstream_of(&[from], None);
            let before = graph.upstream_of(&[to], None);
            let between: Vec<bool> = (0..graph.len()).map(|i| after[i].is_some() && before[i].is_some()).collect();
            match topological(graph, &between) {
                Some(order) => {
                    let routes = best_routes(
                        graph.len(),
                        from,
                        to,
                        |from, to, blocked, cut| heaviest_avoiding(&successors, &order, &weight, from, to, blocked, cut),
                        |path| (std::cmp::Reverse(Total(total(path))), path.len()),
                    );
                    (routes, true)
                }
                None => heaviest_through_loops(&successors, &between, &weight, from, to),
            }
        }
    };
    let mut weightless = Vec::new();
    for &i in routes.iter().flatten() {
        if weights[i].is_none() && !weightless.contains(&i) {
            weightless.push(i);
        }
    }
    weightless.sort_unstable();
    Weighted {
        by,
        order,
        routes: routes.iter().map(|path| WeightedRoute { instances: graph.names(path.iter().copied()), total: crate::results::shown(total(path)) }).collect(),
        weightless: graph.names(weightless),
        exact,
    }
}

/// The lightest route through instances not `blocked`, avoiding the connections in `cut`:
/// Dijkstra's algorithm on the instances' weights, of equal totals the one with fewest
/// connections.
fn lightest_avoiding(successors: &[Vec<usize>], weight: &[f64], from: usize, to: usize, blocked: &[bool], cut: &HashSet<(usize, usize)>) -> Option<Vec<usize>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut best: Vec<Option<(Total, usize)>> = vec![None; successors.len()];
    let mut parent = vec![usize::MAX; successors.len()];
    best[from] = Some((Total(weight[from]), 0));
    let mut heap = BinaryHeap::from([Reverse((Total(weight[from]), 0usize, from))]);
    while let Some(Reverse((sum, hops, at))) = heap.pop() {
        if best[at] != Some((sum, hops)) {
            continue;
        }
        if at == to {
            return Some(route_back(&parent, from, to));
        }
        for &next in &successors[at] {
            if blocked[next] || cut.contains(&(at, next)) || next == from {
                continue;
            }
            let reached = (Total(sum.0 + weight[next]), hops + 1);
            if best[next].is_none_or(|known| reached < known) {
                best[next] = Some(reached);
                parent[next] = at;
                heap.push(Reverse((reached.0, reached.1, next)));
            }
        }
    }
    None
}

/// The heaviest route through instances not `blocked`, avoiding the connections in `cut`, where
/// no loop lies between `from` and `to`: instances in `order`, a topological order of those
/// between, each taking its heaviest way in. Of equal totals, the one with fewest connections.
fn heaviest_avoiding(
    successors: &[Vec<usize>],
    order: &[usize],
    weight: &[f64],
    from: usize,
    to: usize,
    blocked: &[bool],
    cut: &HashSet<(usize, usize)>,
) -> Option<Vec<usize>> {
    let mut inside = vec![false; successors.len()];
    for &at in order {
        inside[at] = true;
    }
    let mut best: Vec<Option<(f64, usize)>> = vec![None; successors.len()];
    let mut parent = vec![usize::MAX; successors.len()];
    best[from] = Some((weight[from], 0));
    for &at in order {
        let Some((sum, hops)) = best[at] else { continue };
        for &next in &successors[at] {
            if !inside[next] || blocked[next] || cut.contains(&(at, next)) || next == from {
                continue;
            }
            let reached = (sum + weight[next], hops + 1);
            let better = best[next].is_none_or(|(known, known_hops)| reached.0 > known || (reached.0 == known && reached.1 < known_hops));
            if better {
                best[next] = Some(reached);
                parent[next] = at;
            }
        }
    }
    best[to]?;
    Some(route_back(&parent, from, to))
}

/// The route to `to` from `from`, following each instance's `parent` back.
fn route_back(parent: &[usize], from: usize, to: usize) -> Vec<usize> {
    let mut path = vec![to];
    while *path.last().expect("not empty") != from {
        path.push(parent[*path.last().expect("not empty")]);
    }
    path.reverse();
    path
}

/// The 21 heaviest routes where loops lie between `from` and `to`, none passing an object twice,
/// found by walking every route, and whether it walked them all before giving up after too many
/// steps. The heaviest route through loops is hard to find in general, so this is the one answer
/// that may not be exact.
fn heaviest_through_loops(successors: &[Vec<usize>], between: &[bool], weight: &[f64], from: usize, to: usize) -> (Vec<Vec<usize>>, bool) {
    use std::cmp::Reverse;
    let mut kept: BTreeSet<(Reverse<Total>, usize, Vec<usize>)> = BTreeSet::new();
    let mut visited = vec![false; successors.len()];
    let mut path = vec![from];
    let mut sum = weight[from];
    let mut work: Vec<usize> = vec![0];
    visited[from] = true;
    let mut steps = 0u64;
    let mut exact = true;
    while let Some(child) = work.last_mut() {
        steps += 1;
        if steps > PATH_COUNT_STEPS {
            exact = false;
            break;
        }
        let at = *path.last().expect("as deep as the work");
        if let Some(&next) = successors[at].get(*child) {
            *child += 1;
            if next == to {
                let mut route = path.clone();
                route.push(to);
                kept.insert((Reverse(Total(sum + weight[to])), route.len(), route));
                if kept.len() > OTHER_PATHS + 1 {
                    kept.pop_last();
                }
            } else if between[next] && !visited[next] {
                visited[next] = true;
                path.push(next);
                sum += weight[next];
                work.push(0);
            }
        } else {
            work.pop();
            let left = path.pop().expect("as deep as the work");
            visited[left] = false;
            sum -= weight[left];
        }
    }
    (kept.into_iter().map(|(_, _, route)| route).collect(), exact)
}

/// How many routes there are from `from` to `to` that pass no object twice, up to 10,000, and
/// whether there are more.
fn count_paths(graph: &Graph, from: usize, to: usize) -> (u64, bool) {
    let after = graph.downstream_of(&[from], None);
    let before = graph.upstream_of(&[to], None);
    let between: Vec<bool> = (0..graph.len()).map(|i| after[i].is_some() && before[i].is_some()).collect();
    if !between[to] {
        return (0, false);
    }
    if let Some(order) = topological(graph, &between) {
        // No loop on the way, so every route passes each object once: count them in order.
        let mut routes = vec![0u64; graph.len()];
        routes[from] = 1;
        for at in order {
            for &next in &graph.successors[at] {
                if between[next] {
                    routes[next] = routes[next].saturating_add(routes[at]).min(PATH_COUNT_LIMIT + 1);
                }
            }
        }
        let count = routes[to];
        return (count.min(PATH_COUNT_LIMIT), count > PATH_COUNT_LIMIT);
    }
    // Loops on the way: walk every route, giving up at the limit or after too many steps.
    let mut visited = vec![false; graph.len()];
    let mut found = 0u64;
    let mut steps = 0u64;
    let mut work: Vec<(usize, usize)> = vec![(from, 0)];
    visited[from] = true;
    while let Some(&mut (at, ref mut child)) = work.last_mut() {
        steps += 1;
        if found > PATH_COUNT_LIMIT || steps > PATH_COUNT_STEPS {
            return (found.min(PATH_COUNT_LIMIT), true);
        }
        if let Some(&next) = graph.successors[at].get(*child) {
            *child += 1;
            if next == to {
                found += 1;
            } else if between[next] && !visited[next] {
                visited[next] = true;
                work.push((next, 0));
            }
        } else {
            visited[at] = false;
            work.pop();
        }
    }
    (found.min(PATH_COUNT_LIMIT), found > PATH_COUNT_LIMIT)
}

/// What a condition compares: a time, a number, or for `=` any value as a file writes it.
enum Wanted {
    Time(SimTime),
    Number(u64),
    Exactly(Value),
}

fn find_instances(
    graph: &Graph,
    what: &str,
    name_contains: Option<String>,
    class: Option<String>,
    description_contains: Option<String>,
    conditions: Vec<Condition>,
    set_on_instance: Option<String>,
) -> Result<QueryModelOutput, OperationError> {
    let model = graph.model;
    let attribute = |name: &str| {
        Attribute::ALL.into_iter().find(|attribute| attribute.name() == name).ok_or_else(|| {
            let known: Vec<&str> = Attribute::ALL.iter().map(|attribute| attribute.name()).collect();
            OperationError::invalid_input(format!("there is no attribute `{name}`: there are {}", known.join(", ")))
        })
    };
    let mut wanted = Vec::new();
    for condition in &conditions {
        let attribute = attribute(&condition.attribute)?;
        let value = match attribute {
            Attribute::Interval | Attribute::ProcessTime => {
                let time = condition.value.as_str().and_then(parse_time).or_else(|| condition.value.as_f64().map(SimTime::from_secs_f64));
                Wanted::Time(time.ok_or_else(|| {
                    OperationError::invalid_input(format!("`{}` is a time: write it as 90, 1:30 or 8:00:00, not {}", attribute.name(), condition.value))
                })?)
            }
            Attribute::StartingMus | Attribute::Capacity => {
                let number = condition.value.as_u64().or_else(|| condition.value.as_str().and_then(|text| text.parse().ok()));
                Wanted::Number(number.ok_or_else(|| OperationError::invalid_input(format!("`{}` is a whole number, not {}", attribute.name(), condition.value)))?)
            }
            _ if condition.op == Op::Equal => Wanted::Exactly(condition.value.clone()),
            _ => return Err(OperationError::invalid_input(format!("`{}` can only be compared with =", attribute.name()))),
        };
        wanted.push((attribute, condition.op, value));
    }
    let set_on_instance = set_on_instance.map(|name| attribute(&name)).transpose()?;
    if let Some(class) = &class
        && model.chain(class).is_none()
    {
        return Err(OperationError::not_found(format!("{what} has no class called `{class}`")));
    }
    let name_contains = name_contains.map(|text| text.to_lowercase());
    let description_contains = description_contains.map(|text| text.to_lowercase());

    let mut found = Vec::new();
    for (index, instance) in model.instances.iter().enumerate() {
        let Some(resolved) = model.resolve(index) else { continue };
        let (chain, built_in) = model.chain(&instance.class).expect("resolved, so the chain is sound");
        if let Some(class) = &class
            && built_in != class
            && !chain.iter().any(|c| c.name == *class)
        {
            continue;
        }
        if name_contains.as_ref().is_some_and(|part| !instance.name.to_lowercase().contains(part)) {
            continue;
        }
        if description_contains.as_ref().is_some_and(|part| !resolved.attributes.description.as_ref().is_some_and(|d| d.to_lowercase().contains(part))) {
            continue;
        }
        if set_on_instance.is_some_and(|attribute| !instance.attributes.is_set(attribute)) {
            continue;
        }
        let values = resolved.attributes.to_json();
        let meets = |&(attribute, op, ref value): &(Attribute, Op, Wanted)| {
            if !attribute.applies_to(built_in) {
                return false;
            }
            let has = &resolved.attributes;
            match value {
                Wanted::Time(time) => {
                    let own = if attribute == Attribute::Interval { has.interval } else { has.process_time };
                    own.is_some_and(|own| op.holds(own.cmp(time)))
                }
                Wanted::Number(number) => {
                    let own = if attribute == Attribute::Capacity { has.capacity } else { has.starting_mus };
                    own.is_some_and(|own| op.holds(own.cmp(number)))
                }
                Wanted::Exactly(value) => values.get(attribute.name()) == Some(value),
            }
        };
        if !wanted.iter().all(meets) {
            continue;
        }
        let mut shown = Map::new();
        for attribute in wanted.iter().map(|(attribute, _, _)| *attribute).chain(set_on_instance) {
            if let Some(value) = values.get(attribute.name()) {
                shown.insert(attribute.name().to_string(), value.clone());
            }
        }
        found.push(Found {
            name: instance.name.clone(),
            class: instance.class.clone(),
            built_in_class: built_in.to_string(),
            values: shown,
        });
    }
    let count = found.len();
    found.truncate(REACHED_LIMIT);
    Ok(QueryModelOutput::FindInstances { instances: found, count, truncated: count > REACHED_LIMIT })
}

/// How the instance at `at` picks its successors, worked out as a run of the model starts.
fn distribution(model: &Model, at: usize, name: String) -> Result<QueryModelOutput, OperationError> {
    let resolved = model.resolve(at).expect("a valid model's instances resolve");
    let successors = model.successors(&name);
    let strategy = resolved.attributes.distribution.clone().unwrap_or(crabsim_model::DistributionStrategy::RoundRobin);
    let description = model.to_description();
    let (strategy, weighted, listed) = match (&strategy, description.ranking(at as u32)) {
        (crabsim_model::DistributionStrategy::Weighted(weighted), Some(ranking)) => {
            let in_seconds = matches!(weighted.by, Attribute::ProcessTime | Attribute::Interval);
            let total = |exit: usize| ranking.totals[exit].map(|total| if in_seconds { total as f64 / 1000.0 } else { total as f64 });
            let listed = ranking
                .groups
                .iter()
                .enumerate()
                .flat_map(|(place, group)| group.iter().map(move |&exit| (place, exit)))
                .map(|(place, exit)| RankedSuccessor { name: successors[exit].to_string(), exit: exit + 1, place: place + 1, total: total(exit) })
                .collect();
            ("weighted", Some(crabsim_model::write_distribution(&strategy)), listed)
        }
        _ => {
            let listed = successors.iter().enumerate().map(|(exit, to)| RankedSuccessor { name: to.to_string(), exit: exit + 1, place: 1, total: None }).collect();
            ("round_robin", None, listed)
        }
    };
    Ok(QueryModelOutput::Distribution { name, strategy: strategy.to_string(), weighted, successors: listed })
}

fn class_tree(model: &Model, what: &str, class: Option<String>, with_instances: bool) -> Result<QueryModelOutput, OperationError> {
    let roots: Vec<&str> = match &class {
        None => BUILT_IN_CLASSES.to_vec(),
        Some(name) if model.chain(name).is_some() => vec![name.as_str()],
        Some(name) => return Err(OperationError::not_found(format!("{what} has no class called `{name}`"))),
    };
    let mut derived: HashMap<&str, Vec<&str>> = HashMap::new();
    for class in model.classes.iter().filter(|class| !class.refines_built_in()) {
        derived.entry(class.parent.as_str()).or_default().push(&class.name);
    }
    let mut own: HashMap<&str, Vec<&str>> = HashMap::new();
    for instance in &model.instances {
        own.entry(instance.class.as_str()).or_default().push(&instance.name);
    }
    // Each class's own instances and those of every class below it. A valid model's classes
    // form a tree, so this ends.
    fn total<'m>(name: &str, derived: &HashMap<&str, Vec<&'m str>>, own: &HashMap<&str, Vec<&'m str>>) -> usize {
        own.get(name).map_or(0, Vec::len) + derived.get(name).into_iter().flatten().map(|child| total(child, derived, own)).sum::<usize>()
    }
    let none = Vec::new();
    let mut classes = Vec::new();
    let mut names_left = REACHED_LIMIT;
    let mut truncated = false;
    let mut work: Vec<(&str, usize)> = roots.into_iter().rev().map(|root| (root, 0)).collect();
    while let Some((name, depth)) = work.pop() {
        let below = derived.get(name).unwrap_or(&none);
        work.extend(below.iter().rev().map(|child| (*child, depth + 1)));
        let mine = own.get(name).unwrap_or(&none);
        let instances = with_instances.then(|| {
            let shown: Vec<String> = mine.iter().take(names_left).map(|name| name.to_string()).collect();
            truncated |= shown.len() < mine.len();
            names_left -= shown.len();
            shown
        });
        classes.push(ClassNode {
            name: name.to_string(),
            parent: model.class(name).filter(|class| !class.refines_built_in()).map(|class| class.parent.clone()),
            depth,
            sets: model.class(name).map_or_else(|| Value::Object(Map::new()), |class| class.attributes.to_json()),
            derived: below.iter().map(|child| child.to_string()).collect(),
            instances_direct: mine.len(),
            instances_total: total(name, &derived, &own),
            instances,
        });
    }
    Ok(QueryModelOutput::ClassTree { classes, truncated })
}
