# Objects in one `Vec` of an enum, with successors as ids

A run holds its objects in one `Vec<ModelObject>` indexed by id, `ModelObject` is an enum over the object types, and objects refer to each other only by id. To handle an event, the run splits the `Vec` around the object in hand, and that object reaches the others through `OtherObjects`. This replaced a `BTreeMap` of `Rc<RefCell<dyn MaterialFlow>>`. We chose it for speed and for what later steps need: a run can move between threads, a clone of the objects is close to a checkpoint, and the `Rc` cycles and their leak are gone.

Measured together with the 32-byte queue entry and the removal of the per-move list clone, on the owner's PC, median of 7 runs in rotation, with identical events and MUs left: 1.74 times as fast on the demo model and about 1.9 to 2.4 times on the 2,800-object model. See `docs/design/first-profile.md`.

## Considered Options

- **Keep `Rc<RefCell<dyn MaterialFlow>>` and only remove the per-event costs.** The profile put most of the possible gain here, but saving objects for a checkpoint would have had to go through `Rc<RefCell<dyn ...>>`, and a run couldn't leave its thread.
- **Raw pointers between objects.** The prototype behind the profile did this. It skips the split and its bounds checks, but it needs `unsafe` on the per-event path. The measured speed of the safe version left no reason to accept that.

## Consequences

- **The set of object types is closed.** How user code gets in is still open. If it ever needs its own object types, one variant can hold a boxed trait object, which costs a dispatch only for those objects.
- **Models are built through `ModelDescription`,** so ids always match positions and tests hold no storage details. It is deliberately minimal: validating connections is still to come with "model as data".
- **Each entry is roughly 1 KB,** mostly random number generator state. Moving rarely used state out of the entry hasn't been tried.
- **A test object that fails part way through an event is a `#[cfg(test)]` variant.** No real object does that, so those few tests are unit tests in `src/`, and production code has no extra variant to match.
