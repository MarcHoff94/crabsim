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

# Run with external renderer at 5x speed
RUST_LOG=info cargo run --bin crabsim -- --renderer-window --speed 5 --hold 1

# Run all tests
cargo test

# Run tests with log output
RUST_LOG=info cargo test -- --nocapture

# Run a single test
cargo test <test_name>
```

## Architecture

This is a Rust discrete-event material-flow simulator structured as a library (`src/`) with a demo binary (`src/main.rs`).

**Core engine** (`src/simulator.rs`):
- `Simulator` — registry of material-flow objects keyed by `u32` object_id (`HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>`). Assigns IDs sequentially via `register_material_flow`. Also holds drain references and optional TCP stream for the renderer.
- `EventController` — discrete-event loop using `BinaryHeap<ScheduledEvent>`. Uses a monotonically increasing `seq` counter to break ties and preserve FIFO order for same-timestamp events. `init()` collects initial events from all objects; `run()` processes the heap until `endtime`.

**Material-flow objects** (`src/materialflow_objects/`):
- `flows.rs` — `MaterialFlow` and `EventInterface` traits; `MoveableUnit`; `ProcessState` enum (Processing/Blocked/Idle); `try_to_move_to_successor` (round-robin via `succ_pointer`, records blocked predecessors in `blockedlist`).
- `source.rs` — creates MUs at `proc_time` intervals and pushes them to successors.
- `station.rs` (`SingleStation`) — processes one MU at a time; tracks `entrancelocked`, `exitlocked`, `blockedlist`, `pred`/`succ` vecs. Emits `BlockedRetry` events to blocked predecessors when it frees up.
- `drain.rs` — consumes MUs, records `exits` (timestamps) and `exit_ids` (in-order).

**Movement semantics**: `try_to_move_to_successor` advances `succ_pointer` (round-robin), skips locked successors, and records the predecessor's `object_id` in each skipped successor's `blockedlist`. When all successors are locked, the predecessor goes `Blocked`. When a station frees up, it emits `BlockedRetry` events to everything in its `blockedlist` so they can retry.

**Event types** (`src/event.rs`): `MUExit` (processing complete, try to transfer) and `BlockedRetry` (reactive — successor freed up, predecessor retries). Keeping them distinct preserves scheduling vs. reactive semantics.

**Visualization** (`src/visualization/`):
- `topology.rs` — `print_network_topology` (console) and `render_network_topology` (string for renderer).
- `state.rs` — `StateSnapshot`/`StateRow` structs; in-place ANSI redraw via `Simulator::live_inplace`.

**External renderer**: a second binary (`crabsim-renderer`) that connects via TCP and receives newline-delimited JSON (`{"kind":"topology",...}` then `{"kind":"snapshot",...}`). Launched automatically when `CRABSIM_RENDERER=1`.

## Key Conventions

- All shared mutable objects use `Rc<RefCell<T>>`. When wiring the topology, both `succ` and `pred` vecs must be set to match — the simulation uses predecessors for `BlockedRetry` routing.
- `register_material_flow` must be called for every object (including the `Source`); `add_drain` must additionally be called for each `Drain` so exit stats are tracked.
- `EventController::seq` tiebreaking is load-bearing for deterministic tests — preserve it when changing scheduling logic.
- `blockedlist` is deduped (`add_blocked` checks before inserting). Tests assert this; don't change to a plain `Vec` push.
- `FailureProfile` is the failure/availability model for stations. Availability is a fraction in `(0.0, 1.0)` where `1.0` = 100% available (inactive). The demo uses a no-failure profile (`availability=1.0`, zero durations, `CONSTANT{value:0.0}`).

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

1. Implement `EventInterface` + `MaterialFlow` for the new type.
2. Expose it in `src/materialflow_objects/mod.rs`.
3. Register instances with `simulator.register_material_flow(obj.clone())`.
4. Wire `succ`/`pred` vecs using `Rc<RefCell<dyn MaterialFlow>>` references.

## Agent skills

### Issue tracker

Issues live in GitHub Issues (MarcHoff94/crabsim), managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
