# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Run demo
RUST_LOG=info cargo run --bin crabsim

# Run with in-place terminal table
RUST_LOG=info cargo run --bin crabsim -- --inplace

# Run all tests
cargo test

# Run tests with log output
RUST_LOG=info cargo test -- --nocapture

# Run a single test
cargo test <test_name>

# Run the long-run test (minutes, on demand, in release)
cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model
```

## Design pillars

**Performance** is the first thing to weigh in design trade-offs: crabsim exists to simulate long periods as fast as possible. It is a guideline and doesn't automatically win. Every significant design states its performance cost. Results stay correct, and the same seed gives the same result.

- **Measure:** the benchmark tracks events per second, simulated time per wall second and peak memory. Changes to the per-event path show before and after numbers. Optimise against a measurement, except for decisions that are expensive to undo, which get settled up front.
- **Memory** stays flat as simulated length grows, unless the user asked for recording.
- **Headless runs** are plain simulation runs. Live sessions, views and animation are optional layers that cost nothing when absent.

The design already agreed for this, including what isn't built yet, is in `docs/agents/handoff.md` and `docs/adr/0002-read-ahead-live-session-with-checkpoints-and-replay.md`. The sections below describe what is built today.

## Architecture

This is a Rust discrete-event material-flow simulator structured as a library (`src/`) with a demo binary (`src/main.rs`).

**Core engine** (`src/simulation_run.rs`): `SimulationRun` owns the objects (`BTreeMap<u32, Rc<RefCell<dyn MaterialFlow>>>`), clock and event queue (`BinaryHeap<ScheduledEvent>`). At one simulation time, `BlockedRetry` events run before all others, and a monotonically increasing `seq` preserves FIFO order within each of those two groups. A run only advances when asked and knows nothing about views or pacing:
- `add_object` — adds an object and assigns its id.
- `init` — sets t=0, calls each object's `init` in id order and queues the first events without processing them. Only allowed from the default state.
- `step` — processes one event. `advance_to(t)` — processes events up to and including `t`, then leaves the clock at `t`. `advance_bounded(t, n)` — the same, for at most `n` events; if events are still due, it reports not having reached `t` and the clock stays at the last event.
- `reset` — returns every object to its default state. Objects stay in the model.
- `state` — the full typed `RunState`. `schedule_event` — queues an event from outside the model.
- Too many events at one simulation time (`ZERO_TIME_EVENT_LIMIT`) return `RunError::ZeroTimeLoop` rather than looping forever.
- An advance that fails returns `AdvanceFailure`, holding the `RunError` and everything recorded before it, so no caller loses occurrences.

**Live session** (`src/live_session.rs`): `LiveSession` owns a thread on which a builder closure creates the whole run, since a run is `!Send`. Commands go in (`start`, `halt`, `reset`, `set_speed`) and `Observation`s come out (occurrences, state, status changes, errors). It takes wall time from a `Clock` and advances in bounded slices so commands stay responsive. Sending never blocks the session thread: past `OBSERVATION_BACKLOG_LIMIT` unread observations the session stops advancing until they are read, keeps handling commands and drops nothing. Tests use a fake clock with `sync`, which blocks until the session has handled earlier commands and caught up with the clock. It returns `Backlogged` instead when observations need reading first, and `SyncTimedOut` at its deadline. It knows nothing about any UI technology.

**Observation** (`src/observation.rs`): every advance returns `Occurrence`s (MU created/moved/left, work started/finished, blocked/unblocked, stop began/ended). Scheduling events are engine internals and are never shown to views. Objects record occurrences through the `EventContext` they receive with each event, which also gives them the time and new MU ids.

**Material-flow objects** (`src/materialflow_objects/`):
- `flows.rs` — `MaterialFlow` and `EventInterface` traits; `MoveableUnit`; `ProcessState` enum (Processing/Blocked/Idle); `try_to_move_to_successor` (round-robin via `succ_pointer`, records blocked predecessors in `blockedlist`).
- `source.rs` — creates MUs at `proc_time` intervals and pushes them to successors.
- `station.rs` (`SingleStation`) — processes one MU at a time; tracks `entrancelocked`, `exitlocked`, `blockedlist`, `pred`/`succ` vecs. Emits `BlockedRetry` events to blocked predecessors when it frees up.
- `drain.rs` — consumes MUs, records `exits` (timestamps) and `exit_ids` (in-order).
- `buffer.rs` — holds MUs up to a capacity. Known to be broken and not yet used by the demo.

**Movement semantics**: `try_to_move_to_successor` advances `succ_pointer` (round-robin), skips locked successors, and records the predecessor's `object_id` in each skipped successor's `blockedlist`, removing it from all of them once its MU moves. When all successors are locked, the predecessor goes `Blocked`. When a station frees up, it emits `BlockedRetry` events to everything in its `blockedlist` so they can retry.

**Event types** (`src/event.rs`): `MUExit` (processing complete, try to transfer) and `BlockedRetry { from }` (reactive — successor `from` freed up, predecessor retries that successor only). Keeping them distinct preserves scheduling vs. reactive semantics.

**Visualization** (`src/visualization/`):
- `mod.rs` — the `RunView` trait and `advance_with_view`, which advances a run and shows each result to a view.
- `terminal.rs` — `render_table` and the in-place `InPlaceTerminalView`.
- `topology.rs` — ASCII topology text. Still downcasts by type, and is meant to be replaced by the UI.

**Demo** (`src/demo.rs`): the example model behind `--inplace` and the default run.

**Tests** (`tests/`): `simulation_run.rs` tests behaviour at the run's interface: Init, advance, Reset, then read occurrences and state. `views.rs` tests the view adapters. `live_session.rs` tests commands in and observations out, with a fake clock.

## Key Conventions

- All shared mutable objects use `Rc<RefCell<T>>`, so a run and its objects stay on one thread. When wiring the topology, set `succ`. `pred` is currently never read: `BlockedRetry` routing uses the ids in `blockedlist`.
- `add_object` must be called for every object. Ids given to constructors are replaced.
- `seq` tiebreaking is load-bearing for deterministic tests — preserve it when changing scheduling logic. The same seed on the same version must always give identical results. Results may differ between versions when behaviour changes on purpose.
- `blockedlist` holds the predecessors waiting for an object, longest-waiting first, each listed once. Objects use `list_blocked` and `retries_for_blocked` from `flows.rs` for this. A predecessor gives up its place at every successor as soon as its MU moves, and an empty station doesn't list itself. Retries run before other events at the same time and offer only the successor that sent them, so nothing can reach a free successor ahead of those waiting for it. A retry that finds its predecessor stopped is used up, so that predecessor lists itself again at the back after its repair. This all serves today's push with one MU per object. Giving up a place on any move assumes the predecessor has no other MU waiting, so revisit it with Station capacity, the Buffer fix and exit strategies.
- Every object must implement `reset`, returning it to its default state, including configured starting MUs. When adding a field, reset it and extend the reset tests in `tests/simulation_run.rs`. A replay alone can't catch leftover events, because they land on the replay's own event times.
- `FailureProfile` is the failure/availability model for stations. Availability is a fraction in `(0.0, 1.0)` where `1.0` = 100% available (inactive). The demo uses `availability=0.9` with an Erlang repair time (mean 60s, k=2); tests use `availability=1.0` unless they exercise failures.
- `MUExit` carries an `ExitTicket` naming its MU and the object's `stop_count` when it was scheduled. The event heap can't cancel events, so a stop bumps `stop_count` and the queued exit is ignored as stale when it fires. Resuming schedules a new exit for the remaining process time.

## Wiki sync

Two ways to keep the wiki in sync with the source:

**Automatic (post-commit)**: `.githooks/post-commit` calls `claude` non-interactively after every commit that touches `src/`. Requires one-time setup:
```bash
git config core.hooksPath .githooks
```

**Manual**: type `/update-wiki` in Claude Code at any time. Checks both uncommitted changes and the last commit.

Both use `git diff` to detect changed `.rs` files and update only the affected wiki pages and their `_ai/` summaries.

## Wiki

Deep-context pages live in `wiki/`. Pull these in with `@` when working on a specific area:

| Topic | AI context file |
|---|---|
| Full project context | `@wiki/_ai/all.md` |
| Blocked/retry mechanics | `@wiki/_ai/concepts/blocked-retry.md` |
| Event scheduling & FIFO | `@wiki/_ai/architecture/simulator-and-event-controller.md` |
| Station/source/drain internals | `@wiki/_ai/architecture/material-flow-objects.md` |
| Adding a new object type | `@wiki/_ai/guides/adding-a-new-station-type.md` |
| Test patterns | `@wiki/_ai/guides/writing-tests.md` |

## Adding a New Material-Flow Type

1. Implement `EventInterface` (including `reset` and `report`) + `StopCtrl` + `MaterialFlow` for the new type, plus `SuccessorContainer` if it has successors.
2. Expose it in `src/materialflow_objects/mod.rs`.
3. Add instances with `run.add_object(obj.clone())`.
4. Wire `succ`/`pred` vecs using `Rc<RefCell<dyn MaterialFlow>>` references.

## Agent skills

### Issue tracker

Issues live in GitHub Issues (MarcHoff94/crabsim), managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
