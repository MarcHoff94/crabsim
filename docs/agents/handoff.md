# crabsim handoff

Written 2026-11-07 at the end of a working session. Read this first, then check it against the code and git, because it describes a moment in time.

**The next session is about the UI:** which technology to use and what matters most. Go to "The UI session" below.

## Start here

1. Read `CLAUDE.md`.
2. Read `GLOSSARY.md` and `docs/adr/`.
3. Read "Agreed design" below. Most of it isn't recorded anywhere else.
4. Read "The UI session" below, then start there with the owner. Decide before building.

## Current state

Checked on 2026-11-07: `master` at `f64aa16` passes all 247 tests, with 1 long-run test ignored (`cargo test --all-targets`).

| Where | State |
|---|---|
| `master`, local and GitHub | Steps 1 to 6 of the build order, through PR 14. No PRs are open |
| `prototype/restore-replay` | The throwaway prototype from step 5. Keep it, and never merge it |
| Other branches | Merged, so they can be deleted, though the app's worktrees may be using some |

The owner merges PRs on GitHub themselves, so fetch before assuming local `master` is current. The repo has no CI, so a local test run is the only check.

## How it got here

- **2026-11-05 and earlier:** the owner came back after about six months away. `master` didn't build, and was repaired. The domain design was agreed, an architecture review was done, and `SimulationRun`, occurrences and the first `LiveSession` were built. `crabsim-renderer`, its TCP code and `serde` were removed.
- **2026-11-06:** performance became the first design pillar, and steps 1 to 6 of the build order were designed and built. See "Build order".

## Agreed design

The owner decided all of this. Don't reopen it without a reason, and flag it if you need to. **Built** means it's in the code now, and **not built** means it's agreed for later.

### Performance (built where noted, otherwise not yet)
Performance is the first thing to weigh in design trade-offs. It is a guideline rather than a hard rule, and `CLAUDE.md` holds the wording and the measuring rules. The target scale is up to a few thousand objects and tens of simulated years per run, with no fixed speed target.

- **Headless run (built):** a plain simulation run with no live session or view, not a separate mode. It gives back its final state and the objects' data, with no reset, including after a run error.
- **Recording (built)** is a runtime setting on the run for which occurrence kinds to keep, changeable at any time, because replay needs it off and then back on. A run records nothing by default, and a live session records everything.
- **Results (built)** of fixed size come through `run.state()`, with `ObjectState` extended. Recorded occurrences come back from each advance (ADR 0004).
- **Statistics (built, for what today's model has)** are fixed-size, always on and independent of occurrences. See "Statistics" below.
- **Live session (built):** read-ahead with checkpoints and replay, on one thread that alternates between computing a slice and releasing what the clock has reached. See ADR 0002 for the mechanism and why.
  - **Buffer:** limited by a maximum number of instants, with no limit on how far ahead in time the run gets. When it's full the run waits and still handles commands.
  - **Time:** Halt, errors and everything else the user experiences follow shown time. Reset throws the buffer away. If computing falls behind the clock, shown time waits and carries on from there, with no debt. A panic while computing also waits for shown time to reach it.
  - **Time modes:** continuous, or skipping, where no gap between instants takes longer than a set wall time (0.25 s to start with). Switchable at any time, and results don't depend on it.
  - **Going back:** `go_back(to)` reaches any earlier instant, also out of a run error, and leaves the session halted with a full state. `step_forward()` shows one instant while halted.
  - **What the UI gets:** one full state at Init, after Reset and after a jump back, occurrences and `ShownTime` in between, and a full state on request, from a replay on a copy. The UI rebuilds contents, process state, stop and remaining time from occurrences. Blocked predecessors and statistics come only from a full state.
- **Checkpoints (built):** kept up to a memory budget the user configures. When it is full, every other checkpoint in the older half goes and the one from Init stays, so the whole run stays reachable. Every 100,000 events, with 256 MB, to start with. A checkpoint is a clone of the run, plus how much of the intervention log is in it, and a test checks that restoring and replaying gives occurrences identical to the original run.
- **Interventions (built, Pause on Stations only):** take effect immediately at shown time, without a Halt, and discard the computed future. Pause on and off, and clearing statistics for a warm-up. Same seed plus same intervention log gives the same result. There are no branching timelines.
- **Object storage (built):** one `Vec` indexed by id, successors stored as ids, an `enum` for the object type. See ADR 0005.
- **Threads:** each run executes on one thread at a time, and can move between threads. Cores are used through distributed simulation, which means many replications at once. One `ModelDescription` can already build many runs, but nothing else is built for this yet.
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
- **A move takes no simulation time.** Any gliding is visual only. Real transport time would be a new material-flow object, such as a conveyor.

### Running a simulation (built, except where noted)
- **Controls:** Init, Start, Halt, Reset. Halt avoids clashing with Stop and Pause, which happen to objects.
- **Init is its own step**, the first step of every run: t=0, every object's init in id order, first events queued but not processed, even those due at t=0. Start does Init if needed, then advances. Start and Halt are commands on a live session.
- **Reset** returns objects to their default state and keeps them in the model. Every run MU is deleted, and configured starting MUs come back as fresh copies. The same seed replays the same run.
- **The default state is what the user configured**, and can be changed. Configuration changes are only allowed after Reset, for now. More mid-run changes can come once their behaviour is defined; a new process time, for example, would apply to the next MU and not the current one.
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
7. ⬜ The earlier domain work, in this order. It comes after the UI's first stage unless the owner says otherwise, and a UI needs some of it: see "What the engine doesn't give a UI yet".
   - Finish stops on the other three object types, including failure profiles. Pause and overlap on Stations are done, through `PauseOn` and `PauseOff` events, while `StopCtrl` is still `todo!()`, so decide what becomes of it.
   - Fix `Buffer`, test it, and add dwelling time.
   - Add the time bases.
   - Add Station capacity and start modes, with the exit strategy seam.

The statistics that depend on domain work arrive with step 7.

`ExitTicket` was designed with capacity in mind, because a whole Station stops at once, but that's untested until capacity exists.

**Baseline:** `cargo bench --bench reference`, tuned release profile, no logger and no view, median of 7 runs on the owner's PC. The demo model is 15.5M events per simulated year.

| Scenario | Events/s | One simulated year | Peak heap |
|---|---|---|---|
| Demo model, headless | 12.93M | 1.20 s | Under 0.05 MB, also over 10 years |
| 2,800 objects, headless | 7.62M | 2 days in 2.23 s | 8.2 MB |
| Demo model, one advance, occurrences kept | 8.85M | 1.75 s | 2,148 MB |

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

## The UI session

The owner wants to completely replace the CLI with a real UI, and to choose its technology in the next session. Nothing below is decided. Use the `grilling` skill, in rounds where "yes" accepts a recommendation, and check current facts about each technology with the `research` skill rather than from memory.

### The goal

- **First:** a visualization of a run, as a basis to build on.
- **End goal:** drag and drop objects, configure them, start, stop, init and reset the simulation, animate and render it, watch it run, and control how fast simulation time passes.
- **Debugging models:** the owner's stated main purpose of going back in time is to make models easier to debug and understand. So stepping back and forward, and seeing why an object is blocked or stopped, are core features rather than extras.

### What the engine gives a UI today

All of it is on `LiveSession` in `src/live_session.rs`, and `CLAUDE.md` has the overview.

- **Commands:** `start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `request_state`, `pause`, `resume`, `clear_statistics`.
- **Observations**, read from a channel: `Occurrences`, `ShownTime`, `State`, `Status`, `Error`.
- **Rebuilding the picture:** one full state at Init, then occurrences give each object's contents, process state, stop and remaining time. `WorkStarted` and `MuCreated` carry durations, so a progress bar needs no further messages. Tests prove this on 3 models: a line with failures, with and without overlapping pauses, a line that blocks, and the full demo. Buffer is left out, because its movement is broken.
- **Statistics:** `request_state` gives exact numbers for the instant on screen in a few milliseconds, so ask a few times a second rather than every frame.
- **Going back:** usually under 15 ms, and up to about 400 ms for far jumps on a large model past the memory budget. See `docs/design/first-profile.md`.
- **Moves take no simulation time:** any gliding of an MU between objects is purely visual, and the UI invents its length.

### What the engine doesn't give a UI yet

- **Model as data (candidate 3):** `ModelDescription` can only be built up. It can't be read, saved or loaded, doesn't validate connections, and holds no names or positions. `serde` was removed. `ObjectState` reports kinds and successors, but not configuration such as process times. Drag and drop needs all of this, while a first visualization needs little of it.
- **Changing a model:** a session builds its run once, from a closure, so a changed model means a new session. Configuration changes after Reset, as agreed, have no API.
- **The picture logic exists only in a test:** `Rebuilt` and `apply_occurrence` in `tests/live_session.rs`. A Rust UI should move it into the library. A UI in another language has to port it, and then has two copies to keep in step.
- **Stepping back by instants:** the UI has to remember the instant times it has seen, because the session can't list them. That list grows with the run, so decide how much to keep.
- **Objects:** Pause works only on Stations, and other objects return an error. Buffer's movement is still broken, so a palette offering Buffer needs step 7 first.
- **Real playback is unmeasured:** tests mostly use a fake clock, and nothing has driven a session on a real clock for long. One thread alternates between computing and releasing, on a 16 ms tick. The first UI is what will show whether that stutters.
- **Reading has to keep up:** past 256 unread observations a session stops releasing, so a UI drains the channel every frame. At high speed, one observation can hold up to 1,000 instants, so it can't draw each of them.

### Questions to settle

In roughly this order, since later ones depend on earlier ones:

1. **Who is it for?** Only the owner, or other people too? That decides how much installing and sharing matter.
2. **Desktop or browser?** Must it run without installing anything? Would it ever run on a remote machine?
3. **Which technology?** See the options below.
4. **What is the first stage, exactly?** For example the demo model drawn with live contents, Start, Halt, speed and stepping. What is deliberately left out?
5. **How does it look at scale?** The target is a few thousand objects. Does the first version have to draw them all, animated? Pan and zoom? What happens at speeds too high to watch?
6. **Where does layout come from?** Positions are chosen by the user in the end. What does the first stage use before drag and drop exists?
7. **How is it structured?** A separate crate in a workspace would keep UI dependencies out of the engine, in line with the pillar that views cost nothing when absent. That means turning this repo into a workspace.
8. **What happens to the CLI?** The UI is meant to replace it. The headless run stays as a library use, and the benchmark and tests need no CLI.
9. **What needs doing first in the engine?** Probably moving the picture logic into the library, then model as data when drag and drop comes closer. Does any of step 7 have to come first?

### Options to weigh

From memory, so check the current state of each with `research`. The main dividing line is whether the UI runs in the same process as the engine, or has to send it everything across a boundary.

| Option | In process with the engine? | Worth knowing |
|---|---|---|
| Native Rust, immediate mode, such as egui | Yes | All Rust, nothing else to install, simple to draw a custom canvas, and redraws everything each frame |
| Native Rust, retained, such as iced or Slint | Yes | All Rust, and more structured, but custom canvases may be more work |
| Game engine, such as Bevy | Yes | Built for drawing many moving things, with heavier builds and more than a form-heavy UI needs |
| Rust with a web front end in a webview, such as Tauri | No | Web diagram and UI libraries, but every observation crosses a boundary, and the picture logic moves to the other side |
| Browser, with a local Rust server over WebSocket | No | Any browser, and the engine at full native speed, but the same boundary, plus a server to run. The removed `crabsim-renderer` used TCP, so its history in git may be useful |
| Browser, with the engine compiled to WebAssembly | Yes | Nothing to install and easy to share, but `LiveSession` uses `thread::spawn` and `Instant`, which don't simply work there, so the session would need rework, and it would run slower than native |

### What should weigh most

- **Performance stays the first pillar.** A UI is an optional layer, and a headless run must keep costing nothing because of it. Show before and after numbers from `cargo bench --bench reference`.
- **The boundary.** A session sends about 2.5 occurrences per event. Anything that serializes them across a boundary pays for that at high speed, and has to be measured before it is relied on.
- **Determinism.** Nothing in a UI may affect results: same seed and same interventions, same outcome. Tests already hold the session to this.
- **Domain language.** Use `GLOSSARY.md` in the UI too: Halt rather than Pause for the whole run, and Pause for an object.

### This PC

Checked on 2026-11-07, on Windows 11:

- **Installed:** Rust 1.91.1 with cargo and rustup, for `x86_64-pc-windows-msvc` only. WebView2 runtime 156, and `gh`.
- **Not installed:** Node, npm, pnpm, bun, `wasm-pack`, `trunk`, Tauri's CLI, and the `wasm32` target.
- **No window yet:** `crabsim-renderer` is gone, so nothing draws one.

## Known problems

- **`Buffer` is broken** (from reading, not run): `process_mu_exit` takes an MU and then the move takes another, so a lone MU never leaves, and with two or more, one is lost per move. It also never notifies blocked predecessors, and as a predecessor it ignores a retry's `from`. Its test only proves it replays consistently.
- **A predecessor gives up its place when any MU moves:** this assumes one MU per object. Station capacity, the Buffer fix and exit strategies must revisit it.
- **A stopped predecessor loses its place** (from reading): a retry that finds it stopped is used up, so it lists itself again at the back after its repair. The owner may want it to keep its place. Decide when `StopCtrl` is built.
- **Source with several successors** (from reading): a `BlockedRetry` can move an MU before its interval ends, because `is_exit_allowed` is always true.
- **Illegal connections panic while running:** an edge into a Source, or a self-loop. `ModelDescription` only rejects a successor on a Drain and ids not in the model, and full validation waits for candidate 3.
- **Dead code** hidden by `#![allow(dead_code)]`: `MUEntrance`, `Finished`, `exitlocked`, `Paused`, `FailureProfile.mtbf`, and much of `topology.rs`, including `print_network_topology` and its helpers.

## Open housekeeping

- **Stale docs:** the README mentions the removed renderer. 29 files across `wiki/`, `docs/` and the README still name `Simulator`, `EventController` or the renderer. Run `/update-wiki`. `wiki.html` is generated from the wiki.
- **Duplicate wiki:** both `wiki/` and `docs/wiki/` exist. Not decided.
- **Wiki hook is off on this PC:** `core.hooksPath` is unset. Turning it on with `git config core.hooksPath .githooks` makes every `src/` commit run `claude` in the background.
- **`crabsim.log` is tracked** in git, and running the demo from the repo root may change it.
- **[Dependabot alert](https://github.com/MarcHoff94/crabsim/security/dependabot/1):** one low-severity alert, not looked into.
- **Finished branches** can be deleted (see "Current state").
- **Not decided:** whether to replace the new-events `Vec`, and whether to tune statistics, which cost about 7 to 12% of a headless run.

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

1. **The UI session,** as above. This is what the owner asked for next.
2. Step 7 of the build order, for whatever the UI turns out to need first.
3. Run `/update-wiki` when convenient, so the docs stop describing removed code.
