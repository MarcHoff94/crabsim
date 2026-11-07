---
title: Event Types
section: reference
related:
  - discrete-event-simulation
  - blocked-retry
  - simulation-run
---

# Event Types

Defined in `src/event.rs` as `pub enum EventType`. Events are engine internals: views never see them, only the occurrences objects record.

## In use

### MUExit(ExitTicket)

**Fires when** an object is done with its MU: a Source's interval or a Station's process time is up.

**Scheduled by** `Source::create_mu` and a Station's `start_work`, or on resuming after a stop, for the remaining process time.

**On a Station:** ignored if the Station is stopped or the ticket is stale (its `stop_count` differs, or it names another MU). Otherwise it records `WorkFinished` and tries `try_to_move_to_successor`. Moved: Idle, entrance unlocked, retries sent to waiting predecessors. Refused everywhere: Blocked.

**On a Source:** tries the move, and creates the next MU once this one has gone.

`ExitTicket { mu_id, stop_count }` exists because the heap can't cancel events.

### BlockedRetry { from }

**Fires when** the successor `from` has room, at the same time, ahead of every other event at that time.

**Scheduled by** `notify_blocked_predecessors`, through `retries_for_blocked`: one per waiting predecessor, longest-waiting first.

**On dispatch:** nothing if the object can't exit (working or stopped) or is empty. Otherwise `retry_move_to_successor` offers the MU to `from` only.

### StartFailure and EndFailure

Random failures on a Station with an active `FailureProfile`.

- `StartFailure`: scheduled first by `init`, then by each `EndFailure`. Discarded if already failed. Begins a `Failure` interruption, counts a failure and schedules `EndFailure` after a sampled repair time.
- `EndFailure`: ends the interruption, schedules the next `StartFailure`, and resumes the Station if no other interruption is active.

### ForceFailureOn and ForceFailureOff

A failure the user starts and ends by hand, scheduled from outside the model (`schedule_event`), which is what the live session does. It has its own flag, so random failures carry on underneath. It counts as a failure in statistics.

### PauseOn and PauseOff

A planned stop, scheduled from outside the model. It doesn't count against availability.

Only the first interruption of a stop freezes a Station's work, and only the last to end resumes it, however the three kinds overlap.

## Defined but unused

| Type | Status |
|---|---|
| `MUEntrance` | A move calls `process_mu_entrance` directly, with no event |
| `Finished` | Never scheduled |

## EventTemplate

```rust
pub struct EventTemplate {
    pub eventtype: EventType,
    pub exec_time: SimTime,  // from now
    pub object_id: u32,      // who gets it
}
```

## ScheduledEvent

```rust
pub struct ScheduledEvent {   // 32 bytes, on the per-event path
    pub time: SimTime,        // now + exec_time
    pub seq: u64,             // assigned by the run, for FIFO order
    pub object_id: u32,
    pub eventtype: EventType,
}
```

Order: earliest time first; at one time, `BlockedRetry` first; then lowest `seq`.

## Code references

- `EventType`, `EventTemplate`, `ScheduledEvent`, `ExitTicket` — `src/event.rs`
- Station handling — `SingleStation::handle_event` in `src/materialflow_objects/station.rs`
- Source handling — `src/materialflow_objects/source.rs`
