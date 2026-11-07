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
- **Commands in:** `start`, `halt`, `reset`, `set_speed`, `set_time_mode`, `go_back`, `step_forward`, `step_back`, `locate_step_back`, `set_reset_keeps_changes`, `request_state`, `pause`, `resume`, `force_failure`, `end_forced_failure`, `set_process_time`, `clear_statistics`.
- **Outputs out:** `Occurrences`, `ShownTime`, `State`, `Status`, `Error`, and `StepBackTarget`, which answers `locate_step_back` with where stepping back would go, changing nothing. A full state comes only at Init, after Reset, after going back and on request. A session records everything, because its observers rebuild what they show from occurrences.
- **Time modes:** `Continuous`, or `Skipping`, where no gap between instants takes longer than a set wall time (0.25 s to start with). Results don't depend on it.
- **Interventions** (Pause, forced failure, a process time, clearing statistics) take effect at shown time and discard the computed future. They go in an intervention log, so the same seed and the same log give the same result.
- **Backlog:** sending never blocks the session thread. Past `OUTPUT_BACKLOG_LIMIT` (256) unread outputs it stops releasing until they are read, keeps handling commands and drops nothing. A UI drains the channel every frame.
- **Clock:** wall time comes from a `Clock`. Tests use a fake clock with `sync`, which blocks until the session has handled earlier commands and shown time has reached the clock.

### Checkpoints and replay

A checkpoint is a clone of the run plus how much of the intervention log is in it. To reach an earlier instant, the session restores the nearest earlier checkpoint and replays with recording off, applying the logged interventions it passes. Going back drops the computed future, later log entries and their checkpoints. `step_back` finds the instant before the one on screen the same way, so nothing remembers past instants. Checkpoints are taken every 100,000 events within a memory budget (256 MB by default); past it, every other checkpoint in the older half goes, and the one from Init always stays.

## Picture

`Picture` (`src/picture.rs`) is what the user knows about a run at shown time. A UI feeds it every output in order through `observe`. A full state replaces it; occurrences then update each object's contents, process state, interruptions with when each began, and remaining time. Statistics and waiting predecessors come only from a full state. It holds nothing about drawing, so a 2D view, a 3D view and tests read the same thing.

## UI

`crabsim-ui` (`ui/`) is a Bevy application with egui panels (ADR 0006). It runs the demo model in a live session at 60 times real time and draws it in 2D, as flat meshes under an orthographic camera:

- state colours, connections, and names that shrink with the model,
- pan and zoom, and a click to select,
- Init, Start, Halt, Reset, stepping back and forward, going back to a typed time, speed on a logarithmic slider, and time mode,
- a banner for errors,
- an inspector with state, why an object is blocked or stopped, process times, waiting predecessors and statistics, plus Pause, forced failure and process time changes on Stations,
- a mark for each intervention on a strip under the top bar; going back past one asks first, naming each it drops. Stepping back asks only on the step that drops one: if the view didn't see the instant it goes to, it halts and asks the session (`locate_step_back`) before deciding,
- MUs drawn in the objects that hold them when zoomed in and slow enough to follow, and otherwise a count.

Its logic is in plain Rust, tested without a window:

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
