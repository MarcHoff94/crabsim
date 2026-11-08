# crabsim — full AI context

Discrete-event material-flow simulator in Rust, built for speed. Engine library at the root, model files in `model/`, the operation catalogue in `catalogue/`, the headless CLI in `cli/`, the MCP server in `mcp/` and the Bevy UI in `ui/`. `CLAUDE.md` describes what is built and `docs/agents/handoff.md` the agreed design. Generated from the summaries in `wiki/_ai/`.

---

## CONCEPTS

### Discrete-Event Simulation

#### Facts
- Time jumps from event to event; nothing changes in between, and no time passes inside an event, so a move takes no simulation time
- `SimulationRun` holds `BinaryHeap<ScheduledEvent>`; processing: pop earliest → clock to its time → object's `execute_event(context, eventtype, others)` → queue returned `EventTemplate`s at `now + exec_time`
- A run advances only when asked (`step`, `advance_to`, `advance_bounded`, `advance_events`) and has no end time of its own
- `SimTime` = 64-bit count of whole milliseconds, for points and lengths; sampled times rounded to the nearest ms; wall time stays `std::time::Duration`

#### Invariants
- At one time: `BlockedRetry` before all other events, then FIFO by `seq` within each group
- Same seed on the same version → identical results; `seq` order is load-bearing

#### Pitfalls
- More than `ZERO_TIME_EVENT_LIMIT` (1,000,000) events at one time → `RunError::ZeroTimeLoop`, not an endless loop

#### Code locations
- `SimulationRun`, `ZERO_TIME_EVENT_LIMIT` — `src/simulation_run.rs`
- `ScheduledEvent` `Ord`, `EventTemplate`, `EventContext` — `src/event.rs`
- `SimTime` — `src/time.rs`

### Material Flow Model

#### Facts
- Object types: Source (creates an MU every interval), Station (one MU at a time for its process time; fails, pauses, forced failure), Buffer (capacity; known broken, unused), Drain (takes MUs out, exit statistics)
- "Station" means only the processing type; Station capacity, Buffer dwelling time and stops on non-Stations are agreed but not built
- Built through `ModelDescription`: `add_source/add_station/add_station_holding/add_buffer/add_drain` return ids (= Init order), `connect(from, to)` appends a successor, `build(seed)` gives a run; one description builds many runs
- Names, positions, dimensions live on the description for views; `build` drops them, so they can't change results
- Topology is static; objects know only successor ids; predecessors appear only in `blockedlist`
- Push: `try_to_move_to_successor` tries successors round-robin from `succ_pointer`, which moves to one past the chosen one; all refuse → `Blocked`

#### Invariants
- Successors are tried in the order connected

#### Pitfalls
- No validation yet: an edge into a Source or a self-loop panics while running; only a successor on a Drain and unknown ids are refused
- Pull, exit strategy and distribution strategy are planned, not built

#### Code locations
- `ModelDescription` — `src/model.rs`
- `try_to_move_to_successor`, `DistributionStrategy` — `src/materialflow_objects/flows.rs`
- Demo model — `src/demo.rs`

### Moveable Units

#### Facts
- `MoveableUnit { id: u32, location_id: u32, left_source: SimTime }`
- Ids are unique across the model, from `EventContext::new_mu_id`, starting above the highest configured starting MU id
- Source holds the MU it is creating (first one at Init); Station holds at most one (`content: Option<MoveableUnit>`), entrance locked while it does; Drain keeps none
- A Station's starting MU (`add_station_holding`) is part of its default state: worked on from Init, a fresh copy after Reset
- Occurrences along the way: `MuCreated` → `MuMoved` → `WorkStarted` → `WorkFinished` → ... → `MuLeft { lead_time }`
- Lead time runs from leaving the Source to entering the Drain

#### Invariants
- Per-MU data exists only through recorded occurrences (ADR 0004); a headless run records nothing, so memory stays flat

#### Code locations
- `MoveableUnit` — `src/materialflow_objects/flows.rs`
- `Source::create_mu` — `src/materialflow_objects/source.rs`
- `Drain::process_mu_entrance` — `src/materialflow_objects/drain.rs`

### Blocked Retry Semantics

#### Facts
- MUExit → `try_to_move_to_successor`; each refusing successor gets the predecessor in its `blockedlist` (`add_blocked` → `list_blocked`)
- All refuse → predecessor `Blocked`, keeps its MU, entrance stays locked, records `Blocked`
- An object that frees up calls `notify_blocked_predecessors` → `retries_for_blocked` empties the list into `BlockedRetry { from }` events due now, longest-waiting first
- On a retry: nothing if `!is_exit_allowed()` (working or stopped) or empty; else `retry_move_to_successor` offers the MU to `from` only
- `MUExit` is scheduled (work done); `BlockedRetry` is reactive (successor freed up)

#### Invariants
- `blockedlist`: longest-waiting first, each predecessor once; a refused retry lists it again
- A predecessor whose MU moves is removed from all its successors' lists
- Retries run before other events at the same time, and offer only the sender, so nothing overtakes those waiting
- A Drain's entrance is always open, so nothing waits for it

#### Pitfalls
- A retry that finds its predecessor stopped is used up; the predecessor relists at the back after repair
- Giving up a place on any move assumes one MU per object: revisit with Station capacity, Buffer fix, exit strategies
- Source's `is_exit_allowed` is always true, so with several successors a retry can move an MU before its interval ends (known problem)
- Without dedup, refused retries once multiplied into a false `ZeroTimeLoop`

#### Code locations
- `try_to_move_to_successor`, `retry_move_to_successor`, `list_blocked`, `retries_for_blocked` — `src/materialflow_objects/flows.rs`
- Retry handling — `src/materialflow_objects/station.rs`, `source.rs`
- Tests — `tests/simulation_run.rs`

---

## ARCHITECTURE

### Architecture Overview

#### Facts
- Workspace: engine `crabsim` (root, library), `crabsim-model` (`model/`, model files, ADR 0009), `crabsim-catalogue` (`catalogue/`, operations, jobs, results store), `crabsim-cli` (`cli/`, binary `crabsim`), `crabsim-mcp` (`mcp/`, MCP server), `crabsim-ui` (`ui/`, Bevy + egui, ADR 0006); the UI and the MCP server are the non-default members; the engine depends on none of them
- A model file → `crabsim_model::load` → `Model` or every `Problem` → `to_description()` → `ModelDescription` → `build(seed)`
- Layers: front-ends (CLI, MCP server, UI) → operation catalogue (CLI and MCP) → optional live session + Picture → `SimulationRun` → material-flow objects (`enum ModelObject`)
- Headless run = a `SimulationRun` with nothing attached: build, `init`, `advance_to`, `state`
- One `Vec<ModelObject>` indexed by id; objects refer to each other by id; the run splits the `Vec` around the object handling an event and passes `OtherObjects` (ADR 0005)
- No shared ownership, no `unsafe`; a run runs on one thread at a time and can move; a clone is a checkpoint

#### Invariants
- An object's id must match its index; build through `ModelDescription`
- Live sessions and views cost a headless run nothing

#### Code locations
- `src/simulation_run.rs` SimulationRun · `src/model.rs` ModelDescription · `src/event.rs` events · `src/report.rs` occurrences and state · `src/statistics.rs` · `src/live_session.rs` · `src/picture.rs` · `src/time.rs` SimTime, format/parse · `src/demo.rs`
- `model/src/`: `lib.rs` (Model, Problem, load), `read.rs`, `check.rs`, `write.rs`, `build.rs`, `schema.rs` (JSON Schema via schemars, drift-tested against the reader); `cli/src/main.rs`; `models/demo.json`, `models/model-file.schema.json`
- `src/materialflow_objects/`: `model_object.rs`, `flows.rs`, `source.rs`, `station.rs`, `buffer.rs`, `drain.rs`
- `ui/src/`: `view.rs`, `camera.rs`, `marks.rs`, `main.rs`

### Simulation Run

#### Facts
- `SimulationRun` holds objects, `BinaryHeap<ScheduledEvent>` (32 bytes each), clock, `next_seq`, zero-time counters, seed, recording, `statistics_since`, `EventContext`
- `init`: t=0, per-object seed, each object's `init` in id order, first events queued but not processed (even at t=0); only from the default state
- `step`, `advance_to(t)` (includes t, clock left at t), `advance_bounded(t, n)`, `advance_events(n)`; splitting an advance changes nothing
- `reset`: every object to its default state; `state()`: full typed `RunState` incl. `ObjectStatistics`, also after an error
- `schedule_event`, `set_process_time` (from the next MU, schedules nothing), `keep_changes_as_default`, `clear_statistics`, `set_recording` (`NOTHING` default, `ALL`, `only(&[...])`)
- Failure → `AdvanceFailure { error, occurrences }`; errors: `NotInitialised`, `AlreadyInitialised`, `EventInThePast`, `UnknownObject`, `NotAStation`, `ObjectFailed`, `ZeroTimeLoop`
- Stale exits: `MUExit(ExitTicket { mu_id, stop_count })`; a stop bumps `stop_count`, so the old exit is ignored

#### Invariants
- Order: time, then `BlockedRetry` first, then `seq`; preserve `seq` assignment
- Control order: Init only from default state, advancing needs Init, Reset any time; violations are errors, not panics

#### Pitfalls
- A zero-time loop leaves its event queued, so the run keeps refusing until Reset

#### Code locations
- `SimulationRun`, `BoundedAdvance` — `src/simulation_run.rs`
- `ScheduledEvent`, `ExitTicket`, `EventContext` — `src/event.rs`
- `RunError`, `AdvanceFailure` — `src/error.rs`

### Material Flow Objects

#### Facts
- `enum ModelObject { Source, Station(SingleStation), Buffer, Drain, #[cfg(test)] RecordsThenFails }`; traits dispatched by `on_the_object!`
- Traits: `EventInterface` (init, execute_event, reset, report, clear_statistics, ids, seed), `StopCtrl` (`todo!()` everywhere), `MaterialFlow` (entrance/exit allowed, `process_mu_entrance`, blocked list, notify), `SuccessorContainer` (successors as ids, `succ_pointer`, `take_content(now)`, `statistics_mut`)
- Station: entrance only when Idle, not stopped, not locked; exit when Idle or Blocked and not stopped, never Processing
- Station stops: random failure, forced failure, Pause, each its own flag; first interruption freezes work, last to end resumes; completion time = entry + process time + time stopped, overlaps once
- `state()` reports the most serious interruption as `stop` (Failure > ForcedFailure > Pause) and all of them, with since when, in `interruptions`
- `FailureProfile::new(availability, mttr_distribution, seed)`; MTBF mean = mttr_mean / (1/availability - 1); 1.0 = never fails; distributions CONSTANT, UNIFORM, NORMAL, ERLANG
- Source creates the first MU at Init, the next once one moves; never accepts MUs; no stops yet
- Drain: entrance always open, records `MuLeft` and exit statistics, keeps no MU, handles no events

#### Invariants
- Every object calls `update_statistics` at the end of everything it handles
- Every object implements `reset` fully, incl. starting MUs; new fields need reset tests
- A second random failure while failed is discarded; random failures continue under a forced one

#### Pitfalls
- Buffer is broken: loses MUs, never notifies predecessors, ignores a retry's `from`

#### Code locations
- `src/materialflow_objects/model_object.rs`, `flows.rs`, `source.rs`, `station.rs`, `buffer.rs`, `drain.rs`
- `src/random_numbers.rs`, `src/statistics.rs`

### Live Session & UI

#### Facts
- `LiveSession` owns a thread that builds the run from a closure, computes ahead into a buffer of instants, and releases by a `Clock`; the user sees shown time, the run is at computed time (ADR 0002)
- Commands: start, halt, reset, set_speed, set_time_mode, go_back, step_forward, step_back, locate_step_back, set_reset_keeps_changes, set_end_time, request_state, pause, resume, force_failure, end_forced_failure, set_process_time, clear_statistics
- Outputs: `Occurrences`, `ShownTime`, `State` (only at Init, after Reset, after going back, on request), `Status`, `Error`, `StepBackTarget { shown, to }` (answer to `locate_step_back`, changes nothing)
- UI step back: `ViewState::step_back()` gives `Now`, `Confirm(to)` or `Locate`; on `Locate` the window halts, sends `locate_step_back`, and `located_step_back()` decides once answered; any other command cancels
- Time modes: `Continuous`, `Skipping` (no gap longer than 0.25 s wall time by default)
- Interventions act at shown time, discard the computed future and go in an intervention log; Pause and forced failure reach the run through `schedule_event`
- Checkpoint = clone of the run + log position; going back restores the nearest earlier one and replays with recording off; every 100,000 events within 256 MB, thinning the older half, Init's always kept
- `Picture`: what the user knows at shown time, fed every output in order; statistics and waiting predecessors only from a full state
- UI (`ui/`): Bevy + egui; `--models <folder>` (required, no built-in demo) lists model files (problems marked and shown instead of running) and runs the one clicked, through the catalogue as `Source::Ui` holding `.crabsim/user.lock`; nothing open until a model is chosen or when the folder is refused; inspector attributes before Init or after Reset via `get_attributes`/`edit_model`, each restored on its own, "Change anyway" as a one-change take-over; unplaced instances laid out (columns by flow, or a row below), never saved; seed (before Init or after Reset) and end time in the top bar; open file's changes picked up by `Workspace::check` (size and mtime, then hash), used at once with no run in progress or on Reset, outside changes marked with Adopt, a watcher thread waking an idle window; 60x real time, 2D flat meshes, controls, inspector, intervention marks, MUs in holders when zoomed in and slow
- UI logic in plain Rust: `attributes.rs` (inspector fields), `workspace.rs` (folder, models, layout, seed, sessions), `view.rs` (view state), `camera.rs`, `marks.rs`; `main.rs` only draws

#### Invariants
- Sending never blocks the session thread; past 256 unread outputs it stops releasing, drops nothing
- Engine never depends on Bevy; Picture has no Bevy types; panels are plain functions of `&mut egui::Ui`
- Nothing in a UI may affect results

#### Code locations
- `src/live_session.rs`, `src/picture.rs`, `ui/src/view.rs`, `ui/src/main.rs`

---

### Operation Catalogue

#### Facts
- `crabsim-catalogue` (`catalogue/`): every capability as an `Operation` (NAME, DESCRIPTION, EFFECT, serde + JsonSchema Input/Output, `run`); front-ends use only `operations()` and `call(name, json)`
- Front-ends: `crabsim-mcp` (`mcp/`, rmcp `=3.9.0`, stdio, not a default member) and the CLI's `run`, `runs`, `results`, `query` subcommands
- Errors: `OperationError { kind, message, problems }`; MCP returns them as tool results marked as errors
- Slice 1 operations: validate/list/read/write/copy/delete_model, start_run, run_status, cancel_run, run_results, query_results, label_run, forget_run
- Jobs: pool of one thread per core, one observation per thread, chunks of 100,000 events, jobs take turns; observation k seed = `splitmix64(seed + k)`
- Store: `<models>/.crabsim/results.sqlite`, WAL, tables runs/observations/statistics (long and thin), ADR 0010

#### Invariants
- Operations touch only `.json` files inside the models folder and `.crabsim/`; never code, commands or the network
- A models folder with `Cargo.toml`, `.git` or `.rs` is refused; paths must stay inside after following links
- A run error fails only its observation; a run is `failed` only if every observation failed
- `query_results` can't write, attach a database, return more than 1,000 rows or run past 5 s

#### Pitfalls
- A run whose process ended first is marked `interrupted` (lock files in `.crabsim/processes/`, `runs.process`, tables version 2), when the store opens and in run_status, cancel_run and forget_run
- Seeds are TEXT in the store: they use all 64 bits
- Debug test builds run the engine unoptimised: keep test runs short

#### Code locations
- `catalogue/src/`: `lib.rs` (Catalogue, Operation), `error.rs`, `folder.rs`, `model_files.rs`, `jobs.rs`, `results.rs`, `store.rs`, `process.rs`
- `mcp/src/main.rs`; `cli/src/main.rs`, `cli/src/tables.rs`; design in `docs/design/operation-catalogue.md`

## GUIDES

### Running crabsim

#### Facts
- CLI: `cargo run --release --bin crabsim -- --seed 7 --length 365d` runs the demo headless (or a model file with `--model models/demo.json`) and prints per-object statistics, Drain throughput and lead time, events and wall time
- `--length` uses the UI's time syntax (`90`, `1:30`, `8:00:00`, `30d 12:00:00`); defaults: seed 0, length `1d`
- Experiments: `crabsim run --models <folder> --model line.json --length 30d --observations 10`, then `runs`, `results <run>`, `query "<sql>"`; results kept in `<folder>/.crabsim/results.sqlite`
- MCP server: `cargo build --release -p crabsim-mcp`, run as `crabsim-mcp --models <folder>` over stdio
- UI: `cargo run -p crabsim-ui --profile ui` (first build several minutes)
- In code: `demo::model().build(seed)`, `init`, `advance_to`, `state`; `set_recording(Recording::ALL)` for occurrences
- Benchmark: `cargo bench --bench reference` (about a minute; demo and 6,006 objects)
- `RUST_LOG=info` logs every event (about 42,000 a simulated day for the demo)

#### Pitfalls
- Debug builds are about 10x slower; use release for long lengths
- A run error still prints statistics, then exits with failure

#### Code locations
- CLI — `cli/src/main.rs`; MCP server — `mcp/src/main.rs`; catalogue — `catalogue/`; model files — `model/`, `models/demo.json`; demo — `src/demo.rs`; benchmark — `benches/reference.rs`

### Adding a New Object Type

#### Facts
- Struct: `BaseObject`, ids for successors and `blockedlist`, `StatisticsKeeper`, default-state fields; must be `Clone` (checkpoints)
- Implement `EventInterface` (incl. `reset`, `report`, `clear_statistics`), `StopCtrl`, `MaterialFlow`, and `SuccessorContainer` if it has successors
- Wire in: `materialflow_objects/mod.rs`, `ModelObject` variant + `on_the_object!`, `ObjectKind` + `ModelDescription::kind/successors` + `Dimensions::default_for`, an `add_` method, `connect`

#### Invariants
- Call `update_statistics` after every event; record occurrences via `context.record`; MU ids via `context.new_mu_id()`
- Reach others only through `OtherObjects`
- Send retries whenever room frees up; on `BlockedRetry` act only if exit allowed and holding an MU, offering only `from`
- Uncancellable events must be recognisable as stale (like `ExitTicket`)

#### Pitfalls
- Check the agreed design first: Station capacity is a setting, not a new type (ADR 0001)
- Reset tests are required; same-seed replay can't catch leftover events

#### Code locations
- `src/materialflow_objects/flows.rs`, `model_object.rs`, `src/model.rs`; example: `station.rs`

### Writing Tests

#### Facts
- `tests/simulation_run.rs`: run behaviour via Init, advance, Reset, occurrences, state; helpers `secs`, `no_failures`, `station`, `preloaded_station`, `recorded`, `exit_ids`, `exit_times`, `statistics_of`
- `tests/live_session.rs`: commands and outputs with `FakeClock` and `sync`; Picture checked against the run
- `model/tests/model_file.rs`: model files, text in, a model, problems or a run out
- `cli/tests/cli.rs`: the binary end to end
- Unit tests in `src/`: failures part way through an event (`RecordsThenFails`), session buffer limit and thinning
- `ui/tests/`: view state against a real session, no window
- Build models through `ModelDescription`; `availability = 1.0` unless testing failures; `Recording::ALL` to read occurrences

#### Invariants
- Check every test can fail by breaking the code on purpose
- Every new field gets a reset test
- No speed or timing assertions
- Refactors of scheduling must keep every occurrence identical (hash against master)

#### Code locations
- `tests/simulation_run.rs`, `tests/live_session.rs`, `model/tests/model_file.rs`, `cli/tests/cli.rs`
- Long run: `cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model`

---

## REFERENCE

### Event Types

#### Facts
- `MUExit(ExitTicket)`: work done; Station ignores it if stopped or stale, else `WorkFinished` + move; Source moves and creates the next MU
- `BlockedRetry { from }`: successor `from` has room; offered to `from` only; runs first at its time
- `StartFailure`/`EndFailure`: random failures; first scheduled by `init`, each `EndFailure` schedules the next; repair time sampled
- `ForceFailureOn`/`ForceFailureOff`: user-forced failure, own flag, counts as failure
- `PauseOn`/`PauseOff`: planned stop, not against availability
- Defined but unused: `MUEntrance`, `Finished`
- `EventTemplate { eventtype, exec_time, object_id }` (relative); `ScheduledEvent { time, seq, object_id, eventtype }` (32 bytes)

#### Invariants
- Order: earliest time; at one time `BlockedRetry` first; then lowest `seq`
- Events are engine internals; views see only occurrences

#### Code locations
- `src/event.rs`; handling in `src/materialflow_objects/station.rs`, `source.rs`

### Configuration Flags

#### Facts
- CLI (`cli/src/main.rs`): `--model <FILE>` (default: the built-in demo), `--seed <N>` (default 0), `--length <TIME>` (default `1d`, UI time syntax)
- `RUST_LOG` controls `env_logger`; per-event `info` logging is slow
- Profiles: `dev` (deps optimised), `release` (thin LTO, 1 codegen unit, `panic = unwind` for `LiveSession`), `ui` (release without LTO)
- Rust version pinned in `rust-toolchain.toml`
- `SessionSettings`: `buffer_instants` 100,000; `checkpoint_spacing` 100,000 events; `checkpoint_budget_bytes` 256 MB; `time_mode` Continuous; `reset_keeps_changes` off
- A session starts halted at speed 1; the UI uses 60x

#### Code locations
- `cli/src/main.rs`, `src/live_session.rs`, `Cargo.toml`
