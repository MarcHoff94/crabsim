//! Building a valid model into the engine's `ModelDescription`, with every instance's values
//! worked out from its chain of classes, so a run never sees classes or files.

use crate::{Attribute, Failures, Model, Origin, RepairTime};
use crabsim::distribution::{DistributionRule, WeighBy, WeightedRule};
use crabsim::random_numbers::Distribution;
use crabsim::{FailureProfile, ModelDescription, SimTime};
use std::collections::HashMap;

pub(crate) fn build(model: &Model) -> ModelDescription {
    let mut description = ModelDescription::new();
    // Starting MUs are numbered in file order, from 1. The run hands out later ids above them.
    let mut next_mu = 1;
    for (index, instance) in model.instances.iter().enumerate() {
        let resolved = model.resolve(index).expect("checked: every instance's chain of classes is sound");
        let attributes = &resolved.attributes;
        let id = match resolved.built_in {
            "Source" => description.add_source(attributes.interval.unwrap_or(SimTime::ZERO)),
            "Station" => {
                let process_time = attributes.process_time.unwrap_or(SimTime::ZERO);
                let failures = failure_profile(attributes.failures.as_ref());
                if attributes.starting_mus.unwrap_or(0) == 1 {
                    next_mu += 1;
                    description.add_station_holding(process_time, failures, next_mu - 1)
                } else {
                    description.add_station(process_time, failures)
                }
            }
            "Buffer" => {
                let capacity = attributes.capacity.expect("checked: a Buffer has a capacity");
                description.add_buffer(usize::try_from(capacity).unwrap_or(usize::MAX))
            }
            "Drain" => description.add_drain(),
            other => unreachable!("checked: `{other}` is a built-in class"),
        };
        description.set_name(id, instance.name.clone());
        description.set_stream(id, instance.stream.unwrap_or(index as u64));
        if let Some(position) = instance.position {
            description.set_position(id, position);
        }
        // A built-in default size is what the description gives an object without one anyway.
        if let Some(dimensions) = attributes.dimensions.filter(|_| resolved.origin(Attribute::Dimensions) != &Origin::BuiltIn) {
            description.set_dimensions(id, dimensions);
        }
    }
    let ids: HashMap<&str, u32> = model.instances.iter().enumerate().map(|(id, instance)| (instance.name.as_str(), id as u32)).collect();
    // Each Source and Station's distribution strategy, its Drains by id. A name that isn't an
    // instance is left out, and the engine ignores ids that aren't Drains.
    for (index, _) in model.instances.iter().enumerate() {
        let resolved = model.resolve(index).expect("checked: every instance's chain of classes is sound");
        if let Some(crate::DistributionStrategy::Weighted(weighted)) = &resolved.attributes.distribution {
            let by = match weighted.by {
                Attribute::Interval => WeighBy::Interval,
                Attribute::StartingMus => WeighBy::StartingMus,
                Attribute::Capacity => WeighBy::Capacity,
                _ => WeighBy::ProcessTime,
            };
            let drains = weighted.drains.as_ref().map(|names| names.iter().filter_map(|name| ids.get(name.as_str()).copied()).collect());
            let rule = WeightedRule { by, order: weighted.order, rank: weighted.rank, drains };
            description.set_distribution(index as u32, DistributionRule::Weighted(rule));
        }
    }
    // Each object offers MUs to its successors in the order of its exits.
    for (from, instance) in model.instances.iter().enumerate() {
        for successor in model.successors(&instance.name) {
            description.connect(from as u32, ids[successor]);
        }
    }
    description
}

fn failure_profile(failures: Option<&Failures>) -> FailureProfile {
    let millis = |time: SimTime| time.as_millis() as f64;
    match failures {
        // Availability 1 never fails, whatever the repair time.
        None => FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0),
        Some(failures) => {
            let repair_time = match failures.repair_time {
                RepairTime::Constant(time) => Distribution::CONSTANT { value: millis(time) },
                RepairTime::Uniform { min, max } => Distribution::UNIFORM { min: millis(min), max: millis(max) },
                RepairTime::Normal { mean, std } => Distribution::NORMAL { mean: millis(mean), std: millis(std) },
                RepairTime::Erlang { mean, k } => Distribution::ERLANG { mean: millis(mean), k: k as usize },
            };
            FailureProfile::new(failures.availability as f32, repair_time, 0)
        }
    }
}
