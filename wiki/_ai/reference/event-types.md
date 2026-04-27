---
title: Event Types (AI context)
for: reference/event-types.md
section: reference
---

## Active types
- `MUExit`: processing complete → call `try_to_move_to_successor`. On success: Idle + notify blocked. On failure: Blocked. Guard: returns `Ok(None)` immediately if `interruption != None`.
- `BlockedRetry`: successor freed → retry transfer. No-op if `is_exit_allowed()==false`.
- `StartFailure`: sets `InterruptionState::Failed`, saves `remain_proc_time` if Processing, schedules `EndFailure` after sampled MTTR. Emitted by `init()` (first failure) and by `EndFailure` handler (next cycle).
- `EndFailure`: clears `InterruptionState`, schedules next `StartFailure`, then: resumes `MUExit` with saved remaining time if interrupted mid-processing; or retries move if was Blocked; or notifies predecessors if was Idle.

## Inactive (defined only)
- `MUEntrance`: entrance is handled directly via `process_mu_entrance`, not as a dispatched event
- `Finished`: defined, not dispatched

## EventTemplate
```rust
EventTemplate { eventtype, exec_time: Duration, object_id: u32, seq: u64 }
```
- `exec_time`: relative offset from current time (EventController computes absolute time)
- `seq`: set to 0 by objects, overwritten by EventController

## ScheduledEvent ordering
- Min-time first; equal times → min-seq first
- Implemented as reversed cmp on a max-BinaryHeap

## Code locations
- `EventType`, `EventTemplate`, `ScheduledEvent` — `src/event.rs`
- `StartFailure`/`EndFailure` dispatch — `src/materialflow_objects/station.rs`
