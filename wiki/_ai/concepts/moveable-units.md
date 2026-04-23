---
title: Moveable Units (AI context)
for: concepts/moveable-units.md
section: concepts
---

## Facts
- `MoveableUnit { id: u32, location_id: u32 }`
- `id`: assigned sequentially by `Source.mu_id` counter, starts at 1, never reused
- `location_id`: `object_id` of the holding station — updated on each transfer
- `SingleStation.content: Option<MoveableUnit>` — max one MU per station
- `Drain` records `exits: Vec<Duration>` (timestamps) and `exit_ids: Vec<u32>` (arrival order)
- FIFO test: compare `exit_ids` to `[1, 2, 3, ...]`

## Invariants
- MU is taken from predecessor via `take_content()` **before** being passed to `process_mu_entrance`
- If `take_content()` returns `None`, no transfer happens (predecessor has no MU to give)
- Drain consumes the MU immediately — it is not stored after `process_mu_entrance`

## Code locations
- `MoveableUnit` — `src/materialflow_objects/flows.rs`
- `Source::create_mu`, `Source.mu_id` — `src/materialflow_objects/source.rs`
- `SingleStation.content` — `src/materialflow_objects/station.rs`
- `Drain.exits`, `Drain.exit_ids` — `src/materialflow_objects/drain.rs`
- `SuccessorContainer::take_content` — `src/materialflow_objects/flows.rs`
