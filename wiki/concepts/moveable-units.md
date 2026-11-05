---
title: Moveable Units
section: concepts
related:
  - material-flow-model
  - blocked-retry
  - event-types
---

# Moveable Units

## What a MU is

A Moveable Unit (MU) is the "part" that travels through the material flow network. In crabsim it is a small value type:

```rust
pub struct MoveableUnit {
    pub id: u32,
    pub location_id: u32,
}
```

`id` is a globally unique identifier assigned sequentially by the `Source`. `location_id` is the `object_id` of the station currently holding the MU — it gets updated as the MU moves.

## Identity and ordering

The `Source` assigns IDs starting at 1, incrementing with each new MU (`mu_id` counter). Because IDs are sequential and the source produces MUs in order, the ID also encodes creation order. The `Drain` records `exit_ids` in arrival order, so comparing `exit_ids` against `[1, 2, 3, ...]` is how tests verify FIFO semantics end-to-end.

## One MU per station

`SingleStation` holds `content: Option<MoveableUnit>`. It can process exactly one MU at a time. When `content` is `Some`, the station's entrance is locked and no further MUs can enter. The MU is taken from `content` (via `take_content()`) only when it successfully transfers to a successor.

## Lifecycle

1. `Source::create_mu()` — allocates MU with next `mu_id`, stores in `content`, schedules `MUExit` after `proc_time`.
2. `try_to_move_to_successor` — takes the MU from the predecessor via `take_content()` and hands it to the successor via `process_mu_entrance(mu)`.
3. `Drain::process_mu_entrance(mu)` — records `current_time` in `exits` and `mu.id` in `exit_ids`. MU is not stored; it is consumed.

## Code references

- `MoveableUnit` struct — `src/materialflow_objects/flows.rs`
- `Source::create_mu` — `src/materialflow_objects/source.rs`
- `SingleStation::content` — `src/materialflow_objects/station.rs`
- `Drain::exits`, `Drain::exit_ids` — `src/materialflow_objects/drain.rs`
- `take_content` (SuccessorContainer trait) — `src/materialflow_objects/flows.rs`
