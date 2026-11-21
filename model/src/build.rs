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
                let failures = failure_profile(attributes.failures.as_deref().and_then(<[_]>::first));
                let starting = attributes.starting_mus.unwrap_or(0) as u32;
                let capacity = attributes.capacity.map_or(1, crabsim::Capacity::limit);
                if starting == 1 && capacity == 1 {
                    next_mu += 1;
                    description.add_station_holding(process_time, failures, next_mu - 1)
                } else {
                    let station = description.add_station(process_time, failures);
                    if capacity > 1 {
                        description.set_station_capacity(station, capacity, attributes.start.unwrap_or_default());
                        description.set_starting_mus(station, &(next_mu..next_mu + starting).collect::<Vec<u32>>());
                        next_mu += starting;
                    }
                    station
                }
            }
            "Buffer" => {
                let buffer = description.add_buffer(attributes.capacity.unwrap_or(crabsim::Capacity::Unlimited));
                description.set_dwelling_time(buffer, attributes.dwelling_time.unwrap_or(SimTime::ZERO));
                description.set_exit_strategy(buffer, attributes.exit_strategy.unwrap_or_default());
                description.set_histogram_bar_width(buffer, attributes.histogram_bar_width);
                description.set_stop_freezes_dwelling(buffer, attributes.stop_freezes_dwelling.unwrap_or(false));
                let starting = attributes.starting_mus.unwrap_or(0) as u32;
                description.set_starting_mus(buffer, &(next_mu..next_mu + starting).collect::<Vec<u32>>());
                next_mu += starting;
                buffer
            }
            "Drain" => description.add_drain(),
            other => unreachable!("checked: `{other}` is a built-in class"),
        };
        // A Station gets a placeholder profile when it is added, which this list replaces.
        if let Some(list) = attributes.failures.as_ref().filter(|list| !list.is_empty()) {
            description.set_failure_profiles(id, list.iter().map(|failures| failure_profile(Some(failures))).collect());
        }
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
            let profile = FailureProfile::new(failures.availability as f32, repair_time, 0).named(failures.name.clone());
            match failures.stream {
                Some(stream) => profile.with_stream(stream),
                None => profile,
            }
        }
    }
}
