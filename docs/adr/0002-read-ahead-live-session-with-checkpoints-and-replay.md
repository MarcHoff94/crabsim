---
status: accepted, not built. Step 5 of the build order in `docs/agents/handoff.md` tests it with a throwaway prototype before step 6 rewrites `LiveSession`. Reconsider this decision if the prototype fails.
---

# Read-ahead live session, with checkpoints and replay

A live session lets its simulation run compute ahead as fast as it can, into a bounded buffer, and its clock releases those results to the user, so everything the user experiences follows shown time while the run is at a later computed time. Interventions must still take effect immediately and users should be able to step back in time, so the session saves checkpoints up to a memory budget the user sets. To reach an earlier instant it restores the nearest earlier checkpoint and replays to that instant with recording off, relying on same-seed determinism. It then empties the buffer, sends a full state, and continues. We chose this because it gives steady playback and lets one mechanism serve both stepping back and interventions, while headless runs, which never take a checkpoint, pay nothing.

## Considered Options

- **The clock sets a target and the run advances only that far (`LiveSession` as first built).** Commands apply immediately with no restore mechanism, but a slow stretch of computing stalls playback.
- **Read-ahead with a limit on how far ahead the run gets, and no restore.** Keeps commands fairly prompt, but after slowing from 1000× to 1× a Pause could land about 17 minutes late, and stepping back is impossible. The owner requires interventions to take effect immediately.
- **Rebuilding an earlier state by undoing occurrences.** Works for the picture but not for a run that must continue: occurrences don't describe the event queue, the positions of the random number generators or internal details such as `succ_pointer`. Adding those would make every occurrence larger, on the fast path.

## Consequences

- `run.state()` describes computed time, so it can't draw the current picture. The UI builds its picture from occurrences, which must therefore carry everything needed to rebuild the state. It gets a full state only at Init, after Reset and after a jump back.
- Every material-flow object must implement save and restore, as it must implement `reset`, and user code must fit in a checkpoint too.
- A seed alone no longer identifies a run with interventions, so a run keeps an intervention log. Going back behind an intervention removes it from the log, and abandoned futures are gone: there are no branching timelines.
- Costs so far are estimates: about 10–20 KB per checkpoint for the demo model, and about 0.2 ms to replay 1,000 events at today's speed.
