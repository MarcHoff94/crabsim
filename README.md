# crabsim

A compact Rust discrete-event material-flow simulator (library + demo binary).

It models Moveable Units (MUs) produced by a `Source`, processed by one or more `SingleStation`s, and consumed by a `Drain`. A central `Simulator` holds the registered objects and an `EventController` advances time via a priority queue with FIFO tiebreaking.


## Quick Start (Windows PowerShell)

```powershell
# Build
cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run demo (prints state snapshots after each event)
$env:RUST_LOG = 'info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run tests
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run tests with logs
$env:RUST_LOG = 'info'
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```

Notes:
- The demo binary calls `crabsim::run_demo()` which initializes `env_logger` so `RUST_LOG` controls runtime logs.
- Set `RUST_LOG=crabsim=debug` for crate-only verbose logs.


## Architecture Overview

- `Simulator` (`src/simulator.rs`): Authoritative registry of material-flow objects and drains. Assigns object IDs on registration and prints a text snapshot after each event via `print_materialflow_state`.
- `EventController` (`src/simulator.rs`): Discrete-event engine with a `BinaryHeap<ScheduledEvent>`. Uses a monotonically increasing sequence (`seq`) to preserve FIFO order for events scheduled at the same time.
- Material-flow objects (`src/materialflow_objects/`):
	- `Source`: creates MUs at a configured processing interval and attempts to transfer them to successors.
	- `SingleStation`: processes one MU at a time and forwards it. Tracks entrance/exit locks, blocked state, and blocked predecessors.
	- `Drain`: consumes MUs, records exit times and IDs.
- Movement semantics (`src/materialflow_objects/flows.rs`):
	- `try_to_move_to_successor` (round-robin by default) attempts to transfer an MU from a predecessor to the first successor that accepts entrance, advancing `succ_pointer` and recording blocked predecessors when denied.
	- Events: `MUExit` (processing-complete transfer attempt) and `BlockedRetry` (successor notified it freed up). They are distinct to preserve scheduling vs. reactive retry semantics.


## Logging

The crate uses `log` + `env_logger`.

- Live state: `Simulator::print_materialflow_state` prints a compact table after each processed event.
- Logs: `info!` for lifecycle events and `debug!` for routing/blocked details.

Examples (PowerShell):

```powershell
# Show info and above
$env:RUST_LOG = 'info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Verbose for this crate only
$env:RUST_LOG = 'crabsim=debug'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
```


## Tests and Guarantees

Unit tests live in `src/lib.rs` and cover:

- Ordering across stations and across parallel branches
- Blocked retry semantics and dedup behavior
- Drain FIFO ordering and throughput characteristics

Run with:

```powershell
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
$env:RUST_LOG = 'info'; cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```


## Project Layout

- `Cargo.toml` — dependencies (`splay_tree`, `log`, `env_logger`) and metadata.
- `src/lib.rs` — public API, re-exports, `run_demo()` and unit tests.
- `src/main.rs` — binary entrypoint that invokes `run_demo()`.
- `src/simulator.rs` — `Simulator`, `EventController`, scheduling and state printing.
- `src/materialflow_objects/` — concrete types and movement traits:
	- `flows.rs` — traits (`MaterialFlow`, `EventInterface`, `SuccessorContainer`), `MoveableUnit`, strategies, and `try_to_move_to_successor`.
	- `source.rs`, `station.rs`, `drain.rs` — implementations.


## Extending

- Add a new material-flow type by implementing `EventInterface` + `MaterialFlow`, exposing it in `materialflow_objects::mod`, registering with `Simulator`, and wiring successors (`Rc<RefCell<dyn MaterialFlow>>`).
- Preserve `EventController` seq tiebreaking to keep deterministic tests.
- Keep `try_to_move_to_successor` semantics (advance `succ_pointer`, record blocked predecessors, return event when appropriate).

If you want a small CLI (e.g., `clap`) to toggle log level or runtime options, or structured logs via `tracing`, open an issue and we can add it without changing default behavior.
