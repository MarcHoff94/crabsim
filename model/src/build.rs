//! Building a valid model into the engine's `ModelDescription`, with every instance's values
//! worked out from its chain of classes, so a run never sees classes or files.

use crate::{Attribute, Failures, Model, Origin, RepairTime};
use crabsim::distribution::{DistributionRule, WeighBy, WeightedRule};
use crabsim::random_numbers::Distribution;
use crabsim::{DestinationId, FailureProfile, ModelDescription, PlanStep, SimTime};
use std::collections::HashMap;

pub(crate) fn build(model: &Model) -> ModelDescription {
    build_naming_mu_kinds(model).0
}

/// The description, and the MU class each MU kind stands for, by its id, for code to read.
pub(crate) fn build_naming_mu_kinds(model: &Model) -> (ModelDescription, Vec<String>) {
    let mut mu_classes = vec![crate::BUILT_IN_MU_CLASS.to_string()];
    let mut description = ModelDescription::new();
    // The model's own values for the built-in MU are those of starting MUs and default Sources.
    let built_in_mu = model.resolve_class(crate::BUILT_IN_MU_CLASS).expect("the built-in MU class always resolves");
    description.set_built_in_mu_kind(crate::mu_kind(&built_in_mu.attributes));
    // Starting MUs are numbered in file order, from 1. The run hands out later ids above them.
    let mut next_mu = 1;
    for (index, instance) in model.instances.iter().enumerate() {
        let resolved = model.resolve(index).expect("checked: every instance's chain of classes is sound");
        let attributes = &resolved.attributes;
        let id = match resolved.built_in {
            // Its MU kind comes once every instance is there, since a work plan names them.
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
            "CrossConveyor" => {
                let cross = description.add_cross_conveyor(attributes.length.unwrap_or(0.6), attributes.width.unwrap_or(0.6), attributes.speed.unwrap_or(0.5));
                description.set_switch_time(cross, attributes.switch_time.unwrap_or(SimTime::from_secs(2)));
                description.set_release_time(cross, attributes.release_time.unwrap_or(SimTime::ZERO));
                cross
            }
            "Conveyor" => {
                let path = attributes.path.as_deref().unwrap_or(&crate::DEFAULT_PATH);
                let conveyor = description.add_conveyor(crate::path_length(path), attributes.width.unwrap_or(0.6), attributes.speed.unwrap_or(0.5));
                description.set_conveyor_gap(conveyor, attributes.gap.unwrap_or(0.0));
                description.set_conveyor_accumulating(conveyor, attributes.accumulating.unwrap_or(true));
                conveyor
            }
            other => unreachable!("checked: `{other}` is a built-in class"),
        };
        // A Station gets a placeholder profile when it is added, which this list replaces.
        if let Some(list) = attributes.failures.as_ref().filter(|list| !list.is_empty()) {
            description.set_failure_profiles(id, list.iter().map(|failures| failure_profile(Some(failures))).collect());
        }
        description.set_name(id, instance.name.clone());
        description.set_stream(id, instance.stream.unwrap_or(index as u64));
        if let Some(position) = instance.position {
            description.set_position(id, model.drawn_position(index, position));
        }
        // A built-in default size is what the description gives an object without one anyway. A
        // view draws a conveyor along its path (`Model::drawing`), so its size there is its path's
        // bounds, unturned, for what needs a centre and a footprint, such as laying out.
        if let Some(mut dimensions) = attributes.dimensions.filter(|_| resolved.origin(Attribute::Dimensions) != &Origin::BuiltIn || resolved.built_in == "Conveyor") {
            if let Some(bounds) = crate::conveyor_bounds(&resolved, crabsim::Position { x: 0.0, y: 0.0, z: 0.0 }) {
                (dimensions.length, dimensions.width, dimensions.rotation_degrees) = (bounds.max_x - bounds.min_x, bounds.max_y - bounds.min_y, 0.0);
            }
            description.set_dimensions(id, dimensions);
        }
    }
    let ids: HashMap<&str, u32> = model.instances.iter().enumerate().map(|(id, instance)| (instance.name.as_str(), id as u32)).collect();
    // Every name a work plan or a detour gives, as the instances it stands for (ADR 0025). Ids
    // follow file order, so an instance's id is its index.
    let mut destinations: HashMap<&str, DestinationId> = HashMap::new();
    for name in model.destination_names() {
        let members: Vec<u32> = model.destination_members(name).expect("checked: a destination is an instance or a class").into_iter().map(|index| index as u32).collect();
        destinations.insert(name, description.add_destination(&members));
    }
    let plan = |steps: &[crate::WorkStep]| -> Vec<PlanStep> { steps.iter().map(|step| PlanStep { at: destinations[step.at.as_str()], process_time: step.process_time }).collect() };
    if let Some(steps) = built_in_mu.attributes.work_plan.as_deref() {
        description.set_built_in_work_plan(plan(steps));
    }
    for (index, _) in model.instances.iter().enumerate() {
        let resolved = model.resolve(index).expect("checked: every instance's chain of classes is sound");
        let id = index as u32;
        match resolved.built_in {
            "Source" => {
                let kind = model.mu_kind_of(index).expect("checked: a Source's MU class is an MU class");
                let steps = model.work_plan_of(index).expect("checked: a Source's MU class is an MU class");
                let kind = description.add_mu_kind_with_plan(kind, plan(&steps));
                description.set_mu_kind(id, kind);
                if mu_classes.len() <= kind as usize {
                    mu_classes.resize(kind as usize + 1, String::new());
                }
                if mu_classes[kind as usize].is_empty() {
                    mu_classes[kind as usize] = resolved.attributes.mu_class.clone().unwrap_or_else(|| crate::BUILT_IN_MU_CLASS.to_string());
                }
            }
            "Station" => {
                if let Some(detour) = &resolved.attributes.detour {
                    description.set_detour(id, destinations[detour.as_str()]);
                }
            }
            "CrossConveyor" => {
                for (side, names) in resolved.attributes.exits.iter().flatten() {
                    let listed: Vec<DestinationId> = names.iter().map(|name| destinations[name.as_str()]).collect();
                    description.set_exit_destinations(id, *side, &listed);
                }
            }
            _ => {}
        }
        // Its distribution strategy, its Drains by id. A name that isn't an instance is left out,
        // and the engine ignores ids that aren't Drains.
        if let Some(strategy) = &resolved.attributes.distribution {
            let rule = distribution_rule(strategy, &ids);
            if rule != DistributionRule::RoundRobin {
                description.set_distribution(id, rule);
            }
        }
    }
    // Each object offers MUs to its successors in the order of its exits, a cross conveyor's at
    // their sides.
    for (from, instance) in model.instances.iter().enumerate() {
        for connection in model.connections_out(&instance.name) {
            description.connect_at(from as u32, connection.from_side, ids[connection.to.as_str()], connection.to_side);
        }
    }
    // Rules (ADR 0030), once every Source's MU kinds are known: a class stands for its own kinds
    // and those of every MU class derived from it.
    let mut kinds: HashMap<String, Vec<crabsim::MuKindId>> = HashMap::new();
    for (kind, class) in mu_classes.iter().enumerate().filter(|(_, class)| !class.is_empty()) {
        let mut names = vec![class.clone()];
        if let Some((chain, built_in)) = model.chain(class) {
            names.extend(chain.iter().map(|link| link.name.clone()));
            names.push(built_in.to_string());
        }
        names.dedup();
        for name in names {
            kinds.entry(name).or_default().push(kind as crabsim::MuKindId);
        }
    }
    let rule_ids = crate::rules::Ids { kinds: &kinds, destinations: &destinations };
    for index in 0..model.instances.len() {
        let resolved = model.resolve(index).expect("checked: every instance's chain of classes is sound");
        if let Some(crate::DistributionStrategy::Rules(rules)) = &resolved.attributes.distribution {
            description.set_rules(index as u32, model.rules_of(index, rules, &rule_ids));
        }
    }
    (description, mu_classes)
}

/// A distribution strategy as the engine takes it.
fn distribution_rule(strategy: &crate::DistributionStrategy, ids: &HashMap<&str, u32>) -> DistributionRule {
    match strategy {
        crate::DistributionStrategy::RoundRobin => DistributionRule::RoundRobin,
        crate::DistributionStrategy::Weighted(weighted) => {
            let by = match weighted.by {
                Attribute::Interval => WeighBy::Interval,
                Attribute::StartingMus => WeighBy::StartingMus,
                Attribute::Capacity => WeighBy::Capacity,
                _ => WeighBy::ProcessTime,
            };
            let drains = weighted.drains.as_ref().map(|names| names.iter().filter_map(|name| ids.get(name.as_str()).copied()).collect());
            DistributionRule::Weighted(WeightedRule { by, order: weighted.order, rank: weighted.rank, drains })
        }
        crate::DistributionStrategy::ByDestination(then) => DistributionRule::ByDestination(Box::new(distribution_rule(then, ids))),
        // Code chooses on top of its default, which the hooks give it when built with code.
        crate::DistributionStrategy::Code(code) => distribution_rule(&code.default, ids),
        crate::DistributionStrategy::Rules(rules) => distribution_rule(&rules.default, ids),
    }
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
            let profile = FailureProfile::new(failures.availability as f32, repair_time, 0).named(failures.name.clone()).with_time_basis(failures.time_basis);
            match failures.stream {
                Some(stream) => profile.with_stream(stream),
                None => profile,
            }
        }
    }
}
