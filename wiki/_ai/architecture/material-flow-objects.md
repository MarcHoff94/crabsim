---
title: Material Flow Objects (AI context)
for: architecture/material-flow-objects.md
section: architecture
---

## Facts
- `enum ModelObject { Source, Station(SingleStation), Buffer, Drain, #[cfg(test)] RecordsThenFails }`; traits dispatched by `on_the_object!`
- Traits: `EventInterface` (init, execute_event, reset, report, clear_statistics, ids, seed), `StopCtrl` (`todo!()` everywhere), `MaterialFlow` (entrance/exit allowed, `process_mu_entrance`, blocked list, notify), `SuccessorContainer` (successors as ids, `succ_pointer`, `take_content(now)`, `statistics_mut`)
- Station: entrance only when Idle, not stopped, not locked; exit when Idle or Blocked and not stopped, never Processing
- Station stops: random failure, forced failure, Pause, each its own flag; first interruption freezes work, last to end resumes; completion time = entry + process time + time stopped, overlaps once
- `state()` reports the most serious interruption as `stop` (Failure > ForcedFailure > Pause) and all of them, with since when, in `interruptions`
- `FailureProfile::new(availability, mttr_distribution, seed)`; MTBF mean = mttr_mean / (1/availability - 1); 1.0 = never fails; distributions CONSTANT, UNIFORM, NORMAL, ERLANG
- Source creates the first MU at Init, the next once one moves; never accepts MUs; no stops yet
- Drain: entrance always open, records `MuLeft` and exit statistics, keeps no MU, handles no events

## Invariants
- Every object calls `update_statistics` at the end of everything it handles
- Every object implements `reset` fully, incl. starting MUs; new fields need reset tests
- A second random failure while failed is discarded; random failures continue under a forced one

## Pitfalls
- Buffer is broken: loses MUs, never notifies predecessors, ignores a retry's `from`

## Code locations
- `src/materialflow_objects/model_object.rs`, `flows.rs`, `source.rs`, `station.rs`, `buffer.rs`, `drain.rs`
- `src/random_numbers.rs`, `src/statistics.rs`
