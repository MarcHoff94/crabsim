---
title: Simulation Run (AI context)
for: architecture/simulation-run.md
section: architecture
---

## Facts
- `SimulationRun` holds objects, `BinaryHeap<ScheduledEvent>` (32 bytes each), clock, `next_seq`, zero-time counters, seed, recording, `statistics_since`, `EventContext`
- `init`: t=0, per-object seed, each object's `init` in id order, first events queued but not processed (even at t=0); only from the default state
- `step`, `advance_to(t)` (includes t, clock left at t), `advance_bounded(t, n)`, `advance_events(n)`; splitting an advance changes nothing
- `reset`: every object to its default state; `state()`: full typed `RunState` incl. `ObjectStatistics`, also after an error
- `schedule_event`, `set_process_time` (from the next MU, schedules nothing), `keep_changes_as_default`, `clear_statistics`, `set_recording` (`NOTHING` default, `ALL`, `only(&[...])`)
- Failure → `AdvanceFailure { error, occurrences }`; errors: `NotInitialised`, `AlreadyInitialised`, `EventInThePast`, `UnknownObject`, `NotAStation`, `ObjectFailed`, `ZeroTimeLoop`
- Stale exits: `MUExit(ExitTicket { mu_id, stop_count })`; a stop bumps `stop_count`, so the old exit is ignored

## Invariants
- Order: time, then `BlockedRetry` first, then `seq`; preserve `seq` assignment
- Control order: Init only from default state, advancing needs Init, Reset any time; violations are errors, not panics

## Pitfalls
- A zero-time loop leaves its event queued, so the run keeps refusing until Reset

## Code locations
- `SimulationRun`, `BoundedAdvance` — `src/simulation_run.rs`
- `ScheduledEvent`, `ExitTicket`, `EventContext` — `src/event.rs`
- `RunError`, `AdvanceFailure` — `src/error.rs`
