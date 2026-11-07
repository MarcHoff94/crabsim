# crabsim handoff

Written 2026-11-07 at the end of a working session, and updated the same day after the UI design session and after steps 1 to 5 of the UI's order of work. Read this first, then check it against the code and git, because it describes a moment in time.

**The engine is ready for UI stage 1, and the UI isn't built yet.** The `ui/` crate only opens a window. The next work is step 6 of the UI's "Order of work": UI stage 1. Go to "UI design" below.

## Start here

1. Read `CLAUDE.md`.
2. Read `GLOSSARY.md` and `docs/adr/`.
3. Read "Agreed design" below. Most of it isn't recorded anywhere else.
4. Read "UI design" below, ADR 0006 and `docs/design/ui-prototype.md`. The prototype's code on `prototype/ui-bevy` shows working Bevy 0.20 and `bevy_egui` code for most of stage 1. Then start the order of work at step 6.

## Current state

Checked on 2026-11-07, on Rust 1.99.1: all 279 tests pass, with 1 long-run test ignored (`cargo test --all-targets`).

| Where | State |
|---|---|
| `master` on GitHub | Steps 1 to 6 of the build order, and steps 1 to 5 of the UI's order of work, through PR 21 (`393fc6e`). No PRs are open |
| `prototype/restore-replay` | The throwaway prototype from build step 5. Keep it, and never merge it |
| `prototype/ui-bevy` | The throwaway UI prototype from UI step 2, with temporary counters in the session. Keep it, and never merge it |
| `docs/adr-0006-ui-technology` | Local only and never pushed. Its commit went into PR 20, so it can be deleted |
| Other branches | Merged, so they can be deleted, though the app's worktrees may be using some |

The owner merges PRs on GitHub themselves, so fetch before assuming local `master` is current. The repo has no CI, so a local test run is the only check.

## How it got here

- **2026-11-05 and earlier:** the owner came back after about six months away. `master` didn't build, and was repaired. The domain design was agreed, an architecture review was done, and `SimulationRun`, occurrences and the first `LiveSession` were built. `crabsim-renderer`, its TCP code and `serde` were removed.
- **2026-11-06:** performance became the first design pillar, and steps 1 to 6 of the build order were designed and built. See "Build order".
- **2026-11-07:** the UI was designed in a grilling session, with technology research, and the target scale went up to 6,000 objects. Then steps 1 to 5 of the UI's order of work were built: Rust 1.99.1 and a new baseline, the Bevy prototype, ADR 0006, the workspace and the Picture, and the engine additions for stage 1. The prototype found a live-session bug that lost about 30% of shown time at high load, fixed in PR 19. After trying the prototype, the owner decided MUs move instantly in the UI. See "UI design".

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
- **The default state is what the user configured**, and can be changed. Configuration changes are only allowed after Reset, for now, except a Station's process time. That can change during a run, as an intervention: the MU in work keeps its time and the next MU uses the new one. *Built.* More mid-run changes can come once their behaviour is defined.
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
- **Workspace (built):** the engine stays at the repo root with its tests and benchmark, and the UI is the member crate `ui/`, called `crabsim-ui`. `cargo test` and `cargo bench` at the root never build Bevy. Run the UI with `cargo run -p crabsim-ui --profile ui`, a release profile without LTO that rebuilds in seconds.
- **Picture (built):** `crabsim::Picture` in `src/picture.rs`, logical state only, what is where and since when. Feed it every observation in order. Visual state, such as where a marker is drawn on screen, stays in the UI.
- **CLI:** a simple headless CLI stays. For now it runs the demo model with a seed and a simulated length, then prints each object's final statistics and how long the run took. Once model as data exists it takes a model file. `--inplace`, the terminal view and the topology text go once UI stage 1 can show the demo.

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
  - **Every active interruption in `ObjectState`:** a picture taken while a failure and a pause overlapped used to lose the pause, because state reported only the most serious.
  - **`NotAStation`:** `SessionError::CannotPause` became `SessionError::NotAStation`, and the run gained `RunError::NotAStation`.

### Order of work
1. ✅ Update Rust, raise the benchmark's large scenario to about 6,000 objects (429 copies of the demo), record the new baseline, and pin the version. See "After the Rust update and the 6,000-object model" in `docs/design/first-profile.md`.
2. ✅ The prototype, with its results in `docs/design/ui-prototype.md`. Bevy with `bevy_egui` holds up. It found a bug in the live session, which lost about 30% of shown time at high load. Fixed in PR 19: `release` now reads the clock once.
3. ✅ ADR 0006 for the technology.
4. ✅ The workspace, and the Picture moved into the library, as `crabsim::Picture`. The `ui/` crate only opens a window so far.
5. ✅ The engine additions above, with names, positions and dimensions on `ModelDescription` (PR 21).
6. ⬜ UI stage 1, as described under "UI stage 1" above. Start from the prototype's code on `prototype/ui-bevy`, which already drains a session into a picture each frame, draws 6,006 objects with state colours, keeps a halted window idle, drags from a palette, moves objects and selects them. Keep the rules under "Technology": panels as plain functions of an egui UI, and no Bevy types outside the UI crate. Test the UI's logic through a view state with no window, as spec issue 3 describes.
7. Trim the CLI to the headless version.

After that: step 7 of the build order and model as data with save and load, then drag and drop, configuration forms and the layout image in 2D, then the 3D view. Drag and drop waits for step 7 because forms and the palette depend on what each object can be configured with, and a palette offering Buffer needs Buffer fixed.

**Left for later:** the model file format, whether a layout image is stored inside the model file, multiple images or floors, where 3D models come from, polish, and signing the download for sharing.

### What the engine gives a UI today

All of it is on `LiveSession` in `src/live_session.rs`, and `CLAUDE.md` has the overview.

- **Commands:** `start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `step_back`, `set_reset_keeps_changes`, `request_state`, `pause`, `resume`, `force_failure`, `end_forced_failure`, `set_process_time`, `clear_statistics`.
- **Observations**, read from a channel: `Occurrences`, `ShownTime`, `State`, `Status`, `Error`.
- **Rebuilding the picture:** `crabsim::Picture` takes every observation in order and keeps what the user knows at shown time. One full state at Init, then occurrences give each object's contents, process state, every active interruption and remaining time. `WorkStarted` and `MuCreated` carry durations, so a progress bar needs no further messages. Tests prove this on 3 models: a line with failures, with overlapping pauses or forced failures, a line that blocks, and the full demo. A further test takes a picture while interruptions overlap. Buffer is left out, because its movement is broken.
- **Model data:** a `ModelDescription` holds names, positions and dimensions in metres. A session builds its run from a closure, so a UI keeps the description beside the session to draw from.
- **Statistics:** `request_state` gives exact numbers for the instant on screen in a few milliseconds.
- **Going back:** usually under 15 ms, and up to about 400 ms for far jumps on a large model past the memory budget. See `docs/design/first-profile.md`.
- **Reading has to keep up:** past 256 unread observations a session stops releasing, so a UI drains the channel every frame. At high speed, one observation can hold up to 1,000 instants, so it can't draw each of them.
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
- **`ui-prototype/` on disk:** a leftover of the prototype branch, untracked on other branches, holding several gigabytes of build folders. It can be deleted.
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

1. **The UI's order of work,** in "UI design". Steps 1 to 5 are done, and so is the session fix the prototype found. Next is step 6: UI stage 1. This is what the owner asked for next.
2. Step 7 of the build order, after UI stage 1.
3. Run `/update-wiki` when convenient, so the docs stop describing removed code.
