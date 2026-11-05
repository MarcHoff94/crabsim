---
title: Running the Demo
section: guides
related:
  - configuration-flags
  - visualization-and-renderer
---

# Running the Demo

## Three modes

### Plain (default)

Prints a new state table to stdout after each event.

```bash
RUST_LOG=info cargo run --bin crabsim
```

### In-place

Rewrites the same table in the terminal using ANSI escape sequences — gives a live-updating dashboard.

```bash
# Via CLI flag
RUST_LOG=info cargo run --bin crabsim -- --inplace

# Via env var
CRABSIM_LIVE_INPLACE=1 RUST_LOG=info cargo run --bin crabsim
```

### External renderer window

Spawns a second terminal running `crabsim-renderer`, connected via TCP, displaying the simulation at a configurable playback speed.

```bash
# At 5× speed, hold window open 1s after finish
RUST_LOG=info cargo run --bin crabsim -- --renderer-window --speed 5 --hold 1

# Via env vars
CRABSIM_RENDERER=1 CRABSIM_RENDERER_SPEED=5 CRABSIM_RENDERER_HOLD=1 RUST_LOG=info cargo run --bin crabsim
```

Use `--speed -1` for unpaced mode: renders each simulation second immediately (fast-forward) then enters hold-open.

## Logging

`run_demo()` calls `env_logger::init()`. Control verbosity via `RUST_LOG`:

| Value | Effect |
|---|---|
| `info` | MU moves, exit times, simulation runtime |
| `debug` | Blocked predecessor registration details |
| `crabsim=debug` | Debug only for this crate |

## Build first

```bash
cargo build
```

The renderer binary (`crabsim-renderer`) is built alongside the main binary. Both must be built before using renderer window mode. `cargo run` builds automatically.

## After the run

The demo prints simulation runtime in milliseconds and the exit interval list from the drain:

```
simulation runtime: 1.23 ms (renderer=false, launcher=n/a)
```
