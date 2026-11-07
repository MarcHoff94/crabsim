# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build

# Run the demo model headless for a simulated day, then print its statistics
cargo run --release --bin crabsim

# The same with a seed and a simulated length, written as the UI shows time
cargo run --release --bin crabsim -- --seed 7 --length 365d

# Run all tests
cargo test

# Run tests with log output
RUST_LOG=info cargo test -- --nocapture

# Run a single test
cargo test <test_name>

# Run the long-run test (minutes, on demand, in release)
cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model

# Run the benchmark (about a minute)
cargo bench --bench reference

# Run the UI (a Bevy application, ADR 0006), optimised without LTO so it rebuilds quickly
cargo run -p crabsim-ui --profile ui

# Run the UI's tests, which builds Bevy (about 5 minutes the first time)
cargo test -p crabsim-ui --profile ui

# Run the benchmark for one scenario, with more repetitions
cargo bench --bench reference -- --reps 9 --scenario demo-dropped
```

## Design pillars

**Performance** is the first thing to weigh in design trade-offs: crabsim exists to simulate long periods as fast as possible. It is a guideline and doesn't automatically win. Every significant design states its performance cost. Results stay correct, and the same seed gives the same result.

- **Measure:** the benchmark (`benches/reference.rs`) tracks events per second, simulated time per wall second and peak memory. Changes to the per-event path show before and after numbers. Optimise against a measurement, except for decisions that are expensive to undo, which get settled up front. The current baseline and profile are in `docs/design/first-profile.md`. The Rust version is pinned in `rust-toolchain.toml`, so changing it means a new baseline.
- **Memory** stays flat as simulated length grows, unless the user asked for recording.
- **Headless runs** are plain simulation runs. Live sessions, views and animation are optional layers that cost nothing when absent.

**AI-native** comes second: anything a person can do with crabsim, an AI agent can do too, through the same operations: build a model, run it, read the results, manage models. No feature exists only behind a mouse. Like performance, it is a guideline that can be overruled case by case with real reasoning. See ADR 0007.

- **One operation catalogue:** every capability is an operation with typed input, typed output and a description, offered to agents through front-ends. The MCP server is the first, and in-app agents with the user's own LLM endpoint come later on the same catalogue.
- **Models are readable text** that agents may write directly, and operations validate, run and read them.
- **Every attribute can change during a run,** as an intervention, so a person or agent can experiment with a running model (ADR 0008).
- **Agents act from outside a simulation run,** as a user does. AI decisions inside a run are a possible later opt-in that keeps runs reproducible through a decision log. See `docs/design/ai-decisions-in-runs.md`.

The design already agreed for this, including what isn't built yet, is in `docs/agents/handoff.md`. The sections below describe what is built today.

## Architecture

This is a Rust discrete-event material-flow simulator structured as a library (`src/`) with a headless CLI (`src/main.rs`), in a Cargo workspace with the UI as a member crate (`ui/`). The engine is the only default member, so `cargo test` and `cargo bench` at the root never build the UI, and the engine never depends on it.

**Core engine** (`src/simulation_run.rs`): `SimulationRun` owns the objects (one `Vec<ModelObject>`, indexed by id), clock and event queue (`BinaryHeap<ScheduledEvent>`, 32 bytes per entry). At one simulation time, `BlockedRetry` events run before all others, and a monotonically increasing `seq` preserves FIFO order within each of those two groups. A run only advances when asked and knows nothing about views or pacing:
- `init` — sets t=0, calls each object's `init` in id order and queues the first events without processing them. Only allowed from the default state.
- `step` — processes one event. `advance_events(n)` — at most `n` events, leaving the clock at the last. `advance_to(t)` — processes events up to and including `t`, then leaves the clock at `t`. `advance_bounded(t, n)` — the same, for at most `n` events; if events are still due, it reports not having reached `t` and the clock stays at the last event.
- `reset` — returns every object to its default state. Objects stay in the model.
- `state` — the full typed `RunState`. `schedule_event` — queues an event from outside the model. `set_process_time` — changes a Station's process time from the next MU on, scheduling nothing. `keep_changes_as_default` — makes changed process times the default state, so Reset keeps them.
- Too many events at one simulation time (`ZERO_TIME_EVENT_LIMIT`) return `RunError::ZeroTimeLoop` rather than looping forever.
- An advance that fails returns `AdvanceFailure`, holding the `RunError` and everything recorded before it, so no caller loses occurrences.

**Model description** (`src/model.rs`): `ModelDescription` is how a model gets built: `add_source`, `add_station`, `add_station_holding`, `add_buffer` and `add_drain` return ids, `connect` wires them, and `build(seed)` gives a run. One description can build any number of runs. Ids follow the order objects are added, which is also the order Init visits them. It doesn't validate connections yet. It can report each object's `kind` and `successors`, and holds each object's name, `Position` and `Dimensions` in metres, for views. `build` leaves them behind, so they can't change a result, and an object without dimensions gets its type's default size.

**Simulation time** (`src/time.rs`): `SimTime` is a 64-bit count of whole milliseconds, for both points in time and lengths of it. Sampled times are rounded to the nearest millisecond. Wall time stays a `std::time::Duration`, so the compiler keeps the two apart. `format_time`, `format_time_exact` and `parse_time` write and read it as people do, such as `1:30` or `2d 06:00:00`, the same way in the UI and the CLI.

**Live session** (`src/live_session.rs`): `LiveSession` owns a thread on which a builder closure creates the whole run. The run computes ahead as fast as it can into a bounded buffer, and the clock releases what it computed, so everything the user sees follows shown time while the run is at a later computed time. One thread alternates between computing a slice and releasing. Commands go in (`start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `step_back`, `set_reset_keeps_changes`, `request_state`, `pause`, `resume`, `force_failure`, `end_forced_failure`, `set_process_time`, `clear_statistics`) and `Observation`s come out (occurrences, shown time, state, status changes, errors). A full state comes only at Init, after Reset, after going back and on request. It takes wall time from a `Clock`. Sending never blocks the session thread: past `OBSERVATION_BACKLOG_LIMIT` unread observations the session stops releasing until they are read, keeps handling commands and drops nothing. Tests use a fake clock with `sync`, which blocks until the session has handled earlier commands and shown time has reached the clock. It returns `Backlogged` instead when observations need reading first, and `SyncTimedOut` at its deadline. It knows nothing about any UI technology.

**Checkpoints and replay** (ADR 0002): a checkpoint is a clone of the run plus how much of the intervention log is in it. To reach an earlier instant, the session restores the nearest earlier checkpoint and replays with recording off, applying the logged actions it passes. Going back drops the computed future, later log entries and their checkpoints. `step_back` finds the instant before the one on screen the same way, replaying a copy from the nearest checkpoint, or an earlier one if that sees too little, so nothing remembers past instants. An instant is a time with occurrences. Reset can keep changed process times as the new default state (`reset_keeps_changes`, off by default), taking them from the log, which holds what is true at shown time. Past the memory budget, every other checkpoint in the older half goes, and the one from Init always stays.

**Headless run**: a `SimulationRun` with nothing attached is one: `model.build(seed)`, `init`, `advance_to`, then read `state`. It records nothing by default, and memory stays flat as simulated length grows.

**Statistics** (`src/statistics.rs`): every object keeps a `StatisticsKeeper`, and `state` reports its fixed-size `ObjectStatistics` per object, including the state in progress. They are raw whole numbers, with means and rates as methods. `clear_statistics` starts them afresh, such as after a warm-up, and changes nothing else. The design is in `docs/design/statistics.md`.

**Observation** (`src/observation.rs`): every advance returns the `Occurrence`s the run is recording, as set by `set_recording`: `Recording::NOTHING` by default, `ALL` or `only(&[...])`. A live session records everything. The kinds are (MU created/moved/left, work started/finished, blocked/unblocked, stop began/ended). Scheduling events are engine internals and are never shown to views. Objects record occurrences through the `EventContext` they receive with each event, which also gives them the time and new MU ids.

**Picture** (`src/picture.rs`): what the user knows about a run at shown time. A UI feeds it every observation from a live session, in order, through `observe`. A full state replaces it, as after Init, Reset or going back. Occurrences update each object's contents, process state, interruptions with when each began, and remaining time, but only once it has a state, because Init's occurrences arrive before Init's state. Statistics and waiting predecessors come only from a full state, so they aren't in it. It holds nothing about drawing, so a 2D view, a 3D view and tests read the same thing.

**UI** (`ui/`): a Bevy application with egui panels (ADR 0006), being built as UI stage 1. It runs the demo model in a live session at 60 times real time and draws it in 2D, as flat meshes under an orthographic 3D camera, with state colours, connections, names that shrink with the model and go only when too small to read, pan and zoom, Init, Start, Halt and Reset, stepping back and forward, going back to a typed time, speed, time mode, a banner for errors, and an inspector for the object clicked, with Pause, forced failure and process time changes on Stations. Each intervention leaves a mark on a strip under the top bar, and going back past one asks first. MUs are drawn in the objects holding them when zoomed in and slow enough to follow, and otherwise holders show a count. It filters the engine's per-event `info` logging, which Bevy's logger would otherwise print.
- `src/view.rs` — the view state, plain Rust with no Bevy types: from the `ModelDescription`, the session's observations and wall time, it works out what is drawn, which controls are offered, and whether frames are needed. A halted window draws nothing once things settle, and a running one at most about 60 frames a second.
- `src/camera.rs` — where the 2D view looks and how close, as plain numbers.
- `src/marks.rs` — the interventions made, mirroring the session's log: going back drops the later ones and Reset all of them. Stepping back goes to an instant the view knows only if it saw the two latest instants arrive, so otherwise its count of what it drops is an upper bound.
- `src/main.rs` — Bevy and egui, drawing only what the view state says. Panels are plain functions taking `&mut egui::Ui`.
- `tests/view_state.rs` and `tests/inspector.rs` — the view state against a real live session on a fake clock, with no window. `tests/common/mod.rs` holds what they share.

**Material-flow objects** (`src/materialflow_objects/`):
- `model_object.rs` — the `ModelObject` enum over the object types, which dispatches the traits with a `match`, and `OtherObjects`, which is how an object handling an event reaches the others by id. The run splits its `Vec` around the object in hand, so this takes no `unsafe` and no shared ownership. See ADR 0005.
- `flows.rs` — `MaterialFlow` and `EventInterface` traits; `MoveableUnit`; `ProcessState` enum (Processing/Blocked/Idle); `try_to_move_to_successor` (round-robin via `succ_pointer`, records blocked predecessors in `blockedlist`).
- `source.rs` — creates MUs at `proc_time` intervals and pushes them to successors.
- `station.rs` (`SingleStation`) — processes one MU at a time; tracks `entrancelocked`, `exitlocked`, `blockedlist` and `succ`, and is stopped while `failed`, `forced` to fail or `paused`. A forced failure has its own flag, so random failures carry on underneath it: each schedules the next only when it ends. Only the first interruption of a stop freezes the work, and only the last to end resumes it. Emits `BlockedRetry` events to blocked predecessors when it frees up.
- `drain.rs` — consumes MUs. Per-MU data exists only through recording `MuLeft` occurrences (ADR 0004).
- `buffer.rs` — holds MUs up to a capacity. Known to be broken and not yet used by the demo.

**Movement semantics**: `try_to_move_to_successor` advances `succ_pointer` (round-robin), skips locked successors, and records the predecessor's `object_id` in each skipped successor's `blockedlist`, removing it from all of them once its MU moves. When all successors are locked, the predecessor goes `Blocked`. When a station frees up, it emits `BlockedRetry` events to everything in its `blockedlist` so they can retry.

**Event types** (`src/event.rs`): `MUExit` (processing complete, try to transfer) and `BlockedRetry { from }` (reactive — successor `from` freed up, predecessor retries that successor only). Keeping them distinct preserves scheduling vs. reactive semantics.

**Headless CLI** (`src/main.rs`): runs the demo model with `--seed` (default 0) and `--length` (default `1d`, written as the UI shows time), then prints each object's statistics, each Drain's throughput and mean lead time, and how many events it ran in how long. A run error still prints the statistics up to it, then exits with failure. It takes a model file once models can be saved.

**Demo** (`src/demo.rs`): the example model behind the CLI and the UI. `model()` gives its description, with a name and a hand-written position for every object.

**Tests** (`tests/`): `simulation_run.rs` tests behaviour at the run's interface: Init, advance, Reset, then read occurrences and state. `cli.rs` runs the CLI's binary end to end. `live_session.rs` tests commands in and observations out, with a fake clock, and checks the `Picture` against the run's own state. Tests build models through `ModelDescription`. A failure part way through an event can't happen in a real object, so the few tests of it are unit tests in `src/`, using `RecordsThenFails`, a `#[cfg(test)]` variant of `ModelObject`. What can't be seen from outside a session, such as the buffer limit and thinning, is unit tested in `src/live_session.rs`.

**Benchmark** (`benches/reference.rs`): runs a copy of the demo model, and 429 copies of it side by side (6,006 objects), and reports speed and peak memory. `cargo test --all-targets` runs it without `--bench`, and then it only checks that the copy still matches the demo model.

## Key Conventions

- Objects refer to each other by id only: `succ` and `blockedlist` hold ids. A run has no shared ownership, so it executes on one thread at a time and can move between threads.
- An object's id must match its position in the run's `Vec`. `ModelDescription` guarantees this, so build models through it.
- `seq` tiebreaking is load-bearing for deterministic tests — preserve it when changing scheduling logic. The same seed on the same version must always give identical results. Results may differ between versions when behaviour changes on purpose.
- `blockedlist` holds the predecessors waiting for an object, longest-waiting first, each listed once. Objects use `list_blocked` and `retries_for_blocked` from `flows.rs` for this. A predecessor gives up its place at every successor as soon as its MU moves, and an empty station doesn't list itself. Retries run before other events at the same time and offer only the successor that sent them, so nothing can reach a free successor ahead of those waiting for it. A retry that finds its predecessor stopped is used up, so that predecessor lists itself again at the back after its repair. This all serves today's push with one MU per object. Giving up a place on any move assumes the predecessor has no other MU waiting, so revisit it with Station capacity, the Buffer fix and exit strategies.
- Every object must tell its `StatisticsKeeper` what it ended up doing, at the end of everything it handles (`update_statistics`). Time doesn't move within an event, so this catches every change of state, and it keeps the time buckets from drifting.
- Every object must implement `reset`, returning it to its default state, including configured starting MUs. When adding a field, reset it and extend the reset tests in `tests/simulation_run.rs`. A replay alone can't catch leftover events, because they land on the replay's own event times.
- Interruptions are reported separately (`InterruptionBegan`, `InterruptionEnded`), so overlapping ones are one stop. Their causes are `Failure`, `ForcedFailure` and `Pause`, from most to least serious. `state` reports the most serious as `stop` and lists every active one in `interruptions`, which a view needs to know an object is still stopped after the most serious ends. Pause and forced failure arrive as `PauseOn`/`PauseOff` and `ForceFailureOn`/`ForceFailureOff` events, on Stations only so far. A forced failure counts as a failure in statistics.
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
| Event scheduling & FIFO | `@wiki/_ai/architecture/simulation-run.md` |
| Station/source/drain internals | `@wiki/_ai/architecture/material-flow-objects.md` |
| Adding a new object type | `@wiki/_ai/guides/adding-a-new-object-type.md` |
| Test patterns | `@wiki/_ai/guides/writing-tests.md` |
| Live session and UI | `@wiki/_ai/architecture/live-session-and-ui.md` |

## Adding a New Material-Flow Type

1. Implement `EventInterface` (including `reset` and `report`) + `StopCtrl` + `MaterialFlow` for the new type, plus `SuccessorContainer` if it has successors.
2. Expose it in `src/materialflow_objects/mod.rs`.
3. Add a variant to `ModelObject` and to the `on_the_object!` macro in `src/materialflow_objects/model_object.rs`.
4. Add an `add_` method to `ModelDescription`, and include its successors in `connect` if it has any.

## Agent skills

### Issue tracker

Issues live in GitHub Issues (MarcHoff94/crabsim), managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
