---
title: Material Flow Model (AI context)
for: concepts/material-flow-model.md
section: concepts
---

## Facts
- Three object types: `Source` (creates MUs) → `SingleStation` (processes) → `Drain` (consumes)
- Topology is a static directed graph wired via `succ`/`pred` vecs before `init()` is called
- Routing: `try_to_move_to_successor` uses `RoundRobinStrategy` by default (starts at `succ_pointer`, wraps)
- `LowestIndexStrategy` always tries index 0 first — pluggable via `DistributionStrategy` trait
- On successful transfer: `succ_pointer` advances to `(chosen_idx + 1) % n`

## Invariants
- Both `succ` and `pred` must be wired consistently — broken pred wiring breaks `BlockedRetry` routing
- Source has no predecessors; Drain has no successors
- `Simulator::init()` must be called after all objects are registered and wired

## Code locations
- `try_to_move_to_successor` / `try_to_move_to_successor_with_strategy` — `src/materialflow_objects/flows.rs`
- `RoundRobinStrategy`, `LowestIndexStrategy` — `src/materialflow_objects/flows.rs`
- `SuccessorContainer` trait — `src/materialflow_objects/flows.rs`
