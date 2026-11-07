---
title: Architecture Overview (AI context)
for: architecture/overview.md
section: architecture
---

## Facts
- Workspace: engine `crabsim` (root, library), `crabsim-model` (`model/`, model files, ADR 0009), `crabsim-catalogue` (`catalogue/`, operations, jobs, results store), `crabsim-cli` (`cli/`, binary `crabsim`), `crabsim-mcp` (`mcp/`, MCP server), `crabsim-ui` (`ui/`, Bevy + egui, ADR 0006); the UI and the MCP server are the non-default members; the engine depends on none of them
- A model file → `crabsim_model::load` → `Model` or every `Problem` → `to_description()` → `ModelDescription` → `build(seed)`
- Layers: front-ends (CLI, MCP server, UI) → operation catalogue (CLI and MCP) → optional live session + Picture → `SimulationRun` → material-flow objects (`enum ModelObject`)
- Headless run = a `SimulationRun` with nothing attached: build, `init`, `advance_to`, `state`
- One `Vec<ModelObject>` indexed by id; objects refer to each other by id; the run splits the `Vec` around the object handling an event and passes `OtherObjects` (ADR 0005)
- No shared ownership, no `unsafe`; a run runs on one thread at a time and can move; a clone is a checkpoint

## Invariants
- An object's id must match its index; build through `ModelDescription`
- Live sessions and views cost a headless run nothing

## Code locations
- `src/simulation_run.rs` SimulationRun · `src/model.rs` ModelDescription · `src/event.rs` events · `src/report.rs` occurrences and state · `src/statistics.rs` · `src/live_session.rs` · `src/picture.rs` · `src/time.rs` SimTime, format/parse · `src/demo.rs`
- `model/src/`: `lib.rs` (Model, Problem, load), `read.rs`, `check.rs`, `write.rs`, `build.rs`, `schema.rs` (JSON Schema via schemars, drift-tested against the reader); `cli/src/main.rs`; `models/demo.json`, `models/model-file.schema.json`
- `src/materialflow_objects/`: `model_object.rs`, `flows.rs`, `source.rs`, `station.rs`, `buffer.rs`, `drain.rs`
- `ui/src/`: `view.rs`, `camera.rs`, `marks.rs`, `main.rs`
