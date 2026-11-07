//! What can be wrong with a model whose shape is fine: names, classes, values out of range,
//! values set nowhere, connections, and a Source that would create MUs forever.

use crate::read::Broken;
use crate::{Failures, Model, Problem, RepairTime, BUILT_IN_CLASSES};
use crabsim::{Dimensions, SimTime};
use std::collections::{HashMap, VecDeque};

/// The attributes each built-in class has.
fn has_attribute(class: &str, attribute: &str) -> bool {
    matches!(
        (class, attribute),
        (_, "dimensions" | "description")
            | ("Source", "interval")
            | ("Station", "process_time" | "failures" | "starting_mus")
            | ("Buffer", "capacity")
    )
}

pub(crate) fn check(model: &Model, broken: &Broken) -> Vec<Problem> {
    let mut checker = Checker { model, broken, problems: Vec::new(), by_name: HashMap::new() };
    checker.names();
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
    /// Each name and the first instance that has it.
    by_name: HashMap<&'m str, usize>,
}

impl<'m> Checker<'m> {
    fn problem(&mut self, index: usize, at: &str, message: impl Into<String>) {
        let name = (!self.broken.names.contains(&index)).then(|| self.model.instances[index].name.clone());
        self.problems.push(Problem { pointer: format!("/instances/{index}{at}"), name, message: message.into() });
    }

    fn class(&self, index: usize) -> Option<&'m str> {
        let class = self.model.instances[index].class.as_str();
        (!self.broken.classes.contains(&index) && BUILT_IN_CLASSES.contains(&class)).then_some(class)
    }

    fn names(&mut self) {
        let model = self.model;
        for (index, instance) in model.instances.iter().enumerate() {
            if self.broken.names.contains(&index) {
                continue;
            }
            let name = instance.name.as_str();
            if name.is_empty() {
                self.problem(index, "/name", "a name can't be empty");
            } else if name.trim() != name {
                self.problem(index, "/name", "a name can't start or end with spaces");
            }
            match self.by_name.get(name) {
                Some(&first) => self.problem(index, "/name", format!("instance {first} already has this name")),
                None => {
                    self.by_name.insert(name, index);
                }
            }
        }
    }

    fn instance(&mut self, index: usize) {
        let model = self.model;
        let instance = &model.instances[index];
        if !self.broken.classes.contains(&index) && !BUILT_IN_CLASSES.contains(&instance.class.as_str()) {
            self.problem(
                index,
                "/class",
                format!("unknown class `{}`: until derived classes exist, use Source, Station, Buffer or Drain", instance.class),
            );
        }
        if let Some(class) = self.class(index) {
            self.attributes(index, class);
        }
        if !self.broken.successors.contains(&index) {
            self.successors(index);
        }
    }

    fn attributes(&mut self, index: usize, class: &str) {
        let model = self.model;
        let attributes = &model.instances[index].attributes;
        let set = [
            ("interval", attributes.interval.is_some()),
            ("process_time", attributes.process_time.is_some()),
            ("failures", attributes.failures.is_some()),
            ("starting_mus", attributes.starting_mus.is_some()),
            ("capacity", attributes.capacity.is_some()),
            ("dimensions", attributes.dimensions.is_some()),
            ("description", attributes.description.is_some()),
        ];
        for (attribute, is_set) in set {
            if is_set && !has_attribute(class, attribute) {
                self.problem(index, &format!("/attributes/{attribute}"), format!("a {class} has no attribute `{attribute}`"));
            }
        }
        let broken = self.broken;
        let missing = |attribute| !broken.attributes.contains(&(index, attribute));
        if class == "Source" && attributes.interval.is_none() && missing("interval") {
            self.problem(index, "", "a Source needs an `interval`: the time between creating an MU and offering it on");
        }
        if class == "Buffer" && attributes.capacity.is_none() && missing("capacity") {
            self.problem(index, "", "a Buffer needs a `capacity`: how many MUs it holds");
        }
        if class == "Buffer" && attributes.capacity == Some(0) {
            self.problem(index, "/attributes/capacity", "a capacity must be at least 1");
        }
        if class == "Station" && attributes.starting_mus.is_some_and(|count| count > 1) {
            self.problem(index, "/attributes/starting_mus", "a Station can start with 0 or 1 MUs, until Stations have a capacity");
        }
        if let (Some(failures), "Station") = (&attributes.failures, class) {
            self.failures(index, failures);
        }
        if let Some(dimensions) = attributes.dimensions {
            self.dimensions(index, dimensions);
        }
    }

    fn failures(&mut self, index: usize, failures: &Failures) {
        let at = "/attributes/failures";
        if !(failures.availability > 0.0 && failures.availability < 1.0) {
            self.problem(
                index,
                &format!("{at}/availability"),
                format!("an availability must be above 0 and below 1, not {}: leave out `failures` for one that never fails", failures.availability),
            );
        }
        let at = format!("{at}/repair_time");
        match failures.repair_time {
            RepairTime::Constant(time) if time == SimTime::ZERO => {
                self.problem(index, &format!("{at}/constant"), "a repair has to take some time");
            }
            RepairTime::Uniform { min, max } if min > max => {
                self.problem(index, &format!("{at}/uniform"), "`min` can't be more than `max`");
            }
            RepairTime::Uniform { max, .. } if max == SimTime::ZERO => {
                self.problem(index, &format!("{at}/uniform"), "a repair has to take some time");
            }
            RepairTime::Normal { mean, .. } | RepairTime::Erlang { mean, .. } if mean == SimTime::ZERO => {
                let kind = if matches!(failures.repair_time, RepairTime::Normal { .. }) { "normal" } else { "erlang" };
                self.problem(index, &format!("{at}/{kind}/mean"), "a repair has to take some time");
            }
            RepairTime::Erlang { k: 0, .. } => {
                self.problem(index, &format!("{at}/erlang/k"), "`k` must be at least 1");
            }
            _ => {}
        }
    }

    fn dimensions(&mut self, index: usize, dimensions: Dimensions) {
        for (field, value) in [("length", dimensions.length), ("width", dimensions.width), ("height", dimensions.height)] {
            if !(value > 0.0 && value.is_finite()) {
                self.problem(index, &format!("/attributes/dimensions/{field}"), format!("`{field}` must be above 0"));
            }
        }
    }

    fn successors(&mut self, index: usize) {
        let model = self.model;
        let instance = &model.instances[index];
        if self.class(index) == Some("Drain") && !instance.successors.is_empty() {
            self.problem(index, "/successors", "a Drain takes MUs out of the model, so it can't have successors");
            return;
        }
        for (place, successor) in instance.successors.iter().enumerate() {
            let at = format!("/successors/{place}");
            if instance.successors[..place].contains(successor) {
                self.problem(index, &at, format!("`{successor}` is listed twice"));
                continue;
            }
            match self.by_name.get(successor.as_str()) {
                None => self.problem(index, &at, format!("no instance is called `{successor}`")),
                Some(&target) if target == index => self.problem(index, &at, "an object can't be its own successor"),
                Some(&target) if self.class(target) == Some("Source") => {
                    self.problem(index, &at, format!("`{successor}` is a Source, which creates MUs and can't take any in"))
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
        for index in 0..self.model.instances.len() {
            let model = self.model;
            let source = &model.instances[index];
            if self.class(index) != Some("Source") || source.attributes.interval != Some(SimTime::ZERO) {
                continue;
            }
            if let Some(path) = self.zero_time_path_to_a_drain(index) {
                let through = if path.len() > 1 {
                    let between: Vec<String> = path[..path.len() - 1].iter().map(|&i| self.model.instances[i].name.clone()).collect();
                    format!(" through zero-time objects ({})", between.join(", "))
                } else {
                    String::new()
                };
                let drain = &self.model.instances[*path.last().expect("a path ends at a Drain")].name;
                self.problem(
                    index,
                    "/attributes/interval",
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
                let passes = match self.class(next) {
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
                    Some("Station") => self.model.instances[next].attributes.process_time.unwrap_or(SimTime::ZERO) == SimTime::ZERO,
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
