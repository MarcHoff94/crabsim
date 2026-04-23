---
title: Material Flow Objects (AI context)
for: architecture/material-flow-objects.md
section: architecture
---

## Trait summary
- `EventInterface`: `init`, `execute_event`, `get/set_object_id`, `set_random_seed`, `as_any`
- `MaterialFlow: EventInterface`: entrance/exit gates, `process_mu_entrance/exit`, `add_blocked`, `notify_blocked_predecessors`
- `SuccessorContainer`: `successors`, `succ_pointer/mut`, `take_content`, `clear_content_and_idle`

## Source
- `mu_id` starts at 0, incremented to 1 on first `create_mu()` — first MU has id=1
- `init()` calls `create_mu()` → schedules `MUExit` at `exec_time = proc_time`
- `is_entrance_allowed()` = false; `process_mu_entrance` panics; `add_blocked` panics
- On `MUExit`: push MU to successor → create next MU. On failure: `ProcessState::Blocked`

## SingleStation key fields
- `content: Option<MoveableUnit>` — None = idle; Some = processing or blocked
- `entrancelocked: bool` — set true on `process_mu_entrance`, cleared when MU exits
- `state: ProcessState` — `Idle | Processing | Blocked`
- `interruption: InterruptionState` — `None | Failed | Paused`; only `None` used in demo
- `is_entrance_allowed()` = `!entrancelocked && state==Idle && interruption==None`
- `is_exit_allowed()` = `!exitlocked && (state==Idle||state==Blocked) && interruption==None`

## Drain
- `entrance_locked` starts false; nothing sets it true in current code → always accepts
- Records `exits: Vec<Duration>` and `exit_ids: Vec<u32>` on each `process_mu_entrance`
- `notify_blocked_predecessors` returns `Ok(None)` — drain never blocks predecessors
- `execute_event` has no active match arms — drain receives no scheduled events

## FailureProfile
- `availability=100.0` → `active=false`, no failure scheduling
- `availability<1.0` → `active=true`, `mtbf` calculated from `mttr/(1/availability - 1)`
- Four distributions: `CONSTANT`, `UNIFORM`, `NORMAL`, `ERLANG`

## Code locations
- Traits + `FailureProfile` + `ProcessState` — `src/materialflow_objects/flows.rs`
- `Source` — `src/materialflow_objects/source.rs`
- `SingleStation` — `src/materialflow_objects/station.rs`
- `Drain` — `src/materialflow_objects/drain.rs`
- `Distribution`, `DistributionSampler` — `src/random_numbers.rs`
