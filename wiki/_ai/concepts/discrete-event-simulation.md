---
title: Discrete-Event Simulation (AI context)
for: concepts/discrete-event-simulation.md
section: concepts
---

## Facts
- Time jumps from event to event; nothing changes in between, and no time passes inside an event, so a move takes no simulation time
- `SimulationRun` holds `BinaryHeap<ScheduledEvent>`; processing: pop earliest → clock to its time → object's `execute_event(context, eventtype, others)` → queue returned `EventTemplate`s at `now + exec_time`
- A run advances only when asked (`step`, `advance_to`, `advance_bounded`, `advance_events`) and has no end time of its own
- `SimTime` = 64-bit count of whole milliseconds, for points and lengths; sampled times rounded to the nearest ms; wall time stays `std::time::Duration`

## Invariants
- At one time: `BlockedRetry` before all other events, then FIFO by `seq` within each group
- Same seed on the same version → identical results; `seq` order is load-bearing

## Pitfalls
- More than `ZERO_TIME_EVENT_LIMIT` (1,000,000) events at one time → `RunError::ZeroTimeLoop`, not an endless loop

## Code locations
- `SimulationRun`, `ZERO_TIME_EVENT_LIMIT` — `src/simulation_run.rs`
- `ScheduledEvent` `Ord`, `EventTemplate`, `EventContext` — `src/event.rs`
- `SimTime` — `src/time.rs`
