---
title: Running crabsim
section: guides
related:
  - configuration-flags
  - live-session-and-ui
---

# Running crabsim

There are four ways to run a model: the headless CLI, experiments on a models folder through the CLI or the MCP server, the UI, and a `SimulationRun` in your own code. The CLI runs a model file or the built-in demo, and the UI runs the demo from `src/demo.rs`.

## Headless CLI

Runs the demo model with a seed for a simulated length, then prints each object's statistics, each Drain's throughput and mean lead time, and how many events it ran in how long.

```bash
cargo run --release --bin crabsim
cargo run --release --bin crabsim -- --seed 7 --length 365d
cargo run --release --bin crabsim -- --model models/demo.json --length 30d
```

An invalid model file prints every problem in it, one per line with where it is, and runs nothing. `models/demo.json` is the demo model as a file, and `docs/design/model-file.md` describes the format.

`--length` is written as the UI shows time: `90`, `1:30`, `8:00:00` or `30d 12:00:00`. The same seed always gives the same statistics. A run error still prints the statistics up to it, then exits with failure. Build in release for long lengths: a debug build is about ten times slower.

## Experiments on a models folder

With a models folder, the CLI runs experiments of several observations in parallel and keeps their results in the folder's results store, `.crabsim/results.sqlite`, the same store an agent uses through the MCP server. Keep the folder outside the repository: a folder holding source code is refused.

```bash
cargo run --release --bin crabsim -- run --models my-models --model line.json --length 30d --warm-up 1d --observations 10 --label baseline
cargo run --release --bin crabsim -- runs --models my-models
cargo run --release --bin crabsim -- results --models my-models run-1 --class Station
cargo run --release --bin crabsim -- query --models my-models "SELECT id, label, state FROM runs"
```

Each observation's seed is printed with its results, and `crabsim --model <file> --seed <that seed>` reruns it alone. See [Operation Catalogue](../architecture/operation-catalogue.md).

## MCP server

```bash
cargo build --release -p crabsim-mcp
```

Then point a harness at `target/release/crabsim-mcp` with `--models <folder>`, over stdio. Every operation is a tool.

## UI

```bash
cargo run -p crabsim-ui --profile ui
```

The `ui` profile is optimised without LTO, so it rebuilds in seconds. The first build compiles Bevy and takes several minutes. See [Live Session & UI](../architecture/live-session-and-ui.md) for what it shows.

## In your own code

```rust
let model = crabsim_model::load(&std::fs::read_to_string("models/demo.json")?).expect("a valid model");
let mut run = model.to_description().build(seed);   // or crabsim::demo::model().build(seed)
run.init()?;
run.advance_to(SimTime::from_secs(86_400)).map_err(|failure| failure.error)?;
let state = run.state();   // each object's ObjectState and statistics
```

Turn on recording (`run.set_recording(Recording::ALL)`) to get occurrences back from each advance. A headless run records nothing by default.

## Logging

The engine logs every event at `info` level through `env_logger`, which the CLI starts. Set `RUST_LOG` to see it, but only for short runs: the demo model logs about 42,000 events a simulated day, and 15.5 million a year.

| Value | Effect |
|---|---|
| `info` | Every event, MU moves and exits |
| `debug` | Also each refused successor |
| `crabsim=debug` | The same, for this crate only |

The UI filters this logging out, because Bevy's logger would otherwise print it.

## Benchmark

```bash
cargo bench --bench reference
```

About a minute. It runs a copy of the demo model and 429 copies side by side (6,006 objects), and reports events per second, simulated time per wall second and peak memory. The baseline is in `docs/design/first-profile.md`.
