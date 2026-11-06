---
title: Material Flow Objects (AI context)
for: architecture/material-flow-objects.md
section: architecture
---

## Trait summary
- `EventInterface`: `init`, `execute_event`, `get/set_object_id`, `set_random_seed`, `as_any`
- `MaterialFlow: EventInterface`: entrance/exit gates, `process_mu_entrance/exit`, `add_blocked`, `remove_blocked`, `notify_blocked_predecessors`
- `SuccessorContainer`: `successors`, `succ_pointer/mut`, `take_content`, `clear_content_and_idle`

## Source
- `mu_id` starts at 0, incremented to 1 on first `create_mu()` — first MU has id=1
- `init()` calls `create_mu()` → schedules `MUExit` at `exec_time = proc_time`
- `is_entrance_allowed()` = false; `process_mu_entrance` panics; `add_blocked` panics
- On `MUExit`: push MU to successor → create next MU. On failure: `ProcessState::Blocked`

## SingleStation key fields
- `content: Option<MoveableUnit>` — None = idle; Some = processing or blocked
- `remain_proc_time: Duration` — tracks time left; reset on each `process_mu_entrance`
- `interrupted_remain_proc_time: Option<Duration>` — saved by `StartFailure` if station was Processing; consumed by `EndFailure` to resume the `MUExit`
- `entrancelocked: bool` — set true on `process_mu_entrance`, cleared when MU exits
- `state: ProcessState` — `Idle | Processing | Blocked`
- `interruption: InterruptionState` — `None | Failed | Paused`
- `is_entrance_allowed()` = `!entrancelocked && state==Idle && interruption==None`
- `is_exit_allowed()` = `!exitlocked && (state==Idle||state==Blocked) && interruption==None`
- `set_random_seed(seed)` also calls `failure_profile.reseed(seed)` so each object has an independent RNG stream

## Drain
- `entrance_locked` starts false; nothing sets it true in current code → always accepts
- Records `exits: Vec<Duration>` and `exit_ids: Vec<u32>` on each `process_mu_entrance`
- `notify_blocked_predecessors` returns `Ok(None)` — drain never blocks predecessors
- `execute_event` has no active match arms — drain receives no scheduled events

## FailureProfile
- `FailureProfile::new(availablity, distribution_mttr, seed)` — 3 args; MTBF distribution derived automatically as same shape scaled to `mttr_mean / (1/availablity - 1)`
- `availability=1.0` → `active=false`, no failure scheduling (availability is a fraction in `(0.0,1.0]`, not a percentage)
- `availability<1.0` → `active=true`; `init()` schedules first `StartFailure`; MTBF mean = `mttr_mean / (1/availability - 1)`
- `reseed(seed)`: replaces both samplers preserving shape (MTBF sampler uses `seed+1`)
- Demo: `availability=0.9`, Erlang(mean=60 000 ms, k=2) MTTR
- `Distribution` has `mean() -> f64` and `with_mean(f64) -> Distribution` utility methods

## Code locations
- Traits + `FailureProfile` + `ProcessState` — `src/materialflow_objects/flows.rs`
- `Source` — `src/materialflow_objects/source.rs`
- `SingleStation` — `src/materialflow_objects/station.rs`
- `Drain` — `src/materialflow_objects/drain.rs`
- `Distribution`, `DistributionSampler` — `src/random_numbers.rs`
