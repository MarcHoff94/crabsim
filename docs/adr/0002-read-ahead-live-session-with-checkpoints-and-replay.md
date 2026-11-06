---
status: accepted, built in step 6 of the build order. The prototype in step 5 is in `docs/design/restore-replay-prototype.md`.
---

# Read-ahead live session, with checkpoints and replay

A live session lets its simulation run compute ahead as fast as it can, into a bounded buffer, and its clock releases those results to the user, so everything the user experiences follows shown time while the run is at a later computed time. Interventions must still take effect immediately and users should be able to step back in time, so the session saves checkpoints up to a memory budget the user sets. To reach an earlier instant it restores the nearest earlier checkpoint and replays to that instant with recording off, relying on same-seed determinism. It then empties the buffer, sends a full state, and continues. We chose this because it gives steady playback and lets one mechanism serve both stepping back and interventions, while headless runs, which never take a checkpoint, pay nothing.

## Considered Options

- **The clock sets a target and the run advances only that far (`LiveSession` as first built).** Commands apply immediately with no restore mechanism, but a slow stretch of computing stalls playback.
- **Read-ahead with a limit on how far ahead the run gets, and no restore.** Keeps commands fairly prompt, but after slowing from 1000× to 1× a Pause could land about 17 minutes late, and stepping back is impossible. The owner requires interventions to take effect immediately.
- **Rebuilding an earlier state by undoing occurrences.** Works for the picture but not for a run that must continue: occurrences don't describe the event queue, the positions of the random number generators or internal details such as `succ_pointer`. Adding those would make every occurrence larger, on the fast path.

## Consequences

- `run.state()` describes computed time, so it can't draw the current picture. The UI builds its picture from occurrences, which must therefore carry everything needed to rebuild the state. It gets a full state only at Init, after Reset, after a jump back and on request, and a state on request comes from a replay on a copy of the run.
- A checkpoint is a clone of the run, so every material-flow object has to be cloneable, and user code has to fit in a checkpoint too.
- A seed alone no longer identifies a run with interventions, so a run keeps an intervention log. A checkpoint records how much of the log is in it, and replay applies the logged actions it passes. Going back behind an intervention removes it from the log, and abandoned futures are gone, along with their checkpoints: there are no branching timelines.
- Measured costs: 17 KB per checkpoint for the demo model and 3.4 MB for 2,800 objects. Going back with a replay of 100,000 events takes 7 to 13 ms, within a frame. Taking a checkpoint every 100,000 events slows a run at full speed by up to 6.5%. A headless run takes none and pays nothing.
- When the memory budget is full, every other checkpoint in the older half goes and the one from Init stays. So the whole run stays reachable, recent history stays dense, and only long jumps take longer. Oldest-first was rejected because it made the early part of a run unreachable, and the main use of going back is to debug models.
