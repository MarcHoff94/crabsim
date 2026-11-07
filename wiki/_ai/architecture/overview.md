---
title: Architecture Overview (AI context)
for: architecture/overview.md
section: architecture
---

## Facts
- Workspace: engine = root package (library + headless CLI); UI = member crate `ui/` (`crabsim-ui`, Bevy + egui, ADR 0006); engine is the only default member and never depends on the UI
- Layers: front-ends (CLI, UI) → optional live session + Picture → `SimulationRun` → material-flow objects (`enum ModelObject`)
- Headless run = a `SimulationRun` with nothing attached: build, `init`, `advance_to`, `state`
- One `Vec<ModelObject>` indexed by id; objects refer to each other by id; the run splits the `Vec` around the object handling an event and passes `OtherObjects` (ADR 0005)
- No shared ownership, no `unsafe`; a run runs on one thread at a time and can move; a clone is a checkpoint

## Invariants
- An object's id must match its index; build through `ModelDescription`
- Live sessions and views cost a headless run nothing

## Code locations
- `src/simulation_run.rs` SimulationRun · `src/model.rs` ModelDescription · `src/event.rs` events · `src/report.rs` occurrences and state · `src/statistics.rs` · `src/live_session.rs` · `src/picture.rs` · `src/time.rs` SimTime, format/parse · `src/main.rs` CLI · `src/demo.rs`
- `src/materialflow_objects/`: `model_object.rs`, `flows.rs`, `source.rs`, `station.rs`, `buffer.rs`, `drain.rs`
- `ui/src/`: `view.rs`, `camera.rs`, `marks.rs`, `main.rs`
