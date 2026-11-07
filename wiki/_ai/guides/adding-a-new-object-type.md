---
title: Adding a New Object Type (AI context)
for: guides/adding-a-new-object-type.md
section: guides
---

## Facts
- Struct: `BaseObject`, ids for successors and `blockedlist`, `StatisticsKeeper`, default-state fields; must be `Clone` (checkpoints)
- Implement `EventInterface` (incl. `reset`, `report`, `clear_statistics`), `StopCtrl`, `MaterialFlow`, and `SuccessorContainer` if it has successors
- Wire in: `materialflow_objects/mod.rs`, `ModelObject` variant + `on_the_object!`, `ObjectKind` + `ModelDescription::kind/successors` + `Dimensions::default_for`, an `add_` method, `connect`

## Invariants
- Call `update_statistics` after every event; record occurrences via `context.record`; MU ids via `context.new_mu_id()`
- Reach others only through `OtherObjects`
- Send retries whenever room frees up; on `BlockedRetry` act only if exit allowed and holding an MU, offering only `from`
- Uncancellable events must be recognisable as stale (like `ExitTicket`)

## Pitfalls
- Check the agreed design first: Station capacity is a setting, not a new type (ADR 0001)
- Reset tests are required; same-seed replay can't catch leftover events

## Code locations
- `src/materialflow_objects/flows.rs`, `model_object.rs`, `src/model.rs`; example: `station.rs`
