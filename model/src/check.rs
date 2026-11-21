//! What can be wrong with a model whose shape is fine: names, classes and their chains, values
//! out of range, values set nowhere, connections, and a Source that would create MUs forever.

use crate::read::Broken;
use crate::{Attribute, Attributes, Capacity, Failures, Model, Problem, RepairTime, BUILT_IN_CLASSES};

/// More starting MUs than this in one Buffer is refused: each takes memory from Init on.
const MOST_STARTING_MUS: u64 = 1_000_000;
/// A Station with capacity looks through its places on each of its events, so a typo such as
/// 1000000 would slow every one of them.
const MOST_STATION_CAPACITY: u64 = 10_000;
use crabsim::{Dimensions, SimTime};
use std::collections::{HashMap, VecDeque};

pub(crate) fn check(model: &Model, broken: &Broken) -> Vec<Problem> {
    let mut checker = Checker { model, broken, problems: Vec::new(), by_name: HashMap::new() };
    checker.class_names();
    for index in 0..model.classes.len() {
        checker.class(index);
    }
    checker.instance_names();
    checker.instance_streams();
    for index in 0..model.instances.len() {
        checker.instance(index);
    }
    checker.connections();
    checker.places(Place::Exit);
    checker.places(Place::Entrance);
    checker.endless_sources();
    checker.problems
}

struct Checker<'m> {
    model: &'m Model,
    broken: &'m Broken,
    problems: Vec<Problem>,
    /// Each instance's name and the first instance that has it.
    by_name: HashMap<&'m str, usize>,
}

/// Whose attributes are being checked, for pointers, names and what couldn't be read.
#[derive(Clone, Copy)]
enum Owner {
    Class(usize),
    Instance(usize),
    Connection(usize),
}

/// One of a connection's two places.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Exit,
    Entrance,
}

impl<'m> Checker<'m> {
    fn problem(&mut self, owner: Owner, at: &str, message: impl Into<String>) {
        let (pointer, name) = match owner {
            Owner::Class(index) => (
                format!("/classes/{index}{at}"),
                (!self.broken.class_names.contains(&index)).then(|| self.model.classes[index].name.clone()),
            ),
            Owner::Instance(index) => (
                format!("/instances/{index}{at}"),
                (!self.broken.names.contains(&index)).then(|| self.model.instances[index].name.clone()),
            ),
            Owner::Connection(index) => {
                let connection = &self.model.connections[index];
                (
                    format!("/connections/{index}{at}"),
                    (!self.broken.connections.contains(&index)).then(|| format!("{} → {}", connection.from, connection.to)),
                )
            }
        };
        self.problems.push(Problem { pointer, name, message: message.into() });
    }

    /// Whether an attribute was there but couldn't be read.
    fn unreadable(&self, owner: Owner, attribute: Attribute) -> bool {
        match owner {
            Owner::Class(index) => self.broken.class_attributes.contains(&(index, attribute.name())),
            Owner::Instance(index) => self.broken.attributes.contains(&(index, attribute.name())),
            Owner::Connection(_) => false,
        }
    }

    /// The built-in class at the root of an instance's chain, if its class and chain are sound.
    fn built_in(&self, index: usize) -> Option<&'static str> {
        if self.broken.classes.contains(&index) {
            return None;
        }
        self.model.chain(&self.model.instances[index].class).map(|(_, built_in)| built_in)
    }

    fn name_problems(&mut self, owner: Owner, name: &str) {
        if name.is_empty() {
            self.problem(owner, "/name", "a name can't be empty");
        } else if name.trim() != name {
            self.problem(owner, "/name", "a name can't start or end with spaces");
        }
    }

    // ---------- Classes ----------

    fn class_names(&mut self) {
        let model = self.model;
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (index, class) in model.classes.iter().enumerate() {
            if self.broken.class_names.contains(&index) {
                continue;
            }
            self.name_problems(Owner::Class(index), &class.name);
            if let Some(&first) = seen.get(class.name.as_str()) {
                self.problem(Owner::Class(index), "/name", format!("class {first} already has this name"));
            } else {
                seen.insert(&class.name, index);
            }
        }
    }

    fn class(&mut self, index: usize) {
        let model = self.model;
        let class = &model.classes[index];
        let owner = Owner::Class(index);
        if !self.broken.class_parents.contains(&index) {
            if class.refines_built_in() {
                if class.parent != class.name {
                    self.problem(owner, "/parent", crate::read::refinement_with_parent(&class.name));
                }
            } else if class.parent == class.name {
                self.problem(owner, "/parent", "a class can't derive from itself");
            } else if !BUILT_IN_CLASSES.contains(&class.parent.as_str()) && model.class(&class.parent).is_none() {
                let message = self.unknown_class(&class.parent);
                self.problem(owner, "/parent", message);
            } else if let Some(circle) = self.circle(index) {
                self.problem(owner, "/parent", format!("the classes derive from each other in a circle: {}", circle.join(" → ")));
            }
        }
        if self.broken.class_names.contains(&index) || self.broken.class_parents.contains(&index) {
            return;
        }
        if let Some((_, built_in)) = model.chain(&class.name) {
            self.attributes(owner, &class.attributes, built_in);
        }
    }

    /// What to say about a class that doesn't exist, which leaves what uses it unchecked.
    fn unknown_class(&self, name: &str) -> String {
        let known = BUILT_IN_CLASSES.iter().copied().chain(self.model.classes.iter().map(|class| class.name.as_str()));
        let guess = crate::suggest::closest(name, known).map_or(String::new(), |class| format!(" (did you mean `{class}`?)"));
        format!("unknown class `{name}`{guess}: its attributes weren't checked")
    }

    /// The circle a class is part of, starting and ending at it, if its chain comes back to it.
    fn circle(&self, index: usize) -> Option<Vec<String>> {
        let start = &self.model.classes[index].name;
        let mut names = vec![start.clone()];
        let mut at = &self.model.classes[index].parent;
        while names.len() <= self.model.classes.len() {
            names.push(at.clone());
            if at == start {
                return Some(names);
            }
            at = &self.model.class(at)?.parent;
        }
        None
    }

    // ---------- Instances ----------

    fn instance_names(&mut self) {
        let model = self.model;
        for (index, instance) in model.instances.iter().enumerate() {
            if self.broken.names.contains(&index) {
                continue;
            }
            let name = instance.name.as_str();
            self.name_problems(Owner::Instance(index), name);
            match self.by_name.get(name) {
                Some(&first) => self.problem(Owner::Instance(index), "/name", format!("instance {first} already has this name")),
                None => {
                    self.by_name.insert(name, index);
                }
            }
        }
    }

    /// Each stream is one instance's own, so no two draw the same random numbers.
    fn instance_streams(&mut self) {
        let model = self.model;
        let mut seen: HashMap<u64, usize> = HashMap::new();
        for (index, instance) in model.instances.iter().enumerate() {
            let Some(stream) = instance.stream.filter(|_| !self.broken.streams.contains(&index)) else { continue };
            match seen.get(&stream) {
                Some(&first) => self.problem(Owner::Instance(index), "/stream", format!("instance {first} already has stream {stream}: every instance needs its own")),
                None => {
                    seen.insert(stream, index);
                }
            }
        }
    }

    fn instance(&mut self, index: usize) {
        let model = self.model;
        let instance = &model.instances[index];
        let owner = Owner::Instance(index);
        if !self.broken.classes.contains(&index)
            && !BUILT_IN_CLASSES.contains(&instance.class.as_str())
            && model.class(&instance.class).is_none()
        {
            let message = self.unknown_class(&instance.class);
            self.problem(owner, "/class", message);
        }
        if let Some(built_in) = self.built_in(index) {
            self.attributes(owner, &instance.attributes, built_in);
            if built_in == "Buffer" || built_in == "Station" {
                self.starting_mus_fit(index);
            }
        }
    }

    /// A Buffer or Station can't start with more MUs than it holds, wherever the two values come
    /// from.
    fn starting_mus_fit(&mut self, index: usize) {
        let model = self.model;
        let Some(resolved) = model.resolve(index) else { return };
        let (Some(starting), Some(Capacity::Limited(capacity))) = (resolved.attributes.starting_mus, resolved.attributes.capacity) else { return };
        // One that was there but couldn't be read has been reported already, and a capacity of 0 too.
        if starting <= capacity || capacity == 0 || self.unreadable_in_chain(index, Attribute::StartingMus) || self.unreadable_in_chain(index, Attribute::Capacity) {
            return;
        }
        let own = &model.instances[index].attributes;
        let at = if own.starting_mus.is_some() {
            "/attributes/starting_mus"
        } else if own.capacity.is_some() {
            "/attributes/capacity"
        } else {
            ""
        };
        self.problem(Owner::Instance(index), at, format!("it starts with {starting} MUs but holds only {capacity}"));
    }

    /// Whether the instance or a class in its chain has this attribute but it couldn't be read, so
    /// its value is unknown and anything that depends on it goes unchecked.
    fn unreadable_in_chain(&self, index: usize, attribute: Attribute) -> bool {
        let model = self.model;
        let Some((chain, _)) = model.chain(&model.instances[index].class) else { return false };
        self.unreadable(Owner::Instance(index), attribute)
            || chain.iter().any(|class| {
                let class_index = model.classes.iter().position(|c| std::ptr::eq(c, *class)).expect("in the model");
                self.unreadable(Owner::Class(class_index), attribute)
            })
    }

    // ---------- Values, on classes and instances alike ----------

    fn attributes(&mut self, owner: Owner, attributes: &Attributes, built_in: &str) {
        for attribute in Attribute::ALL {
            if attributes.is_set(attribute) && !attribute.applies_to(built_in) {
                let name = attribute.name();
                self.problem(owner, &format!("/attributes/{name}"), format!("a {built_in} has no attribute `{name}`"));
            }
        }
        if built_in == "Buffer" && attributes.capacity == Some(Capacity::Limited(0)) {
            self.problem(owner, "/attributes/capacity", "a capacity must be at least 1, or \"unlimited\"");
        }
        if built_in == "Buffer" && attributes.starting_mus.is_some_and(|count| count > MOST_STARTING_MUS) {
            self.problem(owner, "/attributes/starting_mus", "a Buffer can start with at most 1,000,000 MUs");
        }
        if built_in == "Buffer" && attributes.histogram_bar_width == Some(0) {
            self.problem(owner, "/attributes/histogram_bar_width", "a histogram's bars must each cover at least 1 MU: leave it out for automatic");
        }
        if built_in == "Station" && attributes.capacity == Some(Capacity::Limited(0)) {
            self.problem(owner, "/attributes/capacity", "a Station's capacity must be at least 1");
        }
        if built_in == "Station" && attributes.capacity == Some(Capacity::Unlimited) {
            self.problem(owner, "/attributes/capacity", "a Station's capacity is a whole number from 1: only a Buffer can be unlimited");
        }
        if built_in == "Station" && attributes.capacity.is_some_and(|capacity| capacity.limit() > MOST_STATION_CAPACITY) {
            self.problem(owner, "/attributes/capacity", "a Station's capacity can be at most 10,000");
        }
        if let Some(list) = &attributes.failures {
            if list.len() > 255 {
                self.problem(owner, "/attributes/failures", "an object can have at most 255 failure profiles");
            }
            for (index, failures) in list.iter().enumerate() {
                if failures.name.trim().is_empty() {
                    self.problem(owner, &format!("/attributes/failures/{index}/name"), "a failure profile's name can't be empty");
                } else if list[..index].iter().any(|earlier| earlier.name == failures.name) {
                    self.problem(owner, &format!("/attributes/failures/{index}/name"), format!("there is already a failure profile named \"{}\" here", failures.name));
                }
                let stream = |at: usize, failures: &Failures| failures.stream.unwrap_or(at as u64);
                if let Some(earlier) = list[..index].iter().enumerate().find(|(at, earlier)| stream(*at, earlier) == stream(index, failures)) {
                    self.problem(
                        owner,
                        &format!("/attributes/failures/{index}/stream"),
                        format!("\"{}\" already has stream {}: each failure profile of an object needs its own", earlier.1.name, stream(index, failures)),
                    );
                }
                self.failures(owner, index, failures);
            }
        }
        if let Some(dimensions) = attributes.dimensions {
            self.dimensions(owner, dimensions);
        }
        if let Some(crate::DistributionStrategy::Weighted(weighted)) = &attributes.distribution
            && !crate::WEIGHABLE.contains(&weighted.by)
        {
            self.problem(owner, "/attributes/distribution/weighted/by", "weigh by process_time, interval, starting_mus or capacity");
        }
        for (state, note) in attributes.state_notes.iter().flatten() {
            let at = format!("/attributes/state_notes/{}", state.name());
            if !crate::State::of(built_in).contains(state) {
                self.problem(owner, &at, format!("a {built_in} is never {}", state.never()));
            } else if note.trim().is_empty() {
                self.problem(owner, &at, "a note can't be empty: leave the state out instead");
            }
        }
    }

    fn failures(&mut self, owner: Owner, index: usize, failures: &Failures) {
        let at = format!("/attributes/failures/{index}");
        let at = at.as_str();
        if !(failures.availability > 0.0 && failures.availability < 1.0) {
            self.problem(
                owner,
                &format!("{at}/availability"),
                format!("an availability must be above 0 and below 1, not {}: leave out `failures` for one that never fails", failures.availability),
            );
        }
        let at = format!("{at}/repair_time");
        match failures.repair_time {
            RepairTime::Constant(time) if time == SimTime::ZERO => {
                self.problem(owner, &format!("{at}/constant"), "a repair has to take some time");
            }
            RepairTime::Uniform { min, max } if min > max => {
                self.problem(owner, &format!("{at}/uniform"), "`min` can't be more than `max`");
            }
            RepairTime::Uniform { max, .. } if max == SimTime::ZERO => {
                self.problem(owner, &format!("{at}/uniform"), "a repair has to take some time");
            }
            RepairTime::Normal { mean, .. } if mean == SimTime::ZERO => {
                self.problem(owner, &format!("{at}/normal/mean"), "a repair has to take some time");
            }
            RepairTime::Erlang { mean, .. } if mean == SimTime::ZERO => {
                self.problem(owner, &format!("{at}/erlang/mean"), "a repair has to take some time");
            }
            RepairTime::Erlang { k: 0, .. } => {
                self.problem(owner, &format!("{at}/erlang/k"), "`k` must be at least 1");
            }
            _ => {}
        }
    }

    fn dimensions(&mut self, owner: Owner, dimensions: Dimensions) {
        for (field, value) in [("length", dimensions.length), ("width", dimensions.width), ("height", dimensions.height)] {
            if !(value > 0.0 && value.is_finite()) {
                self.problem(owner, &format!("/attributes/dimensions/{field}"), format!("`{field}` must be above 0"));
            }
        }
    }

    // ---------- Connections ----------

    fn connections(&mut self) {
        let model = self.model;
        let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
        for (index, connection) in model.connections.iter().enumerate() {
            if self.broken.connections.contains(&index) {
                continue;
            }
            let owner = Owner::Connection(index);
            let mut ends = [None, None];
            for (end, field, name) in [(0, "from", &connection.from), (1, "to", &connection.to)] {
                match self.by_name.get(name.as_str()) {
                    Some(&found) => ends[end] = Some(found),
                    None => {
                        let guess = crate::suggest::did_you_mean(name, model.instances.iter().map(|instance| instance.name.as_str()));
                        self.problem(owner, &format!("/{field}"), format!("no instance is called `{name}`{guess}"));
                    }
                }
            }
            if let [Some(from), Some(to)] = ends {
                if from == to {
                    self.problem(owner, "/to", "an object can't be its own successor");
                } else if self.built_in(to) == Some("Source") {
                    self.problem(owner, "/to", format!("`{}` is a Source, which creates MUs and can't take any in", connection.to));
                } else if self.built_in(from) == Some("Drain") {
                    self.problem(owner, "/from", format!("`{}` is a Drain, which takes MUs out of the model, so no connection can leave it", connection.from));
                }
            }
            match seen.get(&(connection.from.as_str(), connection.to.as_str())) {
                Some(&first) => self.problem(owner, "", format!("connection {first} already connects `{}` to `{}`", connection.from, connection.to)),
                None => {
                    seen.insert((&connection.from, &connection.to), index);
                }
            }
        }
    }

    /// Each object's exits, or entrances, run 1 to n with no gaps or repeats, n being how many
    /// connections leave it, or lead into it.
    fn places(&mut self, place: Place) {
        let model = self.model;
        let broken = match place {
            Place::Exit => &self.broken.exits,
            Place::Entrance => &self.broken.entrances,
        };
        let mut by_object: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, connection) in model.connections.iter().enumerate() {
            if self.broken.connections.contains(&index) {
                continue;
            }
            let object = if place == Place::Exit { &connection.from } else { &connection.to };
            by_object.entry(object).or_default().push(index);
        }
        let mut problems = Vec::new();
        for (object, connections) in by_object {
            let count = connections.len() as u64;
            let mut taken: HashMap<u64, usize> = HashMap::new();
            for index in connections {
                if broken.contains(&index) {
                    continue;
                }
                let connection = &model.connections[index];
                let (number, field, way) = match place {
                    Place::Exit => (connection.exit, "exit", "out of"),
                    Place::Entrance => (connection.entrance, "entrance", "into"),
                };
                if number > count {
                    let (plural, verb) = if count == 1 { ("", "leads") } else { ("s", "lead") };
                    let range = if count == 1 { "1".to_string() } else { format!("1 to {count}") };
                    problems.push((
                        index,
                        format!("/{field}"),
                        format!("{count} connection{plural} {verb} {way} `{object}`, so its {field}s are numbered {range}, not {number}"),
                    ));
                } else if let Some(&first) = taken.get(&number) {
                    problems.push((index, format!("/{field}"), format!("connection {first} {way} `{object}` already has {field} {number}")));
                } else {
                    taken.insert(number, index);
                }
            }
        }
        for (index, at, message) in problems {
            self.problem(Owner::Connection(index), &at, message);
        }
    }

    /// A Source with an interval of 0 creates the next MU as soon as one leaves. If it can reach a
    /// Drain or an unlimited Buffer through objects that take no time, nothing ever fills, so it
    /// would create MUs forever without time passing. A Station takes no time with a process time
    /// of 0, and a Buffer with a capacity when its dwelling time is 0.
    fn endless_sources(&mut self) {
        let model = self.model;
        for index in 0..model.instances.len() {
            if self.built_in(index) != Some("Source") {
                continue;
            }
            let Some(resolved) = model.resolve(index) else { continue };
            if resolved.attributes.interval != Some(SimTime::ZERO) || self.unreadable_in_chain(index, Attribute::Interval) {
                continue;
            }
            if let Some(path) = self.zero_time_path_to_a_drain(index) {
                let through = if path.len() > 1 {
                    let between: Vec<&str> = path[..path.len() - 1].iter().map(|&i| model.instances[i].name.as_str()).collect();
                    format!(" through zero-time objects ({})", between.join(", "))
                } else {
                    String::new()
                };
                let end = *path.last().expect("a path ends at a Drain or an unlimited Buffer");
                let name = &model.instances[end].name;
                let reaches = if self.built_in(end) == Some("Drain") { format!("Drain `{name}`") } else { format!("`{name}`, a Buffer with no limit,") };
                let at = if model.instances[index].attributes.interval.is_some() { "/attributes/interval" } else { "" };
                self.problem(
                    Owner::Instance(index),
                    at,
                    format!(
                        "with an interval of 0 this Source reaches {reaches}{through}, so nothing ever blocks it \
                         and it would create MUs forever without time passing"
                    ),
                );
            }
        }
    }

    /// The objects from a Source's successor to a Drain or an unlimited Buffer, if every one
    /// before it takes no time.
    fn zero_time_path_to_a_drain(&self, source: usize) -> Option<Vec<usize>> {
        let mut came_from: HashMap<usize, usize> = HashMap::new();
        let mut queue = VecDeque::from([source]);
        while let Some(at) = queue.pop_front() {
            for successor in self.model.successors(&self.model.instances[at].name) {
                let Some(&next) = self.by_name.get(successor) else { continue };
                if next == source || came_from.contains_key(&next) {
                    continue;
                }
                let resolved = self.model.resolve(next);
                let unlimited = self.built_in(next) == Some("Buffer") && resolved.as_ref().is_some_and(|resolved| resolved.attributes.capacity == Some(Capacity::Unlimited));
                let passes = match self.built_in(next) {
                    Some("Drain" | "Buffer") if unlimited || self.built_in(next) == Some("Drain") => {
                        came_from.insert(next, at);
                        let mut path = vec![next];
                        while let Some(&before) = came_from.get(path.last().expect("not empty")) {
                            if before == source {
                                break;
                            }
                            path.push(before);
                        }
                        path.reverse();
                        return Some(path);
                    }
                    Some("Station") => resolved.is_some_and(|resolved| resolved.attributes.process_time == Some(SimTime::ZERO)),
                    Some("Buffer") => resolved.is_some_and(|resolved| resolved.attributes.dwelling_time == Some(SimTime::ZERO)),
                    _ => false,
                };
                if passes {
                    came_from.insert(next, at);
                    queue.push_back(next);
                }
            }
        }
        None
    }
}
