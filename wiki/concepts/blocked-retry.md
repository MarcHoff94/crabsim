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

A `blockedlist` holds the predecessors waiting for an object, longest-waiting first, each listed once:

- `list_blocked` leaves a predecessor that is already listed in its place. `retries_for_blocked` empties the list into retries in that order, and a retry that is refused lists its predecessor again.
- When a predecessor's MU moves, `try_to_move_to_successor` removes it from all its successors' lists, so it can't keep a place from an earlier wait.
- An object that is empty when a retry arrives does nothing, because it has nothing to wait with.

Without the first of these, each refused retry added another entry until one release fired over a million retries at one instant, which tripped `ZERO_TIME_EVENT_LIMIT` in long runs.

Two more things stop anything reaching a free successor ahead of those waiting for it:

- At one simulation time, retries run before all other events, so an MU finishing at the instant a successor frees up goes behind those already waiting.
- A retry names the successor that sent it, and the predecessor tries only that one. Any other successor with room has sent retries of its own.

A retry that finds its predecessor stopped is used up, so that predecessor lists itself again at the back after its repair. The owner may want it to keep its place: that's to decide when stops are built for every object type.

All this serves today's push with one MU per object. Giving up a place on any move assumes the predecessor has no other MU waiting, so revisit it with Station capacity, the Buffer fix and exit strategies.

## Source and Drain

A Source handles `BlockedRetry` too: once its MU has moved, it creates the next one at once. Its `is_exit_allowed` is always true, so with several successors a retry can move an MU before its interval ends. That's a known problem.

A Drain's entrance is always open, so predecessors never wait for it, and it never sends retries.

## Code references

- `try_to_move_to_successor`, `retry_move_to_successor`, `list_blocked`, `retries_for_blocked` — `src/materialflow_objects/flows.rs`
- `is_entrance_allowed`, `is_exit_allowed`, `BlockedRetry` handling — `src/materialflow_objects/station.rs` and `source.rs`
- Retry order in the queue — `ScheduledEvent`'s `Ord` in `src/event.rs`
- Tests in `tests/simulation_run.rs`: `a_station_blocks_when_no_successor_accepts`, `a_retry_while_the_predecessor_is_working_does_not_move_its_mu`, `the_longest_waiting_predecessor_retries_first`, `a_predecessor_refused_on_every_retry_is_listed_once`, `a_predecessor_that_moved_on_loses_its_place_in_the_queue`, `an_mu_finishing_as_a_successor_frees_up_goes_behind_those_already_waiting`, `a_retry_only_offers_the_successor_that_sent_it`
