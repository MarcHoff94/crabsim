---
title: Discrete-Event Simulation (AI context)
for: concepts/discrete-event-simulation.md
section: concepts
---

## Facts
- Time advances in discrete jumps from event to event; nothing happens between events
- `EventController` holds a `BinaryHeap<ScheduledEvent>` ordered by (time asc, seq asc)
- Run loop: pop event → advance clock → dispatch to object → schedule returned templates
- `exec_time` in `EventTemplate` is a **relative** offset; EventController adds `current_time`
- Simulation stops when queue is empty OR `endtime` is exceeded

## Invariants
- `seq` is monotonically increasing; FIFO order for same-timestamp events depends on it
- Never change seq assignment order — deterministic tests rely on it
- `endtime: Option<Duration>` — `None` means run until queue is empty

## Code locations
- `EventController`, run loop — `src/simulator.rs`
- `ScheduledEvent` ordering (`Ord` impl) — `src/event.rs`
- `EventTemplate` (returned by objects) — `src/event.rs`
