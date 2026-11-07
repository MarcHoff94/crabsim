---
title: Operation Catalogue (AI context)
for: architecture/operation-catalogue.md
section: architecture
---

## Facts
- `crabsim-catalogue` (`catalogue/`): every capability as an `Operation` (NAME, DESCRIPTION, EFFECT, serde + JsonSchema Input/Output, `run`); front-ends use only `operations()` and `call(name, json)`
- Front-ends: `crabsim-mcp` (`mcp/`, rmcp `=3.9.0`, stdio, not a default member) and the CLI's `run`, `runs`, `results`, `query` subcommands
- Errors: `OperationError { kind, message, problems }`; MCP returns them as tool results marked as errors
- Slice 1 operations: validate/list/read/write/copy/delete_model, start_run, run_status, cancel_run, run_results, query_results, label_run, forget_run
- Jobs: pool of one thread per core, one observation per thread, chunks of 100,000 events, jobs take turns; observation k seed = `splitmix64(seed + k)`
- Store: `<models>/.crabsim/results.sqlite`, WAL, tables runs/observations/statistics (long and thin), ADR 0010

## Invariants
- Operations touch only `.json` files inside the models folder and `.crabsim/`; never code, commands or the network
- A models folder with `Cargo.toml`, `.git` or `.rs` is refused; paths must stay inside after following links
- A run error fails only its observation; a run is `failed` only if every observation failed
- `query_results` can't write, attach a database, return more than 1,000 rows or run past 5 s

## Pitfalls
- A run whose process ended first is marked `interrupted` (lock files in `.crabsim/processes/`, `runs.process`, tables version 2), when the store opens and in run_status, cancel_run and forget_run
- Seeds are TEXT in the store: they use all 64 bits
- Debug test builds run the engine unoptimised: keep test runs short

## Code locations
- `catalogue/src/`: `lib.rs` (Catalogue, Operation), `error.rs`, `folder.rs`, `model_files.rs`, `jobs.rs`, `results.rs`, `store.rs`, `process.rs`
- `mcp/src/main.rs`; `cli/src/main.rs`, `cli/src/tables.rs`; design in `docs/design/operation-catalogue.md`
