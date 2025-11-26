## Quick orientation

This repository implements a compact discrete-event material-flow simulator in Rust. The code is organized as a small library (top-level `src/` modules) with a binary entrypoint that calls `crabsim::run_demo()`.

Keep these core concepts in mind when changing or extending behavior:

- Simulator: `src/simulator.rs::Simulator` is the authoritative registry of material-flow objects and drains. It assigns numeric `object_id`s via `register_material_flow` and exposes `print_materialflow_state` used by the live terminal output.
- Event controller / queue: `src/simulator.rs::EventController` holds a `BinaryHeap<ScheduledEvent>` and uses a sequence counter (`seq`) to preserve FIFO ordering for events scheduled at the same time.
- Material-flow objects: trait `MaterialFlow` and `EventInterface` are implemented by the concrete types in `src/materialflow_objects/` (`Source`, `SingleStation`, `Drain`). See `src/materialflow_objects/flows.rs` for the movement helpers and trait definitions.
- Movement semantics: the core helper is `try_to_move_to_successor(_...)` / `try_to_move_to_successor_with_strategy` (round-robin/default or alternate strategies). It takes `MoveableUnit`s from a predecessor and attempts entrance into successors, recording blocked predecessors when entrances are denied.

## Files to inspect first (good entry points)

- `README.md` — short project overview and runnable examples (PowerShell commands).
- `src/lib.rs` — re-exports, the `run_demo()` demo runner and unit tests. Good place to see end-to-end usage.
- `src/simulator.rs` — Simulator and EventController logic (scheduling, time advancement, printing state).
- `src/materialflow_objects/flows.rs` — traits (`MaterialFlow`, `EventInterface`), `MoveableUnit`, distribution strategies, and `try_to_move_to_successor` logic.
- `src/materialflow_objects/{source.rs,station.rs,drain.rs}` — concrete implementations and event handling.

## Developer workflows and important commands

- Build: `cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml`
- Run demo (PowerShell):
  - `$env:RUST_LOG='info'; cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml`
  - `RUST_LOG` controls `env_logger` output. Use `crabsim=debug` for crate-level debugging.
- Tests with logs: `$env:RUST_LOG='info'; cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture`

Notes: the demo runner calls `env_logger::init()` (see `src/lib.rs::run_demo`) so logs will appear when running the binary.

## Project-specific patterns and conventions

- Single-file design split into modules: while code lives under `src/` with small modules, internal APIs are Rust-native (traits + Rc<RefCell<...>>). Expect `Rc<RefCell<T>>` wrappers for shared mutable objects.
- Object id assignment: `Simulator::register_material_flow` assigns sequential `object_id` numbers and stores the object in `material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>`. When modifying registration or wiring, update callers that rely on stable ids in tests.
- Event ordering: events scheduled at the same EcTime are disambiguated by a `seq` (see `EventController::add_event`). Preserve seq behavior when changing scheduling to keep deterministic tests.
- Movement routing: `try_to_move_to_successor_with_strategy` implements pluggable strategies (round-robin default). Modifying movement logic should preserve these semantics: advancing `succ_pointer`, recording blocked predecessors via `add_blocked`, and returning an `EventTemplate` when appropriate.
- State transitions and blocked semantics: `ProcessState` (Processing, Blocked, Idle) is used extensively; tests rely on blocked/retry behavior (see tests in `src/lib.rs`). Keep blockedlist dedup semantics when updating `add_blocked` behavior.

## Integration & dependencies

- Rust crates used: `splay_tree` (priority queue helper), `log` + `env_logger`. No external network or system integrations.
- Logging: use `log::info!` / `log::debug!` consistent with existing calls. The demo initializes `env_logger` so prefer crate-level filters via `RUST_LOG`.

## Common change patterns and where to update

- Adding a new MaterialFlow type: implement `EventInterface` + `MaterialFlow` and expose it via `src/materialflow_objects/mod.rs`. Register instances with `Simulator::register_material_flow` and wire successors as `Rc<RefCell<dyn MaterialFlow>>`.
- Changing event timing or scheduling: modify `EventController::add_event` / `run`. Preserve stable FIFO `seq` assignment to avoid breaking deterministic tests.
- Changing the live output: `Simulator::print_materialflow_state` is the single place to change the text-based snapshot printed after each event.

## Useful code examples (copyable patterns)

- Registering and wiring objects (from `src/lib.rs::run_demo`):

  - Create stations and wrap with `Rc<RefCell<...>>`.
  - Call `simulator.register_material_flow(obj.clone())` for each object.
  - Connect successors via `station.borrow_mut().succ = vec![other.clone(), ...];`

- Scheduling an event manually (tests):

  - `eventcontroller.add_event(EcTime::from(100), EventTemplate { eventtype: EventType::MUExit, exec_time: EcTime::from(0), object_id: b_id, seq: 0 });`

## Tests and behavior guarantees the agent should preserve

- Many unit tests in `src/lib.rs` assert ordering, blocked-retry semantics, drain FIFO, and throughput characteristics. When refactoring core scheduling or movement logic, run tests and keep these behaviors intact.

## If you need to make larger changes

- Split `src/main.rs` into modules gradually: keep public API in `src/lib.rs` (already done) and move concrete types into `src/materialflow_objects/*` (already split). Add new modules rather than collapsing logic into `main`.
- Consider adding a small CLI (e.g. `clap`) to toggle `live_inplace` and log-level. If you add runtime flags, keep default behavior unchanged for tests and CI.

---

If any section is unclear or you'd like more examples (e.g., common refactors, test harness additions, or a short walkthrough of `try_to_move_to_successor`), tell me which part and I'll iterate. 
