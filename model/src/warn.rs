//! What a valid model can run with but is probably a mistake: warnings, which `validate_model`
//! gives beside problems. A warning never stops a run, and nothing is worked out on each edit:
//! they're found only when asked for.

use crate::{DistributionStrategy, Model, Origin, Problem, Attribute};
use std::collections::{HashMap, VecDeque};

impl Model {
    /// Every warning about a model without problems, in file order: an instance no Source
    /// reaches, one with no route to a Drain, a Drain list naming something that isn't a Drain,
    /// a weighted object whose weight is nothing on every route onward, so its successors all
    /// rank the same, and an unlimited Buffer that can get MUs faster than it can pass them on.
    /// For a model `load` gave, so without problems: checking again would cost as much as the
    /// warnings themselves.
    pub fn warnings(&self) -> Vec<Problem> {
        let count = self.instances.len();
        let index: HashMap<&str, usize> = self.instances.iter().enumerate().map(|(i, instance)| (instance.name.as_str(), i)).collect();
        let mut successors = vec![Vec::new(); count];
        let mut predecessors = vec![Vec::new(); count];
        for connection in &self.connections {
            let (from, to) = (index[connection.from.as_str()], index[connection.to.as_str()]);
            successors[from].push(to);
            predecessors[to].push(from);
        }
        let built_in: Vec<&str> = (0..count).map(|i| self.resolve(i).expect("checked").built_in).collect();
        let reached = |starts: Vec<usize>, next: &Vec<Vec<usize>>| {
            let mut seen = vec![false; count];
            let mut queue: VecDeque<usize> = starts.into_iter().collect();
            for &start in &queue {
                seen[start] = true;
            }
            while let Some(at) = queue.pop_front() {
                for &to in &next[at] {
                    if !seen[to] {
                        seen[to] = true;
                        queue.push_back(to);
                    }
                }
            }
            seen
        };
        let from_sources = reached((0..count).filter(|&i| built_in[i] == "Source").collect(), &successors);
        let to_drains = reached((0..count).filter(|&i| built_in[i] == "Drain").collect(), &predecessors);

        let mut warnings = Vec::new();
        let mut warn = |pointer: String, name: &str, message: String| warnings.push(Problem { pointer, name: Some(name.to_string()), message });
        let description = crate::build::build(self);
        for (i, instance) in self.instances.iter().enumerate() {
            let name = instance.name.as_str();
            if !from_sources[i] && built_in[i] != "Source" {
                warn(format!("/instances/{i}"), name, "no Source reaches it, so it gets no MU but those it starts with".to_string());
            }
            if !to_drains[i] && built_in[i] != "Drain" {
                warn(format!("/instances/{i}"), name, "it has no route to a Drain, so an MU that reaches it can never leave the model".to_string());
            }
            let resolved = self.resolve(i).expect("checked");
            if built_in[i] == "Source" {
                self.mus_that_dont_fit(i, &successors, &built_in, &mut warn);
            }
            if built_in[i] == "Buffer" && resolved.attributes.capacity == Some(crate::Capacity::Unlimited) {
                // MUs per hour in and out, where the objects on both sides say: a Source's
                // interval and a Station's process time. Anything else, and nothing is known.
                let rate = |others: &Vec<usize>| -> Option<f64> {
                    others.iter().try_fold(0.0, |sum, &other| {
                        let has = self.resolve(other).expect("checked").attributes;
                        let (time, at_once) = match built_in[other] {
                            "Source" => (has.interval, 1),
                            // A Station with capacity works on that many at once.
                            "Station" => (has.process_time, has.capacity.map_or(1, crate::Capacity::limit)),
                            _ => (None, 1),
                        };
                        Some(sum + at_once as f64 * 3_600_000.0 / time?.as_millis() as f64)
                    })
                };
                if let (Some(coming), Some(going)) = (rate(&predecessors[i]), rate(&successors[i]))
                    && !successors[i].is_empty()
                    && coming > going
                {
                    let per_hour = |rate: f64| if rate.is_finite() { format!("{rate:.0}") } else { "any number of".to_string() };
                    let at = if instance.attributes.capacity.is_some() { format!("/instances/{i}/attributes/capacity") } else { format!("/instances/{i}") };
                    warn(
                        at,
                        name,
                        format!(
                            "it has no limit and can get {} MUs an hour but pass on only {}, so it would grow for as long as the run lasts: give it a capacity, or make sure that is meant",
                            per_hour(coming),
                            per_hour(going)
                        ),
                    );
                }
            }
            let Some(DistributionStrategy::Weighted(weighted)) = &resolved.attributes.distribution else { continue };
            let at = match resolved.origin(Attribute::Distribution) {
                Origin::Instance => format!("/instances/{i}/attributes/distribution/weighted"),
                _ => format!("/instances/{i}"),
            };
            for (place, drain) in weighted.drains.iter().flatten().enumerate() {
                if !index.get(drain.as_str()).is_some_and(|&d| built_in[d] == "Drain") {
                    warn(
                        format!("{at}/drains/{place}"),
                        name,
                        format!("`{drain}` isn't a Drain in the model, so its routes don't count it"),
                    );
                }
            }
            let ranking = description.ranking(i as u32).expect("weighted");
            if successors[i].len() > 1 && ranking.totals.iter().all(|total| total.unwrap_or(0) == 0) {
                warn(
                    at,
                    name,
                    format!(
                        "{} weighs nothing on every route onward, so its successors all rank the same and take turns, as round robin does",
                        weighted.by.name()
                    ),
                );
            }
        }
        warnings
    }

    /// Warns about each conveyor the Source at `source` reaches through objects without geometry
    /// whose width or length its MUs don't fit: the conveyor refuses them, and they wait for good.
    fn mus_that_dont_fit(&self, source: usize, successors: &[Vec<usize>], built_in: &[&str], warn: &mut impl FnMut(String, &str, String)) {
        let Some(kind) = self.mu_kind_of(source) else { return };
        let mut seen = vec![false; successors.len()];
        let mut queue: VecDeque<usize> = successors[source].iter().copied().collect();
        while let Some(at) = queue.pop_front() {
            if std::mem::replace(&mut seen[at], true) {
                continue;
            }
            if built_in[at] != "Conveyor" {
                queue.extend(successors[at].iter().copied());
                continue;
            }
            let conveyor = self.resolve(at).expect("checked").attributes;
            let (width, length) = (conveyor.width.unwrap_or(0.6), crate::path_length(conveyor.path.as_deref().unwrap_or_default()));
            let too = if kind.width > width {
                format!("{} m wide, and conveyor `{}` only {width} m", kind.width, self.instances[at].name)
            } else if kind.length > length {
                format!("{} m long, and conveyor `{}` only {length} m", kind.length, self.instances[at].name)
            } else {
                continue;
            };
            let class = self.resolve(source).expect("checked").attributes.mu_class.unwrap_or_default();
            warn(
                format!("/instances/{source}"),
                &self.instances[source].name,
                format!("its MUs, of MU class `{class}`, are {too}, which refuses them, so they wait there for good"),
            );
        }
    }
}
