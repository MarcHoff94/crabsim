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
- **A model inside an input or output** is the model file's text: a string marked `contentMediaType: application/json`, so a harness never refuses a draft that doesn't fit the schema yet, which `write_model` must be able to save. Its schema, generated from `crabsim_model::schema`, comes from `model_file_format` alone, with a worked example. Until slice 2 the schema rode along in every operation a model passes through as `contentSchema`, which JSON Schema treats as information only, at about 7 KB each, so a harness carried three copies in every session. Leaving them out took the tool list from about 46 KB to about 28 KB, and the longer descriptions after the first harness trial brought it to about 29 KB. Measured again on 2026-11-08 as the MCP server's whole `tools/list` response, it was about 63 KB after the rest of slice 2 and `compare_runs`, and `describe_model` and `query_model` took it to 80.6 KB: 6.9 KB and 10.5 KB, mostly their output schemas. The largest tools are `edit_model` (10.7 KB, mostly its input schema), `query_model`, `run_results` and `describe_model`.
- **Defaults** of an input are in its schema, as `default`, so an agent sees them.
- **Errors** are structured: a `kind`, a `message` and, for an invalid model, every `problem` as `crabsim-model` reports it (JSON Pointer, name, message). The kinds are `invalid_input`, `invalid_model`, `not_found`, `outside_models_folder`, `already_exists`, `run_error`, and from slice 2 `not_editor`, `blocked`, `changed_outside` and `conflict`. Over MCP they are tool results marked as errors, so the agent reads them, rather than protocol errors its harness would hide.
- **What an output means is in the description,** not only in the output schema, because a harness may not show output schemas to the model: the first harness trial's agent never saw them, and asked what `here` and a summed `wall_time_s` meant.
- **Effects** become MCP's hints: read-only for `validate_model`, `list_models`, `read_model`, `model_file_format`, `get_attributes`, `describe_model`, `query_model`, `run_status`, `run_results` and `query_results`, destructive for `write_model`, `edit_model`, `delete_model` and `forget_run`, and neither for the rest.

Slice 1's operations, with `model_file_format`, `edit_model` and `get_attributes` from slice 2:

| Operation | Does |
|---|---|
| `validate_model { path? , text? }` | Every problem in a model, by path or by text |
| `list_models { folder? }` | The `.json` files in the folder or a subfolder, and below |
| `read_model { path }` | A file's text, exactly |
| `write_model { path, text, overwrite?, expected_hash? }` | Saves a valid model in canonical form, an invalid one exactly as given, returning its problems either way. An existing file only with `overwrite: true` |
| `adopt_model { path }` | Accepts a file changed outside crabsim as it is, so crabsim may change it again |
| `copy_model { from, to }` | Copies, refused if `to` exists |
| `delete_model { path }` | Deletes a file, never a folder |
| `edit_model { path, changes }` | Applies a batch of typed changes to a valid model, all or nothing, and saves it canonical |
| `get_attributes { path, target }` | Every attribute of an instance or class, with its value and where it comes from |
| `describe_model { path? , run? }` | A summary of a model whose size doesn't grow with it, also of a model with problems |
| `query_model { path? , run?, query }` | One of seven questions about a model's structure, named in `query.kind` |
| `model_file_format {}` | The model file's JSON Schema, how the built-in classes behave, and a small valid example in canonical form |
| `start_run { model, seed?, length, warm_up?, observations?, label?, wait? }` | Starts a job and returns its id at once, or with `wait` (1 to 60 s) once it ended or the wait is up, with `run_status`'s output. Seed 0 and 10 observations if left out |
| `run_status { run }` | Queued, running, done, failed, cancelled or interrupted, observations done, and where each running one has reached |
| `cancel_run { run }` | Stops a job, returning once nothing of it runs |
| `run_results { run, objects?, classes?, values? }` | A short summary, or the chosen objects, with each observation's values if asked |
| `compare_runs { a, b, objects?, classes? }` | Each figure's difference b minus a with a 95% interval, paired where the runs drew the same random numbers |
| `query_results { sql }` | One read-only SQL query on the store |
| `label_run { run, label? }` | Sets or clears a run's label |
| `forget_run { run }` | Deletes a run's results |

## Changing a model: `edit_model` and `get_attributes`

Slice 2 of the catalogue, spec issue 45. Models change only through operations (ADR 0011), so an agent changes one value without resending the whole file, which the first harness trial had to do: about 5 KB to change `15` to `10`.

- **A batch on the file's JSON:** `edit_model` loads the file, refusing one with problems (`write_model` saves work in progress), takes its canonical form as JSON, applies each change in order, then checks the result with `crabsim_model::load` and saves it canonical. Only the result is checked, so a batch may pass through invalid states, such as adding a Source and then setting its interval, and values are written and checked exactly as in a file, with the same messages and "did you mean". A problem's pointer points into the file the batch would have produced. Working on JSON rather than the typed model keeps the change kinds free of the attribute types, so a new attribute needs no new change.
- **Every problem at once** (from the second harness trial): a change that can't apply doesn't stop the others, so the refusal lists every change that can't apply and the problems of the model the rest would make, and one round fixes them all.
- **All or nothing:** a change that can't apply, such as naming an unknown instance, connecting twice or restoring an attribute that isn't set, refuses the whole batch with `invalid_input` naming its place, such as `changes/2 (connect)`. A result with problems refuses it with `invalid_model`. Either way the file is untouched.
- **Change kinds**, each a tagged object (`"change": "connect"`): `add_instance`, `remove_instance`, `rename_instance`, `move_instance`, `set_class`, `duplicate_instance`, `connect` (with an optional `place` in the successor order), `disconnect`, `add_class`, `remove_class` (with `delete_dependents`), `rename_class`, `set_parent`, `duplicate_class`, `set_attribute` and `restore_inheritance` (each on `{"instance": name}` or `{"class": name}`), and `set_model_description`.
- **What else changes:** removing an instance removes its connections. Removing a class is refused while classes derive from it or instances are of it, listing them, unless `delete_dependents: true`, which removes them all with their connections. Renames update every reference. New and duplicated instances go at the end, and a duplicate has no connections, nor a duplicated class its children. The output lists what was removed beyond what the changes named.
- **Streams:** every instance keeps its stream (ADR 0012), so runs of a model before and after an edit stay paired for every instance they share. New and duplicated instances get a stream of their own, and `set_stream` is for the user only.
- **`get_attributes { path, target }`** gives every attribute the target's built-in class has, in a fixed order, each with its value as a file writes it and `from`: `instance`, `class` (naming the nearest class that sets it), `built_in` (the default) or `not_set`. Also the chain of classes, an instance's position and successors, and the file's hash. A built-in class as target gives the defaults. It uses `Model::resolve` and `Model::resolve_class`.
- **Hashes, named apart** after the second harness trial, whose agent nearly took one for the other: `file_hash` is the SHA-256 of the file's bytes as they are, which `expected_hash` compares, and `model_hash` that of the model in canonical form, as a run records it, so it tells whether a run ran this model. For a file crabsim saved the two are equal. `read_model`, `get_attributes`, `edit_model` and `adopt_model` give both, `model_hash` only for a valid model.
- **The cost,** measured on 2026-11-08 on the benchmark's model as a file (429 copies of the demo with unique names, 6,006 objects, 1.66 MB), in release, median of 7 calls each on the owner's PC: `get_attributes` 21 ms, `edit_model` with one change 85 ms, and `validate_model` 21 ms. An edit loads and checks the file twice and saves it, so it costs about four validations, whatever the size of the batch. It touches no simulation run. The measurement is an ignored test, `get_attributes_and_edit_model_on_6006_objects` in `catalogue/tests/edit_model.rs`.

## A model as a graph: `describe_model` and `query_model`

Slice 3 of the catalogue, spec issue 56, PR 1 of 3 (exports and the CLI follow). An agent can't take in a model of 6,000 objects, a 1.8 MB file, so it starts with a summary and narrows down with questions, never reading the whole file. Both are in `catalogue/src/graph.rs`, computed on the model in memory, with no graph database.

- **Which model:** `path`, a file in the models folder, or `run`, the model that run ran, as the results store keeps it, so a run's results can be understood after the file changed. Exactly one.
- **The graph:** instances numbered by their place in the file, with successors and predecessors by number. Unknown successors and repeats are left out, so a model with problems still gives one. Lists come nearest first where there is a distance, and otherwise in file order, so answers are the same every time.
- **`describe_model`** reads whatever can be read (`crabsim_model::load_what_can_be_read`) and lists the problems. Its summary is fixed in size: about 3 KB for 6,006 objects. It gives the description, the number of instances, how many instances are of each class itself (built-in classes with any first, then the model's own in file order, up to 50), the Sources and Drains (a count and the first 10 names), branch and merge points (the 10 widest), for each of the first 10 Sources the Drains it reaches (up to 10) with the fewest and most objects an MU passes between them, loops (up to 5), instances no Source reaches, dead ends (non-Drains with no successor), and the problems (the first 10).
  - **Most objects** is the longest route in a graph without loops. When a route goes round a loop, an MU can pass any number of objects, so the summary says `through_loop` and gives no most.
  - **A loop** is a group of instances that MUs can go round in, a strongly connected component of more than one instance, found by Tarjan's algorithm without recursion. Each comes with its size and the shortest cycle through its first instance in file order.
- **`query_model`** refuses a model with problems, since its answers would describe a graph that isn't the model. `query` is a tagged object, like `edit_model`'s changes, so the tool list grows by one tool rather than seven:
  - `neighbours { name }`: predecessors and successors, each with its class and built-in class.
  - `upstream` / `downstream { name, steps? }`: every instance that can reach it, or that it reaches, within `steps`, with its distance, never the instance itself even in a loop. At most 1,000, with `count` and `truncated`.
  - `paths { from, to }`: routes that pass no object twice. The shortest, and up to 20 others shortest first, by Yen's algorithm, equally short ones in file order compared instance by instance. `count` counts routes up to 10,000, and `more` says there are more: without a loop on the way it counts them exactly in topological order, and with one it walks them, giving up after 2,000,000 steps with `more` set, so `count` is then a lower bound.
  - `subgraph_around { name, steps? }`: the instances within `steps` (2 by default) following connections either way, so it includes an instance's fellow predecessors of a successor, and the connections between them. At most 500, nearest first.
  - `find_instances { name_contains?, class?, description_contains?, conditions?, set_on_instance? }`: every criterion must hold. Names and descriptions ignore case, and the description is the inherited one. `class` includes classes derived from it. A condition compares an attribute's value, inherited or its own, with `=`, `<`, `>`, `<=` or `>=`: times as times (`interval`, `process_time`), whole numbers as numbers (`starting_mus`, `capacity`), and any other attribute only with `=`, as a file writes it. An instance whose built-in class lacks the attribute doesn't match. Each found instance comes with the values the criteria asked about. At most 1,000.
  - `class_tree { class?, instances? }`: the classes from the four built-in classes down, or from one class, each followed by those derived from it, with its parent, depth, the attributes it sets, the classes derived from it directly, and how many instances are of it and of it or a class below it. With `instances`, the names of its own instances, at most 1,000 in all.
- **Results and the graph** aren't joined: a query gives names, which an agent passes to `run_results` or `compare_runs` as `objects`. Joining belongs to studies.
- **The cost,** measured on 2026-11-08 on the same 6,006-object file as `edit_model`, in release, median of 7 calls each on the owner's PC: loading the file is most of every call, with `validate_model` at 22 ms. `describe_model` took 23 ms, and `neighbours`, `upstream`, `downstream`, `paths`, `subgraph_around` and `class_tree` 22 to 23 ms. `find_instances`, which works out every instance's values, took 35 ms. The measurement is an ignored test, `describe_and_query_on_6006_objects` in `catalogue/tests/graph.rs`. It doesn't cover a model with many loops or many routes, where `paths` is bounded by its step limit rather than the model's size.

## Who may change a model: the editor and the edit queue

Slice 2's PR 3a, from spec issue 45 and ADR 0011. Only one editor may change a model file at a time, through `edit_model`, `write_model` or `delete_model`. Reading, validating, copying to a new path and running need no right.

- **Agents** call `claim_model { path, purpose }`, with the purpose required, and get `granted`, or `queued` with their place. A queued agent asks again from time to time, which keeps its place, and an editor's every change and claim renews its right. `release_model` gives the right or the place back. A single change to a file nobody holds or waits for needs no claim: it holds the right for that call only. Any other change is refused with `not_editor`, naming the editor, its purpose and how many wait.
- **A right ends** on release, when the agent's process ends, or after the edit timeout without use: 10 minutes, set with `--edit-timeout` when the MCP server starts. A queued place not asked for in as long is dropped. The right then passes to the first waiting.
- **The user** (the CLI now, the UI later) never waits and is never timed out. Their own operations are `take_model`, `give_model`, `move_in_queue { path, agent, place }`, `remove_from_queue`, `block_agent`, `unblock_agent` and `release_model`. They're marked `user_only`: the MCP server doesn't list them, and the catalogue refuses them to an agent, which is the AI-native pillar's one exception (ADR 0011). A user's change while an agent edits is refused, so the user takes the right first, and the CLI's `--take-over` lends it for one command (PR 4).
- **Blocking** applies to one agent session. A blocked agent loses its right and places at once, and every claim and change it makes gets `blocked`, saying that waiting won't help.
- **`get_edit_queue { path? }`** shows a file's editor and queue, or every file's, and the blocked agents. For an agent it also shows `you`, its own id.
- **One agent is one crabsim process opened as an MCP server,** known by its process lock (`.crabsim/processes/`). It gets an id in the folder, such as `agent-3`, and is shown with the name its harness gave in MCP's `clientInfo`, which the server passes on with every call, its start time and its purpose. Subagents of one harness session share its server, so they count as one agent.
- **The state** is one JSON file, `.crabsim/editing.json`: the agents, and per file its editor and queue, with times as milliseconds since 1970. It's read and written only while holding an OS lock on `.crabsim/editing.lock`, and written to a temporary file renamed over the old one, so a crash never leaves half of it. A database was rejected because it's little data: no tables or migrations, and readable while debugging. Every call first sweeps it: agents whose process ended go, rights and places past the timeout go, and free rights pass on.
- **A change holds the lock until its file is written,** so two changes nobody claimed can't both go through. Other processes wait for that long: tens of milliseconds, about 85 ms for one change to a 6,006-object model.
- **Copied along with a models folder,** the state names processes that don't hold the copy, so they count as ended.

## Changes from outside crabsim

Slice 2's PR 3b, from spec issue 45 and ADR 0011. The editor right covers only changes made through crabsim, and crabsim can't stop another program writing a model file: a text editor, a harness's own file tools, git, or a copy dropped in. So it notices.

- **Records:** each time crabsim saves a model file (`edit_model`, `write_model`) or adopts it, the edit state records the file's SHA-256 under its path, beside the queues in `.crabsim/editing.json`. Deleting a file forgets its record.
- **Changed outside** is a file whose hash differs from its record, or that has no record: one from before slice 2, or one that appeared. `edit_model`, `write_model` and `delete_model` refuse it with `changed_outside`, so an outside change is never built on unchecked or overwritten. Reads and runs still work, and `read_model`, `validate_model` (by path), `get_attributes` and `start_run` report `changed_outside: true`, so a run says it used an unadopted version.
- **`adopt_model { path }`** records the file as it is, after the same editor rules as a change. It accepts an invalid file too, giving its problems, because `write_model` would otherwise refuse to fix a file that adopting refused to accept.
- **A copy** is recorded only if its source was crabsim's own, so an outside change can't slip in through `copy_model`, whose output says `changed_outside` so an agent knows to adopt the copy. `adopt_model`'s description says comparing the copy's `file_hash` with its source's is enough, so a copy needs no whole read before adopting.
- **`expected_hash`** on `edit_model` and `write_model`, from `read_model` or `get_attributes`, refuses a change with `conflict` if the file isn't that version any more, also after a change through crabsim. It's optional, since the editor right prevents most of this.
- **`write_model`** refuses an existing file unless given `overwrite: true`, as `copy_model` never overwrites. `edit_model` is the way to change a model.
- **Other people's runs:** over MCP, `forget_run` and `label_run` are refused on runs whose source isn't `mcp`, the user's. An agent may forget only the runs its own session started, by the process recorded in `runs.process`, and may label any agent's, since labels help everyone. Agents still read every run.
- **Its limits:** this guards against accidents, not against someone determined. Any program can still write a model file or `editing.json`, and an agent can run the CLI through its harness's shell and count as the user. The owner wants these security implications revisited later (spec issue 45, Out of Scope).
- **The cost:** a hash of the file per change and per read that reports it, beside the edit state's lock. On the 6,006-object model (1.66 MB) the medians didn't move measurably from PR 2's: `get_attributes` 21.7 ms, `edit_model` with one change 85.1 ms and `validate_model` 20.6 ms, median of 7 in release, one run each on the owner's PC.

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

- **`runs`:** `number` (counting up, never reused), `id` (`run-<number>`), `model_path`, `model_text` (the canonical model the run used), `model_hash` (its SHA-256, so equal hashes mean the same model), `seed`, `length_s`, `warm_up_s`, `observations`, `label`, `source` (`mcp` or `cli`, later `ui`), `crabsim_version`, `started` and `finished` (UTC, ISO 8601), `state` (`queued`, `running`, `done`, `failed`, `cancelled`, `interrupted`) `process` (the lock id of the process that ran it), `crabsim_build` (the git commit crabsim was built from, `-dirty` with uncommitted changes, `unknown` outside git, from `catalogue/build.rs`, since `crabsim_version` never changes) and `agent` (such as `agent-3 (claude-code)` or `the user`). The tables are version 3, kept in `user_version`: a store of version 1 gains `process`, and one of version 1 or 2 `crabsim_build` and `agent`, in place when it's opened. Runs from before have them empty, shown as `unknown`.
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

- **`run_results { run, objects?, classes?, values? }`** gives the settings, the events and wall time of all observations added up, every observation that failed with its seed, error and time, and for every figure the mean over the observations that ran to their end, a 95% confidence interval (Student's *t*, none for fewer than two values), the minimum and the maximum. By default, a short summary. A model of 20 instances or fewer is shown whole, each tagged `all`, since picking the most of anything from so few picks nearly all of them. Otherwise every Drain, the 10 `busiest` by mean working share, which is where a bottleneck is, the 10 `most_blocked` above a mean blocked share of 1%, and as `most_failed` at most 10 objects whose failed share's 95% interval lies wholly above that of the median object, so objects failing alike, such as instances of one class, aren't ranked by chance. With one observation there are no intervals, so none are most failed. Figures are named as in the store, with lead time as `mean_lead_time_s` in seconds, which the CLI prints as a time. With `objects` (instance names) or `classes` (every instance whose chain of classes includes one of them, built-in classes too), exactly those instead.
- **The same size whatever the number of observations:** each observation's own figures, and each observation's seed, events and wall time, come only with `values: true`. A figure that is 0 in every observation, such as a Source's `failures`, is left out. Numbers are shown to 4 significant digits but never fewer than their whole number, so 0.74604 is 0.746 and 131,650.75 is 131,651. The store keeps every digit, for `query_results`. The first trial of the MCP server as a harness drives it found the summary at 31 KB for 8 observations of the demo model and 162 KB for 100, more than a harness usually accepts from one tool. Now it is about 10 KB for 8 observations and 9 KB for 100.
- **`compare_runs { a, b, objects?, classes? }`**, designed with the owner on 2026-11-08 after the first harness trial, whose agent wrote a paired difference by hand in SQL. Per object and figure it gives a's mean, b's mean, the difference b − a with a 95% interval, the relative difference, `clear` when the interval leaves out 0, the `method` and how many observations were `counted`, to 4 significant digits.
  - **Pairing by observation:** runs with the same seed and number of observations are paired, and every figure is compared observation by observation with a Student's *t* interval. That is valid whatever the models, since different observations are independent, as the second harness trial's agent pointed out. Each object also says whether it drew the same random numbers in both, `common_random_numbers`: the same stream in both models (ADR 0012). That decides how much narrower its intervals are, not whether they're valid. Other runs are compared with the difference of the means and Welch's interval, its Welch–Satterthwaite degrees of freedom rounded down.
  - **Warnings:** runs of different lengths or warm-ups are compared all the same, with a warning: counts aren't comparable then, but rates and shares are, and intervals that agree mean the shorter length was enough.
  - **Unchanged:** figures identical in every pair, such as failure counts with common random numbers, are left out, and objects left with none are listed in `unchanged`. The trial found the full comparison of the demo at 20 KB.
  - **Dropped:** every observation that gave no figures is listed with why: a pair dropped because observation *k* failed in either run, naming the run and its error, or didn't end because the run was cancelled or interrupted. For unpaired runs, each observation left out of either run.
  - **Objects:** by default the union of what `run_results` shows for each run, and `objects` or `classes` for exactly those, from either model. Objects in only one model are listed apart, and figures that are 0 in both runs are left out. The description warns that at 95%, a few of many figures are clear by chance alone.
  - Two runs only: simulation studies (#28 step 6) will compare more, reusing it. The CLI has `crabsim compare`.
- **`query_results { sql }`** runs one `SELECT` (or `WITH ... SELECT`) on a connection of its own that is read-only, can't attach another database, stops after 5 seconds and returns at most 1,000 rows, saying if more matched. Its description holds the tables and statistic names, so an agent can write the query. Besides SQLite's own functions it has `sqrt`, `power`, `ln`, `exp` and `log10`, registered on its connection, which the bundled SQLite lacks, so a query can work out a standard deviation.
- **`label_run`** changes a run's label, and **`forget_run`** deletes a run's rows from all three tables, refused while it's still running.

## What the first harness trial changed

On 2026-11-08 an agent in Claude Code used `crabsim-mcp` alone, without crabsim's source, to find the demo model's bottleneck, make a variant and compare the two. It worked end to end, and its report led to: `model_file_format` explaining how the built-in classes behave, output fields explained in descriptions, `start_run` saying that the same seed pairs runs, "did you mean" in validation, one name for lead time, the summary above, and the math functions. Further findings are in spec issue 45: agents refused the user's runs, `write_model` refusing to overwrite without `overwrite: true`, and `compare_runs` as a slice of its own.

## The CLI as a front-end

`crabsim` gains subcommands, each calling catalogue operations through `call(name, JSON)`, with the same scope rules on its models folder:

- `crabsim run --models <folder> --model <path> [--seed --length --warm-up --observations --label]`: starts a run with source `cli`, waits for it, showing progress, and prints the results. Seed 0 and 10 observations by default, as for agents.
- `crabsim runs --models <folder>`: lists runs, newest first.
- `crabsim results --models <folder> <run> [--object ...] [--class ...] [--values]`: the `run_results` summary, as tables, with every observation in a table of its own with `--values`.
- `crabsim query --models <folder> "<sql>"`: `query_results`, as a table.
- `crabsim compare --models <folder> <run-a> <run-b> [--object ...] [--class ...]`: `compare_runs`, as a table per object.
- `crabsim edit --models <folder> <path> <changes.json | -> [--expected-hash] [--take-over]`: `edit_model` with the changes from a file or stdin. A person using the CLI is the user, so `--take-over` makes this one change while an agent holds the right, which the agent keeps.
- `crabsim attributes --models <folder> <path> <name> [--class]`: `get_attributes`, as a table.
- `crabsim adopt --models <folder> <path>`: `adopt_model`.
- `crabsim queue show|take|release|give|move|remove|block|unblock --models <folder> ...`: the user's controls of the edit queue, each printing the queue after. They're grouped under `queue`, where issue 45 had `crabsim take` and `crabsim release` at the top level.

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
