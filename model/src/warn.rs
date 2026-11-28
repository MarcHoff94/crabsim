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
        let arrivals = self.destinations_not_reached(&successors, &predecessors, &built_in, &mut warn);
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
            if built_in[i] == "CrossConveyor" {
                self.cross_conveyor_sides(i, &resolved, arrivals.as_ref().map(|arrivals| &arrivals[i]), &mut warn);
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
            let Some(weighted) = resolved.attributes.distribution.as_ref().and_then(DistributionStrategy::weighted) else { continue };
            let at = match (resolved.origin(Attribute::Distribution), &resolved.attributes.distribution) {
                (Origin::Instance, Some(DistributionStrategy::ByDestination(_))) => format!("/instances/{i}/attributes/distribution/by_destination/then/weighted"),
                (Origin::Instance, _) => format!("/instances/{i}/attributes/distribution/weighted"),
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
        warnings.extend(self.reference_warnings());
        warnings.extend(self.expectation_bounds());
        warnings
    }

    /// Warns about validation-stage expectations that a run with randomness would hardly meet as
    /// written: `=` without a tolerance, which only that exact value meets in every observation,
    /// and a reference figure, an `=` with a tolerance, that doesn't say where it came from.
    fn reference_warnings(&self) -> Vec<Problem> {
        let mut warnings = Vec::new();
        for (index, expectation) in self.expectations.iter().enumerate() {
            if expectation.stage != crate::CheckStage::Validation {
                continue;
            }
            let Ok(check) = crate::expect::Check::parse(&expectation.text) else { continue };
            let numeric = matches!(check.target, crate::expect::Target::Number { .. });
            let warn = |at: &str, message: String| Problem { pointer: format!("/expectations/{index}{at}"), name: Some(expectation.id.clone()), message };
            if numeric && check.comparison == crate::expect::Comparison::Equal && check.tolerance.is_none() && !check.is_limit() {
                warnings.push(warn(
                    "/text",
                    "at the validation stage, `=` without a tolerance is met only by exactly that value in every observation: give the reference figure its tolerance, such as `± 3%`".into(),
                ));
            } else if check.tolerance.is_some() && expectation.source.as_deref().is_none_or(|source| source.trim().is_empty()) {
                warnings.push(warn("", "a reference figure says where it came from in its `source`, such as the line's MES for October".into()));
            }
        }
        warnings
    }

    /// Warns about each object routing by destination that none of its successors leads from to a
    /// destination MUs can have there (ADR 0025): it then offers them to every successor, which
    /// is probably not what was meant. Follows each Source's MUs through the model as their plans
    /// and detours change on the way, up to a limit of places visited, so a large model with
    /// many plans is only partly looked through. A cross conveyor sends them on by its exits.
    ///
    /// Returns, by instance, where MUs arriving there can be going next, `None` for nowhere left,
    /// for a cross conveyor's warnings: unless nothing routes and there is no cross conveyor, or
    /// the model was only partly looked through.
    fn destinations_not_reached(
        &self,
        successors: &[Vec<usize>],
        predecessors: &[Vec<usize>],
        built_in: &[&str],
        warn: &mut impl FnMut(String, &str, String),
    ) -> Option<Vec<std::collections::HashSet<Option<String>>>> {
        const MOST_VISITED: usize = 200_000;
        let count = self.instances.len();
        let routes: Vec<bool> = (0..count)
            .map(|i| matches!(self.resolve(i).expect("checked").attributes.distribution, Some(DistributionStrategy::ByDestination(_))))
            .collect();
        if !routes.contains(&true) && !built_in.contains(&"CrossConveyor") {
            return None;
        }
        let mut arrivals: Vec<std::collections::HashSet<Option<String>>> = vec![std::collections::HashSet::new(); count];
        // A cross conveyor's exits, as the successor at each of its exit sides, the side and the
        // names that side lists, and the side each predecessor comes in by.
        let side_of = |from: usize, to: usize| {
            self.connections.iter().find(|c| c.from == self.instances[from].name && c.to == self.instances[to].name).and_then(|c| c.to_side)
        };
        let exits: Vec<Vec<(usize, Option<crate::Side>, Vec<String>)>> = (0..count)
            .map(|i| {
                if built_in[i] != "CrossConveyor" {
                    return Vec::new();
                }
                let listed = self.resolve(i).expect("checked").attributes.exits.unwrap_or_default();
                let name = &self.instances[i].name;
                self.connections
                    .iter()
                    .filter(|connection| &connection.from == name)
                    .filter_map(|connection| {
                        let to = self.instances.iter().position(|instance| instance.name == connection.to)?;
                        Some((to, connection.from_side, connection.from_side.and_then(|side| listed.get(&side).cloned()).unwrap_or_default()))
                    })
                    .collect()
            })
            .collect();
        // Where each destination is, and every instance from which it can be reached.
        let mut members: HashMap<&str, Vec<bool>> = HashMap::new();
        let mut reaches: HashMap<&str, Vec<bool>> = HashMap::new();
        for name in self.destination_names() {
            let mut is = vec![false; count];
            let mut seen = vec![false; count];
            let mut queue: VecDeque<usize> = VecDeque::new();
            for index in self.destination_members(name).unwrap_or_default() {
                is[index] = true;
                seen[index] = true;
                queue.push_back(index);
            }
            while let Some(at) = queue.pop_front() {
                for &before in &predecessors[at] {
                    if !seen[before] {
                        seen[before] = true;
                        queue.push_back(before);
                    }
                }
            }
            members.insert(name, is);
            reaches.insert(name, seen);
        }
        let detours: Vec<Option<String>> = (0..count).map(|i| self.resolve(i).expect("checked").attributes.detour).collect();
        // An MU at an instance: its plan, by place in `plans`, how far along it is, and its detour.
        let mut plans: Vec<Vec<crate::WorkStep>> = Vec::new();
        // The last item is where it came from, which a cross conveyor reads its entry side from.
        let mut queue: VecDeque<(usize, usize, usize, Option<String>, bool, usize)> = VecDeque::new();
        for source in (0..count).filter(|&i| built_in[i] == "Source") {
            let plan = self.work_plan_of(source).unwrap_or_default();
            let id = plans.iter().position(|known| *known == plan).unwrap_or_else(|| {
                plans.push(plan);
                plans.len() - 1
            });
            queue.push_back((source, id, 0, None, false, usize::MAX));
        }
        let mut seen: std::collections::HashSet<(usize, usize, usize, Option<String>, usize)> = std::collections::HashSet::new();
        let mut warned: std::collections::HashSet<(usize, String)> = std::collections::HashSet::new();
        while let Some((at, plan, mut step, mut detour, arrived, came_from)) = queue.pop_front() {
            if seen.len() >= MOST_VISITED {
                return None;
            }
            // Only a cross conveyor treats MUs differently by where they came from.
            let from_key = if built_in[at] == "CrossConveyor" { came_from } else { usize::MAX };
            if !seen.insert((at, plan, step, detour.clone(), from_key)) {
                continue;
            }
            let next = |step: usize, detour: &Option<String>| detour.clone().or_else(|| plans[plan].get(step).map(|step| step.at.clone()));
            if arrived && let Some(going) = next(step, &detour) {
                if members[going.as_str()][at] {
                    if detour.is_some() {
                        detour = None;
                    } else {
                        step += 1;
                    }
                }
            }
            // As the engine does: a Station's detour replaces any the MU had.
            if built_in[at] == "Station" && detours[at].is_some() {
                detour = detours[at].clone();
            }
            let mut onward: Vec<usize> = successors[at].clone();
            if arrived {
                arrivals[at].insert(next(step, &detour));
            }
            // A cross conveyor turns an MU towards the exits whose lists take where it goes, and
            // otherwise sends it straight on, by whichever exit is across from where it came in.
            if built_in[at] == "CrossConveyor" {
                let going = next(step, &detour);
                let takes = |names: &Vec<String>| {
                    going.as_deref().is_some_and(|going| names.iter().any(|name| (0..count).any(|i| members[going][i] && members.get(name.as_str()).is_some_and(|is| is[i]))))
                };
                let entry = (came_from != usize::MAX).then(|| side_of(came_from, at)).flatten();
                let straight = exits[at].iter().find(|(_, side, _)| side.is_some() && *side == entry.map(crate::Side::opposite));
                // As the engine does: any exit whose list takes it, straight across included, and
                // straight across when none does.
                let taking: Vec<usize> = exits[at].iter().filter(|(_, _, names)| takes(names)).map(|&(to, _, _)| to).collect();
                onward = if taking.is_empty() { straight.map(|&(to, _, _)| to).into_iter().collect() } else { taking };
            }
            if routes[at] && let Some(going) = next(step, &detour) {
                let reaching: Vec<usize> = onward.iter().copied().filter(|&to| reaches[going.as_str()][to]).collect();
                if reaching.is_empty() {
                    if warned.insert((at, going.clone())) {
                        warn(
                            format!("/instances/{at}"),
                            &self.instances[at].name,
                            format!("it routes by destination, but none of its successors leads to `{going}`, where MUs can be going from here, so it offers those to every successor"),
                        );
                    }
                } else {
                    onward = reaching;
                }
            }
            for to in onward {
                queue.push_back((to, plan, step, detour.clone(), true, at));
            }
        }
        Some(arrivals)
    }

    /// Warns about a cross conveyor's sides (spec issue 148): an entry side with no exit straight
    /// across, where an MU that no exit side takes would wait for good, and exits listed for a side
    /// no connection leaves by. `arrivals`, where MUs arriving can be going, if known: then only
    /// if some MU can arrive going where no exit side lists, so a corner every MU turns at is fine.
    fn cross_conveyor_sides(
        &self,
        index: usize,
        resolved: &crate::Resolved,
        arrivals: Option<&std::collections::HashSet<Option<String>>>,
        warn: &mut impl FnMut(String, &str, String),
    ) {
        let name = &self.instances[index].name;
        let entries: Vec<crate::Side> = self.connections.iter().filter(|c| &c.to == name).filter_map(|c| c.to_side).collect();
        let exits: Vec<crate::Side> = self.connections.iter().filter(|c| &c.from == name).filter_map(|c| c.from_side).collect();
        let listed = resolved.attributes.exits.clone().unwrap_or_default();
        let members = |name: &str| self.destination_members(name).unwrap_or_default();
        let taken = |going: &str| {
            let going = members(going);
            listed.iter().filter(|(side, _)| exits.contains(side)).flat_map(|(_, names)| names).any(|name| members(name).iter().any(|i| going.contains(i)))
        };
        let some_go_straight = arrivals.is_none_or(|arrivals| arrivals.iter().any(|going| going.as_deref().is_none_or(|going| !taken(going))));
        for side in entries.iter().filter(|side| some_go_straight && !exits.contains(&side.opposite())) {
            warn(
                format!("/instances/{index}"),
                name,
                format!(
                    "MUs come in by its {} side, and no connection leaves by its {} side straight across, so an MU going where no exit side lists waits there for good",
                    side.name(),
                    side.opposite().name()
                ),
            );
        }
        for side in resolved.attributes.exits.iter().flat_map(|exits| exits.keys()).filter(|side| !exits.contains(side)) {
            let at = match resolved.origin(Attribute::Exits) {
                Origin::Instance => format!("/instances/{index}/attributes/exits/{}", side.name()),
                _ => format!("/instances/{index}"),
            };
            warn(at, name, format!("its exits list destinations for its {} side, but no connection leaves by it, so no MU goes that way", side.name()));
        }
    }

    /// Warns about each conveyor the Source at `source` reaches, through objects without geometry
    /// and conveyors its MUs fit, whose width or length its MUs don't fit: the conveyor refuses
    /// them, and they wait for good. Also about one in whose tightest bend they stick out past its
    /// edge. An MU lies as it does there: crosswise, its width along the belt, after a cross
    /// conveyor turns it, until another turns it back or an object without geometry passes it on
    /// lengthwise, so the search follows each MU's way with how it lies, and a cross conveyor's
    /// side it comes in by.
    fn mus_that_dont_fit(&self, source: usize, successors: &[Vec<usize>], built_in: &[&str], warn: &mut impl FnMut(String, &str, String)) {
        let Some(kind) = self.mu_kind_of(source) else { return };
        let class = || self.resolve(source).expect("checked").attributes.mu_class.unwrap_or_default();
        let side_between = |from: usize, to: usize| {
            let (from, to) = (&self.instances[from].name, &self.instances[to].name);
            self.connections.iter().find(|c| &c.from == from && &c.to == to).map(|c| (c.from_side, c.to_side))
        };
        // Each place an MU can be, coming from where, and whether it lies crosswise.
        let mut seen: std::collections::HashSet<(usize, usize, bool)> = std::collections::HashSet::new();
        let mut queue: VecDeque<(usize, usize, bool)> = successors[source].iter().map(|&next| (next, source, false)).collect();
        let mut warned: std::collections::HashSet<(usize, bool)> = std::collections::HashSet::new();
        while let Some((at, from, crosswise)) = queue.pop_front() {
            if !seen.insert((at, from, crosswise)) {
                continue;
            }
            // Its extent along the way it goes, and across it.
            let (along, across) = if crosswise { (kind.width, kind.length) } else { (kind.length, kind.width) };
            let lying = if crosswise { ", lying crosswise after a cross conveyor turned them," } else { "" };
            if built_in[at] == "CrossConveyor" {
                let cross = self.resolve(at).expect("checked").attributes;
                let (length, width) = (cross.length.unwrap_or(0.6), cross.width.unwrap_or(0.6));
                let name = &self.instances[at].name;
                let Some(entry) = side_between(from, at).and_then(|(_, side)| side) else { continue };
                // Coming in by its front or back its way is its length, by its left or right its width.
                let (way, breadth) = if entry.along_length() { (length, width) } else { (width, length) };
                if along > way + 1e-9 || across > breadth + 1e-9 {
                    if warned.insert((at, crosswise)) {
                        warn(
                            format!("/instances/{source}"),
                            &self.instances[source].name,
                            format!(
                                "its MUs, of MU class `{}`{lying} are {along} m long and {across} m wide on their way, and cross conveyor `{name}`, {length} by {width} m, has no way across them from its {} side, where they come in, so it refuses them and they wait there for good",
                                class(),
                                entry.name()
                            ),
                        );
                    }
                    continue;
                }
                // Straight across it lies as it came in, and turned it lies the other way.
                for &next in &successors[at] {
                    let exit = side_between(at, next).and_then(|(side, _)| side);
                    let turned = exit.is_some_and(|exit| exit != entry.opposite());
                    queue.push_back((next, at, crosswise != turned));
                }
                continue;
            }
            if built_in[at] != "Conveyor" {
                // An object without geometry passes MUs on lengthwise.
                queue.extend(successors[at].iter().map(|&next| (next, at, false)));
                continue;
            }
            let conveyor = self.resolve(at).expect("checked").attributes;
            let (width, length) = (conveyor.width.unwrap_or(0.6), crate::path_length(conveyor.path.as_deref().unwrap_or_default()));
            let too = if across > width {
                format!("{across} m wide, and conveyor `{}` only {width} m", self.instances[at].name)
            } else if along > length {
                format!("{along} m long, and conveyor `{}` only {length} m", self.instances[at].name)
            } else {
                // A footprint centred on the path in a bend reaches out furthest at its outer
                // corners. Only a view sees it: the conveyor still carries the MU.
                if let Some(radius) = crate::path::tightest_radius(conveyor.path.as_deref().unwrap_or_default()) {
                    let reach = ((radius + across / 2.0).powi(2) + (along / 2.0).powi(2)).sqrt() - radius;
                    if reach > width / 2.0 + 1e-9 && warned.insert((at, crosswise)) {
                        warn(
                            format!("/instances/{source}"),
                            &self.instances[source].name,
                            format!(
                                "its MUs, of MU class `{}`{lying} are {} by {} m, and in conveyor `{}`'s tightest bend, of radius {radius} m, their corners stick out {:.2} m past its edge, as it is {width} m wide",
                                class(),
                                kind.length,
                                kind.width,
                                self.instances[at].name,
                                reach - width / 2.0
                            ),
                        );
                    }
                }
                // It fits, so it travels on to what comes after, lying as it does.
                queue.extend(successors[at].iter().map(|&next| (next, at, crosswise)));
                continue;
            };
            if warned.insert((at, crosswise)) {
                warn(
                    format!("/instances/{source}"),
                    &self.instances[source].name,
                    format!("its MUs, of MU class `{}`{lying} are {too}, which refuses them, so they wait there for good", class()),
                );
            }
        }
    }
}
