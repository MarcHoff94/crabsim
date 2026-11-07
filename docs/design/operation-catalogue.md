# The operation catalogue

Agreed with the owner on 2026-11-07 in a design round, and recorded in spec issue 37, as steps 3 and 4 of the AI-native order of work (issue 28, ADR 0007). The code is in `catalogue/` (`crabsim-catalogue`), with front-ends in `cli/` (`crabsim`) and `mcp/` (`crabsim-mcp`). This document covers slice 1, "MCP v1", built in three steps: the catalogue with the model file operations and the results store, then jobs and results with the CLI's subcommands, then the MCP server. Building and attribute operations, `describe_model`, the graph operations and exports follow in slices 2 and 3 of the same catalogue.

## What it is

Every capability of crabsim is an operation: a name, a description written for an agent, a typed input and a typed output, defined once in Rust. Front-ends only translate between their protocol and the catalogue:

- **The MCP server** offers every operation as a tool, so any agent harness can validate models, manage model files, run experiments and read results.
- **The CLI** offers the same operations as subcommands, so a person can do everything an agent can, before the UI's results views exist.

Jobs and the results store are for people as much as for agents. Both front-ends write to the same store, where each sees the other's runs.

## Operations

An operation implements the `Operation` trait: `NAME`, `DESCRIPTION`, `EFFECT` (read-only, additive or destructive), an `Input` that deserialises with `serde` and an `Output` that serialises, both deriving `schemars::JsonSchema`, and `run(catalogue, input)`. The catalogue lists every operation with its input and output schemas (`Catalogue::operations`), and `Catalogue::call(name, JSON)` runs one by name, giving JSON back. Front-ends use only these two.

- **Names** are snake case, verb first.
- **Inputs and outputs** are always JSON objects. Input schemas leave `null` out of optional fields, as the model file's schema does: leave a field out instead.
- **A model inside an input** is the model file's text, a string whose schema carries the model file's own schema as `contentSchema`, generated from `crabsim_model::schema`. So the two can't disagree, and a harness doesn't refuse a draft that doesn't fit the schema yet, which `write_model` must be able to save.
- **Errors** are structured: a `kind`, a `message` and, for an invalid model, every `problem` as `crabsim-model` reports it (JSON Pointer, name, message). The kinds are `invalid_input`, `invalid_model`, `not_found`, `outside_models_folder`, `already_exists` and `run_error`. Over MCP they are tool results marked as errors, so the agent reads them, rather than protocol errors its harness would hide.
- **Effects** become MCP's hints: read-only for `validate_model`, `list_models`, `read_model`, `run_status`, `run_results` and `query_results`, destructive for `write_model`, `delete_model` and `forget_run`, and neither for the rest.

Slice 1's operations:

| Operation | Does |
|---|---|
| `validate_model { path? , text? }` | Every problem in a model, by path or by text |
| `list_models { folder? }` | The `.json` files in the folder or a subfolder, and below |
| `read_model { path }` | A file's text, exactly |
| `write_model { path, text }` | Saves a valid model in canonical form, an invalid one exactly as given, returning its problems either way |
| `copy_model { from, to }` | Copies, refused if `to` exists |
| `delete_model { path }` | Deletes a file, never a folder |
| `start_run { model, seed, length, warm_up?, observations, label? }` | Starts a job and returns its id at once |
| `run_status { run }` | Queued, running, done, failed, cancelled or interrupted, observations done, and where each running one has reached |
| `cancel_run { run }` | Stops a job, returning once nothing of it runs |
| `run_results { run, objects?, classes? }` | A short summary, or the chosen objects |
| `query_results { sql }` | One read-only SQL query on the store |
| `label_run { run, label? }` | Sets or clears a run's label |
| `forget_run { run }` | Deletes a run's results |

## Scope: what crabsim's operations can touch

The owner's concern: an agent working through crabsim must never be able to change crabsim itself.

- Only `.json` model files inside the models folder, and crabsim's own `.crabsim/` folder there, which holds the results store. Never crabsim's code, never commands, never the network.
- The models folder is given when the server or the CLI starts (`--models <folder>`). Model paths are relative to it, with `/` between folders, and subfolders are allowed.
- **Refused** with `outside_models_folder`: absolute paths, including Windows paths such as `C:model.json` and `/model.json`, any `..`, any path naming `.crabsim`, and anything whose real path, after following links, lies outside the folder or inside `.crabsim`. A path that doesn't exist yet is checked by its nearest existing folder.
- **`list_models`** doesn't follow links into folders, so a link can neither lead it out nor round in a circle, and never shows `.crabsim`.
- **A source tree is refused as a models folder:** the server and the CLI refuse a folder holding a `Cargo.toml`, a `.git` or any `.rs` file at any depth, so crabsim's own repository can't be exposed by mistake. The repository's own `models/` folder passes, but a folder of the user's own models outside the repository is recommended.
- **`query_results`** can't reach outside either: its connection is read-only and can attach no other database.
- Agents in harnesses also have the harness's own file tools, governed by the harness's permissions, not by crabsim. The scope above is what crabsim itself allows.

## Runs as jobs

- `start_run` reads and checks the model, refusing an invalid one with every problem, keeps its canonical text, and builds every observation from that, so later changes to the file don't reach the run. It returns the run's id at once: `run-<n>`, numbered by the results store, so ids are unique across restarts and a forgotten run's number isn't used again.
- Lengths and warm-ups are written as the UI shows time, such as `8:00:00` or `30d`.
- **Seeds:** observation *k*, counting from 1, gets `splitmix64(seed + k)`, with the sum wrapping at 2^64:

  ```text
  z = x + 0x9E3779B97F4A7C15
  z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9
  z = (z ^ (z >> 27)) * 0x94D049BB133111EB
  z ^ (z >> 31)
  ```

  Each observation's seed is reported, and `crabsim --model <file> --seed <that seed>` reproduces it as a single headless run.
- **Threads:** one pool with one thread per CPU core (`available_parallelism`), each running one observation from start to end, since a simulation run executes on one thread at a time. Jobs take turns: whenever a thread frees up, it takes the next observation of the next job in turn that has one waiting, so a quick check started during a long study runs as soon as one thread is free. Within a job, observations start in order. Results don't depend on the number of threads.
- **Warm-up:** advance to the warm-up, clear statistics, advance to warm-up plus length. Without a warm-up, statistics aren't cleared.
- **Progress and cancelling:** each observation advances in chunks of at most 100,000 events, recording the simulated time reached and checking its job's cancel flag between chunks. So `cancel_run` takes effect within one chunk, and returns once nothing of the job runs. Observations that finished before stay in the store.
- **A run error** fails only its observation, which is recorded with the error and the simulated time it happened, and gives no figures. A run is `failed` only if every observation failed, and `done` otherwise.
- **A run whose process ended first,** such as in a crash or a kill, is marked `interrupted`, and the observations that ended keep their results. Each catalogue that uses the store holds an exclusive lock on `.crabsim/processes/<id>.lock` for as long as it lives, and each run records that id in `process`. The operating system releases the lock however the process ends, so a process that can take another's lock knows it has ended. The check runs when a catalogue opens the store, and in `run_status`, `cancel_run` and `forget_run`. A heartbeat was rejected because a sleeping laptop would look dead, and process ids because they get reused. Runs from a store of the first tables' version have no process recorded, so any process but their own marks them interrupted. Lock files of ended processes are removed.
- **Only the process running a run can cancel it,** and `forget_run` refuses a run another live process is running. A run that is interrupted can be forgotten, or run again from its stored model and seeds.

## The results store

One SQLite database, `<models folder>/.crabsim/results.sqlite`, created on first use, in WAL mode so a CLI and a server can use it at once. Why SQLite is in ADR 0010. Its tables are an interface: skills, scripts and later the UI rely on them.

- **`runs`:** `number` (counting up, never reused), `id` (`run-<number>`), `model_path`, `model_text` (the canonical model the run used), `model_hash` (its SHA-256, so equal hashes mean the same model), `seed`, `length_s`, `warm_up_s`, `observations`, `label`, `source` (`mcp` or `cli`, later `ui`), `crabsim_version`, `started` and `finished` (UTC, ISO 8601), `state` (`queued`, `running`, `done`, `failed`, `cancelled`, `interrupted`) and `process` (the lock id of the process that ran it). The tables are version 2, kept in `user_version`: a store of version 1 gains `process` in place when it's opened.
- **`observations`:** `run`, `number` (from 1), `seed`, `events`, `wall_time_s`, and for a failed one `error` and `error_time_s`.
- **`statistics`**, long and thin: `run`, `observation`, `object` (the instance's name), `class`, `built_in_class`, `statistic`, `value`. One row per figure of an observation that ran to its end, so new statistics later need no new column. About 2 million rows for 6,000 objects and 20 observations.
- **Seeds are text,** because they use all 64 bits and SQLite's integers are signed.
- **Times are seconds,** so SQL can compare them. JSON results show them as the UI does.
- **Statistic names,** fixed:

  | Name | Means |
  |---|---|
  | `mus_in`, `mus_out` | MUs in and out. A Source's in are the MUs it created |
  | `working_share`, `blocked_share`, `idle_share`, `failed_share`, `paused_share` | Each 0 to 1 of the time measured after the warm-up |
  | `failures` | How many failures began |
  | `availability` | Up time over up time plus failed time, 0 to 1 |
  | `throughput_per_hour` | Drains only: MUs an hour from the first exit to the end |
  | `mean_lead_time_s` | Drains only: mean lead time of the MUs that left, in seconds |

  A figure that doesn't exist for an observation, such as a Drain's throughput before two MUs, has no row.

## Reading results

- **`run_results { run, objects?, classes? }`** gives the settings, each observation's seed, events, wall time and any error, and for every figure the mean over the observations that ran to their end, a 95% confidence interval (Student's *t*, none for fewer than two values), the minimum and the maximum, beside each observation's own value. By default, a short summary: every Drain, plus the 10 objects with the highest mean blocked share and the 10 with the highest mean failed share, leaving out shares of 0, so even a 6,000-object model's results fit in an agent's context. With `objects` (instance names) or `classes` (every instance whose chain of classes includes one of them, built-in classes too), exactly those instead.
- **`query_results { sql }`** runs one `SELECT` (or `WITH ... SELECT`) on a connection of its own that is read-only, can't attach another database, stops after 5 seconds and returns at most 1,000 rows, saying if more matched. Its description holds the tables and statistic names, so an agent can write the query.
- **`label_run`** changes a run's label, and **`forget_run`** deletes a run's rows from all three tables, refused while it's still running.

## The CLI as a front-end

`crabsim` gains subcommands, each calling catalogue operations through `call(name, JSON)`, with the same scope rules on its models folder:

- `crabsim run --models <folder> --model <path> [--seed --length --warm-up --observations --label]`: starts a run with source `cli`, waits for it, showing progress, and prints the results.
- `crabsim runs --models <folder>`: lists runs, newest first.
- `crabsim results --models <folder> <run> [--object ...] [--class ...]`: the `run_results` summary, as tables.
- `crabsim query --models <folder> "<sql>"`: `query_results`, as a table.

Without a subcommand, `crabsim` behaves exactly as before: one run, printed, nothing saved. That suits a bigger machine running long runs with no models folder.

## The MCP server

`crabsim-mcp --models <folder>`, a binary of its own crate (`mcp/`), not a default member, built on `rmcp` pinned to `=3.9.0` with tokio, over stdio only, so it runs in any harness and on a machine without a display. It offers every catalogue operation as a tool with its description, input and output schemas and hints, returns each output as structured content, and each failure as a tool result marked as an error. It refuses to start on a folder holding source code. A harness's configuration names the binary and the folder, such as for Claude Code:

```json
{ "mcpServers": { "crabsim": { "command": "crabsim-mcp", "args": ["--models", "C:/Users/me/crabsim-models"] } } }
```

## The UI's results views (planned, not built)

Planned with the owner, to be built later on the same store:

1. a list of runs: label, model, when, who, how long, the main Drain figures
2. comparing runs or models side by side, with 95% intervals and where they overlap
3. the model coloured by a figure from a chosen run, such as blocked share as a heat map on the 2D view
4. clicking an object to see its figures in every observation and across runs
5. CSV export

## Performance

None of this touches the per-event path. A run is built from a `ModelDescription` as before, results are written once per observation, at its end, and progress and cancelling add one check per chunk of 100,000 events.
