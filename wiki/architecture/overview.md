---
title: Architecture Overview
section: architecture
related:
  - simulation-run
  - material-flow-objects
  - live-session-and-ui
---

# Architecture Overview

## Crates

crabsim is a Cargo workspace:

| Crate | Folder | What it is |
|---|---|---|
| `crabsim` | root | The engine, a library. It depends on none of the others |
| `crabsim-model` | `model/` | Model files: reading, checking, saving, and building them into a `ModelDescription` (ADR 0009) |
| `crabsim-cli` | `cli/` | The headless CLI, binary `crabsim` |
| `crabsim-ui` | `ui/` | The UI, a Bevy application (ADR 0006) |

The UI is the only member that isn't a default one, so `cargo test` and `cargo bench` at the root never build Bevy.

## Layers

```
┌──────────────────────────────────────────────────────────────┐
│  Front-ends                                                  │
│  headless CLI (cli/) · UI (ui/, Bevy + egui)                 │
├──────────────────────────────────────────────────────────────┤
│  Model files (model/): JSON in, ModelDescription out         │
├──────────────────────────────────────────────────────────────┤
│  Live session (optional)                                     │
│  LiveSession: computes ahead, releases on a clock,           │
│  checkpoints and replay · Picture: what the user knows       │
├──────────────────────────────────────────────────────────────┤
│  Simulation run                                              │
│  SimulationRun: Vec<ModelObject>, BinaryHeap<ScheduledEvent>,│
│  clock, recording, statistics                                │
├──────────────────────────────────────────────────────────────┤
│  Material-flow objects                                       │
│  enum ModelObject { Source, Station, Buffer, Drain }         │
└──────────────────────────────────────────────────────────────┘
```

A run with nothing attached is a headless run: build, Init, advance, read the state. The live session and views are optional layers that cost nothing when absent.

## Building and running a model

```
crabsim_model::load(text)       a model file, or every problem in it
  .to_description()              → ModelDescription

ModelDescription::new()         or in code

  add_source / add_station / add_buffer / add_drain → ids
  connect(from, to)
  build(seed) → SimulationRun

run.init()          t = 0, every object's init in id order, first events queued
run.advance_to(t)   process events up to and including t, clock left at t
run.state()         RunState: every object's ObjectState, with statistics
run.reset()         every object back to its default state
```

Every advance returns the occurrences the run is recording, none by default. A failed advance returns an `AdvanceFailure` holding the `RunError` and everything recorded before it.

## Ownership

A run owns its objects in one `Vec<ModelObject>`, indexed by id, and objects refer to each other only by id (ADR 0005). While one object handles an event, the run splits the `Vec` around it, and the object reaches the others through `OtherObjects`. So there is no shared ownership and no `unsafe`, and a run executes on one thread at a time and can move between threads. A clone of a run is a checkpoint.

## Data flow for one MU

```
Source::init / create_mu         MuCreated, MUExit after the interval
Source MUExit                    try_to_move_to_successor
  → Station::process_mu_entrance  MuMoved, WorkStarted, MUExit after the process time
Station MUExit                   WorkFinished, try_to_move_to_successor
  moved     → Idle, entrance unlocked, retries to waiting predecessors
  refused   → Blocked, listed at each successor that refused
Drain::process_mu_entrance       MuLeft with lead time, exit statistics
```

## Module map

```
src/
  lib.rs              public API, re-exports
  demo.rs             the demo model, with names and positions
  model.rs            ModelDescription, Position, Dimensions
  simulation_run.rs   SimulationRun
  event.rs            EventType, EventTemplate, ScheduledEvent, EventContext, ExitTicket
  report.rs           Occurrence, Recording, ObjectState, RunState, StopCause
  statistics.rs       StatisticsKeeper, ObjectStatistics
  live_session.rs     LiveSession, SessionOutput, checkpoints and replay
  picture.rs          Picture
  time.rs             SimTime, format_time, parse_time
  error.rs            RunError, AdvanceFailure and others
  random_numbers.rs   Distribution, DistributionSampler
  materialflow_objects/
    model_object.rs   ModelObject, OtherObjects
    flows.rs          traits, MoveableUnit, FailureProfile, movement helpers
    source.rs · station.rs · buffer.rs · drain.rs
model/src/
  lib.rs              Model, Instance, Attributes, Problem, load
  read.rs · check.rs  reading the JSON, then checking what it means
  write.rs · build.rs canonical saving, building a ModelDescription
cli/src/main.rs       the headless CLI
models/demo.json      the demo model as a file
ui/src/
  main.rs             Bevy and egui, drawing what the view state says
  view.rs             the view state, plain Rust
  camera.rs · marks.rs
```

## Code references

- `SimulationRun` — `src/simulation_run.rs`
- `ModelObject`, `OtherObjects` — `src/materialflow_objects/model_object.rs`
- `ModelDescription` — `src/model.rs`
- ADRs — `docs/adr/`
