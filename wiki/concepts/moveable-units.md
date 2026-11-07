---
title: Moveable Units
section: concepts
related:
  - material-flow-model
  - blocked-retry
  - event-types
---

# Moveable Units

## What an MU is

A moveable unit (MU) is whatever travels through the model, such as a part. In crabsim it is a small value:

```rust
pub struct MoveableUnit {
    pub id: u32,
    pub location_id: u32,
    /// When it left its Source, where its lead time starts.
    pub left_source: SimTime,
}
```

## Identity

MU ids are unique across the whole model. The run hands them out through `EventContext::new_mu_id`, starting above the highest id of any configured starting MU. Because a Source creates MUs in order, a lower id from one Source also means an earlier MU.

## Where an MU lives

- **Source:** holds the MU it is creating, from its creation until it moves on. A Source creates its first MU at Init.
- **Station:** holds at most one MU (`content: Option<MoveableUnit>`). While it holds one, its entrance is locked. A Station can be configured with a **starting MU**, which is part of its default state: worked on from Init, and back as a fresh copy after Reset.
- **Drain:** keeps no MU. It counts it in its statistics and records `MuLeft` with its lead time.

## Lifecycle

1. `Source::create_mu` gives the MU a new id, records `MuCreated` and schedules its `MUExit` after the interval.
2. `try_to_move_to_successor` takes it from its holder (`take_content`), records `MuMoved` and hands it to the successor (`process_mu_entrance`).
3. A Station records `WorkStarted`, and `WorkFinished` when its process time is up.
4. `Drain::process_mu_entrance` records `MuLeft` with the lead time, from leaving its Source to entering the Drain.

## Per-MU data

Nothing is kept per MU unless the run records occurrences (ADR 0004). A headless run records nothing by default, so its memory stays flat however long it runs. Tests that check order read `MuLeft` occurrences from a run with recording on.

## Code references

- `MoveableUnit` — `src/materialflow_objects/flows.rs`
- `Source::create_mu` — `src/materialflow_objects/source.rs`
- `SingleStation::content`, `starting_content` — `src/materialflow_objects/station.rs`
- `Drain::process_mu_entrance` — `src/materialflow_objects/drain.rs`
- `EventContext::new_mu_id` — `src/event.rs`
