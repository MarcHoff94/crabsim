---
title: Adding a New Station Type (AI context)
for: guides/adding-a-new-station-type.md
section: guides
---

## Checklist
1. Struct with `base: BaseObject` + domain fields
2. `impl EventInterface` — `init`, `execute_event` (handle `MUExit` + `BlockedRetry`), id methods
3. `impl MaterialFlow` — entrance/exit gates, `process_mu_entrance`, `add_blocked`, `remove_blocked`, `notify_blocked_predecessors`
4. `impl SuccessorContainer` if object has successors — enables `try_to_move_to_successor`
5. `pub use` in `src/materialflow_objects/mod.rs`
6. `simulator.register_material_flow(obj.clone())` — returns assigned id
7. Wire `succ`/`pred` on all involved objects

## Rules
- `notify_blocked_predecessors()` must be called whenever the object frees capacity
- `BlockedRetry` handler: check `is_exit_allowed()` first; return `Ok(None)` if false
- `EventTemplate.seq = 0` — EventController overwrites it
- `take_content()` in `SuccessorContainer` must leave `content` as `None` after call

## Code locations
- `EventInterface`, `MaterialFlow`, `SuccessorContainer` traits — `src/materialflow_objects/flows.rs`
- `SingleStation` as reference implementation — `src/materialflow_objects/station.rs`
- Registration — `src/simulator.rs::register_material_flow`
