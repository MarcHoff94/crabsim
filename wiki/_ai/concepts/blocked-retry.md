---
title: Blocked Retry Semantics (AI context)
for: concepts/blocked-retry.md
section: concepts
---

## Facts
- When all successors deny entrance: predecessor → `ProcessState::Blocked`, entrance stays locked, self recorded in each denied successor's `blockedlist`
- When a station frees up: calls `notify_blocked_predecessors()`, drains `blockedlist`, emits `BlockedRetry` at `exec_time = 0` for each
- `BlockedRetry` on a station where `is_exit_allowed() == false` is a **no-op** (e.g. if it is currently Processing a new MU)
- `Source` also handles `BlockedRetry`: retries push, then creates next MU if successful
- `Drain` never blocks predecessors: `notify_blocked_predecessors` returns `None`; entrance always open

## Invariants
- `MUExit` = scheduled; `BlockedRetry` = reactive. Keep them separate.
- Always call `notify_blocked_predecessors()` when a station transitions to Idle
- Always check `is_exit_allowed()` at the top of the `BlockedRetry` handler

## Retry rule
- `blockedlist` = predecessors waiting, longest-waiting first, each once (`list_blocked`, `retries_for_blocked` in flows.rs)
- A refused retry lists its predecessor again, so those still waiting keep their order
- `try_to_move_to_successor` removes a predecessor from all its successors' lists when its MU moves (`remove_blocked`)
- A `BlockedRetry` on an empty station does nothing
- Known gap: an MU finishing at the instant a successor frees up can take it before the longest-waiting predecessor's retry runs

## Code locations
- `try_to_move_to_successor` — `src/materialflow_objects/flows.rs`
- `SingleStation::notify_blocked_predecessors` — `src/materialflow_objects/station.rs`
- `BlockedRetry` dispatch in `execute_event` — `src/materialflow_objects/station.rs`
- `ProcessState`, `is_exit_allowed` — `src/materialflow_objects/station.rs`
- Tests — `src/lib.rs`: `blocked_retry_while_predecessor_processing_does_not_move_mu`, `station_becomes_blocked_when_no_successor_accepts`, `blocked_retry_full_flow`, `blockedlist_dedup`
