---
title: Event Types (AI context)
for: reference/event-types.md
section: reference
---

## Facts
- `MUExit(ExitTicket)`: work done; Station ignores it if stopped or stale, else `WorkFinished` + move; Source moves and creates the next MU
- `BlockedRetry { from }`: successor `from` has room; offered to `from` only; runs first at its time
- `StartFailure`/`EndFailure`: random failures; first scheduled by `init`, each `EndFailure` schedules the next; repair time sampled
- `ForceFailureOn`/`ForceFailureOff`: user-forced failure, own flag, counts as failure
- `PauseOn`/`PauseOff`: planned stop, not against availability
- Defined but unused: `MUEntrance`, `Finished`
- `EventTemplate { eventtype, exec_time, object_id }` (relative); `ScheduledEvent { time, seq, object_id, eventtype }` (32 bytes)

## Invariants
- Order: earliest time; at one time `BlockedRetry` first; then lowest `seq`
- Events are engine internals; views see only occurrences

## Code locations
- `src/event.rs`; handling in `src/materialflow_objects/station.rs`, `source.rs`
