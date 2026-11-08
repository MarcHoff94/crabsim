# crabsim handoff

Written 2026-11-07 at the end of a working session, and updated after each step since, last on 2026-11-08 after slice 3 of the operation catalogue was merged. Read this first, then check it against the code and git, because it describes a moment in time.

**The UI's order of work is done.** UI stage 1 was built in four slices, PRs 24 to 27: the demo drawn live with state colours and MUs, pan and zoom, the run controls, speed, time mode, stepping, going back to a time, the error banner, the inspector, interventions on Stations and their marks. Step 7 trimmed the CLI to a headless run of the demo, in PR 30.

**The model file is complete.** Issue 28's steps 1 and 2 are merged: the `SessionOutput` rename (PR 33), and the model file in three slices (PRs 34 to 36): `crabsim-model` reads, checks and saves model files with classes and inheritance, `models/demo.json` is the demo as a file, the CLI runs model files from `cli/` (ADR 0009), and `models/model-file.schema.json` is the published schema. The design is in `docs/design/model-file.md`.

**The operation catalogue is built through slice 3,** with the MCP server and the CLI as its front-ends, 24 tools over MCP. The design is in `docs/design/operation-catalogue.md`.
- **Slice 1, MCP v1** ([spec issue 37](https://github.com/MarcHoff94/crabsim/issues/37), PRs 39 to 42): model file operations, the SQLite results store (ADR 0010), runs as jobs, results, the CLI's subcommands and the MCP server, and runs marked interrupted when their process ends first.
- **Slice 2** ([spec issue 45](https://github.com/MarcHoff94/crabsim/issues/45), PRs 47 to 52): `edit_model` and `get_attributes`, an editor per model file with an edit queue the user controls (ADR 0011), changes from outside detected and adopted, and the first harness trial's fixes. Then `compare_runs` (#53), a random-number stream per instance (#54, ADR 0012) and the second harness trial's fixes (#55).
- **Slice 3** ([spec issue 56](https://github.com/MarcHoff94/crabsim/issues/56), PRs 59 to 61): `describe_model`, `query_model` with seven kinds of graph question, `export_model` into `exports/`, and the CLI's `describe`, `graph` and `export`.

**Next, in the owner's order of 2026-11-08: UI stage 2, [spec issue 63](https://github.com/MarcHoff94/crabsim/issues/63), then the domain model (build step 7).** See "Possible next steps".

## Start here

1. Read `CLAUDE.md`.
2. Read `GLOSSARY.md` and `docs/adr/`.
3. Read "Agreed design" below. Most of it isn't recorded anywhere else.
4. Read "UI design" below, ADR 0006 and `docs/design/ui-prototype.md` when working on the UI.
5. For AI-native work, read "AI-native" below, ADR 0007 to 0012, `docs/design/model-file.md`, `docs/design/operation-catalogue.md`, and spec issue 28 with its sub-issues 37, 45, 56 and 63 (`gh issue view 28`).
6. Then go on from "Possible next steps".

## Current state

Checked on 2026-11-08, on Rust 1.99.1, at `master` after PR 61 (`4e0f772`): 492 tests of the default members and the MCP server pass, with 5 ignored: the long run, the test that writes the schema file again, and three timing tests of the catalogue on 6,006 objects (`cargo test --all-targets` and `cargo test -p crabsim-mcp`). The UI's 55 tests passed when last run, before the catalogue work (`cargo test -p crabsim-ui --profile ui`), and haven't been run since, though nothing they test has changed.

| Where | State |
|---|---|
| `master` on GitHub | Steps 1 to 6 of the build order, all of the UI's order of work, and steps 1 to 4 of the AI-native order of work through slice 3 of the catalogue, through PR 61 |
| Open issues | Specs #3 (only build step 7 left), #28 (steps 5 to 8 left) and #63 (UI stage 2, designed, not built), and #58, the MCP tool list's size, to monitor |
| `prototype/restore-replay` | The throwaway prototype from build step 5. Keep it, and never merge it |
| `prototype/ui-bevy` | The throwaway UI prototype from UI step 2, with temporary counters in the session. Keep it, and never merge it |
| Merged branches still on GitHub | 23, from `feat/headless-cli` to `feat/cli-graph`, all merged into `master`: safe to delete (`git branch -r --merged origin/master`). The old worktree `crabsim-catalogue` was removed on 2026-11-08 |
| Folders outside the repository | `C:\Users\marcu\crabsim-models`, the models folder of the first MCP trial, and `C:\Users\marcu\crabsim-trial`, where agents ran the two harness trials, with their reports |

The owner merges PRs on GitHub themselves, so fetch before assuming local `master` is current. The repo has no CI, so a local test run is the only check.

## How it got here

- **2026-11-05 and earlier:** the owner came back after about six months away. `master` didn't build, and was repaired. The domain design was agreed, an architecture review was done, and `SimulationRun`, occurrences and the first `LiveSession` were built. `crabsim-renderer`, its TCP code and `serde` were removed.
- **2026-11-06:** performance became the first design pillar, and steps 1 to 6 of the build order were designed and built. See "Build order".
- **2026-11-07:** the UI was designed in a grilling session, with technology research, and the target scale went up to 6,000 objects. Then steps 1 to 5 of the UI's order of work were built: Rust 1.99.1 and a new baseline, the Bevy prototype, ADR 0006, the workspace and the Picture, and the engine additions for stage 1. The prototype found a live-session bug that lost about 30% of shown time at high load, fixed in PR 19. After trying the prototype, the owner decided MUs move instantly in the UI. See "UI design". Later that day, AI-native became the second design pillar, in another grilling session. See "AI-native".
- **2026-11-07, later:** the owner tried the UI and found the step-back warning asked on every step, fixed in PR 32. Then issue 28's first steps: the `SessionOutput` rename, the model file designed in a round and built in three slices, and a design round for the operation catalogue and MCP v1, written up as spec issue 37.
- **2026-11-07, at night:** slice 1 of issue 37 was built test-first through `call(name, JSON)` in three stacked PRs, 39 to 41, each test checked to be able to fail. The owner then asked how runs left by a crashed process get cleaned up, chose lock files and an `interrupted` state in a short round, and that was PR 42.
- **2026-11-08:** `crabsim-mcp` was driven over stdio as a harness drives it, on a models folder outside the repository (`C:\Users\marcu\crabsim-models`). Everything worked, but `validate_model`'s schema required `text` (fixed in PR 44, with a test that every operation's shortest input passes its schema), `run_results` grew to 162 KB for 100 observations, and the tool list cost about 46 KB. The owner then held slice 2's design round before the harness trial, written up as spec issue 45, with ADR 0011 and the glossary's User, Editor and Edit queue. Slice 2's PR 1 (#47) fixed the sizes, and then an agent in Claude Code ran the first harness trial from `C:\Users\marcu\crabsim-trial`: it found the demo model's bottleneck and showed with paired intervals that a faster Station 2 raises throughput by 18.9%. Its report led to PR 1b and three more decisions in issue 45. It also showed that random numbers follow an object's position among the instances, so removing or inserting an instance breaks the pairing of runs.
- **2026-11-08, later:** the rest of slice 2 (PRs 49 to 52), `compare_runs` (#53), streams (#54, ADR 0012) and a second harness trial with its fixes (#55). Then slice 3 was designed in a round of two rounds, written up as spec issue 56, and built in PRs 59 to 61. Measuring the MCP tool list during slice 3 showed it at 80.6 KB, up from about 29 KB, and the owner chose to watch it in issue 58 and shrink it later.

## Agreed design

The owner decided all of this. Don't reopen it without a reason, and flag it if you need to. **Built** means it's in the code now, and **not built** means it's agreed for later.

### Performance (built where noted, otherwise not yet)
Performance is the first thing to weigh in design trade-offs. It is a guideline rather than a hard rule, and `CLAUDE.md` holds the wording and the measuring rules. The target scale is up to 6,000 objects (raised from "a few thousand" on 2026-11-07) and tens of simulated years per run, with no fixed speed target. The benchmark's large scenario is 6,006 objects (429 copies of the demo) since step 1 of the UI's order of work.

- **Headless run (built):** a plain simulation run with no live session or view, not a separate mode. It gives back its final state and the objects' data, with no reset, including after a run error.
- **Recording (built)** is a runtime setting on the run for which occurrence kinds to keep, changeable at any time, because replay needs it off and then back on. A run records nothing by default, and a live session records everything.
- **Results (built)** of fixed size come through `run.state()`, with `ObjectState` extended. Recorded occurrences come back from each advance (ADR 0004).
- **Statistics (built, for what today's model has)** are fixed-size, always on and independent of occurrences. See "Statistics" below.
- **Live session (built):** read-ahead with checkpoints and replay, on one thread that alternates between computing a slice and releasing what the clock has reached. See ADR 0002 for the mechanism and why.
  - **Buffer:** limited by a maximum number of instants, with no limit on how far ahead in time the run gets. When it's full the run waits and still handles commands.
  - **Time:** Halt, errors and everything else the user experiences follow shown time. Reset throws the buffer away. If computing falls behind the clock, shown time waits and carries on from there, with no debt. A panic while computing also waits for shown time to reach it.
  - **Time modes:** continuous, or skipping, where no gap between instants takes longer than a set wall time (0.25 s to start with). Switchable at any time, and results don't depend on it.
  - **Going back:** `go_back(to)` reaches any earlier instant, also out of a run error, and leaves the session halted with a full state. `step_forward()` shows one instant while halted, and `step_back()` goes to the instant before the one on screen. An instant is a time with occurrences.
  - **What the UI gets:** one full state at Init, after Reset and after a jump back, occurrences and `ShownTime` in between, and a full state on request, from a replay on a copy. `crabsim::Picture` rebuilds contents, process state, every active interruption and remaining time from them. Blocked predecessors and statistics come only from a full state.
- **Checkpoints (built):** kept up to a memory budget the user configures. When it is full, every other checkpoint in the older half goes and the one from Init stays, so the whole run stays reachable. Every 100,000 events, with 256 MB, to start with. A checkpoint is a clone of the run, plus how much of the intervention log is in it, and a test checks that restoring and replaying gives occurrences identical to the original run.
- **Interventions (built, on Stations only):** take effect immediately at shown time, without a Halt, and discard the computed future. Pause on and off, forced failure on and off, changing a process time, and clearing statistics for a warm-up. Same seed plus same intervention log gives the same result. There are no branching timelines. Reset can keep changed process times as the new default state, a session setting that is off by default.
- **Object storage (built):** one `Vec` indexed by id, successors stored as ids, an `enum` for the object type. See ADR 0005.
- **Threads:** each run executes on one thread at a time, and can move between threads. Cores are used through distributed simulation, which means many simulation runs at once, such as the observations of an experiment. One `ModelDescription` can already build many runs, but nothing else is built for this yet.
- **Time representation (built):** simulation time is a 64-bit count of milliseconds behind its own type, `SimTime`. Process times and sampled failure times are whole milliseconds, with sampled times rounded rather than cut off.

### Model
- **Material-flow objects** are Source, Station, Buffer and Drain. Each handles MUs and represents a real thing. "Station" means only the processing type. This is a general simulator for any material flow, such as a factory line.
- **Every material-flow object can fail**, so all of them get `StopCtrl`, even ones that consume no time, like a Buffer. Each gets a failure profile, and availability 1.0 means it never fails. *Not built: only Station has failures, and `StopCtrl` is `todo!()` everywhere.*
- **Station capacity** replaces a separate ParallelStation (ADR 0001). It's one machine working on several MUs at once, **not several parallel machines**. Capacity 1 is the default. *Not built.*
  - **Start modes:** batch start (work begins when full) and individual start (each MU starts on entry).
  - **Partly filled batch start:** waits indefinitely, with the waiting MUs visible in state. A timeout can come later if needed.
  - **Batch entrance:** closed once full, until the Station is completely empty.
  - **Individual entrance:** open whenever a slot is free. A finished MU still waiting to leave occupies its slot.
- **Buffer** holds MUs and stays a separate concept from Station even though it can behave alike (ADR 0001). *Buffer's movement is still broken.*
  - **Dwelling time:** one fixed value per Buffer, default 0, counted individually from each MU's entry.
  - **Stops and dwelling time:** a stop doesn't affect dwelling time by default, and this is configurable.
  - **What a stopped Buffer does:** entrance and exit are closed, and it keeps its contents.

### Stops
- **Interruption** is the cause (a failure or a pause). **Stop** is the resulting state of the object. A failure from t=20 to 30 plus a pause from t=25 to 35 is two interruptions and one stop from t=20 to 35.
- **Failure** is unplanned, with random timing. **Pause** is planned and commanded from outside, and doesn't count against availability. They are independent of each other. *Built on Stations only, including overlap. A second failure while already failed is still discarded.*
- **Forced failure** is a failure the user starts and ends by hand, to see what happens if a machine breaks down now. It counts as a failure, and random failures carry on underneath it. It has its own stop cause, `ForcedFailure`, between `Failure` and `Pause` in seriousness. *Built on Stations only.*
- **Reporting overlapping interruptions:** each begins and ends as its own occurrence, and a full state reports the most serious as `stop` and lists all of them in `interruptions`. *Built.*
- **Completion time** = entry + process time + time spent stopped. Overlapping stops count once. *Built on Stations, including overlap.*
- **A stop by type** (*built only for Stations*):
  - **Station:** takes no MU, releases no MU, work is frozen. A whole Station stops at once, whatever its capacity.
  - **Source:** creates no MUs, and its next creation moves later by the stop's duration.
  - **Buffer:** as above.
  - **Drain:** refuses MUs, so predecessors become `Blocked` and retry when it's back.
- **Failure time bases**, configurable, with not every object needing all three (*not built, behaves as simulation time*):
  - **Simulation time (default):** all time counts, whatever the object is doing, including during a pause.
  - **Working time:** idle and blocked count, and only a pause freezes the clock.
  - **Processing time:** only time actively working on at least one MU counts, once however many MUs are in work. A finished MU waiting to leave doesn't count.
- **Availability** is measured against the chosen time basis, so a rarely busy Station on a processing-time basis is available more of real time than its setting. That's expected.

### Moving MUs
- **Push** is the current concept. **Pull** is planned, as a setting on the *receiving* object, where the successor chooses which MU it wants from its predecessor. Only build a seam for "which MU leaves" now, not pull itself. Mixed push and pull cases get decided when pull is designed.
- **Exit strategy** picks which MU leaves, and **distribution strategy** picks which successor gets it. They're two separate rules. The default exit strategy is one at a time, earliest finished first, ties broken by entry order. More strategies may come later. *Not built as a seam.*
- **Under push, MUs can't overtake within a Station**, because successors belong to the Station rather than to one MU. Only pull could change that.
- **Blocked predecessors:** the longest-waiting one retries first, and each is listed once. Built, including at one instant: retries run before other events at the same time, and offer only the successor that sent them.
- **Reproducibility:** the same seed on the same version always gives identical results. Results may differ between versions when behaviour changes on purpose.
- **A move takes no simulation time,** and the UI shows it that way: the MU jumps straight to its new object. Real transport time would be a new material-flow object, such as a conveyor, and gliding belongs to that.

### Running a simulation (built, except where noted)
- **Controls:** Init, Start, Halt, Reset. Halt avoids clashing with Stop and Pause, which happen to objects.
- **Init is its own step**, the first step of every run: t=0, every object's init in id order, first events queued but not processed, even those due at t=0. Start does Init if needed, then advances. Start and Halt are commands on a live session.
- **Reset** returns objects to their default state and keeps them in the model. Every run MU is deleted, and configured starting MUs come back as fresh copies. The same seed replays the same run.
- **The default state is what the user configured**, and can be changed. Today configuration changes are only allowed after Reset, except a Station's process time. That can change during a run, as an intervention: the MU in work keeps its time and the next MU uses the new one. *Built.* The agreed rule for later is that every attribute can change during a run, so each one needs its mid-run behaviour defined (see "AI-native").
- **Control order:** Init only from the default state, advancing requires Init, Reset allowed at any time. Violations return errors rather than panicking.
- **A run advances only when asked** and has no end time of its own. `advance_to(t)` includes events at t. Speed control belongs to the caller.
- **A run executes on one thread at a time.** A live session still creates its run on its own dedicated thread, though a run can now move.
- **Occurrences** are returned by each advance today. Views get no callbacks. A run can also be asked for its full typed state. "We might add more states later." A run records only when asked.
- **MU ids** are unique across the model.

### Statistics (built for what today's model has)
See `docs/design/statistics.md`, ADR 0003 and ADR 0004. Objects keep fixed-size, always-on statistics at two levels: machine state and MU time. Warm-up is a `clear_statistics()` call, and per-MU data exists only through recording occurrences. Pause and overlap are built on Stations only. Capacity above 1, dwelling time and the other time bases aren't built, so their numbers stay at zero or at today's behaviour until step 7.

### User code (not built)
The owner wants users to add their own source code later. The decisions so far:
- Each object has fixed init and reset parts that **users can't change**. For example, Reset always deletes all MUs.
- A second user phase runs after the fixed phase **has finished for all objects**, in id order, for both Init and Reset.
- User code must be deterministic and fit in a checkpoint, and objects that use none pay nothing for the feature.
- How the code gets in (scripting, plugins, WebAssembly) is open, and deserves its own ADR when decided.
- Don't build the user phase until there's real user code.

## Build order

Agreed on 2026-11-06, replacing the original repair plan:

1. ✅ Fix `blockedlist` (longest waiting first, deduped), with a fast test of the rule and a long-run test of 25 simulated years, marked `#[ignore]` and run on demand in release (PR 6, PR 7). This cured the false `ZeroTimeLoop`.
2. ✅ Add the benchmark, a tuned release profile and a first profile. `panic = "abort"` is out, because `LiveSession` relies on `catch_unwind`. See `docs/design/first-profile.md`.
3. ✅ Make the object storage change and add the millisecond time type. Tests build their models through a minimal model description, so storage details stay out of them. See ADR 0005.
4. ✅ Headless run without recording, with statistics for what today's model has. Memory is now flat with simulated length.
5. ✅ Throwaway prototype of restore and replay, for real cost figures. It held up: see `docs/design/restore-replay-prototype.md`. The code is on the branch `prototype/restore-replay`, never to be merged.
6. ✅ Read-ahead `LiveSession`, with Pause on Stations, going back, stepping forward and time modes.
7. ⬜ The earlier domain work, in this order. It comes after UI stage 1 unless the owner says otherwise, and drag and drop needs it, because the palette and forms depend on what each object can be configured with.
   - Finish stops on the other three object types, including failure profiles. Pause and overlap on Stations are done, through `PauseOn` and `PauseOff` events, while `StopCtrl` is still `todo!()`, so decide what becomes of it.
   - Fix `Buffer`, test it, and add dwelling time.
   - Add the time bases.
   - Add Station capacity and start modes, with the exit strategy seam.

The statistics that depend on domain work arrive with step 7.

`ExitTicket` was designed with capacity in mind, because a whole Station stops at once, but that's untested until capacity exists.

**Baseline:** `cargo bench --bench reference`, tuned release profile, no logger and no view, Rust 1.99.1, median of 7 repetitions on the owner's PC, measured on 2026-11-07. The demo model is 15.5M events per simulated year.

| Scenario | Events/s | One simulated year | Peak heap |
|---|---|---|---|
| Demo model, headless | 13.23M | 1.17 s | Under 0.05 MB, also over 10 years |
| 6,006 objects, headless | 6.75M | 2 days in 5.41 s | 16.9 MB |
| Demo model, one advance, occurrences kept | 9.15M | 1.69 s | 2,148 MB |

**What's left on the per-event path:** the new-events `Vec`, the likely source of most of the remaining 0.86 allocations per event, and statistics, which cost about 7 to 12% of a headless run and haven't been tuned. `docs/design/first-profile.md` has the figures and their limits.

## Architecture review

Candidates from the review. "Strong" and "worth exploring" were the review's own ratings.

| # | Candidate | Rating | Status |
|---|---|---|---|
| 1 | Simulation run you can step, pause and reset | Strong | ✅ Done |
| 2 | Observation seam, presentation out of the engine | Strong | ✅ Done |
| 3 | Model as data, with one module that validates and builds it | Strong | Started: `ModelDescription` builds a model, but it can't be read, saved or validated. Needed for drag and drop, load and save |
| 4 | Objects that report themselves | Strong | Partly done: state, connections and no more downcasts. Open: trim traits |
| 5 | One MU movement module | Strong | Open. The same exit logic exists in about 6 slightly different copies |
| 6 | Live session seam for the UI | Worth exploring | Built as read-ahead (ADR 0002) |
| 7 | One shared stop module | Worth exploring | Open. Worth deciding before stops reach the other object types |
| 8 | Reproducible runs and real errors | Worth exploring | Partly done: init order, event errors, stuck-timestamp guard, time representation |

ADR 0001 must not be contradicted without flagging it. The review's HTML report was in a temporary folder and may be gone.

## UI design

Decided with the owner on 2026-11-07, in a grilling session. Steps 1 to 5 of its order of work are built, and the UI itself isn't yet. The technology research is in `docs/design/ui-technology-research.md`, and the prototype's results in `docs/design/ui-prototype.md`. Like the rest of "Agreed design", don't reopen it without a reason.

### Who, where and what matters
- **Who:** the owner first. Later it's shared as a program to download, with no hosted service or accounts. Nothing should rule that out, but no effort goes into it now.
- **Where:** engine and window are always on the same PC. A long run on a bigger machine is a headless run with no UI.
- **Priority, by default:** building models first, then smooth, good-looking animation, then debugging. Animation matters because it's how non-technical people understand a model and come to trust it. Like performance, this is a default and not a rule: it can be overturned case by case, given real reasoning and a trade-off worth making.
- **Views:** a 2D view now. A 3D view **will** come later, as a second view of the same model, not a replacement. 2D stays for building and debugging, and 3D is for showing and watching. Keep that move in mind in every UI decision.
- **Scale:** up to 6,000 objects. The whole model can always be drawn, with pan and zoom, and every object always shows its state as a colour. MUs appear only when zoomed in far enough and running slowly enough to follow, drawn in the object that holds them. Above that, objects show counts and state only.

### Technology
- **Bevy with `bevy_egui`,** settled by the prototype and recorded in ADR 0006, with eframe and a custom wgpu canvas as the fallback. Bevy won because 2D and 3D share one renderer and one world, so the 3D view costs least, and its forms and panels are egui, the same as eframe's. Its price is upgrade churn, heavier builds and an ECS to learn.
- **Rules that limit the risk:**
  - The engine never depends on Bevy, and Bevy's ECS stays in the UI crate, so ADR 0005's storage is untouched.
  - The Picture lives in the engine library with no Bevy types, and Bevy only reads it.
  - Panels are plain functions taking `&mut egui::Ui`, so they could move to eframe. A wrong choice then costs the canvas, not the whole UI.
  - Bevy's version is pinned, and upgraded only when there's a reason.
- **ADR 0006** records the choice and what it costs, such as builds of 4 to 8 minutes clean and a UI that must show nothing changing while halted.
- **Toolchain (done):** Rust 1.91.1 was too old (Bevy 0.20 needs 1.97.1, egui 0.37 needs 1.95). It is now 1.99.1, pinned in `rust-toolchain.toml`, with a new baseline. The update changed no result.

### The prototype (done)
Throwaway, on `prototype/ui-bevy`, never to be merged, with its results in `docs/design/ui-prototype.md`. Every mark passed: 6,006 objects at 367 to 484 frames per second, a real session at 3,600 times real time with no frame over 33 ms, and a halted window at 0.006 CPU seconds per second. The owner tried it by hand. It answered these four things:
1. **Scale:** 6,000 flat objects with state colours, 5,000 gliding markers and labels on zoom, at a steady 60 frames per second on the owner's AMD Radeon RX 9070 XT. Compare sprites with flat meshes under an orthographic camera, since that decides how cheap the 3D view really is.
2. **Real session:** a live session at 6,000 objects and 1 simulated hour per second behind a window, on a real clock, with no visible stutter. That's about 1.9M occurrences per second by calculation. Nothing has driven a session on a real clock for long, so this is needed whatever the technology.
3. **Panels and canvas together:** dragging from an egui palette into the canvas, clicks reaching the right layer, selecting one of 6,000 objects, and little CPU while Halted.
4. **Build times,** clean and incremental, judged rather than held to a limit.

If 1 or 3 fails, try eframe. If 2 fails, the problem is in the session, not the technology.

### Structure
- **Workspace (built):** the engine stays at the repo root with its tests and benchmark. The member crates are `model/` (`crabsim-model`), `cli/` (`crabsim-cli`) and the UI, `ui/` (`crabsim-ui`), the only one that isn't a default member, so `cargo test` and `cargo bench` at the root never build Bevy (ADR 0009). Run the UI with `cargo run -p crabsim-ui --profile ui`, a release profile without LTO that rebuilds in seconds.
- **Picture (built):** `crabsim::Picture` in `src/picture.rs`, logical state only, what is where and since when. Feed it every session output in order. Visual state, such as where a marker is drawn on screen, stays in the UI.
- **CLI (built):** a simple headless CLI. It runs the demo model with `--seed` and `--length`, the length written as the UI shows time, then prints each object's final statistics, each Drain's throughput and mean lead time, and how many events it ran in how long. A run error still prints the statistics up to it. Since slice 1 of the model file it also runs a model file, with `--model`, and it lives in `cli/`. `--inplace`, `RunView`, the terminal table and the topology text are gone.

### Model data for the UI
- **Names and positions (built)** on `ModelDescription`, as optional fields: `set_name`, `set_position`, `set_dimensions`, with getters, `kind` and `object_count`. Stage 1 writes them by hand for the demo, with no automatic layout. Drag and drop will later write the same fields. The run ignores them, so a headless run pays nothing.
- **Units:** metres for everything. Position is the object's centre, as x, y and z, with z = 0 for now.
- **Dimensions (built):** length, width and height, plus a rotation around the vertical axis in degrees. All optional, with a default size per object type: placeholders of 2 × 1.5 × 1.5 m for a Station, 2 × 1 × 1 m for a Buffer, and 1 m each way for a Source or Drain. Stage 1 draws objects to size, and height waits for 3D. They are **visual only**: MU moves stay instant whatever the distance. Dimensions matter for results only in future transport objects such as a conveyor.
- **Layout image** (with drag and drop, step 3 of the order below): an image of the real site under the model. Its scale comes from clicking two points and entering the real distance. One per model, lying flat, and the floor in 3D. A headless run and the CLI never load it.

### UI stage 1
- **In:** the demo model drawn in 2D with state colours and contents, MUs shown in the object that holds them, Init, Start, Halt, Reset, speed and time mode, shown time, step forward and back, jumping to an earlier time, Pause, forced failure and process time changes on a Station, an inspector, and pan and zoom.
- **Out:** building or editing a model, save and load, scale work beyond a stress check, and polish.
- **Window:** a fixed layout with no docking yet. The top bar has Init, Start, Halt, Reset, step back and forward, jump to a time, speed, time mode and shown time. The 2D view is in the centre, and the inspector is on the right. Defaults are continuous time mode at 60 times real time, which is about 30 events per second on the demo by calculation. Speed is on a logarithmic slider. When playback can't keep up, the top bar shows the speed actually achieved. A run error shows as a banner, and going back out of it stays possible.
- **Inspector:**
  - **State:** type, name, process state, contents and remaining time.
  - **Why it's blocked:** the MU it holds and each successor's state, such as "holding MU 17: Station 4 is working, Station 5 has a forced failure".
  - **Why it's stopped:** every active interruption and since when.
  - **Process time:** current and configured.
  - **Statistics and waiting predecessors:** these come only from a full state, so the UI calls `request_state` about twice a second while something is selected, and never otherwise.
- **Moves are instant:** an MU is drawn in the object that holds it and jumps straight to the next one, with no glide. The owner decided this after trying the prototype: gliding suggested travel time that doesn't exist. The gliding code is kept for a future conveyor, which has real transport time.
- **Interventions on the time display:** each shows as a mark. Going back past one asks for confirmation, saying how many will be lost. The engine rule stays that going back drops later interventions.

### Engine additions for stage 1 (built)
All built in PR 21, as planned except where noted. The benchmark gave identical results and speed within noise.
- **Forced failure,** on Stations for now: an intervention the user starts and ends, with no sampled repair time. It counts as a failure in statistics and against availability, random failures carry on underneath it, and an overlap is one stop. Built as its own stop cause, `ForcedFailure`, with `force_failure` and `end_forced_failure` on the session. It **can't reuse** `StartFailure` and `EndFailure`, because each random failure schedules the next only when it ends. Reusing them would start a second chain of failures, or, when a random failure came due during a forced one, discard it and end that Station's random failures for the rest of the run. *Cost:* two event kinds, handled only at a stop.
- **Changing a Station's process time** during a run: an intervention at shown time. The MU in work keeps its time and the next uses the new one. It goes in the intervention log, so replay and going back work. It applies directly to the run, as clearing statistics does, rather than as an event, because a time inside `EventType` would push `ScheduledEvent` past its 32 bytes. *Cost:* nothing per event.
- **Reset keeps changes,** a session setting, off by default, read when Reset is pressed. When on, a changed process time becomes the new default state, and Pauses, forced failures and clears of statistics are dropped. A change already dropped by going back is not kept: the values come from the intervention log, which holds what is true at shown time, rather than from the run, which may be ahead or mid-replay. Replaying the whole intervention log in the next run, as saved intervention scripts, could come later as its own feature.
- **`step_back`:** a session command that finds the previous instant by replaying from the nearest checkpoint, at about the cost of a `go_back`, or two when it has to fall back to an earlier checkpoint. The UI keeps no list of instants, so memory stays flat. An instant is a time with occurrences, so an event that records nothing, such as a stale exit, makes none.
- **Configuration in `ObjectState`:** a Station's current and configured process time, for the inspector.
- **Added on the way:**
  - **Every active interruption in `ObjectState`:** a picture taken while a failure and a pause overlapped used to lose the pause, because state reported only the most serious. Since UI slice 3, each comes as an `Interruption` with when it began, for the inspector's "since when". A Station keeps one start time per cause, written only when an interruption begins, so nothing per event.
  - **`NotAStation`:** `SessionError::CannotPause` became `SessionError::NotAStation`, and the run gained `RunError::NotAStation`.

### Order of work
1. ✅ Update Rust, raise the benchmark's large scenario to about 6,000 objects (429 copies of the demo), record the new baseline, and pin the version. See "After the Rust update and the 6,000-object model" in `docs/design/first-profile.md`.
2. ✅ The prototype, with its results in `docs/design/ui-prototype.md`. Bevy with `bevy_egui` holds up. It found a bug in the live session, which lost about 30% of shown time at high load. Fixed in PR 19: `release` now reads the clock once.
3. ✅ ADR 0006 for the technology.
4. ✅ The workspace, and the Picture moved into the library, as `crabsim::Picture`. The `ui/` crate only opens a window so far.
5. ✅ The engine additions above, with names, positions and dimensions on `ModelDescription` (PR 21).
6. ✅ UI stage 1, as described under "UI stage 1" above. Start from the prototype's code on `prototype/ui-bevy`, which already drains a session into a picture each frame, draws 6,006 objects with state colours, keeps a halted window idle, drags from a palette, moves objects and selects them. Keep the rules under "Technology": panels as plain functions of an egui UI, and no Bevy types outside the UI crate. Test the UI's logic through a view state with no window, as spec issue 3 describes. Built in four slices, each one runnable:
   1. ✅ The first working version: the view state (`ui/src/view.rs`) and camera, tested without a window, the demo laid out by hand in `demo::model()`, a live session drained every frame, objects drawn to size as flat meshes with state colours, connections, names, pan and zoom, and Init, Start, Halt, Reset and shown time. A halted window uses no CPU once settled, and a running one about 0.26 cores, capped at about 60 frames a second. **Known problem:** in about 1 launch in 7, a thread kept a core busy before the first Init or Start, until a burst of frames. 3 s of frames at startup made it rarer, and the cause wasn't found. Names shrink and grow with the model, up to 16 pixels, and go only below 5 pixels: the owner asked for this after trying it, instead of the prototype's rule of no names below 25 pixels per metre.
   2. ✅ Time controls: speed on a logarithmic slider from 0.1 to 1,000,000 times, whose number can be typed, with the speed reached shown in orange when playback falls below 95% of it over the last 2 s. Continuous or skipping gaps, step back and forward, going back to a typed time (as shown time is written, such as `1:30` or `2d 06:00:00`), and an error banner. A run error stays until Reset or going back, and a refused command's error until the next command. At 1,000,000 times the demo kept up, at about 0.4 cores. Intervention marks moved to slice 3, because there are no interventions to mark before it.
   3. ✅ Inspector and interventions: a click selects, a drag still pans. The inspector shows type, name, state, contents and work left, why it's blocked (the MU held and each successor's state), why it's stopped (each interruption and since when), process time now and configured, waiting predecessors and statistics. Those last come from `request_state`, at most twice a second while something is selected and only when the last full state may be out of date, so a halted window stays idle. Pause, forced failure and process time on Stations. Each intervention leaves a mark on a strip under the top bar, and going back or stepping back past one asks first, naming what it drops. Stepping back at first gave only an upper bound when the view hadn't seen the instant it goes to, so after one step back it asked on every step. The owner found this on 2026-11-07, and the view now asks the session where stepping back goes (`locate_step_back`), so it asks only on the step that drops an intervention. **Found on the way:** `request_state` straight after an intervention reported the run before it had handled the intervention's event at shown time, so a Pause was missing from the state. Fixed, with a session test.
   4. ✅ MUs drawn in the object that holds them, as off-white dots 0.5 m across, spread over a grid when there are several. They show from 6 pixels per metre, while halted or advancing at up to 120 times real time. Faster, each object holding MUs shows a count in its middle, where it can be read. A move takes no time, so an MU is drawn in its new holder at once, and after going back where it was then. A Source holds the MU it is working on from Init on, so it shows one too. A halted window with MUs drawn stays idle, at 0.004 cores.
7. ✅ Trim the CLI to the headless version, as described under "Structure". `format_time`, `format_time_exact` and `parse_time` moved from the UI into `crabsim::time`, so the CLI reads a length as the UI does. An end-to-end test runs the binary (`tests/cli.rs`). A simulated year in release ran at 14.1M events/s, a single run, so in line with the baseline.

After that: model as data with the file format and save and load, with the MCP server on top (see "AI-native"), then step 7 of the build order, then drag and drop, configuration forms and the layout image in 2D, then the 3D view. Drag and drop waits for step 7 because forms and the palette depend on what each object can be configured with, and a palette offering Buffer needs Buffer fixed.

**Left for later:** the model file format, whether a layout image is stored inside the model file, multiple images or floors, where 3D models come from, polish, and signing the download for sharing.

### What the engine gives a UI today

All of it is on `LiveSession` in `src/live_session.rs`, and `CLAUDE.md` has the overview.

- **Commands:** `start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `step_back`, `locate_step_back`, `set_reset_keeps_changes`, `set_end_time`, `request_state`, `pause`, `resume`, `force_failure`, `end_forced_failure`, `set_process_time`, `clear_statistics`.
- **Session outputs** (`SessionOutput`), read from a channel: `Occurrences`, `ShownTime`, `State`, `Status`, `Error`, and `StepBackTarget` in answer to `locate_step_back`.
- **Rebuilding the picture:** `crabsim::Picture` takes every observation in order and keeps what the user knows at shown time. One full state at Init, then occurrences give each object's contents, process state, every active interruption and remaining time. `WorkStarted` and `MuCreated` carry durations, so a progress bar needs no further messages. Tests prove this on 3 models: a line with failures, with overlapping pauses or forced failures, a line that blocks, and the full demo. A further test takes a picture while interruptions overlap. Buffer is left out, because its movement is broken.
- **Model data:** a `ModelDescription` holds names, positions and dimensions in metres. A session builds its run from a closure, so a UI keeps the description beside the session to draw from.
- **Statistics:** `request_state` gives exact numbers for the instant on screen in a few milliseconds.
- **Going back:** usually under 15 ms, and up to about 400 ms for far jumps on a large model past the memory budget. See `docs/design/first-profile.md`.
- **Reading has to keep up:** past 256 unread observations a session stops releasing, so a UI drains the channel every frame. At high speed, one session output can hold up to 1,000 instants, so it can't draw each of them.
- **The boundary:** a session sends about 2.5 occurrences per event. The research found that even a UI in another language wouldn't need them: the Picture could stay in Rust and send one small message per frame. This matters only if the technology changes.
- **Changing a model:** a session builds its run once, from a closure, so a changed model means a new session. Apart from a process time, which can change during a run and be kept by Reset, configuration changes have no API yet.
- **Determinism:** nothing in a UI may affect results. Same seed and same interventions, same outcome.
- **Domain language:** use `GLOSSARY.md` in the UI too, such as Halt for the whole run and Pause for an object.

### This PC

Checked on 2026-11-07, on Windows 11:

- **Installed:** Rust 1.99.1 (stable, and pinned by the repo) with cargo and rustup, for `x86_64-pc-windows-msvc` only. WebView2 runtime 156, and `gh`.
- **GPU:** AMD Radeon RX 9070 XT, plus the processor's integrated Radeon graphics.
- **Not installed:** Node, npm, pnpm, bun, `wasm-pack`, `trunk`, Tauri's CLI, and the `wasm32` target. Bevy needs none of these.
- **A window:** `crabsim-ui` opens one, and the prototype on `prototype/ui-bevy` draws a working model. A clean debug build of the UI takes about 8 minutes.

## AI-native

Decided with the owner on 2026-11-07, in a grilling session. AI-native is the second design pillar, after performance (`CLAUDE.md`, ADR 0007). The model file and slice 1 of the operation catalogue (MCP v1) are built. The spec is GitHub issue 28, a sub-issue of spec issue 3, with the user stories, the order of work and the test seams.

### Decided
- **The pillar:** anything a person can do with crabsim, an AI agent can do too: build a model, run it, read the results, manage models. It's a guideline like performance, and comes second to it.
- **One operation catalogue,** in Rust, with typed inputs and outputs and descriptions, offered through front-ends (ADR 0007). The MCP server is the first front-end and the starting point. In-app agents and skills, which call the user's own LLM endpoint with the user's tokens, come second, on the same catalogue.
- **Models are readable text files,** changed only through crabsim's operations: in the UI, the user's main way, in the CLI, or through an agent's harness (ADR 0011, decided on 2026-11-08, replacing the earlier decision that agents write files with their own tools). The user controls who edits: see "Catalogue slice 2" below.
- **MCP v1:** validate a model file with precise errors, a headless run with seed, length and optional warm-up, the observations of one experiment, and each object's statistics. Live-session control comes later. Designed in detail in spec issue 37: see "Operation catalogue and MCP v1" below.
- **Where:** a standalone `crabsim-mcp` crate over stdio, a non-default workspace member, so the engine never depends on it. The server inside the UI app, so the owner can watch an agent work, comes later as another front-end.
- **No LLM inside a simulation run** by default. The owner wants AI decisions inside a run as a later opt-in that accepts its cost. The concerns are in `docs/design/ai-decisions-in-runs.md`.
- **When:** after UI stage 1. Then model as data with the file format and MCP v1, on today's objects, before build step 7. Step 7 extends the format, and each new object option comes with its operation.
- **What agents do:** everything a user does, by piloting the UI or by changing the model file: run experiments, Start and Reset, change parameters, create objects and connect them. A models folder, which the server is started with, gets list, read, write, copy and delete operations limited to it, for agents without file tools of their own.
- **Changing a model in an open UI:** after Reset or before Init, through operations, after which the run is rebuilt. The UI applies each change on its own, rather than reloading the model, because a 3D view with many detailed objects would make a full reload slow. Changing a model's structure mid-run, such as adding a Station at t = 3 days and carrying on, is an open idea and not agreed: it would be a new kind of intervention that checkpoints, replay, statistics and the Picture all have to handle.
- **Model file:** designed in detail on 2026-11-07, in `docs/design/model-file.md`, and built in three slices: slice 1 (today's objects with built-in classes, validation, canonical saving, `models/demo.json`, the CLI's `--model`) and slice 2 (derived classes, inheritance per attribute, `Model::resolve` with each value's origin) and slice 3 (`models/model-file.schema.json`, generated with `schemars` from mirror types, with drift tests against the reader, and an optional `$schema` field in files) are built. The design round also decided that a Station's process time defaults to 0 (a dummy Station), a Source's interval may be 0, and a zero-interval Source that can reach a Drain through only zero-time objects is a validation error. The file code lives in its own crate and the engine gains nothing (ADR 0009), and the CLI moved to `cli/` because of it. JSON, with a JSON Schema generated from the Rust types and published beside it, which the MCP operations reuse as their input schema. It holds the model only: objects with their configuration, names, positions and dimensions, and connections. Seed, length, warm-up and the number of observations are inputs to a run. Every object has a unique name, connections refer to names, and ids follow the file's order. 3D models will be referenced by path, never embedded, and a run never loads them. The benchmark's 429 copies need unique names.
- **Runs over MCP are jobs:** `start_run` returns an id, with `run_status`, `run_results` and `cancel_run`. Observations run on parallel threads, the first real use of distributed simulation. Results always carry their seed and crabsim's version.
- **MCP library:** `rmcp`, the official Rust SDK, pinned to `=3.9.0`, the latest on 2026-11-07, needing Rust 1.88. tokio stays in `crabsim-mcp`. The engine gains nothing (ADR 0009).
- **Skills** use the `SKILL.md` format of the Agent Skills open standard (agentskills.io), which Claude Code, GitHub Copilot, OpenAI Codex, Gemini CLI and Cursor read, each from its own folder. crabsim's own skills, such as building a model from a description or finding the bottleneck, are written alongside MCP v1, as the test that the operations are enough. The same skills work in the in-app agents.
- **LLM endpoints (phase 2):** tokens only in the operating system's credential store, never in a model file or settings file. Two API shapes: Anthropic's Messages API, and the OpenAI-compatible chat API, which also covers local models.

### Operation catalogue and MCP v1 (slice 1 built)

Decided with the owner on 2026-11-07 in a design round, and written up as [spec issue 37](https://github.com/MarcHoff94/crabsim/issues/37), which has the user stories, every decision and the test seams. The short version:

- **Crates:** `crabsim-catalogue` (default member: the engine, `crabsim-model`, `serde`, `schemars`, `rusqlite` with SQLite built in, threads, no async) and `crabsim-mcp` (not a default member: `rmcp` `=3.9.0`, tokio, stdio). The CLI becomes a second front-end on the catalogue, for people.
- **Operations:** a Rust trait with a name, a description and typed input and output that derive `schemars::JsonSchema`. `call(name, JSON)` runs one, and front-ends only translate. Structured errors (`invalid_input`, `invalid_model` with every problem, `not_found`, `outside_models_folder`, `already_exists`, `run_error`), which MCP returns as tool errors. Read-only and destructive hints over MCP.
- **Scope, the owner's concern:** crabsim's operations touch only `.json` model files in one models folder and crabsim's own `.crabsim/` there. Never crabsim's code, commands or the network. The server refuses a folder holding a `Cargo.toml`, `.git` or `.rs` file. A harness's own file tools are the harness's business.
- **Slice 1, MCP v1:**
  - `validate_model` (path or text), `list_models`, `read_model`, `write_model` (valid saved canonical, invalid saved as given with its problems), `copy_model`, `delete_model`
  - `start_run` as a job, `run_status` with simulated time reached, `cancel_run`
  - `run_results` (a short summary by default, filterable by objects and classes), `query_results` (read-only SQL, capped), `label_run`, `forget_run`
- **Runs:** observation *k* gets `splitmix64(seed + k)`. One thread per core, each running one observation, and jobs take turns for free cores. A run error fails only its observation.
- **Results store:** `<models>/.crabsim/results.sqlite`, WAL mode, with tables `runs` (including the model's canonical text and hash, label and source), `observations` and a long, thin `statistics`. Statistic names: `mus_in`, `mus_out`, `*_share`, `failures`, `availability`, `throughput_per_hour`, `mean_lead_time_s`. Mean, 95% interval, minimum and maximum over observations.
- **The CLI:** `crabsim run`, `runs`, `results` and `query` with `--models`, writing to the same store with source `cli`. Without `--models` it's unchanged.
- **People, not only agents:** the owner was clear that jobs and results are for people too. The CLI covers that until the UI's results views, which are planned in issue 37's Further Notes: a list of runs, side-by-side comparison with intervals, the model coloured by a figure, an object's figures across runs, and CSV export.
- **Recorded:** `docs/design/operation-catalogue.md`, ADR 0010 for SQLite as the results store, and the glossary words Models folder, Job and Results store.
- **Built in slice 1, and what it settled beyond the spec:** inputs carry a model as the file's text, with the model file's schema as `contentSchema`, so a harness never refuses a draft `write_model` must save. Effects are read-only, additive or destructive, and MCP's hints follow from them. Seeds are stored as text, since they use all 64 bits. The default summary leaves out objects with a blocked or failed share of 0. `cancel_run` returns once nothing of the run runs, and observations that ended keep their results. A failed observation gives no figures. `forget_run` is refused while a run is still running here, and run numbers are never reused.
- **Interrupted runs:** a run whose process ended first is marked `interrupted`, found through the lock file each catalogue holds in `.crabsim/processes/` (tables version 2). Agreed with the owner on 2026-11-07 after slice 1.
- **Known limits:** only the process that started a run can cancel it. `list_models` doesn't follow links into folders. Statistics come from what today's objects keep, so Buffers and per-MU figures wait for later.
- **Later slices of the catalogue:** 2, building and attributes, and 3, `describe_model`, the graph operations and exports, are built. Then skills, experiments and studies, and live-session operations.

### Catalogue slice 2 (built)

Decided with the owner on 2026-11-08 in a design round of six rounds, and written up as [spec issue 45](https://github.com/MarcHoff94/crabsim/issues/45) with user stories, every decision and the test seams. ADR 0011 holds the decisions that are hard to reverse. The short version:

- **`edit_model { path, changes, expected_hash? }`:** a batch of typed changes to a model file, all or nothing, checked once at the end, refused with every problem if the result is invalid. Instances, connections, classes, attributes and the model's description. `remove_class` is refused while anything depends on it unless `delete_dependents: true`. **`get_attributes`** gives every attribute with its value and origin. Files by path, no interventions yet.
- **The editor:** one per model file. Agents `claim_model` with a required purpose and wait in the edit queue, polling for now. A one-off change goes through when nobody holds or waits. An agent's right ends on release, when its process ends, or after 10 idle minutes.
- **The user controls the queue,** in the UI and the CLI, never over MCP: take the right at once, give it, choose who is next, remove, block and unblock an agent session. The user never waits and never times out. This is the AI-native pillar's first deliberate exception.
- **One person's front-end per models folder:** one UI window, holding `.crabsim/user.lock`. A second window means an independent copy of the folder. While the UI has it, the CLI can only read and run.
- **Changes from outside** crabsim, or files it has no record of, need `adopt_model` before crabsim changes them.
- **The trial's fixes:** `run_results` without per-observation values by default, `model_file_format` instead of three copies of the schema, `start_run` defaulting to seed 0 and 10 observations.
- **The CLI:** `edit`, `attributes`, `adopt` and the queue controls.

### Classes (decided, not built)
The owner wants models built in the UI later, by drag and drop, by setting values, and by deriving a class from an existing one or duplicating it and changing it. The terms are in `GLOSSARY.md`: class, instance, derived class, duplicate, inheritance. Class and instance are the words the UI uses too, because users know that much programming.
- **Derive and duplicate:** a derived class stays linked to its parent, and a duplicate is an independent copy. Classes can be derived from derived classes, and the UI and operations always show where a value comes from: built-in default, which class, or set on the instance.
- **Inheritance covers everything a user can change,** decided value by value: numbers such as a process time, and choices such as which exit strategy, once several exist. Setting a value cuts its inheritance.
- **What a derived class can change:** values only, at first. Behaviour comes later, from built-in choices such as exit strategies and from user code. A class made of several objects, as a sub-model, is its own later discussion.
- **Where classes live:** the model file holds every class it uses, so it stands alone. Shared libraries are a later discussion, with versioning as their main question.
- **A run never sees classes:** building a run works out every instance's values from its chain of classes. The per-event path is unchanged, and a headless run pays nothing.
- **Changing a class mid-run** reaches every instance whose inheritance of that value isn't cut, as an intervention at shown time. The session works out which instances that is from the model description and applies the change to each, so the run still knows nothing of classes.
- **Experiment and simulation study,** as the owner uses them: an experiment is a model with chosen values, run with a defined seed, several times with different seeds. A simulation study is several experiments of one model, each changing different values, each run a defined number of times. MCP v1 runs one experiment's runs. Studies follow, on the same job mechanism.

- **Observation** is one run of an experiment, the owner's word. "Replication" is to be avoided, and the code's `Observation`, a live session's output, was renamed `SessionOutput` as a small refactor of its own, along with `outputs()`, `OUTPUT_BACKLOG_LIMIT` and the module `observation.rs`, now `report.rs`.
- **Seeds:** an experiment has one defined seed, and its observation *k* gets a seed derived from it in a fixed way. In a study, observation *k* of every experiment uses the same seed by default (common random numbers), which can be switched off per study.
- **A study reports,** per experiment and statistic, the mean over its observations, a 95% confidence interval, and the minimum and maximum. Each observation's own figures stay available.
- **Attribute** is anything about a class or instance a user can set. Name, position and connections belong to the instance, and every other attribute, including dimensions and later the 3D model, can be inherited.
- **Interventions on instances:** setting a value on an instance cuts its inheritance, before or during a run. Restoring inheritance is its own intervention, and the instance then takes its class's current value.
- **Every attribute can change during a run, as an intervention.** That's the owner's general rule: users see attributes, never how an object works inside, and add or change behaviour only at defined hooks. A class may change an attribute during a run just as an instance may. "Reset keeps changes" applies to classes too. Today only a Station's process time can change during a run, so each attribute needs its mid-run behaviour defined when it's built. See ADR 0008, which the owner chose knowingly: watch its performance cost and reconsider it if the cost shows.
  - **Default rule:** what has already started or been scheduled keeps its values, and the change applies from the next decision on. Exceptions are decided per attribute when it's built.
  - **Refused changes:** a change that would leave the model invalid at that moment gets an error, and the value stays. For example, a capacity below the MUs held right now.
  - **Structure isn't an attribute:** instances and connections change only after Reset. The owner wants to explore mid-run structure changes later. Visual attributes change at any time, outside the intervention log.
  - **Cost:** interventions change the run directly, never as events, so the per-event path is unchanged. Each attribute's mid-run support is benchmarked as it's built.
  - **Tests:** every attribute gets one that changes it mid-run, goes back, replays and compares.
- **Attribute operations:** `set_attribute(target, attribute, value)` with an instance or class as target, `restore_inheritance(target, attribute)`, and `get_attributes(target)` with where each value comes from. Before Init or after Reset they change the model, and during a run they are interventions. Types and limits come from the Rust types and the JSON Schema, so an invalid value gets the same precise error in the file, the UI and over MCP.
- **In-app agents ask first** before writing or deleting a model file or making an intervention in the open session, with "ask every time", "allow for this session" and "always allow". Reading, validating and running are free. Over MCP the harness asks.

- **In-app agents are Markdown files** with a header (name, description, LLM endpoint and model, the operations it may call, the skills it loads, its default approvals) and its instructions below, shaped like Claude Code's subagents. Agents and skills live in the user's own crabsim folder, or beside a models folder when they belong to those models.
- **What an agent sees of a live run:** never the occurrence stream, which reaches about 1.9M occurrences per second. It reads state and statistics on request, runs until a time or a condition (such as a Drain's throughput falling below 50 per hour), and asks for recorded occurrences filtered to chosen objects and a time window.

### The model as a graph, for agents (decided)
An agent should never need to read a whole model: a model of 6,000 objects is probably 1 to 3 MB of JSON, an estimate. It retrieves only the context it needs.
- **What it can ask:** structure (neighbours, upstream and downstream within *n* steps, paths from Source to Drain, cycles), classes (the inheritance tree, which instances follow a class, where a value comes from), search by attribute, and structure joined with a study's results.
- **One source of truth:** the graph is derived from the model, kept up to date as operations change it, and read-only. Changes go through operations.
- **Graph operations in the catalogue first:** `neighbours`, `upstream`, `downstream`, `paths`, `subgraph_around`, `find_instances`, `class_tree`, computed on the model in memory, with no extra dependency. Beside them, export to standard graph formats (a Cypher script or Neo4j's CSV import, and GraphML), so anyone can load a model into their own graph database. An embedded graph database waits until the operations prove too limited, and gets its own ADR then. When this was decided the field was unsettled: Kùzu is archived, its fork RyuGraph is mostly C++, and NervusDB, in Rust, is young with a Cypher subset.
- **Results in the graph:** queries can include a study's results attached to instances, per experiment and observation. Results stay outside the model file.
- **Description:** classes and instances get an optional free-text description, visual-only, searchable and inherited, also shown in the UI.
- **`describe_model`** is an agent's entry point: instances per class, Sources and Drains, the main flows, cycles and validation problems, in a summary whose size doesn't grow with the model.

### Later, not agreed yet
- Search by meaning with embeddings (GraphRAG), if search by name, description and attribute isn't enough.
- An embedded graph database, if the graph operations prove too limited.
- Details of the in-app agent loop and LLM client.
- The MCP server inside the UI, so the owner can watch an agent work.
- Changing a model's structure during a run.
- AI decisions inside a run (`docs/design/ai-decisions-in-runs.md`).
- Shared class libraries and how they're versioned, together with a class library and explorer in the UI, with folders, deriving and duplicating there: its own design round.
- A built-in crabsim agent the user can opt in to give control of the edit queue, which knows the overall task, delegates to subagents and manages their access to the model, perhaps to parts of it. It would bring the queue's control back in line with the AI-native pillar.
- The edit queue notifying a waiting agent instead of being polled: a `claim_model` call that stays open with progress notifications, or MCP resource subscriptions.
- **The security implications** of the editor right and of detecting outside changes, which the owner wants revisited (2026-11-08): both guard against accidents, not against someone determined. See issue 45's Out of Scope.
- Sub-models: classes made of several objects.

JSON is the only model file format: YAML is out.

The grilling session ended here on 2026-11-07, with nothing left open.

## Known problems

- **`Buffer` is broken** (from reading, not run): `process_mu_exit` takes an MU and then the move takes another, so a lone MU never leaves, and with two or more, one is lost per move. It also never notifies blocked predecessors, and as a predecessor it ignores a retry's `from`. Its test only proves it replays consistently.
- **A predecessor gives up its place when any MU moves:** this assumes one MU per object. Station capacity, the Buffer fix and exit strategies must revisit it.
- **A stopped predecessor loses its place** (from reading): a retry that finds it stopped is used up, so it lists itself again at the back after its repair. The owner may want it to keep its place. Decide when `StopCtrl` is built.
- **Source with several successors** (from reading): a `BlockedRetry` can move an MU before its interval ends, because `is_exit_allowed` is always true.
- **Illegal connections panic while running:** an edge into a Source, or a self-loop. `ModelDescription` only rejects a successor on a Drain and ids not in the model, and full validation waits for candidate 3.
- **Dead code** hidden by `#![allow(dead_code)]`: `MUEntrance`, `Finished`, `exitlocked`, `Paused`, and `FailureProfile.mtbf`.

## Open housekeeping

- **The wiki hook stays off,** by the owner's choice: `core.hooksPath` is unset on this PC, so run `/update-wiki` by hand after changes to `src/`. The command only looks at the last commit and uncommitted changes, so after several commits check every page against the code, as the full rewrite on 2026-11-07 did.
- **Not decided:** whether to replace the new-events `Vec`, and whether to tune statistics, which cost about 7 to 12% of a headless run.
- **The MCP tool list's size, [issue 58](https://github.com/MarcHoff94/crabsim/issues/58):** 84.3 KB at the end of slice 3, with 24 tools, of which 38.9 KB (an estimated 11k to 13k tokens) is what Claude Code shows the model. To monitor, measuring again after each slice. The owner chose on 2026-11-08 to finish slice 3, then update the UI and start on the domain model (build step 7), before a design round on shrinking it.

Done on 2026-11-07: the wiki was rewritten against today's code, with four pages renamed and `wiki.html` rebuilt from it. The duplicate `docs/wiki/`, `docs/ExtensionGuide.md`, the stale `.github/copilot-instructions.md` and the tracked `crabsim.log` were deleted, and `*.log` is ignored. `rand` went to 0.9.3 for the Dependabot alert, with identical statistics on 5 seeds over 30 simulated days. Merged branches, the app's old worktrees and the 12 GB `ui-prototype/` build folder were deleted.

## How to work with the owner

- **Plain language and domain precision matter.** The owner knows material-flow simulation well and corrects loose models, for example that a ParallelStation is one machine rather than several. Listen for this and update `GLOSSARY.md` as terms settle.
- **Design before code.** The `grilling` and `domain-modeling` skills worked well, in rounds with recommendations where "yes" accepts. The owner often adds a requirement to a "yes", so read the whole answer.
- **Commit, push and merge only when asked.** The owner merges PRs on GitHub.
- **Test-first at agreed seams.** Behaviour tests go through `SimulationRun` in `tests/`, and session tests go through commands and observations with a fake clock.
- **Check that tests can fail.** Deliberately breaking the code keeps exposing real gaps. A same-seed replay can't detect events left over after Reset, because they land on the replay's own event times, and in step 6 this found a test that never exercised the rule it was named after. See `CLAUDE.md`.
- **Prove behaviour didn't change.** Steps 3, 4 and 6 hashed every occurrence and the final state against `master` before trusting a refactor, with a throwaway program outside the repo. Event counts alone wouldn't have shown a change in order.
- **Say what wasn't measured.** The owner reads the limits of a result, so say what a figure doesn't cover, and which ones are single runs.
- **The shell is Windows PowerShell 5.1.** `&&` doesn't work there, so give commands one per line. Git Bash is available to agents through the Bash tool.

## Possible next steps

In the owner's order of 2026-11-08:

1. **UI stage 2, [spec issue 63](https://github.com/MarcHoff94/crabsim/issues/63),** designed with the owner on 2026-11-08 in five rounds. The UI opens a models folder (`--models`) and runs its model files, laying out unplaced instances by flow and showing a model's problems instead of running it. It reaches the folder only through the catalogue, in-process, as a new `Source::Ui` that counts as the user. It checks the open model and the edit queue about once a second, behind one seam a file watcher could replace: it rebuilds the model at once when no run is in progress, and otherwise says Reset will use it. An "Editing" tab gives the user's queue controls, each checked against the latest queue before it acts. One window holds `.crabsim/user.lock`, and the CLI's changing commands are refused while it does. The top bar gains a seed, changeable before Init or after Reset, and an **End time**, a new live-session setting at which the run halts exactly. Build it in five PRs: the end time (#65, merged), the user lock with `Source::Ui` (#66, merged), opening a folder (PR 3, on `feat/ui-models-folder`), picking up changes, then the Editing tab. After that the owner can try taking the editor right from an agent in the UI.
2. **Step 7 of the build order,** the domain work: stops on Source, Buffer and Drain with failure profiles (and what becomes of `StopCtrl`), fixing Buffer with dwelling time, the failure time bases, and Station capacity with start modes and the exit-strategy seam. Drag and drop, forms and the layout image wait for it. Each new attribute arrives with its operation and its mid-run behaviour (ADR 0008).
3. **The rest of issue 28:** crabsim's first skills, as the test that the operations are enough (step 5), experiments and studies (6), live-session operations and class changes as interventions (7), then in-app agents (8).
4. **The MCP tool list's size, [issue 58](https://github.com/MarcHoff94/crabsim/issues/58):** a design round on shrinking it, when the owner chooses. Measure it again after each slice.

Every model file written before slice 2, such as those in `C:\Users\marcu\crabsim-models`, needs `adopt_model` once before crabsim changes it. A results store at an older tables version is upgraded in place the first time a new build opens it.
