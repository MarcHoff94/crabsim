---
title: Material Flow Model (AI context)
for: concepts/material-flow-model.md
section: concepts
---

## Facts
- Object types: Source (creates an MU every interval), Station (one MU at a time for its process time; fails, pauses, forced failure), Buffer (capacity; known broken, unused), Drain (takes MUs out, exit statistics)
- "Station" means only the processing type; Station capacity, Buffer dwelling time and stops on non-Stations are agreed but not built
- Built through `ModelDescription`: `add_source/add_station/add_station_holding/add_buffer/add_drain` return ids (= Init order), `connect(from, to)` appends a successor, `build(seed)` gives a run; one description builds many runs
- Names, positions, dimensions live on the description for views; `build` drops them, so they can't change results
- Topology is static; objects know only successor ids; predecessors appear only in `blockedlist`
- Push: `try_to_move_to_successor` tries successors round-robin from `succ_pointer`, which moves to one past the chosen one; all refuse → `Blocked`

## Invariants
- Successors are tried in the order connected

## Pitfalls
- No validation yet: an edge into a Source or a self-loop panics while running; only a successor on a Drain and unknown ids are refused
- Pull, exit strategy and distribution strategy are planned, not built

## Code locations
- `ModelDescription` — `src/model.rs`
- `try_to_move_to_successor`, `DistributionStrategy` — `src/materialflow_objects/flows.rs`
- Demo model — `src/demo.rs`
