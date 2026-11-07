---
title: Material Flow Model
section: concepts
related:
  - moveable-units
  - blocked-retry
  - material-flow-objects
---

# Material Flow Model

## Objects

A model is made of material-flow objects, each standing for a real thing. MUs flow through them:

| Object | Role | State |
|---|---|---|
| Source | Creates an MU every interval and pushes it on | Works |
| Station | Works on one MU for its process time, then pushes it on. Can fail, be paused or be forced to fail | Works |
| Buffer | Holds MUs up to a capacity | Known to be broken, so models don't use it yet |
| Drain | Takes MUs out of the model and keeps exit statistics | Works |

"Station" means only the processing type. Station capacity above 1, Buffer's dwelling time and stops on objects other than Stations are agreed but not built. See the handoff.

## Building a model

Models are built through `ModelDescription`:

```rust
let mut model = ModelDescription::new();
let source = model.add_source(SimTime::from_secs(5));
let station = model.add_station(SimTime::from_secs(10), failures);
let drain = model.add_drain();
model.connect(source, station);
model.connect(station, drain);
let mut run = model.build(seed);
```

- Each `add_` returns the new object's id. Ids follow the order objects are added, which is also the order Init visits them.
- `connect(from, to)` adds `to` as the next successor of `from`. Successors are tried in the order they were connected.
- One description builds any number of runs, each with its own seed.
- It also holds each object's name, position and dimensions, for views. `build` leaves them behind, so they can't change a result.
- It doesn't validate connections yet. It refuses a successor on a Drain and ids not in the model, but an edge into a Source or from an object to itself panics while running.

## Topology

The topology is a static directed graph. Objects know only their successors, as ids. A successor learns about a predecessor only when that predecessor is refused and lists itself as waiting (see [Blocked Retry Semantics](blocked-retry.md)).

```
Source → Station 1 → Station 2 → Station 3 → ... → Drain
                              ↘ Station 5 → ... ↗
```

## Moves

crabsim uses **push**: when an object is done with its MU, it offers it to its successors. `try_to_move_to_successor` tries them **round-robin** from `succ_pointer`, which moves to one past the chosen successor after a move. If every successor refuses, the object becomes `Blocked` and keeps its MU.

A move takes no simulation time: the MU is in its new object at once.

**Planned, not built:** pull, set on the receiving object; an exit strategy, which picks which MU leaves; and a distribution strategy, which picks which successor gets it. `DistributionStrategy` in `flows.rs` is a start on the second.

## Code references

- `ModelDescription` — `src/model.rs`
- `try_to_move_to_successor`, `DistributionStrategy`, `RoundRobinStrategy` — `src/materialflow_objects/flows.rs`
- `Source`, `SingleStation`, `Buffer`, `Drain` — `src/materialflow_objects/`
- The demo model — `src/demo.rs`
