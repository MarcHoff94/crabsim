//! The likely bottleneck, by one rule for everyone: the window applies it to a live run's state,
//! and `find_bottleneck` to a run in the results store, so a person and an agent never disagree.
//!
//! The rule: among Stations, the one busy or stopped (failed or paused) the largest share of the
//! time. The glossary's bottleneck is what limits the flow, with what is upstream of it mostly
//! blocked and what is downstream mostly starved, so its direct neighbours' shares come with it as
//! evidence. It is likely, not certain: changing it and comparing runs settles it.

use crate::error::OperationError;
use crate::jobs::run_number;
use crate::results::shown;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use rusqlite::OptionalExtension;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<FindBottleneck>()]
}

/// How many Stations the ranking lists.
const RANKING_LENGTH: usize = 5;

/// What the rule says, for anyone reading its answer.
pub const RULE: &str = "The likely bottleneck is the Station busy or stopped (failed or paused) the largest share of the time. It is likely, not certain: what is upstream of a bottleneck is mostly blocked and what is downstream mostly idle, waiting for MUs, so check its neighbours, and settle it by running the model with it faster and comparing the runs.";

/// One object's shares of the time measured, from 0 to 1, for judging the bottleneck.
#[derive(Debug, Clone, PartialEq)]
pub struct Shares {
    pub name: String,
    /// Source, Station, Buffer or Drain.
    pub built_in: String,
    pub working: f64,
    pub blocked: f64,
    pub idle: f64,
    pub failed: f64,
    pub paused: f64,
}

impl Shares {
    /// Busy or stopped: the share of time it was doing its work or kept from it.
    pub fn not_idle(&self) -> f64 {
        self.working + self.failed + self.paused
    }
}

/// The rule's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct LikelyBottleneck {
    pub instance: String,
    /// Busy plus failed plus paused.
    pub not_idle_share: f64,
    pub working_share: f64,
    pub failed_share: f64,
    pub paused_share: f64,
    /// Each direct predecessor with its blocked share, in the order the connections are listed.
    pub upstream: Vec<(String, f64)>,
    /// Each direct successor but a Drain with its idle share: waiting for MUs.
    pub downstream: Vec<(String, f64)>,
    /// The Stations most busy or stopped, most first, at most five, with that share.
    pub ranking: Vec<(String, f64)>,
}

/// The likely bottleneck among `objects`, with `connections` from predecessor to successor by
/// name, or `None` without Stations. A tie goes to the one listed first.
pub fn likely_bottleneck(objects: &[Shares], connections: &[(String, String)]) -> Option<LikelyBottleneck> {
    let mut stations: Vec<&Shares> = objects.iter().filter(|object| object.built_in == "Station").collect();
    // Stable, so a tie keeps the order the objects were listed in.
    stations.sort_by(|a, b| b.not_idle().total_cmp(&a.not_idle()));
    let found = *stations.first()?;
    let shares_of = |name: &str| objects.iter().find(|object| object.name == name);
    let upstream = connections
        .iter()
        .filter(|(_, to)| *to == found.name)
        .filter_map(|(from, _)| shares_of(from).map(|shares| (from.clone(), shares.blocked)))
        .collect();
    // A Drain takes MUs at once, so it is always idle and says nothing about the flow.
    let downstream = connections
        .iter()
        .filter(|(from, _)| *from == found.name)
        .filter_map(|(_, to)| shares_of(to).filter(|shares| shares.built_in != "Drain").map(|shares| (to.clone(), shares.idle)))
        .collect();
    Some(LikelyBottleneck {
        instance: found.name.clone(),
        not_idle_share: found.not_idle(),
        working_share: found.working,
        failed_share: found.failed,
        paused_share: found.paused,
        upstream,
        downstream,
        ranking: stations.iter().take(RANKING_LENGTH).map(|station| (station.name.clone(), station.not_idle())).collect(),
    })
}

// --- find_bottleneck ---

pub struct FindBottleneck;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindBottleneckInput {
    /// The run's id, such as `run-3`.
    pub run: String,
}

/// A neighbour upstream, with how much of the time it was blocked, from 0 to 1.
#[derive(Serialize, JsonSchema)]
pub struct Upstream {
    pub instance: String,
    pub blocked_share: f64,
}

/// A neighbour downstream, with how much of the time it was idle, waiting for MUs, from 0 to 1.
#[derive(Serialize, JsonSchema)]
pub struct Downstream {
    pub instance: String,
    pub idle_share: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct Likely {
    pub instance: String,
    /// Busy plus failed plus paused, from 0 to 1.
    pub not_idle_share: f64,
    pub working_share: f64,
    pub failed_share: f64,
    pub paused_share: f64,
    /// Its direct predecessors, each with its blocked share: high for a bottleneck.
    pub upstream: Vec<Upstream>,
    /// Its direct successors but Drains, each with its idle share: high for a bottleneck. A Drain
    /// is always idle, so it says nothing.
    pub downstream: Vec<Downstream>,
}

#[derive(Serialize, JsonSchema)]
pub struct Ranked {
    pub instance: String,
    pub not_idle_share: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct FindBottleneckOutput {
    pub run: String,
    /// How the answer was found, and how to make sure of it.
    pub rule: String,
    /// None when the model has no Station, or no observation has ended yet.
    pub likely: Option<Likely>,
    /// The Stations most busy or stopped, most first, at most five.
    pub ranking: Vec<Ranked>,
}

impl Operation for FindBottleneck {
    const NAME: &'static str = "find_bottleneck";
    const DESCRIPTION: &'static str = "Names a run's likely bottleneck: the Station busy or stopped (failed or paused) the largest share of the time, from the means over its observations, with its direct predecessors' blocked shares and direct successors' idle shares as evidence (a bottleneck's upstream is mostly blocked and its downstream mostly idle), and the five Stations most busy or stopped. Shares are from 0 to 1, to 4 significant digits. It is likely, not certain: settle it by running the model with that Station faster and comparing the runs with compare_runs. The window's Results tab uses the same rule.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = FindBottleneckInput;
    type Output = FindBottleneckOutput;

    fn run(catalogue: &Catalogue, input: FindBottleneckInput) -> Result<FindBottleneckOutput, OperationError> {
        let number = run_number(&input.run)?;
        let id = input.run;
        let store = catalogue.store()?;
        let connection = store.connection();
        let model_text: String = connection
            .query_row("SELECT model_text FROM runs WHERE number = ?1", [number], |row| row.get(0))
            .optional()?
            .ok_or_else(|| OperationError::not_found(format!("there is no run `{id}` in the results store")))?;

        // Each object's mean share of every state over the observations that have it, in the order
        // the figures were written: model order.
        let mut objects: Vec<Shares> = Vec::new();
        let mut statement = connection.prepare(
            "SELECT object, built_in_class, statistic, avg(value) FROM statistics
             WHERE run = ?1 AND statistic IN ('working_share', 'blocked_share', 'idle_share', 'failed_share', 'paused_share')
             GROUP BY object, statistic ORDER BY min(rowid)",
        )?;
        let rows = statement.query_map([&id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, f64>(3)?)))?;
        let mut index: HashMap<String, usize> = HashMap::new();
        for row in rows {
            let (name, built_in, statistic, mean) = row?;
            let at = *index.entry(name.clone()).or_insert_with(|| {
                objects.push(Shares { name, built_in, working: 0.0, blocked: 0.0, idle: 0.0, failed: 0.0, paused: 0.0 });
                objects.len() - 1
            });
            let shares = &mut objects[at];
            match statistic.as_str() {
                "working_share" => shares.working = mean,
                "blocked_share" => shares.blocked = mean,
                "idle_share" => shares.idle = mean,
                "failed_share" => shares.failed = mean,
                _ => shares.paused = mean,
            }
        }
        let connections: Vec<(String, String)> = crabsim_model::load(&model_text)
            .map(|model| {
                model.instances.iter().flat_map(|instance| model.successors(&instance.name).into_iter().map(|to| (instance.name.clone(), to.to_string()))).collect()
            })
            .unwrap_or_default();

        let found = likely_bottleneck(&objects, &connections);
        let ranking = found
            .as_ref()
            .map(|found| found.ranking.iter().map(|(instance, share)| Ranked { instance: instance.clone(), not_idle_share: shown(*share) }).collect())
            .unwrap_or_default();
        let likely = found.map(|found| Likely {
            instance: found.instance,
            not_idle_share: shown(found.not_idle_share),
            working_share: shown(found.working_share),
            failed_share: shown(found.failed_share),
            paused_share: shown(found.paused_share),
            upstream: found.upstream.into_iter().map(|(instance, share)| Upstream { instance, blocked_share: shown(share) }).collect(),
            downstream: found.downstream.into_iter().map(|(instance, share)| Downstream { instance, idle_share: shown(share) }).collect(),
        });
        Ok(FindBottleneckOutput { run: id, rule: RULE.to_string(), likely, ranking })
    }
}
