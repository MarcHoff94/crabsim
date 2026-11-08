---
title: Live Session & UI
section: architecture
related:
  - simulation-run
  - running-crabsim
  - configuration-flags
---

# Live Session & UI

A headless run needs none of this. A live session plays a run on a clock for someone to watch, and the UI draws it. Both are optional layers that cost a headless run nothing.

## Live session

`LiveSession` (`src/live_session.rs`, ADR 0002) owns a thread on which a builder closure creates the whole run. It knows nothing about any UI technology.

- **Read-ahead:** the run computes ahead as fast as it can into a bounded buffer of instants, and the clock releases what it computed. Everything the user sees follows **shown time**, while the run is at a later **computed time**. One thread alternates between computing a slice and releasing.
- **Commands in:** `start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `step_back`, `locate_step_back`, `set_reset_keeps_changes`, `set_end_time`, `request_state`, `pause`, `resume`, `force_failure`, `end_forced_failure`, `set_process_time`, `clear_statistics`.
- **Outputs out:** `Occurrences`, `ShownTime`, `State`, `Status`, `Error`, and `StepBackTarget`, which answers `locate_step_back` with where stepping back would go, changing nothing. A full state comes only at Init, after Reset, after going back and on request. A session records everything, because its observers rebuild what they show from occurrences.
- **End time:** `set_end_time(Some(t))`, later than shown time, makes the session compute no further than `t` and halt there by itself, with everything at `t` done, as status `ReachedEnd`. Start and stepping forward are refused there (`AtEndTime`) until it moves later or is cleared, and stepping forward goes no further than it. It isn't an intervention: it isn't logged, and Reset and going back keep it, like the speed. Each computed slice stops at it, so it costs nothing per event.
- **Time modes:** `Continuous`, or `Skipping`, where no gap between instants takes longer than a set wall time (0.25 s to start with). Results don't depend on it.
- **Interventions** (Pause, forced failure, a process time, clearing statistics) take effect at shown time and discard the computed future. They go in an intervention log, so the same seed and the same log give the same result.
- **Backlog:** sending never blocks the session thread. Past `OUTPUT_BACKLOG_LIMIT` (256) unread outputs it stops releasing until they are read, keeps handling commands and drops nothing. A UI drains the channel every frame.
- **Clock:** wall time comes from a `Clock`. Tests use a fake clock with `sync`, which blocks until the session has handled earlier commands and shown time has reached the clock.

### Checkpoints and replay

A checkpoint is a clone of the run plus how much of the intervention log is in it. To reach an earlier instant, the session restores the nearest earlier checkpoint and replays with recording off, applying the logged interventions it passes. Going back drops the computed future, later log entries and their checkpoints. `step_back` finds the instant before the one on screen the same way, so nothing remembers past instants. Checkpoints are taken every 100,000 events within a memory budget (256 MB by default); past it, every other checkpoint in the older half goes, and the one from Init always stays.

## Picture

`Picture` (`src/picture.rs`) is what the user knows about a run at shown time. A UI feeds it every output in order through `observe`. A full state replaces it; occurrences then update each object's contents, process state, interruptions with when each began, and remaining time. Statistics and waiting predecessors come only from a full state. It holds nothing about drawing, so a 2D view, a 3D view and tests read the same thing.

## UI

`crabsim-ui` (`ui/`) is a Bevy application with egui panels (ADR 0006). Started with `--models <folder>`, which is required, it lists the folder's model files and runs the one clicked. Until one is chosen the canvas is empty, and when the folder can't be opened or another window has it, a banner says why and nothing opens. It runs the model in a live session at 60 times real time and draws it in 2D, as flat meshes under an orthographic camera:

- a "Models" panel listing the folder's model files, marking those with problems; a model that can't run shows `describe_model`'s summary and every problem instead, and the model open stays; one clicked while a run is in progress replaces it once confirmed,
- instances a file leaves unplaced laid out by the window: in columns by the fewest connections from a Source when nothing is placed, or in a row below the placed ones, never saved,
- the model's name, a seed (only before Init or after Reset, kept across models) and an end time in the top bar,
- the folder reached only through the catalogue as `Source::Ui`, holding `.crabsim/user.lock`, and a banner when another window has the folder,
- before Init or after Reset, every attribute of the selected instance in the inspector, with where its value comes from, changed in the model file through `edit_model` and each restored on its own; while an agent holds the file, "Change anyway" takes over for one change,
- an Editing tab beside the inspector: who holds the open model's editor right and who waits, other files and blocked agents, with the user's controls, each checked against the latest queue before it acts; taking the right from an agent asks first, and the agent waits first in line,
- changes to the open model file picked up, such as an agent's edit: used at once with no run in progress, and otherwise a banner says Reset will; a file changed outside crabsim is marked, with an Adopt button; a halted window stays idle, woken by a thread only when the file's size or modified time changes,

- state colours, connections, and names that shrink with the model,
- pan and zoom, and a click to select,
- Init, Start, Halt, Reset, stepping back and forward, going back to a typed time, speed on a logarithmic slider, and time mode,
- a banner for errors,
- an inspector with state, why an object is blocked or stopped, process times, waiting predecessors and statistics, plus Pause, forced failure and process time changes on Stations,
- a mark for each intervention on a strip under the top bar; going back past one asks first, naming each it drops. Stepping back asks only on the step that drops one: if the view didn't see the instant it goes to, it halts and asks the session (`locate_step_back`) before deciding,
- MUs drawn in the objects that hold them when zoomed in and slow enough to follow, and otherwise a count.

Its logic is in plain Rust, tested without a window:

- `ui/src/workspace.rs` — what the window works on: the models folder through the catalogue, the model list, the model open and laid out, its problems, the seed, and sessions built from them. `check` is the one seam for noticing the open file changed: size and modified time first, then its hash.
- `ui/src/editing.rs` — the edit queue as the Editing tab shows it, and what changed between two readings.
- `ui/src/attributes.rs` — the inspector's fields for each attribute, from a model file's value to text and back.
- `ui/src/view.rs` — the view state: from the `ModelDescription`, the session's outputs and wall time, it works out what is drawn, which controls are offered and whether frames are needed. A halted window draws nothing once things settle.
- `ui/src/camera.rs` — where the view looks and how close.
- `ui/src/marks.rs` — the interventions made, mirroring the session's log.
- `ui/src/main.rs` — Bevy and egui, drawing only what the view state says. Panels are plain functions taking `&mut egui::Ui`.

The rules from ADR 0006: the engine never depends on Bevy, Bevy's ECS stays in the UI crate, and the Picture has no Bevy types.

## Code references

- `LiveSession`, `SessionOutput`, `SessionSettings`, `TimeMode`, `Clock` — `src/live_session.rs`
- `Picture`, `ObjectPicture` — `src/picture.rs`
- `ViewState` — `ui/src/view.rs`
- ADR 0002 (read-ahead session), ADR 0006 (UI technology) — `docs/adr/`
