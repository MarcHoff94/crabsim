---
title: Material Flow Objects
section: architecture
related:
  - material-flow-model
  - blocked-retry
  - moveable-units
---

# Material Flow Objects

## Storage

A run holds its objects as one `Vec<ModelObject>`, indexed by id (ADR 0005):

```rust
pub enum ModelObject {
    Source(Source),
    Station(SingleStation),
    Buffer(Buffer),
    Drain(Drain),
    #[cfg(test)]
    RecordsThenFails(RecordsThenFails),  // only tests fail part way through an event
}
```

`ModelObject` implements every trait by dispatching with a `match` (the `on_the_object!` macro), so there are no trait objects and no downcasts. Objects refer to each other by id only: `succ` and `blockedlist` hold ids. While one object handles an event it reaches the others through `OtherObjects`, which the run builds by splitting the `Vec` around it.

## Traits

```
EventInterface
  init(context) → first events
  execute_event(context, eventtype, others) → new events
  reset()                     back to the default state
  report(now) → ObjectState    state and statistics
  clear_statistics(now)
  get/set_object_id, set_random_seed

StopCtrl                      todo!() everywhere: stops work through events instead
  set_stop, remove_stop

MaterialFlow : EventInterface + StopCtrl
  is_entrance_allowed, is_exit_allowed
  process_mu_entrance(mu, context) → maybe an event
  add_blocked, remove_blocked, notify_blocked_predecessors

SuccessorContainer            Source, Station and Buffer
  successors() → &[u32], succ_pointer, succ_pointer_mut
  take_content(now), clear_content_and_idle, statistics_mut
```

`SuccessorContainer` is what `try_to_move_to_successor` works on, so it needs no concrete type. What becomes of `StopCtrl` is to decide when stops reach the other object types.

Every object must:
- tell its `StatisticsKeeper` what it ended up doing, at the end of everything it handles (`update_statistics`),
- return to its default state in `reset`, including configured starting MUs. A new field needs resetting, and a reset test in `tests/simulation_run.rs`.

## Source

- Creates an MU every `proc_time` (its interval). It creates the first at Init, records `MuCreated`, and schedules its `MUExit`.
- On `MUExit` it pushes the MU on. Once it has moved, it creates the next at once. If every successor refuses, it becomes `Blocked` and waits for a retry.
- It never accepts an MU, and has no failures or stops yet.

## Station (`SingleStation`)

Works on one MU at a time for its process time.

```rust
pub struct SingleStation {
    pub proc_time: SimTime,              // can change during a run
    pub configured_proc_time: SimTime,   // Reset returns proc_time to this
    pub completion_time: Option<SimTime>,
    pub interrupted_remain_proc_time: Option<SimTime>,
    pub stop_count: u32,                 // makes queued exits stale
    pub failure_profile: FailureProfile,
    pub starting_content: Option<MoveableUnit>,
    pub content: Option<MoveableUnit>,
    pub succ: Vec<u32>,
    pub succ_pointer: usize,
    pub blockedlist: Vec<u32>,
    pub entrancelocked: bool,
    pub state: ProcessState,             // Idle | Processing | Blocked
    failed: bool, forced: bool, paused: bool,
    interrupted_since: [SimTime; 3],     // when each kind of interruption began
    pub statistics: StatisticsKeeper,
    // ...
}
```

- **Entrance** is open only when Idle, not stopped and not locked. **Exit** is allowed when Idle or Blocked and not stopped, never while Processing.
- **Work:** an MU entering starts work at once: `WorkStarted`, then an `MUExit` after the process time. On `MUExit` it records `WorkFinished` and pushes the MU on. If that works it becomes Idle and sends retries to waiting predecessors; if not, it becomes Blocked.
- **Stops:** a random failure, a forced failure and a Pause are separate interruptions, each with its own flag, and the Station is stopped while any is active. Only the first interruption of a stop freezes the work (remaining time saved, `stop_count` bumped), and only the last to end resumes it: work carries on with its remaining time, a Blocked Station tries its move again, and an Idle one sends retries. So completion time is entry plus process time plus time stopped, with overlaps counted once.
- **Random failures:** `init` schedules the first `StartFailure` if the profile is active. `StartFailure` samples a repair time and schedules `EndFailure`, and `EndFailure` schedules the next `StartFailure`. A failure arriving while already failed is discarded.
- **Forced failure:** started and ended by the user (`ForceFailureOn`, `ForceFailureOff`). It has its own flag, so random failures carry on underneath it, and it counts as a failure in statistics.
- **Pause:** `PauseOn`, `PauseOff`. Planned, so it doesn't count against availability.
- `state()` reports the most serious active interruption as `stop` (failure, then forced failure, then pause) and lists every active one, with when it began, in `interruptions`.

### FailureProfile

```rust
FailureProfile::new(
    availability: f32,               // in (0.0, 1.0); 1.0 = never fails
    distribution_mttr: Distribution, // repair time; the time between failures is derived
    seed: u64,
)
```

The mean time between failures is `mttr_mean / (1/availability - 1)`, with the same distribution shape as the repair time. `availability = 1.0` makes the profile inactive. The run gives every object its own seed at Init, and `reseed` keeps each sampler's shape. Distributions are `CONSTANT`, `UNIFORM`, `NORMAL` and `ERLANG`. The demo uses availability 0.9 with an Erlang repair time (mean 60 s, k = 2).

## Buffer

Holds MUs, up to its `Capacity` or without limit (ADR 0019), each for the `dwelling_time` from its own entry, and passes on one at a time.

- **Contents:** a `VecDeque` of MUs in entry order, each with the time its dwelling ends. One dwelling time for all means that is also the order they become ready in.
- **Exit strategy:** `offered` picks the MU to pass on among those whose dwelling time is over: the first in, or the last in. `take_content` and `offered_since` go through it, so the code that moves MUs never knows the strategy.
- **Events:** every MU that enters schedules one `MUExit` for the end of its dwelling time. On it, the Buffer passes on every MU that may leave until one is refused (`pass_on`), and then it is blocked. A blocked Buffer ignores further exits: it already waits in line. On a `BlockedRetry` it does the same, offering only the successor that sent it.
- **Room again:** after any MU has left, it sends retries to the predecessors waiting for it.
- **States:** `Idle` when empty, `Processing` while holding MUs, `Blocked` when an MU that may leave can't. Holding time is kept as busy time.
- **Starting MUs** count as entered at Init, in their order, and Reset brings them back.
- It can't be stopped yet.

## Drain

- Its entrance is always open, so predecessors never wait for it.
- On entry it counts the exit in its statistics (lead time, exit intervals) and records `MuLeft`. It keeps no MU.
- It handles no events of its own.

## Code references

- `ModelObject`, `OtherObjects`, `on_the_object!` — `src/materialflow_objects/model_object.rs`
- Traits, `MoveableUnit`, `FailureProfile`, `ProcessState`, `InterruptionState` — `src/materialflow_objects/flows.rs`
- `Source` — `src/materialflow_objects/source.rs`
- `SingleStation` — `src/materialflow_objects/station.rs`
- `Buffer` — `src/materialflow_objects/buffer.rs`
- `Drain` — `src/materialflow_objects/drain.rs`
- `Distribution`, `DistributionSampler` — `src/random_numbers.rs`
- `StatisticsKeeper` — `src/statistics.rs`
