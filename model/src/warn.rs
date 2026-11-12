//! What a valid model can run with but is probably a mistake: warnings, which `validate_model`
//! gives beside problems. A warning never stops a run, and nothing is worked out on each edit:
//! they're found only when asked for.

use crate::{DistributionStrategy, Model, Origin, Problem, Attribute};
use std::collections::{HashMap, VecDeque};

impl Model {
    /// Every warning about a model without problems, in file order: an instance no Source
    /// reaches, one with no route to a Drain, a Drain list naming something that isn't a Drain,
    /// and a weighted object whose weight is nothing on every route onward, so its successors
    /// all rank the same. For a model `load` gave, so without problems: checking again would
    /// cost as much as the warnings themselves.
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
}
