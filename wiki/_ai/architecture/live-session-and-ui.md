---
title: Live Session & UI (AI context)
for: architecture/live-session-and-ui.md
section: architecture
---

## Facts
- `LiveSession` owns a thread that builds the run from a closure, computes ahead into a buffer of instants, and releases by a `Clock`; the user sees shown time, the run is at computed time (ADR 0002)
- Commands: start, halt, reset, set_speed, set_time_mode, go_back, step_forward, step_back, locate_step_back, set_reset_keeps_changes, set_end_time, request_state, pause, resume, force_failure, end_forced_failure, set_process_time, clear_statistics
- Outputs: `Occurrences`, `ShownTime`, `State` (only at Init, after Reset, after going back, on request), `Status`, `Error`, `StepBackTarget { shown, to }` (answer to `locate_step_back`, changes nothing)
- UI step back: `ViewState::step_back()` gives `Now`, `Confirm(to)` or `Locate`; on `Locate` the window halts, sends `locate_step_back`, and `located_step_back()` decides once answered; any other command cancels
- End time: `set_end_time(Some(t))` (later than shown time) stops computing at `t` and halts there with everything at `t` done, status `ReachedEnd`; Start and step forward refused there (`AtEndTime`); not an intervention, kept by Reset and going back
- Time modes: `Continuous`, `Skipping` (no gap longer than 0.25 s wall time by default)
- Interventions act at shown time, discard the computed future and go in an intervention log; Pause and forced failure reach the run through `schedule_event`
- Checkpoint = clone of the run + log position; going back restores the nearest earlier one and replays with recording off; every 100,000 events within 256 MB, thinning the older half, Init's always kept
- `Picture`: what the user knows at shown time, fed every output in order; statistics and waiting predecessors only from a full state
- UI (`ui/`): Bevy + egui; `--models <folder>` (required, no built-in demo) lists model files (problems marked and shown instead of running) and runs the one clicked, through the catalogue as `Source::Ui` holding `.crabsim/user.lock`; nothing open until a model is chosen or when the folder is refused; inspector attributes before Init or after Reset via `get_attributes`/`edit_model`, each restored on its own, "Change anyway" as a one-change take-over; unplaced instances laid out (columns by flow, or a row below), never saved; seed (before Init or after Reset) and end time in the top bar; open file's changes picked up by `Workspace::check` (size and mtime, then hash), used at once with no run in progress or on Reset, outside changes marked with Adopt, a watcher thread waking an idle window; 60x real time, 2D flat meshes, controls, inspector, intervention marks, MUs in holders when zoomed in and slow
- UI logic in plain Rust: `attributes.rs` (inspector fields), `workspace.rs` (folder, models, layout, seed, sessions), `view.rs` (view state), `camera.rs`, `marks.rs`; `main.rs` only draws

## Invariants
- Sending never blocks the session thread; past 256 unread outputs it stops releasing, drops nothing
- Engine never depends on Bevy; Picture has no Bevy types; panels are plain functions of `&mut egui::Ui`
- Nothing in a UI may affect results

## Code locations
- `src/live_session.rs`, `src/picture.rs`, `ui/src/view.rs`, `ui/src/main.rs`
