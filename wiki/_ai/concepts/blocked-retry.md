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
- `blockedlist`: `Waiting { since, id }` per predecessor, once each, ordered by when its offered MU became ready to leave (ADR 0018), ties in the order they joined; a refused retry puts it back in the place it had; when its MU moves it gives up its place and waits on by its next ready MU, if any (`relist_blocked`)
- A predecessor whose MU moves is removed from all its successors' lists
- Retries run before other events at the same time, and offer only the sender, so nothing overtakes those waiting
- A Drain's entrance is always open, so nothing waits for it

## Pitfalls
- A retry that finds its predecessor stopped is used up; the predecessor relists at the back after repair
- With one MU per object that is the order there always was; a Buffer is the first object with several MUs
- A Source answers a retry only once its MU's interval is over; a Buffer only while blocked
- Without dedup, refused retries once multiplied into a false `ZeroTimeLoop`

## Code locations
- `try_to_move_to_successor`, `retry_move_to_successor`, `list_blocked`, `retries_for_blocked` — `src/materialflow_objects/flows.rs`
- Retry handling — `src/materialflow_objects/station.rs`, `source.rs`
- Tests — `tests/simulation_run.rs`
