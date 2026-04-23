---
title: Event Types (AI context)
for: reference/event-types.md
section: reference
---

## Active types
- `MUExit`: processing complete → call `try_to_move_to_successor`. On success: Idle + notify blocked. On failure: Blocked.
- `BlockedRetry`: successor freed → retry transfer. No-op if `is_exit_allowed()==false`.

## Inactive (defined only)
- `MUEntrance`: entrance is handled directly via `process_mu_entrance`, not as a dispatched event
- `Finished`, `StartFailure`, `EndFailure`: defined, not yet dispatched

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
