---
title: Material Flow Objects
section: architecture
related:
  - material-flow-model
  - blocked-retry
  - moveable-units
---

# Material Flow Objects

## Trait hierarchy

Every material flow object implements two traits, and objects with successors implement a third:

```
EventInterface
  init() → Vec<EventTemplate>
  execute_event(simulator, eventtype) → Option<Vec<EventTemplate>>
  get/set_object_id, set_random_seed, as_any

MaterialFlow : EventInterface
  is_entrance_allowed() → bool
  is_exit_allowed() → bool
  process_mu_entrance(mu) → Option<EventTemplate>
  process_mu_exit() → Option<EventTemplate>
  add_blocked(id)
  remove_blocked(id)
  notify_blocked_predecessors() → Option<Vec<EventTemplate>>

SuccessorContainer   (Source and SingleStation only)
  successors() → &Vec<Rc<RefCell<dyn MaterialFlow>>>
  succ_pointer() / succ_pointer_mut()
  take_content() → Option<MoveableUnit>
  clear_content_and_idle()
```

`SuccessorContainer` is the interface `try_to_move_to_successor` operates on — it does not need to know the concrete type of the predecessor.

## BaseObject

Every object holds a `BaseObject { object_id: u32, random_seed: u64 }`. The `object_id` is assigned at registration time by `Simulator::register_material_flow`. Before registration it holds whatever value was passed to `new()` — tests sometimes rely on the registration-assigned ID, so store the return value of `register_material_flow`.

## Source

- Creates MUs with a sequentially incrementing `mu_id` (starts at 1).
- `init()` immediately calls `create_mu()` to schedule the first MU at time 0.
- `execute_event(MUExit)`: tries to push its MU to a successor. On success, creates the next MU (scheduling another `MUExit` after `proc_time`). On failure (all successors locked), goes `Blocked`.
- Never accepts entrance: `is_entrance_allowed()` returns `false`; `process_mu_entrance` panics.
- Has no predecessors: `notify_blocked_predecessors()` returns `None`; `add_blocked` panics.

## SingleStation

The core processing unit.

```rust
pub struct SingleStation {
    pub proc_time: Duration,
    pub remain_proc_time: Duration,
    pub interrupted_remain_proc_time: Option<Duration>,  // saved on StartFailure if Processing
    pub content: Option<MoveableUnit>,  // None = idle
    pub succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub succ_pointer: usize,
    pub blockedlist: Vec<u32>,
    pub entrancelocked: bool,
    pub exitlocked: bool,
    pub state: ProcessState,             // Idle | Processing | Blocked
    pub interruption: InterruptionState, // None | Failed | Paused
    pub failure_profile: FailureProfile,
}
```

Key state transitions:

| Event | Before | After |
|---|---|---|
| `process_mu_entrance` | Idle | Processing, entrance locked |
| `MUExit` (move succeeds) | Processing | Idle, entrance unlocked |
| `MUExit` (all blocked) | Processing | Blocked, entrance stays locked |
| `MUExit` (interruption active) | any | no-op (returns `Ok(None)`) |
| `BlockedRetry` (exit allowed, move succeeds) | Blocked | Idle, entrance unlocked |
| `BlockedRetry` (exit not allowed) | Processing | Processing (no-op) |
| `StartFailure` | any | `InterruptionState::Failed`; saves `remain_proc_time` if Processing; schedules `EndFailure` |
| `EndFailure` | Failed | `InterruptionState::None`; resumes MUExit or notifies predecessors; schedules next `StartFailure` |

`is_entrance_allowed()` returns `true` only when `state == Idle` AND `interruption == None` AND `!entrancelocked`.

### FailureProfile

`SingleStation` carries a `FailureProfile` for failure modelling. Availability is a fraction in `(0.0, 1.0)` where `1.0` means 100% available; `availability = 1.0` sets `active = false` and disables all failure scheduling.

Constructor:
```rust
FailureProfile::new(
    availablity: f32,               // fraction in (0.0,1.0]; 1.0 = always available
    distribution_mttr: Distribution, // MTTR distribution; MTBF derived automatically
    seed: u64,                       // base RNG seed (MTBF sampler uses seed+1)
)
```
MTBF mean is derived as `mttr_mean / (1/availability - 1)`. The MTBF distribution uses the same shape as the MTTR distribution, scaled to the derived mean.

`reseed(seed)` replaces both samplers while preserving their shape — called by `set_random_seed` so each registered object gets a unique RNG stream.

`init()` schedules the first `StartFailure` event if `active == true`.

The demo currently uses `availability = 0.9` with an Erlang(mean=60 000 ms, k=2) MTTR distribution.

## Drain

- Always accepts entrance (`entrance_locked` starts `false`, nothing sets it `true` in current code).
- `process_mu_entrance(mu)`: records `current_time` in `exits` and `mu.id` in `exit_ids`. Computes running average `exit_interval`.
- Does **not** notify blocked predecessors (`notify_blocked_predecessors` returns `None`). This is safe because the Drain never becomes a blocker.
- `execute_event` has no active match arms — the Drain receives no scheduled events.

## FailureProfile and distributions

`FailureProfile` encapsulates two `DistributionSampler`s (MTTR and MTBF). Available distributions: `CONSTANT`, `UNIFORM`, `NORMAL`, `ERLANG`. Each sampler holds a seeded `StdRng` for reproducibility.

`Distribution` provides two utility methods:
- `mean() -> f64` — returns the configured mean of the distribution.
- `with_mean(mean) -> Distribution` — returns a new distribution of the same shape with a different mean (used to derive the MTBF distribution from the MTTR distribution).

## Code references

- `EventInterface`, `MaterialFlow`, `SuccessorContainer`, `FailureProfile`, `ProcessState` — `src/materialflow_objects/flows.rs`
- `Source` — `src/materialflow_objects/source.rs`
- `SingleStation` — `src/materialflow_objects/station.rs`
- `Drain` — `src/materialflow_objects/drain.rs`
- `Distribution`, `DistributionSampler` — `src/random_numbers.rs`
