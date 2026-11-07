---
title: Blocked Retry Semantics (AI context)
for: concepts/blocked-retry.md
section: concepts
---

## Facts
- MUExit → `try_to_move_to_successor`; each refusing successor gets the predecessor in its `blockedlist` (`add_blocked` → `list_blocked`)
- All refuse → predecessor `Blocked`, keeps its MU, entrance stays locked, records `Blocked`
- An object that frees up calls `notify_blocked_predecessors` → `retries_for_blocked` empties the list into `BlockedRetry { from }` events due now, longest-waiting first
- On a retry: nothing if `!is_exit_allowed()` (working or stopped) or empty; else `retry_move_to_successor` offers the MU to `from` only
- `MUExit` is scheduled (work done); `BlockedRetry` is reactive (successor freed up)

## Invariants
- `blockedlist`: longest-waiting first, each predecessor once; a refused retry lists it again
- A predecessor whose MU moves is removed from all its successors' lists
- Retries run before other events at the same time, and offer only the sender, so nothing overtakes those waiting
- A Drain's entrance is always open, so nothing waits for it

## Pitfalls
- A retry that finds its predecessor stopped is used up; the predecessor relists at the back after repair
- Giving up a place on any move assumes one MU per object: revisit with Station capacity, Buffer fix, exit strategies
- Source's `is_exit_allowed` is always true, so with several successors a retry can move an MU before its interval ends (known problem)
- Without dedup, refused retries once multiplied into a false `ZeroTimeLoop`

## Code locations
- `try_to_move_to_successor`, `retry_move_to_successor`, `list_blocked`, `retries_for_blocked` — `src/materialflow_objects/flows.rs`
- Retry handling — `src/materialflow_objects/station.rs`, `source.rs`
- Tests — `tests/simulation_run.rs`
