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
    pub content: Option<MoveableUnit>,  // None = idle
    pub succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub pred: Vec<Rc<RefCell<dyn MaterialFlow>>>,
    pub succ_pointer: usize,
    pub blockedlist: Vec<u32>,
    pub entrancelocked: bool,
    pub exitlocked: bool,
    pub state: ProcessState,         // Idle | Processing | Blocked
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
| `BlockedRetry` (exit allowed, move succeeds) | Blocked | Idle, entrance unlocked |
| `BlockedRetry` (exit not allowed) | Processing | Processing (no-op) |

`is_entrance_allowed()` returns `true` only when `state == Idle` AND `interruption == None` AND `!entrancelocked`.

### FailureProfile

`SingleStation` carries a `FailureProfile` for failure modelling (availability, MTTR, MTBF, distributions). Availability is expressed as a fraction in `(0.0, 1.0)` where `1.0` means 100% available. In the current demo all stations use a no-failure profile (`availability = 1.0`, `CONSTANT{0.0}` distributions). `FailureProfile::set_availability(1.0)` sets `active = false` which disables all failure scheduling.

## Drain

- Always accepts entrance (`entrance_locked` starts `false`, nothing sets it `true` in current code).
- `process_mu_entrance(mu)`: records `current_time` in `exits` and `mu.id` in `exit_ids`. Computes running average `exit_interval`.
- Does **not** notify blocked predecessors (`notify_blocked_predecessors` returns `None`). This is safe because the Drain never becomes a blocker.
- `execute_event` has no active match arms — the Drain receives no scheduled events.

## FailureProfile and distributions

`FailureProfile` encapsulates two `DistributionSampler`s (one for MTTR, one for MTBF). Available distributions: `CONSTANT`, `UNIFORM`, `NORMAL`, `ERLANG`. Each sampler holds a seeded `StdRng` for reproducibility.

## Code references

- `EventInterface`, `MaterialFlow`, `SuccessorContainer`, `FailureProfile`, `ProcessState` — `src/materialflow_objects/flows.rs`
- `Source` — `src/materialflow_objects/source.rs`
- `SingleStation` — `src/materialflow_objects/station.rs`
- `Drain` — `src/materialflow_objects/drain.rs`
- `Distribution`, `DistributionSampler` — `src/random_numbers.rs`
