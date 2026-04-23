---
title: Blocked Retry Semantics
section: concepts
related:
  - event-types
  - simulator-and-event-controller
  - material-flow-objects
---

# Blocked Retry Semantics

## The problem

A station finishing its work does not guarantee a free downstream station. The successor might already be processing another part. crabsim handles this with a **blocked/retry** mechanism: when a station cannot forward its MU, it parks itself as blocked and waits to be woken up — rather than polling or re-scheduling blind retries.

## Step-by-step flow

### 1. Transfer attempt (MUExit)

When a station finishes processing it fires a `MUExit` event and calls `try_to_move_to_successor`. This function walks the successor list (round-robin from `succ_pointer`) and checks `is_entrance_allowed()` on each.

### 2. All successors locked → Blocked

If every successor returns `false` from `is_entrance_allowed()`, `try_to_move_to_successor` records the predecessor's `object_id` in each denied successor's `blockedlist` (via `add_blocked`). The predecessor then:

- Stays in `ProcessState::Blocked`
- Keeps its MU in `content`
- Keeps `entrancelocked = true` (prevents new MUs entering while it holds the current one)

### 3. Successor frees up → notify

When a station successfully transfers its MU and becomes idle, it calls `notify_blocked_predecessors()`. This drains its `blockedlist` and emits one `BlockedRetry` event per recorded predecessor, scheduled at `exec_time = 0` (i.e., immediately).

### 4. Retry

The predecessor receives `BlockedRetry` and calls `is_exit_allowed()` before attempting anything. If the station is currently processing a new MU (`ProcessState::Processing`), `is_exit_allowed()` returns `false` and the retry is a no-op — the MU will move when that processing completes via the normal `MUExit` path.

If exit is allowed, `try_to_move_to_successor` is called again.

## Why two separate event types

`MUExit` and `BlockedRetry` both trigger a transfer attempt but carry different meaning:

- `MUExit` is **scheduled**: processing finished at a known future time.
- `BlockedRetry` is **reactive**: a successor just became available.

Keeping them separate means a `BlockedRetry` arriving on a busy station is correctly ignored. Merging them would require extra state to distinguish "just finished my own work" from "woken up while already working".

## Source and Drain differences

`Source` also handles `BlockedRetry` — if it was blocked trying to push its MU, it retries on notification, then immediately creates the next MU once the current one is forwarded.

`Drain` never blocks predecessors. Its entrance is always open (`entrance_locked` starts `false`), so predecessors never end up in a Drain's `blockedlist`.

## Known subtlety: dedup vs runtime path

`SingleStation` has two `add_blocked` methods with different behaviour:

- The **inherent method** (`impl SingleStation`) checks for duplicates before inserting.
- The **trait method** (`impl MaterialFlow for SingleStation`) pushes unconditionally.

`try_to_move_to_successor` calls through the `MaterialFlow` trait (dynamic dispatch), so the dedup check does **not** apply in the runtime path. Duplicate predecessors in `blockedlist` result in multiple `BlockedRetry` events to the same object, which is harmless (the retry is a no-op if the station is busy), but the test `blockedlist_dedup` tests the inherent method path, not the runtime path.

## Code references

- `try_to_move_to_successor` — `src/materialflow_objects/flows.rs`
- `notify_blocked_predecessors` — `src/materialflow_objects/station.rs`
- `ProcessState` enum — `src/materialflow_objects/flows.rs`
- `is_exit_allowed`, `is_entrance_allowed` — `src/materialflow_objects/station.rs`
- `BlockedRetry` dispatch in `execute_event` — `src/materialflow_objects/station.rs`
- Tests: `blocked_retry_while_predecessor_processing_does_not_move_mu`, `station_becomes_blocked_when_no_successor_accepts`, `blocked_retry_full_flow`, `blockedlist_dedup` — `src/lib.rs`
