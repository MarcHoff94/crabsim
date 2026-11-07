---
title: Adding a New Object Type
section: guides
related:
  - material-flow-objects
  - blocked-retry
  - writing-tests
---

# Adding a New Object Type

How to add a new kind of material-flow object, such as a conveyor. Read [Material Flow Objects](../architecture/material-flow-objects.md) first, and check the agreed design in `docs/agents/handoff.md`: Station capacity, for example, is agreed as a setting on Station rather than a new type (ADR 0001).

## 1. The struct

Hold a `BaseObject`, everything the object does, and what Reset returns it to. Successors and waiting predecessors are ids. It must be `Clone`, because a clone of a run is a checkpoint.

```rust
#[derive(Clone)]
pub struct Conveyor {
    pub base: BaseObject,
    pub succ: Vec<u32>,
    pub succ_pointer: usize,
    pub blockedlist: Vec<u32>,
    pub statistics: StatisticsKeeper,
    // ...
}
```

## 2. EventInterface

```rust
impl EventInterface for Conveyor {
    fn init(&mut self, context: &mut EventContext) -> Result<Vec<EventTemplate>, InitError> { ... }
    fn execute_event(
        &mut self,
        context: &mut EventContext,
        eventtype: &EventType,
        others: &mut OtherObjects<'_>,
    ) -> Result<Option<Vec<EventTemplate>>, RunTimeError> {
        let outcome = self.handle_event(context, eventtype, others);
        self.update_statistics(context.time);   // after everything it handles
        outcome
    }
    fn reset(&mut self) { ... }                  // every field back to the default state
    fn report(&self, now: SimTime) -> ObjectState { ... }
    fn clear_statistics(&mut self, now: SimTime) { self.statistics.clear(now); }
    fn get_object_id(&self) -> &u32 { &self.base.object_id }
    fn set_object_id(&mut self, id: u32) { self.base.object_id = id; }
    fn set_random_seed(&mut self, seed: u64) { self.base.random_seed = seed; }
}
```

- Record what happens through `context.record(OccurrenceKind::...)`, and get new MU ids from `context.new_mu_id()`.
- Reach other objects only through `others.get(id)` and `others.get_mut(id)`.
- Return an error for an event the object doesn't handle.

## 3. StopCtrl and MaterialFlow

`StopCtrl` is `todo!()` on every type today; stops work through events (see [Event Types](../reference/event-types.md)). For `MaterialFlow`:

```rust
impl MaterialFlow for Conveyor {
    fn is_entrance_allowed(&self) -> bool { ... }
    fn is_exit_allowed(&self) -> bool { ... }
    fn process_mu_entrance(&mut self, mu: MoveableUnit, context: &mut EventContext)
        -> Result<Option<EventTemplate>, MaterialflowError> { ... }
    fn add_blocked(&mut self, id: u32) { list_blocked(&mut self.blockedlist, id); }
    fn remove_blocked(&mut self, id: u32) { self.blockedlist.retain(|&listed| listed != id); }
    fn notify_blocked_predecessors(&mut self) -> Result<Option<Vec<EventTemplate>>, MaterialflowError> {
        Ok(retries_for_blocked(&mut self.blockedlist, self.base.object_id))
    }
}
```

## 4. SuccessorContainer, if it has successors

So that `try_to_move_to_successor` and `retry_move_to_successor` can move its MUs: `successors`, `succ_pointer`, `succ_pointer_mut`, `take_content(now)`, `clear_content_and_idle` and `statistics_mut`.

## 5. Wire it in

1. Expose it in `src/materialflow_objects/mod.rs`.
2. Add a variant to `ModelObject` and to the `on_the_object!` macro in `src/materialflow_objects/model_object.rs`.
3. Add an `ObjectKind` variant, and handle it in `ModelDescription::kind` and `successors`, and in `Dimensions::default_for`.
4. Add an `add_` method to `ModelDescription`, and include its successors in `connect` if it has any.

## Rules to keep

- Send retries whenever the object frees up room, or waiting predecessors are never woken.
- On `BlockedRetry`, do nothing unless `is_exit_allowed()` and there is an MU to move, then offer it only to `from` with `retry_move_to_successor`.
- An event the heap can't cancel must be recognisable as stale when it fires, as `ExitTicket` does for Stations.
- Keep the per-event path cheap: no allocation where it can be avoided, and measure with the benchmark.
- Add reset tests in `tests/simulation_run.rs`. A same-seed replay can't catch leftover events, because they land on the replay's own event times.

## Code references

- Traits and helpers — `src/materialflow_objects/flows.rs`
- `ModelObject`, `on_the_object!` — `src/materialflow_objects/model_object.rs`
- `ModelDescription` — `src/model.rs`
- A full example — `SingleStation` in `src/materialflow_objects/station.rs`
