# crabsim

A discrete-event simulator for any material flow, such as a factory line, written in Rust. A model is made of Sources, Stations, Buffers and Drains that create, work on, hold and take out moveable units (MUs). Stations fail at random, can be paused or forced to fail, and keep statistics such as busy, blocked and failed time, lead time and throughput.

crabsim exists to simulate long periods as fast as possible: a year of the demo model takes about 1.2 seconds, at about 13 million events per second, and a run's memory stays flat however long it is. A live session can also play a run at a chosen speed, step forward and back through it, and take interventions while it plays, which is what the window shows.

**Status:** the engine, the window, the command line and the server for AI agents all work on the same model files and the same operations.

- **Models** are model files, readable JSON, kept in a models folder. Classes derive from the built-in Source, Station, Buffer and Drain, or from each other, and an instance sets only the values it changes.
- **The window**, a Bevy application with egui panels (ADR 0006), lists a folder's models and plays the one you choose live in 2D. You build models in it too: place instances from a class, connect them, set values and make classes, with the mouse or from the keyboard.
- **The command line** runs model files headless, runs experiments of several observations and keeps their results, compares runs and changes models.
- **The MCP server** gives AI agents the same operations. An agent claims a model before changing it, and you see and manage its claims.
- **Undo and redo** take back any change to a model file, yours or an agent's, from a change history kept beside the models.
- **Not yet:** Buffer's movement is known to be broken, so models don't use it, and only Stations fail. The plan is in `docs/agents/handoff.md`.

## Requirements

Rust, installed through [rustup](https://rustup.rs). The repo pins its Rust version in `rust-toolchain.toml`, so rustup fetches the right one by itself the first time you build. On Windows you also need the Visual Studio C++ build tools, which the rustup installer offers.

## Quick start

The commands are for Windows PowerShell, one per line. Run them in the repo's folder.

Run the tests:

```powershell
cargo test
```

Open the window, then click `demo.json` in the list on the left and press Start. The first build takes several minutes. It works on a models folder, `crabsim-models` in your Documents, made the first time with the demo model in it:

```powershell
cargo run -p crabsim-ui --profile ui
```

To work on another models folder, such as the repo's own `models`, name it:

```powershell
cargo run -p crabsim-ui --profile ui -- --models models
```

Run the demo model headless for a simulated day, then print each object's statistics and how long the run took. `models/demo.json` is the demo model as a file, `models/model-file.schema.json` is the format's JSON Schema, which editors such as VS Code use to check a file as you type, and `docs/design/model-file.md` describes the format:

```powershell
cargo run --release --bin crabsim -- --model models/demo.json
```

The same with a seed and a simulated length, written as the window shows time, such as `1:30`, `8:00:00` or `30d 12:00:00`:

```powershell
cargo run --release --bin crabsim -- --model models/demo.json --seed 7 --length 365d
```

Run the benchmark, which takes about a minute and reports events per second, simulated time per wall second and peak memory:

```powershell
cargo bench --bench reference
```

The engine logs every event at `info` level. To see that while running the demo or the tests, set `RUST_LOG` first:

```powershell
$env:RUST_LOG = 'info'
```

## Working in the window

Hover over any control to see what it does, and press `?` for the list of keys.

- **Run:** Init, Start, Halt and Reset, a speed, stepping back and forward, and going to a time. With Animate off a run goes as fast as it can.
- **Look:** click an object for the inspector, which shows its state, statistics and every attribute with where its value comes from. The Results tab gives each Drain's throughput, every Station's shares of time and the likely bottleneck. The Paths tab draws routes, weights and distribution strategies on the canvas.
- **Experiment during a run:** pause a Station, force it to fail or change its process time. Each intervention leaves a mark, and going back past one asks first.
- **Build**, with no run in progress: drag a class onto the canvas, drag from an object's handle to connect it, drag objects to move them, and change values in the inspector. Right-click a class to derive, duplicate, rename or remove it.
- **From the keyboard:** `h` `j` `k` `l` move between objects, Space starts and halts, and `:` opens a command line, such as `:add Mill; connect Split > Mill 3`. `qa` records the lines you give into a macro and `@a` runs it.
- **Undo:** `u` takes back the latest change to the model and Ctrl+R redoes it, with a count such as `3u`. It asks first before taking back an agent's change. The Agents tab lists every change with who made it, and a click on one goes back to it.
- **Agents:** the Agents tab shows which agent holds which model, its goal and what it's doing, and lets you take the model, revoke the claim or block the agent.

Undo is for the model file. In a run, going back in time is what takes an intervention away.

## The command line on a models folder

With `--models`, `crabsim` works on a models folder as you do in the window. These examples use the repo's `models` folder.

Run an experiment of 5 observations of 30 simulated days and keep its results:

```powershell
cargo run --release --bin crabsim -- run --models models --model demo.json --length 30d --observations 5
```

List the runs, then print one's results:

```powershell
cargo run --release --bin crabsim -- runs --models models
```

```powershell
cargo run --release --bin crabsim -- results --models models run-1
```

The other subcommands, each with `--help`:

| Subcommand | Does |
|---|---|
| `compare` | Compares two runs, each figure's difference with a 95% interval |
| `query` | Runs one read-only SQL query on the results store |
| `validate` | Lists a model's problems and warnings |
| `describe`, `graph`, `export` | Sums up a model, answers questions about its structure, and writes it for graph tools |
| `attributes` | Shows an object's values and where each comes from |
| `edit` | Applies a batch of changes from a file or stdin |
| `history`, `undo`, `redo` | Lists a model's changes and steps through them |
| `adopt` | Accepts a file changed outside crabsim, such as by a text editor |
| `editors` | Shows which agents hold which models, and revokes or blocks them |

While a window has a models folder open, the command line can read and run there but changes nothing.

## AI agents

`crabsim-mcp` is an MCP server over stdio that offers every operation as a tool: building and changing models, running experiments, reading and comparing results, and undoing its own changes. Build it once:

```powershell
cargo build --release -p crabsim-mcp
```

Then point your harness, such as Claude Code or VS Code with GitHub Copilot, at `target\release\crabsim-mcp.exe` with the arguments `--models <folder>`. The server refuses a folder that holds source code, so give it a folder of models only. What the operations are and what they may touch is in `docs/design/operation-catalogue.md`.

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
- `model/` reads, checks and saves model files, and `models/` holds the demo model and the format's schema.
- `catalogue/` is the operation catalogue: every capability as an operation with typed input and output, which the front-ends below all use.
- `cli/` is the command line, `mcp/` the MCP server for agents, and `ui/` the window, a Bevy application.
- `tests/` holds the behaviour tests, and `benches/reference.rs` the benchmark.
- `docs/` holds the decisions (`adr/`), design notes and measurements (`design/`), and the handoff for whoever works on it next (`agents/handoff.md`).
- `GLOSSARY.md` defines the terms, such as Halt, Stop, Pause and Interruption, which mean different things here.

## More

- **`CLAUDE.md`:** how the code is built and the conventions to keep.
- **`docs/agents/handoff.md`:** the agreed design, including what isn't built yet, and the order of work.
- **`docs/design/first-profile.md`:** the current performance baseline.
- **`docs/design/ui-prototype.md`:** the throwaway prototype that settled the window's technology, on the branch `prototype/ui-bevy`.
