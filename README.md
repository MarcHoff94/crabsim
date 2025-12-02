# crabsim

A compact Rust discrete-event material-flow simulator (library + demo binary) with a separate live renderer.

It models Moveable Units (MUs) produced by a `Source`, processed by one or more `SingleStation`s, and consumed by a `Drain`. A central `Simulator` holds the registered objects and an `EventController` advances time via a priority queue with FIFO tiebreaking.


## Quick Start (Windows PowerShell)

```powershell
# Build
cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run demo (prints state snapshots after each event)
$env:RUST_LOG = 'info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim

# Run demo with in-place table updates in this terminal
$env:RUST_LOG = 'info'
# via CLI: inplace mode
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim -- --inplace
# or env var
$env:CRABSIM_LIVE_INPLACE = '1'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim

# Run demo with external renderer window (maximized) at 5x speed
$env:RUST_LOG = 'info'
# via CLI: renderer window at 5x speed, hold 1s after finish
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim -- --renderer-window --speed 5 --hold 1
# or env vars
$env:CRABSIM_RENDERER = '1'
$env:CRABSIM_RENDERER_SPEED = '5'
$env:CRABSIM_RENDERER_HOLD = '1'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim

# Run tests
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run tests with logs
$env:RUST_LOG = 'info'
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```

Notes:
- The demo binary calls `crabsim::run_demo()` which initializes `env_logger` so `RUST_LOG` controls runtime logs.
- Set `RUST_LOG=crabsim=debug` for crate-only verbose logs.
- External renderer launch on Windows uses PowerShell `Start-Process -WindowStyle Maximized` with a `cmd /C start /MAX` fallback.
- CLI flags available: `--inplace`, `--renderer-window`, `--speed <factor>`, `--hold <seconds>`.
- Environment variables (equivalents): `CRABSIM_LIVE_INPLACE`, `CRABSIM_RENDERER`, `CRABSIM_RENDERER_SPEED`, `CRABSIM_RENDERER_HOLD`.


## Architecture Overview

- `Simulator` (`src/simulator.rs`): Authoritative registry of material-flow objects and drains. Assigns object IDs on registration and prints a text snapshot after each event via `print_materialflow_state`. When a renderer is connected, it sends JSON snapshots and the static topology.
- `EventController` (`src/simulator.rs`): Discrete-event engine with a `BinaryHeap<ScheduledEvent>`. Uses a monotonically increasing sequence (`seq`) to preserve FIFO order for events scheduled at the same time.
- Material-flow objects (`src/materialflow_objects/`):
	- `Source`: creates MUs at a configured processing interval and attempts to transfer them to successors.
	- `SingleStation`: processes one MU at a time and forwards it. Tracks entrance/exit locks, blocked state, and blocked predecessors.
	- `Drain`: consumes MUs, records exit times and IDs.
- Movement semantics (`src/materialflow_objects/flows.rs`):
	- `try_to_move_to_successor` (round-robin by default) attempts to transfer an MU from a predecessor to the first successor that accepts entrance, advancing `succ_pointer` and recording blocked predecessors when denied.
	- Events: `MUExit` (processing-complete transfer attempt) and `BlockedRetry` (successor notified it freed up). They are distinct to preserve scheduling vs. reactive retry semantics.


## Visualization

- Topology visualizer (`src/visualization/topology.rs`):
	- `print_network_topology(...)` prints the ASCII network topology locally.
	- `render_network_topology(...) -> String` returns a string with branches and connectors for the renderer.
- State visualizer (`src/visualization/state.rs`):
	- Snapshot structs (`StateRow`, `StateSnapshot`) and helpers to collect and render state tables.
	- Supports in-place redraw in the same terminal via ANSI/VT sequences (Windows VT is enabled automatically).

## External Renderer

- Binary `crabsim-renderer` (built alongside the demo) displays:
	- Header, static topology string, then the live state table.
	- It consumes newline-delimited JSON messages from the simulator:
		- `{ "kind": "topology", "text": "..." }`
		- `{ "kind": "snapshot", "snapshot": { ... } }`
	- Pacing: 1 simulated second maps to `1 / speed` wall seconds; use `CRABSIM_RENDERER_SPEED` to adjust.
- Launching:
	- Set `CRABSIM_RENDERER=1` to auto-spawn and connect.
	- On Windows, the renderer opens as a maximized console window.

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
- `src/simulator.rs` — `Simulator`, `EventController`, scheduling, state printing, and renderer JSON emission.
- `src/materialflow_objects/` — concrete types and movement traits:
	- `flows.rs` — traits (`MaterialFlow`, `EventInterface`, `SuccessorContainer`), `MoveableUnit`, strategies, and `try_to_move_to_successor`.
	- `source.rs`, `station.rs`, `drain.rs` — implementations.
	- `src/visualization/` — topology and state rendering helpers used by both console and renderer.


## Extending

- Add a new material-flow type by implementing `EventInterface` + `MaterialFlow`, exposing it in `materialflow_objects::mod`, registering with `Simulator`, and wiring successors (`Rc<RefCell<dyn MaterialFlow>>`).
- Preserve `EventController` seq tiebreaking to keep deterministic tests.
- Keep `try_to_move_to_successor` semantics (advance `succ_pointer`, record blocked predecessors, return event when appropriate).

If you want a small CLI (e.g., `clap`) to toggle log level or runtime options, or structured logs via `tracing`, open an issue and we can add it without changing default behavior.

## Troubleshooting

- Renderer window doesn’t appear:
	- Ensure `CRABSIM_RENDERER='1'` is set before `cargo run`.
	- Check PowerShell execution: the demo uses `Start-Process` and falls back to `cmd /C start /MAX`.
	- Verify Windows allows console VT sequences (for in-place mode) — modern terminals do; classic `cmd` also works.
- No topology in renderer:
	- The simulator sends topology immediately after connecting; ensure the renderer is running and connected.
- Too fast/slow updates:
	- Adjust `CRABSIM_RENDERER_SPEED` (e.g., `1` for realtime, `5` for faster).
