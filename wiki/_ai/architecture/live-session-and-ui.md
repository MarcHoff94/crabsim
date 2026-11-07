---
title: Live Session & UI (AI context)
for: architecture/live-session-and-ui.md
section: architecture
---

## Facts
- `LiveSession` owns a thread that builds the run from a closure, computes ahead into a buffer of instants, and releases by a `Clock`; the user sees shown time, the run is at computed time (ADR 0002)
- Commands: start, halt, reset, set_speed, set_time_mode, go_back, step_forward, step_back, locate_step_back, set_reset_keeps_changes, request_state, pause, resume, force_failure, end_forced_failure, set_process_time, clear_statistics
- Observations: `Occurrences`, `ShownTime`, `State` (only at Init, after Reset, after going back, on request), `Status`, `Error`, `StepBackTarget { shown, to }` (answer to `locate_step_back`, changes nothing)
- UI step back: `ViewState::step_back()` gives `Now`, `Confirm(to)` or `Locate`; on `Locate` the window halts, sends `locate_step_back`, and `located_step_back()` decides once answered; any other command cancels
- Time modes: `Continuous`, `Skipping` (no gap longer than 0.25 s wall time by default)
- Interventions act at shown time, discard the computed future and go in an intervention log; Pause and forced failure reach the run through `schedule_event`
- Checkpoint = clone of the run + log position; going back restores the nearest earlier one and replays with recording off; every 100,000 events within 256 MB, thinning the older half, Init's always kept
- `Picture`: what the user knows at shown time, fed every observation in order; statistics and waiting predecessors only from a full state
- UI (`ui/`): Bevy + egui, demo at 60x real time, 2D flat meshes, controls, inspector, intervention marks, MUs in holders when zoomed in and slow
- UI logic in plain Rust: `view.rs` (view state), `camera.rs`, `marks.rs`; `main.rs` only draws

## Invariants
- Sending never blocks the session thread; past 256 unread observations it stops releasing, drops nothing
- Engine never depends on Bevy; Picture has no Bevy types; panels are plain functions of `&mut egui::Ui`
- Nothing in a UI may affect results

## Code locations
- `src/live_session.rs`, `src/picture.rs`, `ui/src/view.rs`, `ui/src/main.rs`
