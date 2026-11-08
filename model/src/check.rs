//! What can be wrong with a model whose shape is fine: names, classes and their chains, values
//! out of range, values set nowhere, connections, and a Source that would create MUs forever.

use crate::read::Broken;
use crate::{Attribute, Attributes, Failures, Model, Problem, RepairTime, BUILT_IN_CLASSES};
use crabsim::{Dimensions, SimTime};
use std::collections::{HashMap, VecDeque};

pub(crate) fn check(model: &Model, broken: &Broken) -> Vec<Problem> {
    let mut checker = Checker { model, broken, problems: Vec::new(), by_name: HashMap::new() };
    checker.class_names();
    for index in 0..model.classes.len() {
        checker.class(index);
    }
    checker.instance_names();
    for index in 0..model.instances.len() {
        checker.instance(index);
    }
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
        };
        self.problems.push(Problem { pointer, name, message: message.into() });
    }

    /// Whether an attribute was there but couldn't be read.
    fn unreadable(&self, owner: Owner, attribute: Attribute) -> bool {
        match owner {
            Owner::Class(index) => self.broken.class_attributes.contains(&(index, attribute.name())),
            Owner::Instance(index) => self.broken.attributes.contains(&(index, attribute.name())),
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
            if BUILT_IN_CLASSES.contains(&class.name.as_str()) {
                self.problem(Owner::Class(index), "/name", format!("`{}` is a built-in class", class.name));
            } else if let Some(&first) = seen.get(class.name.as_str()) {
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
            if class.parent == class.name {
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
            self.required(index, built_in);
        }
        if !self.broken.successors.contains(&index) {
            self.successors(index);
        }
    }

    /// Values that must be set somewhere in the chain, and aren't.
    fn required(&mut self, index: usize, built_in: &str) {
        let attribute = match built_in {
            "Source" => Attribute::Interval,
            "Buffer" => Attribute::Capacity,
            _ => return,
        };
        let model = self.model;
        let Some(resolved) = model.resolve(index) else { return };
        if resolved.attributes.is_set(attribute) {
            return;
        }
        // One that was there but couldn't be read has been reported already.
        let (chain, _) = model.chain(&model.instances[index].class).expect("resolved, so the chain is sound");
        let unreadable_in_chain = self.unreadable(Owner::Instance(index), attribute)
            || chain.iter().any(|class| {
                let class_index = model.classes.iter().position(|c| std::ptr::eq(c, *class)).expect("in the model");
                self.unreadable(Owner::Class(class_index), attribute)
            });
        if unreadable_in_chain {
            return;
        }
        let what = match attribute {
            Attribute::Interval => "an `interval`: the time between creating an MU and offering it on",
            _ => "a `capacity`: how many MUs it holds",
        };
        self.problem(Owner::Instance(index), "", format!("a {built_in} needs {what}. Set it on the instance or one of its classes"));
    }

    // ---------- Values, on classes and instances alike ----------

    fn attributes(&mut self, owner: Owner, attributes: &Attributes, built_in: &str) {
        for attribute in Attribute::ALL {
            if attributes.is_set(attribute) && !attribute.applies_to(built_in) {
                let name = attribute.name();
                self.problem(owner, &format!("/attributes/{name}"), format!("a {built_in} has no attribute `{name}`"));
            }
        }
        if built_in == "Buffer" && attributes.capacity == Some(0) {
            self.problem(owner, "/attributes/capacity", "a capacity must be at least 1");
        }
        if built_in == "Station" && attributes.starting_mus.is_some_and(|count| count > 1) {
            self.problem(owner, "/attributes/starting_mus", "a Station can start with 0 or 1 MUs, until Stations have a capacity");
        }
        if let (Some(failures), "Station") = (&attributes.failures, built_in) {
            self.failures(owner, failures);
        }
        if let Some(dimensions) = attributes.dimensions {
            self.dimensions(owner, dimensions);
        }
    }

    fn failures(&mut self, owner: Owner, failures: &Failures) {
        let at = "/attributes/failures";
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

    fn successors(&mut self, index: usize) {
        let model = self.model;
        let instance = &model.instances[index];
        let owner = Owner::Instance(index);
        if self.built_in(index) == Some("Drain") && !instance.successors.is_empty() {
            self.problem(owner, "/successors", "a Drain takes MUs out of the model, so it can't have successors");
            return;
        }
        for (place, successor) in instance.successors.iter().enumerate() {
            let at = format!("/successors/{place}");
            if instance.successors[..place].contains(successor) {
                self.problem(owner, &at, format!("`{successor}` is listed twice"));
                continue;
            }
            match self.by_name.get(successor.as_str()) {
                None => {
                    let guess = crate::suggest::did_you_mean(successor, model.instances.iter().map(|instance| instance.name.as_str()));
                    self.problem(owner, &at, format!("no instance is called `{successor}`{guess}"))
                }
                Some(&target) if target == index => self.problem(owner, &at, "an object can't be its own successor"),
                Some(&target) if self.built_in(target) == Some("Source") => {
                    self.problem(owner, &at, format!("`{successor}` is a Source, which creates MUs and can't take any in"))
                }
                Some(_) => {}
            }
        }
    }

    /// A Source with an interval of 0 creates the next MU as soon as one leaves. If it can reach a
    /// Drain through objects that take no time, nothing ever fills, so it would create MUs forever
    /// without time passing. Buffers count as taking no time until they have dwelling times and
    /// can fill up.
    fn endless_sources(&mut self) {
        let model = self.model;
        for index in 0..model.instances.len() {
            if self.built_in(index) != Some("Source") {
                continue;
            }
            let Some(resolved) = model.resolve(index) else { continue };
            if resolved.attributes.interval != Some(SimTime::ZERO) {
                continue;
            }
            if let Some(path) = self.zero_time_path_to_a_drain(index) {
                let through = if path.len() > 1 {
                    let between: Vec<&str> = path[..path.len() - 1].iter().map(|&i| model.instances[i].name.as_str()).collect();
                    format!(" through zero-time objects ({})", between.join(", "))
                } else {
                    String::new()
                };
                let drain = &model.instances[*path.last().expect("a path ends at a Drain")].name;
                let at = if model.instances[index].attributes.interval.is_some() { "/attributes/interval" } else { "" };
                self.problem(
                    Owner::Instance(index),
                    at,
                    format!(
                        "with an interval of 0 this Source reaches Drain `{drain}`{through}, so nothing ever blocks it \
                         and it would create MUs forever without time passing"
                    ),
                );
            }
        }
    }

    /// The objects from a Source's successor to a Drain, if every one before the Drain takes no time.
    fn zero_time_path_to_a_drain(&self, source: usize) -> Option<Vec<usize>> {
        let mut came_from: HashMap<usize, usize> = HashMap::new();
        let mut queue = VecDeque::from([source]);
        while let Some(at) = queue.pop_front() {
            for successor in &self.model.instances[at].successors {
                let Some(&next) = self.by_name.get(successor.as_str()) else { continue };
                if next == source || came_from.contains_key(&next) {
                    continue;
                }
                let passes = match self.built_in(next) {
                    Some("Drain") => {
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
                    Some("Station") => {
                        self.model.resolve(next).is_some_and(|resolved| resolved.attributes.process_time == Some(SimTime::ZERO))
                    }
                    Some("Buffer") => true,
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
