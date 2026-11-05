---
title: Event Types
section: reference
related:
  - discrete-event-simulation
  - blocked-retry
  - simulator-and-event-controller
---

# Event Types

Defined in `src/event.rs` as `pub enum EventType`.

## Active event types

### MUExit

**Fires when:** processing is complete and the object should try to move its MU to a successor.

**Who emits it:**
- `Source::create_mu()` — schedules itself after `proc_time` to signal the new MU is ready to move.
- `SingleStation::process_mu_entrance()` — schedules itself after `proc_time` to signal processing complete.

**What happens on dispatch:**
1. `try_to_move_to_successor` is called.
2. If a successor accepts: predecessor goes Idle, `notify_blocked_predecessors()` fires.
3. If all successors deny: predecessor goes Blocked, stays locked.

### BlockedRetry

**Fires when:** a successor that previously denied entrance has just freed up, and is notifying its blocked predecessors to retry.

**Who emits it:** `SingleStation::notify_blocked_predecessors()` — one event per `object_id` in `blockedlist`, at `exec_time = 0` (immediate).

**What happens on dispatch:**
1. `is_exit_allowed()` is checked first. Returns `false` if `ProcessState::Processing` — the retry is a **no-op**.
2. If exit allowed: `try_to_move_to_successor` is called. If successful, predecessor goes Idle.

### StartFailure

**Fires when:** a station's next failure is due (scheduled by `FailureProfile`).

**Who emits it:**
- `SingleStation::init()` — if `failure_profile.active`, schedules the first `StartFailure` at time-to-next-failure.
- `EndFailure` handler — schedules the next failure cycle after repair.

**What happens on dispatch:**
1. Sets `interruption = InterruptionState::Failed`.
2. If station was `Processing`, saves `remain_proc_time` into `interrupted_remain_proc_time`.
3. Schedules `EndFailure` after MTTR (sampled from `dist_sampler_mttr`).

### EndFailure

**Fires when:** the repair period expires.

**Who emits it:** `StartFailure` handler, after sampling MTTR.

**What happens on dispatch:**
1. Clears `interruption` back to `InterruptionState::None`.
2. Schedules the next `StartFailure` (if profile still active).
3. If `interrupted_remain_proc_time` is set: resumes processing, schedules `MUExit` with the saved remaining time.
4. If station was `Blocked` when failure hit: retries `try_to_move_to_successor`.
5. If station was `Idle`: notifies blocked predecessors they can push MUs again.

## Defined but not yet active

| Type | Status |
|---|---|
| `MUEntrance` | Defined; entrance is handled directly via `process_mu_entrance()` from `try_to_move_to_successor`, not via a scheduled event |
| `Finished` | Defined; not dispatched in current implementation |

## EventTemplate

```rust
pub struct EventTemplate {
    pub eventtype: EventType,
    pub exec_time: Duration,  // relative offset from current time when scheduling
    pub object_id: u32,       // which object receives the event
    pub seq: u64,             // set to 0 by objects; overwritten by EventController
}
```

## ScheduledEvent

```rust
pub struct ScheduledEvent {
    pub time: Duration,       // absolute simulation time (current_time + exec_time)
    pub template: EventTemplate,
    pub seq: u64,             // for FIFO tiebreaking — assigned by EventController
}
```

Ordering: earlier time wins; equal time → lower seq wins.
