---
title: Material Flow Model
section: concepts
related:
  - moveable-units
  - blocked-retry
  - material-flow-objects
---

# Material Flow Model

## The production line abstraction

A material flow network models a production line: parts enter at a source, pass through one or more processing stations, and exit at a drain. crabsim uses three object types to represent this:

| Object | Role |
|---|---|
| `Source` | Creates new Moveable Units at a fixed interval and pushes them into the network |
| `SingleStation` | Holds and processes one MU at a time, then forwards it |
| `Drain` | Consumes MUs and records their exit times |

## Topology

Objects are connected by wiring `succ` (successors) and `pred` (predecessors) vectors on each object before the simulation starts. The topology is static — it does not change during a run.

```
Source → Station A → Station B → Drain
                  ↘ Station C ↗
```

A predecessor can have multiple successors (fan-out); a successor can have multiple predecessors (fan-in). `SingleStation`s track both directions. `Source` has no predecessors; `Drain` has no successors.

## Routing

When a predecessor has multiple successors, `try_to_move_to_successor` uses **round-robin** routing by default: it starts at `succ_pointer`, tries successors in order, and advances `succ_pointer` to one past the chosen index on success. This distributes load across parallel branches over time.

An alternative `LowestIndexStrategy` always tries from index 0 first (useful for preferring a primary branch). Both strategies implement the `DistributionStrategy` trait.

## Wiring conventions

Both `succ` and `pred` must be wired to match before `eventcontroller.init()` is called. Forgetting to wire one direction doesn't crash immediately but breaks `BlockedRetry` routing — a freed successor won't know which predecessors to notify.

## Code references

- `MaterialFlow` trait — `src/materialflow_objects/flows.rs`
- `SuccessorContainer` trait — `src/materialflow_objects/flows.rs`
- `try_to_move_to_successor` / `try_to_move_to_successor_with_strategy` — `src/materialflow_objects/flows.rs`
- `RoundRobinStrategy`, `LowestIndexStrategy` — `src/materialflow_objects/flows.rs`
- `Source`, `SingleStation`, `Drain` — `src/materialflow_objects/`
