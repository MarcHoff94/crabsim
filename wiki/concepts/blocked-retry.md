---
title: Blocked Retry Semantics
section: concepts
related:
  - event-types
  - simulation-run
  - material-flow-objects
---

# Blocked Retry Semantics

## The problem

An object finishing with its MU has no guarantee of a free successor: the successor may be working on another MU. crabsim handles this with **blocked and retry**: an object that can't pass its MU on lists itself as waiting at each successor that refused it, and is woken when one frees up, rather than polling.

## Step by step

### 1. A move is tried (MUExit)

When a Station's process time is up, its `MUExit` fires and it calls `try_to_move_to_successor`. That walks the successors round-robin from `succ_pointer` and asks each `is_entrance_allowed()`.

### 2. Every successor refuses: Blocked

Each successor that refuses gets the predecessor's id in its `blockedlist` (`add_blocked`). If none accepts, the predecessor:

- becomes `ProcessState::Blocked` and records a `Blocked` occurrence,
- keeps its MU,
- keeps its entrance locked, so nothing enters while it holds the MU.

### 3. A successor frees up: retries

When an object's MU moves on, or it comes back from a stop with room, it calls `notify_blocked_predecessors()`. That empties its `blockedlist` into one `BlockedRetry { from }` event per waiting predecessor, due at once, where `from` is the object that freed up.

### 4. The retry

The predecessor checks `is_exit_allowed()` first. If it is working on an MU, or is stopped, the retry does nothing: a working MU moves through the normal `MUExit` later. Otherwise `retry_move_to_successor` offers the MU to the successor named by `from`, and only that one.

## Why two event types

`MUExit` and `BlockedRetry` both lead to a move, but mean different things:

- `MUExit` is **scheduled**: work finished at a known time.
- `BlockedRetry` is **reactive**: a successor just freed up.

Keeping them apart lets a retry that reaches a busy object be ignored, and lets retries run first at one time without extra state.

## The retry rule

A `blockedlist` holds the predecessors waiting for an object, each listed once as a `Waiting` with the time the MU it offered became ready to leave. The line is ordered by that time, longest ready first, and those ready at the same instant in the order they joined (ADR 0018):

- `list_blocked` inserts a predecessor by its MU's time and leaves one that is already listed in its place.
- `retries_for_blocked` sends a retry to everyone in the line, in order, and marks each entry as retried. A retry that is refused puts its predecessor back in the place it had. An entry whose retry came to nothing, because the predecessor was stopped, is dropped at the next round and replaced if the predecessor lists itself afresh.
- When a predecessor's MU moves, `try_to_move_to_successor` calls `relist_blocked` at all its successors: it gives up its place, and if it has another MU ready, as a Buffer can, it waits on in the same lines by that MU's time.
- An object that is empty when a retry arrives does nothing, because it has nothing to wait with. A Buffer answers a retry only while it is blocked.

Without the first of these, each refused retry added another entry until one release fired over a million retries at one instant, which tripped `ZERO_TIME_EVENT_LIMIT` in long runs.

Two more things stop anything reaching a free successor ahead of those waiting for it:

- At one simulation time, retries run before all other events, so an MU finishing at the instant a successor frees up goes behind those already waiting.
- A retry names the successor that sent it, and the predecessor tries only that one. Any other successor with room has sent retries of its own.

A retry that finds its predecessor stopped is used up, so that predecessor lists itself again at the back after its repair. The owner may want it to keep its place: that's to decide when stops are built for every object type.

With one MU per object this gives the order crabsim always had. A Buffer is the first object with several MUs, and a Station with a capacity follows.

## Source and Drain

A Source handles `BlockedRetry` too: once its MU has moved, it creates the next one at once. It answers a retry only while its MU's interval is over, so a retry from a successor it no longer waits for can't move the next MU early.

A Drain's entrance is always open, so predecessors never wait for it, and it never sends retries.

## Code references

- `try_to_move_to_successor`, `retry_move_to_successor`, `list_blocked`, `retries_for_blocked` — `src/materialflow_objects/flows.rs`
- `is_entrance_allowed`, `is_exit_allowed`, `BlockedRetry` handling — `src/materialflow_objects/station.rs` and `source.rs`
- Retry order in the queue — `ScheduledEvent`'s `Ord` in `src/event.rs`
- Tests in `tests/simulation_run.rs`: `a_station_blocks_when_no_successor_accepts`, `a_retry_while_the_predecessor_is_working_does_not_move_its_mu`, `the_longest_waiting_predecessor_retries_first`, `a_predecessor_refused_on_every_retry_is_listed_once`, `a_predecessor_that_moved_on_loses_its_place_in_the_queue`, `an_mu_finishing_as_a_successor_frees_up_goes_behind_those_already_waiting`, `a_retry_only_offers_the_successor_that_sent_it`
