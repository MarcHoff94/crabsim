# crabsim

A discrete-event simulator for any material flow, such as a factory line, written in Rust. A model is made of Sources, Stations, Buffers and Drains that create, work on, hold and take out moveable units (MUs). Stations fail at random, can be paused or forced to fail, and keep statistics such as busy, blocked and failed time, lead time and throughput.

crabsim exists to simulate long periods as fast as possible: a year of the demo model takes about 1.2 seconds, at about 13 million events per second, and a run's memory stays flat however long it is. A live session can also play a run at a chosen speed, step forward and back through it, and take interventions while it plays, for a UI to show.

**Status:** the engine and live session work. The UI's first stage works: a Bevy application with egui panels (ADR 0006) that plays the demo model live in 2D, with the run controls, stepping back and forward, interventions on Stations and an inspector. Building models in it comes later. Buffer's movement is known to be broken, so models don't use it yet. The plan is in `docs/agents/handoff.md` and in [spec issue 3](https://github.com/MarcHoff94/crabsim/issues/3).

## Requirements

Rust, installed through [rustup](https://rustup.rs). The repo pins its Rust version in `rust-toolchain.toml`, so rustup fetches the right one by itself the first time you build. On Windows you also need the Visual Studio C++ build tools, which the rustup installer offers.

## Quick start

The commands are for Windows PowerShell, one per line. Run them in the repo's folder.

Run the tests:

```powershell
cargo test
```

Run the demo model headless for a simulated day, then print each object's statistics and how long the run took:

```powershell
cargo run --release --bin crabsim
```

The same with a seed and a simulated length, written as the UI shows time, such as `1:30`, `8:00:00` or `30d 12:00:00`:

```powershell
cargo run --release --bin crabsim -- --seed 7 --length 365d
```

Run a model file. `models/demo.json` is the demo model as a file, and `docs/design/model-file.md` describes the format:

```powershell
cargo run --release --bin crabsim -- --model models/demo.json --length 30d
```

Run the benchmark, which takes about a minute and reports events per second, simulated time per wall second and peak memory:

```powershell
cargo bench --bench reference
```

Open the UI, which plays the demo model live. The first build takes several minutes:

```powershell
cargo run -p crabsim-ui --profile ui
```

The engine logs every event at `info` level. To see that while running the demo or the tests, set `RUST_LOG` first:

```powershell
$env:RUST_LOG = 'info'
```

## Trying the UI prototype

The prototype is throwaway code on the branch `prototype/ui-bevy`, never to be merged. It drew 6,006 objects at hundreds of frames per second and drove a real live session, to settle the UI's technology. Its results are in `docs/design/ui-prototype.md`.

It needs a clean working tree, because it switches branches. Commit or stash your changes first.

1. Switch to the prototype's branch and go into its folder:

   ```powershell
   git switch prototype/ui-bevy
   ```

   ```powershell
   cd ui-prototype
   ```

2. Start it. This one runs a real session of 429 copies of the demo line, zoomed in on one copy, at 60 times real time, so you can watch MUs move:

   ```powershell
   cargo run --release -- --mode session --render meshes --speed 60 --close
   ```

   The first build takes about 5 minutes, unless a build is already on disk.

3. When you're done, close the window and go back:

   ```powershell
   cd ..
   ```

   ```powershell
   git switch master
   ```

**In the window:**
- Drag with the right or middle mouse button to pan, and use the wheel to zoom.
- Click an object to select it and see it in the inspector, or drag it to move it.
- Drag a button from the palette on the left into the model to add an object.
- Use Start and Halt in the top bar.

The top bar shows frames per second, shown time and the speed achieved.

**Other ways to run it**, each after `cargo run --release --`:

| Options | What it shows |
|---|---|
| `--mode session --render meshes --speed 3600` | The whole model at 1 simulated hour per second, with states as colours |
| `--mode scale --render sprites` | A stress test with no simulation: 6,006 objects changing colour and 5,000 gliding markers |
| `--mode scale --render meshes --close` | The same stress test zoomed in, with labels |
| `--mode windowless --speed 3600 --seconds 10` | The session alone, with no window, printing the speed it reached |

More options:
- **`--render sprites` or `--render meshes`:** sprites, or flat meshes under a 3D camera.
- **`--lines N`:** copies of the demo line.
- **`--markers N`:** the most MUs drawn at once.
- **`--seconds N`:** quit after that long and print frame times.
- **`--halt-after N`:** halt after that long.
- **`--screenshot path.png`:** save a screenshot, with `--seconds`.
- **`--vsync`:** ask for vsync.

In this version MUs jump straight to their next object, because a move takes no simulation time. The gliding markers in the stress test stand in for a future conveyor.

## Using it as a library

A headless run, which records nothing and keeps memory flat:

```rust
use crabsim::random_numbers::Distribution;
use crabsim::{FailureProfile, ModelDescription, SimTime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = ModelDescription::new();
    let source = model.add_source(SimTime::from_secs(5));
    // Available 90% of the time, with repairs taking 60 s on average.
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let station = model.add_station(SimTime::from_secs(4), failures);
    let drain = model.add_drain();
    model.connect(source, station);
    model.connect(station, drain);

    let mut run = model.build(42);
    run.init()?;
    run.advance_to(SimTime::from_secs(365 * 86_400))?;
    let state = run.state();
    println!("{} MUs left the model", state.objects[drain as usize].statistics.mus_in);
    Ok(())
}
```

The same seed always gives the same result. For a run to watch, `LiveSession` takes commands, such as `start`, `halt`, `set_speed`, `go_back`, `step_back` and `pause`, and sends back outputs, which `Picture` turns into what a UI shows. `CLAUDE.md` describes all of it.

## Project layout

- `src/` is the engine:
  - **`simulation_run.rs`:** a run you can advance, reset and read.
  - **`model.rs`:** builds a model.
  - **`live_session.rs`:** plays a run on a clock.
  - **`picture.rs`:** what a UI shows.
  - **`statistics.rs`:** statistics.
  - **`materialflow_objects/`:** Sources, Stations, Buffers and Drains.
- `model/` reads, checks and saves model files, `models/` holds example models, and `cli/` is the headless command line.
- `ui/` is the UI, a Bevy application.
- `tests/` holds the behaviour tests, and `benches/reference.rs` the benchmark.
- `docs/` holds the decisions (`adr/`), design notes and measurements (`design/`), and the handoff for whoever works on it next (`agents/handoff.md`).
- `GLOSSARY.md` defines the terms, such as Halt, Stop, Pause and Interruption, which mean different things here.

## More

- **`CLAUDE.md`:** how the code is built and the conventions to keep.
- **`docs/agents/handoff.md`:** the agreed design, including what isn't built yet, and the order of work.
- **`docs/design/first-profile.md`:** the current performance baseline.
