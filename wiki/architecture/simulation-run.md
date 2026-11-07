---
title: Simulation Run
section: architecture
related:
  - discrete-event-simulation
  - material-flow-objects
  - event-types
---

# Simulation Run

`SimulationRun` in `src/simulation_run.rs` is one execution of a model, from its Init until it is reset or discarded. It is the engine's public interface, and it knows nothing about views or pacing.

## What it holds

```rust
pub struct SimulationRun {
    objects: Vec<ModelObject>,        // indexed by object id
    queue: BinaryHeap<ScheduledEvent>,
    time: SimTime,
    next_seq: u64,                    // FIFO order within one time
    counted_time: SimTime,            // for the zero-time loop guard
    events_at_this_time: u64,
    seed: u64,
    initialised: bool,
    recording: Recording,
    statistics_since: SimTime,
    context: EventContext,
}
```

A `ScheduledEvent` is 32 bytes (time, seq, object id, event type), because it is on the per-event path. Build a run through `ModelDescription::build(seed)`, which guarantees each object's id matches its place in the `Vec`.

## Controls

| Method | What it does |
|---|---|
| `init` | Sets t=0, gives each object its own seed, calls each object's `init` in id order and queues the first events without processing them, even those due at t=0. Only allowed from the default state |
| `step` | Processes one event |
| `advance_to(t)` | Processes events up to and including `t`, then leaves the clock at `t` |
| `advance_bounded(t, n)` | The same for at most `n` events. If events are still due, it reports not having reached `t`, and the clock stays at the last event |
| `advance_events(n)` | At most `n` events, however far apart, leaving the clock at the last |
| `reset` | Returns every object to its default state. Objects stay in the model, and configured starting MUs come back as fresh copies |
| `state` | The full typed `RunState`, including each object's statistics |
| `schedule_event` | Queues an event from outside, such as a Pause |
| `set_process_time` | Changes a Station's process time from the next MU on, scheduling nothing |
| `keep_changes_as_default` | Makes changed process times the default state, so Reset keeps them |
| `clear_statistics` | Starts every object's statistics afresh, such as after a warm-up, and changes nothing else |
| `set_recording` | Which occurrence kinds to keep from now on: `Recording::NOTHING` (the default), `ALL` or `only(&[...])` |

Advancing before Init, or Init twice, returns an error rather than panicking. Splitting an advance up gives exactly the same result as advancing at once.

## Processing an event

1. Count events at the next event's time. Past `ZERO_TIME_EVENT_LIMIT` the run returns `RunError::ZeroTimeLoop` and leaves the event queued, so it keeps refusing until Reset.
2. Pop the event. The clock moves to its time.
3. Split the objects around the one named, and call its `execute_event` with the `EventContext` and `OtherObjects`.
4. Queue every returned `EventTemplate` at `now + exec_time`, with the next `seq`.

An event for an id not in the model returns `RunError::UnknownObject`, and an error from an object becomes `RunError::ObjectFailed`. An advance that fails returns `AdvanceFailure`, holding the error and everything recorded before it, so no caller loses occurrences.

## Order at one time

`ScheduledEvent::cmp` orders by time, then puts `BlockedRetry` before everything else, then by `seq`. `BinaryHeap` is a max-heap, so the comparisons are reversed. Preserve `seq` assignment when changing scheduling: the same seed on the same version must always give identical results.

## Results

- **Occurrences** come back from each advance, only for the kinds being recorded. Objects record them through `EventContext::record`, which costs one predictable check when nothing is recorded.
- **State** comes from `state()` at any time, including after an error: each object's `ObjectState`, with contents, process state, stop and every active interruption, remaining time, waiting predecessors, successors, capacity, process times and fixed-size `ObjectStatistics`.

A headless run records nothing, so its memory stays flat however long it runs.

## Stale exits

The heap can't cancel events. A Station's `MUExit` carries an `ExitTicket` with the MU's id and the Station's `stop_count` when it was scheduled. A stop bumps `stop_count`, so the queued exit is ignored as stale when it fires, and resuming schedules a new exit for the remaining process time.

## Code references

- `SimulationRun`, `BoundedAdvance`, `ZERO_TIME_EVENT_LIMIT` — `src/simulation_run.rs`
- `ScheduledEvent`, `EventContext`, `ExitTicket` — `src/event.rs`
- `RunState`, `ObjectState`, `Recording` — `src/report.rs`
- `RunError`, `AdvanceFailure` — `src/error.rs`
