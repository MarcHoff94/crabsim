//! The Paths tab: a view of a model's routes, weights and distribution strategies, drawn on the
//! canvas. It edits nothing: values change in the inspector. Plain Rust with no egui types, so it
//! is tested without a window. See spec issue 96.

use crate::view::ObjectView;
use crabsim::ModelDescription;
use crabsim_model::{Attribute, DistributionStrategy, Model};
use serde_json::{json, Value};

/// What an object weighs on a route.
#[derive(Debug, Clone, PartialEq)]
pub enum Weigh {
    /// Every connection counts one: the routes with fewest connections first.
    Connections,
    /// An attribute, by its name in a model file, resolved through inheritance.
    Attribute(&'static str),
    /// A figure of the live run at shown time, by its name in the results store.
    Figure(&'static str),
}

/// The attributes routes can be weighed by, with their labels.
pub const ATTRIBUTES: [(&str, &str); 4] = crate::attributes::WEIGHABLE;

/// The live run's figures routes can be weighed by, with their labels.
pub const FIGURES: [(&str, &str); 6] = [
    ("working_share", "Busy share"),
    ("blocked_share", "Blocked share"),
    ("idle_share", "Idle share"),
    ("failed_share", "Failed share"),
    ("mus_in", "MUs in"),
    ("mus_out", "MUs out"),
];

impl Weigh {
    /// How the window names it.
    pub fn label(&self) -> &'static str {
        match self {
            Weigh::Connections => "Connections",
            Weigh::Attribute(name) => ATTRIBUTES.iter().find(|(n, _)| n == name).map_or("?", |(_, label)| label),
            Weigh::Figure(name) => FIGURES.iter().find(|(n, _)| n == name).map_or("?", |(_, label)| label),
        }
    }

    /// Whether its values are times, shown in seconds.
    fn is_time(&self) -> bool {
        matches!(self, Weigh::Attribute("process_time" | "interval"))
    }

    /// A value as the window shows it, such as "45 s", "0.81" or "12".
    pub fn show(&self, value: f64) -> String {
        let rounded = (value * 1000.0).round() / 1000.0;
        if self.is_time() { format!("{rounded} s") } else { format!("{rounded}") }
    }
}

/// One route the tab lists.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// The objects on it, both ends included, by id.
    pub ids: Vec<u32>,
    /// Its total weight, or for connections how many it takes.
    pub total: f64,
}

/// The routes found from one object to another.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Best first: up to 21.
    pub routes: Vec<Route>,
    /// The objects on them without the value, which weigh 0.
    pub weightless: Vec<u32>,
    /// False when loops stopped the search for the largest totals early.
    pub exact: bool,
    /// How many routes there are, counted up to 10,000, and whether there are more.
    pub count: u64,
    pub more: bool,
}

/// The Paths tab's state: which views are on, what the routes are between, how they're weighed,
/// and what was found.
#[derive(Debug, Clone)]
pub struct Paths {
    pub route_on: bool,
    pub weights_on: bool,
    pub strategies_on: bool,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub weigh: Weigh,
    pub largest: bool,
    /// The routes found for the inputs in `found_for`, or why there are none.
    pub found: Option<Result<Found, String>>,
    /// Which route is highlighted, by place in the list.
    pub chosen: usize,
    found_for: Option<Inputs>,
}

/// What the routes depend on, so they're found again only when it changes.
#[derive(Debug, Clone, PartialEq)]
struct Inputs {
    from: u32,
    to: u32,
    weigh: Weigh,
    largest: bool,
    /// For a live figure: the values weighed, which change as the run goes.
    values: Option<Vec<Option<f64>>>,
    model: u64,
}

impl Default for Paths {
    fn default() -> Paths {
        Paths {
            route_on: false,
            weights_on: false,
            strategies_on: false,
            from: None,
            to: None,
            weigh: Weigh::Connections,
            largest: false,
            found: None,
            chosen: 0,
            found_for: None,
        }
    }
}

impl Paths {
    /// For another model: the views stay as they were, and the route's ends and what was found
    /// go, since they were the other model's.
    pub fn for_another_model(&self) -> Paths {
        Paths { route_on: self.route_on, weights_on: self.weights_on, strategies_on: self.strategies_on, ..Paths::default() }
    }

    /// Whether the routes have to be found again for these inputs: `model` changes whenever the
    /// model does, and `values` are a live figure's.
    pub fn needs_finding(&self, model: u64, values: Option<&[Option<f64>]>) -> bool {
        let (Some(from), Some(to)) = (self.from, self.to) else { return false };
        self.route_on && from != to && self.found_for != Some(self.inputs(from, to, model, values))
    }

    fn inputs(&self, from: u32, to: u32, model: u64, values: Option<&[Option<f64>]>) -> Inputs {
        Inputs { from, to, weigh: self.weigh.clone(), largest: self.largest, values: values.map(<[Option<f64>]>::to_vec), model }
    }

    /// The `query_model` question for the routes, with live `values` by object for a figure.
    pub fn query(&self, objects: &[ObjectView], values: Option<&[Option<f64>]>) -> Option<Value> {
        let (from, to) = (objects.get(self.from? as usize)?, objects.get(self.to? as usize)?);
        let mut query = json!({ "kind": "paths", "from": from.name, "to": to.name });
        match &self.weigh {
            Weigh::Connections => return Some(query),
            Weigh::Attribute(name) => query["weight"] = json!({ "attribute": name }),
            Weigh::Figure(_) => {
                let given: serde_json::Map<String, Value> = objects
                    .iter()
                    .zip(values?)
                    .filter_map(|(object, value)| value.map(|value| (object.name.clone(), json!(value.max(0.0)))))
                    .collect();
                query["weight"] = json!({ "values": given });
            }
        }
        query["order"] = json!(if self.largest { "largest" } else { "smallest" });
        Some(query)
    }

    /// Keeps what `query_model` answered for these inputs, highlighting the best route.
    pub fn take_answer(&mut self, answer: Result<Value, String>, objects: &[ObjectView], model: u64, values: Option<&[Option<f64>]>) {
        let (Some(from), Some(to)) = (self.from, self.to) else { return };
        self.found_for = Some(self.inputs(from, to, model, values));
        self.found = Some(answer.and_then(|answer| found(&answer, objects)));
        self.chosen = 0;
    }

    /// The highlighted route's objects, by id, while the route view is on.
    pub fn highlighted(&self) -> Option<&[u32]> {
        if !self.route_on {
            return None;
        }
        let found = self.found.as_ref()?.as_ref().ok()?;
        found.routes.get(self.chosen).map(|route| route.ids.as_slice())
    }
}

/// The routes in a `query_model` paths answer, by id: weighted ones with their totals, or the
/// shortest and the others by how many connections they take.
fn found(answer: &Value, objects: &[ObjectView]) -> Result<Found, String> {
    let id = |name: &Value| objects.iter().find(|object| Some(object.name.as_str()) == name.as_str()).map(|object| object.id);
    let ids = |names: &Value| names.as_array().into_iter().flatten().filter_map(id).collect::<Vec<u32>>();
    let count = answer["count"].as_u64().unwrap_or_default();
    let more = answer["more"] == true;
    if let Some(weighted) = answer.get("weighted") {
        let routes = weighted["routes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|route| Route { ids: ids(&route["instances"]), total: route["total"].as_f64().unwrap_or_default() })
            .collect();
        return Ok(Found { routes, weightless: ids(&weighted["weightless"]), exact: weighted["exact"] != false, count, more });
    }
    let mut routes = Vec::new();
    for route in std::iter::once(&answer["shortest"]).filter(|route| !route.is_null()).chain(answer["others"].as_array().into_iter().flatten()) {
        let ids = ids(route);
        routes.push(Route { total: ids.len().saturating_sub(1) as f64, ids });
    }
    Ok(Found { routes, weightless: Vec::new(), exact: true, count, more })
}

/// Each object's weight, by id, `None` where it has none, as `query_model` weighs it: an
/// attribute resolved through inheritance, times in seconds, or a live figure's `values`.
pub fn weights(model: &Model, weigh: &Weigh, values: Option<&[Option<f64>]>) -> Vec<Option<f64>> {
    match weigh {
        Weigh::Connections => vec![None; model.instances.len()],
        Weigh::Figure(_) => values.map_or_else(|| vec![None; model.instances.len()], <[Option<f64>]>::to_vec),
        Weigh::Attribute(name) => (0..model.instances.len())
            .map(|index| {
                let resolved = model.resolve(index)?;
                let has = &resolved.attributes;
                match *name {
                    "process_time" if Attribute::ProcessTime.applies_to(resolved.built_in) => has.process_time.map(|time| time.as_millis() as f64 / 1000.0),
                    "interval" if Attribute::Interval.applies_to(resolved.built_in) => has.interval.map(|time| time.as_millis() as f64 / 1000.0),
                    "starting_mus" if Attribute::StartingMus.applies_to(resolved.built_in) => has.starting_mus.map(|count| count as f64),
                    "capacity" if Attribute::Capacity.applies_to(resolved.built_in) => has.capacity.map(|count| count as f64),
                    _ => None,
                }
            })
            .collect(),
    }
}

/// How an object with successors picks one, as the Strategies view marks it.
#[derive(Debug, Clone, PartialEq)]
pub struct Strategy {
    pub weighted: bool,
    /// For a weighted one: each successor's place in its ranking, from 1, by id. Those sharing
    /// place 1 are preferred.
    pub places: Vec<(u32, usize)>,
}

/// Each object's strategy, by id, `None` for one without successors: the model's own setting,
/// and the engine's ranking for the weighted ones.
pub fn strategies(model: &Model, description: &ModelDescription) -> Vec<Option<Strategy>> {
    (0..description.object_count())
        .map(|id| {
            let successors = description.successors(id);
            if successors.is_empty() {
                return None;
            }
            let weighted = model
                .resolve(id as usize)
                .is_some_and(|resolved| matches!(resolved.attributes.distribution, Some(DistributionStrategy::Weighted(_))));
            let places = match description.ranking(id).filter(|_| weighted) {
                Some(ranking) => ranking
                    .groups
                    .iter()
                    .enumerate()
                    .flat_map(|(place, group)| group.iter().map(move |&exit| (successors[exit], place + 1)))
                    .collect(),
                None => Vec::new(),
            };
            Some(Strategy { weighted, places })
        })
        .collect()
}
