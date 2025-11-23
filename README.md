# crabsim

A small Rust discrete-event material-flow simulator used for prototyping and testing process/network semantics.

This repository contains a compact single-file simulator (`src/main.rs`) that implements:

- Simulator: authoritative registry for material-flow objects and drains.
- EventController: discrete-event queue implemented with `BinaryHeap` and stable FIFO tiebreaking via a sequence number.
- MaterialFlow trait and concrete types: `Source`, `SingleStation`, and `Drain`.
- MoveableUnit (MU) lifecycle: created by `Source`, moved through `SingleStation`s, and consumed by `Drain`.
- Live in-place terminal state printer (optional) to view object states after each event.


## Requirements

- Rust toolchain (1.70+ recommended or current stable)
- Cargo to build and run


## Build

From the repository root (PowerShell):

```powershell
# build
cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# run
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
```


## Logging

This project uses the `log` crate with `env_logger` for runtime logging. There are two kinds of output:

- Interactive live-inplace table printed directly to the terminal (kept as `println!`s in `print_materialflow_state`) when `Simulator::enable_live_inplace(true)` is set.
- Diagnostic and lifecycle logs via `log::info!` / `log::debug!` calls.

Control log verbosity using the `RUST_LOG` environment variable in PowerShell. Examples:

```powershell
# Show info and above (default useful runtime view)
$env:RUST_LOG = 'info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Show debug for only this crate (very verbose)
$env:RUST_LOG = 'crabsim=debug'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# When running tests and you want to see logs
$env:RUST_LOG = 'info'
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```

If you want both a live terminal UI and full log output, consider writing logs to a file (future enhancement) to avoid cluttering the terminal UI.


## Live-inplace UI

The simulator prints the material-flow state after each processed event. The output is printed sequentially (one state snapshot per event). If you later want an in-place UI that clears and refreshes the terminal instead of appending, we can add a toggle for that, but for now sequential printing is used for simplicity and portability.


## Tests

Run unit tests with cargo:

```powershell
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
```

To see logging output during tests:

```powershell
$env:RUST_LOG = 'info'
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```


## Project layout (single-file)

- `Cargo.toml` — dependencies and metadata. Current deps: `splay_tree`, `log`, `env_logger`.
- `src/main.rs` — all code: types, traits, simulator, controller, and tests.

Key symbols you may want to inspect:

- `Simulator` — registry and live-inplace control
- `EventController` — event queue and run loop
- `EventTemplate`, `ScheduledEvent` — event representation and scheduling
- `MaterialFlow` trait and `SingleStation`, `Source`, `Drain`


## Next suggestions

If you'd like, I can:

- Add a small CLI (via `clap`) to toggle `live_inplace` and set default log level from command-line flags.
- Configure `env_logger` or a custom writer to write logs to a file while keeping the terminal for the live UI.
- Migrate to `tracing`/`tracing-subscriber` for structured logs (JSON output, better downstream processing).
- Split `src/main.rs` into modules for maintainability as the simulator grows.

Tell me which of the above you'd like next and I can implement it and run tests.
