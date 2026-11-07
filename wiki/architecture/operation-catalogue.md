---
title: Operation Catalogue
section: architecture
related:
  - overview
  - running-crabsim
  - simulation-run
---

# Operation Catalogue

## Why it exists

crabsim is AI-native: anything a person can do, an agent can do through the same operations (ADR 0007). So every capability is defined once, in `crabsim-catalogue`, as an operation with a name, a description written for an agent, a typed input and a typed output. Front-ends only translate: the MCP server (`crabsim-mcp`) offers every operation as a tool, and the CLI (`crabsim`) offers them as subcommands for people. Neither has logic of its own, so they can't drift apart. The full design is in `docs/design/operation-catalogue.md`.

## Operations

An operation implements `Operation`: `NAME`, `DESCRIPTION`, `EFFECT` (read-only, additive or destructive), an `Input` that deserialises and an `Output` that serialises, both deriving `schemars::JsonSchema`, and `run`. Front-ends use only two calls:

- `Catalogue::operations()`: every operation with its input and output JSON Schemas.
- `Catalogue::call(name, json)`: runs one, giving JSON back, or an `OperationError` with a `kind` (`invalid_input`, `invalid_model`, `not_found`, `outside_models_folder`, `already_exists`, `run_error`), a message and, for an invalid model, every problem.

A model inside an input is the model file's text, whose schema carries the model file's own schema as `contentSchema`, generated from `crabsim_model::schema`. Slice 1 has `validate_model`, `list_models`, `read_model`, `write_model`, `copy_model`, `delete_model`, `start_run`, `run_status`, `cancel_run`, `run_results`, `query_results`, `label_run` and `forget_run`.

## The models folder

A catalogue is opened on one models folder, and that is all its operations touch: `.json` model files inside it and crabsim's own `.crabsim/` folder there. A folder holding a `Cargo.toml`, a `.git` or any `.rs` file is refused, so crabsim's own code can't be exposed by mistake. Every path must be relative, without `..`, and stay inside the folder after following links. Paths naming `.crabsim` are refused, and `list_models` doesn't follow links into folders.

## Runs as jobs

`start_run` reads and checks the model, keeps its canonical text and returns an id such as `run-3` at once. The observations run on a pool of one thread per core, started on the first run. Each thread runs one observation from Init to its end, in chunks of 100,000 events, recording the simulated time reached and checking the job's cancel flag between chunks. Jobs take turns: a freed thread takes the next observation of the next job in turn, so a quick run started during a long one doesn't wait for all of it.

Observation *k* gets `splitmix64(seed + k)` (`observation_seed`), so the experiment's seed reproduces everything, and results don't depend on the number of threads. With a warm-up, the observation advances to it, clears statistics and advances on. A run error fails only its observation, which is recorded with the error and its simulated time and gives no figures.

## The results store

`<models folder>/.crabsim/results.sqlite`, opened on first use in WAL mode, so a CLI and an MCP server can share it (ADR 0010). Tables `runs`, `observations` and a long, thin `statistics`, one row per figure, written once per observation at its end. `run_results` gives the mean, a 95% interval (Student's *t*), minimum and maximum over the observations, by default for every Drain and the 10 most blocked and 10 most failed objects. `query_results` runs one `SELECT` on a read-only connection with no attached databases, capped at 1,000 rows and 5 seconds.

## Performance

Nothing here touches the per-event path: a run is built from a `ModelDescription` as before, and the pool adds one check per chunk of events.

## Code references

- `Catalogue`, `Operation`, `Effect`, `Settings` — `catalogue/src/lib.rs`
- `OperationError`, `ErrorKind` — `catalogue/src/error.rs`
- `ModelsFolder` — `catalogue/src/folder.rs`
- Model file operations — `catalogue/src/model_files.rs`
- Jobs, the pool, `observation_seed` — `catalogue/src/jobs.rs`
- `run_results`, `label_run`, `forget_run`, `STATISTICS` — `catalogue/src/results.rs`
- `Store`, `query_results` — `catalogue/src/store.rs`
- MCP server — `mcp/src/main.rs`; CLI subcommands — `cli/src/main.rs`, `cli/src/tables.rs`
