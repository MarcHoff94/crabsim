# Building during a run, costed

Written on 2026-11-12, alongside spec issue 87, as a sketch: what changing a model's structure while a run is in progress would take, what each choice costs and what it rules out. **Nothing here is decided.** Today the window builds only with no run in progress, before Init or after Reset, and greys the building controls during a run with "Reset first to change the model".

"Structure" means adding or removing an object, and adding, removing or reordering a connection. Changing an attribute during a run is already an intervention (ADR 0008), and stays outside this sketch.

## What a run assumes today

- **Ids are places.** `SimulationRun` holds its objects in one `Vec<ModelObject>`, and an object's id is its place in it (ADR 0005). Events, `succ`, `blockedlist`, statistics, the `Picture` and the view all name objects by id.
- **Connections live in the objects.** Each object's successors are a list of ids in exit order, with `succ_pointer` remembering whose turn is next. `blockedlist` holds the predecessors waiting for it, longest-waiting first.
- **The event heap can't cancel.** A queued `MUExit` for an object is made stale by bumping `stop_count`. `BlockedRetry { from }` names the successor that freed up.
- **Random numbers come from each object's stream** (ADR 0012), so adding an object never changes another's numbers.
- **Going back replays.** A checkpoint is a clone of the run plus how much of the intervention log is in it. Replay restores the nearest earlier checkpoint and applies the logged `Action`s it passes. Every `Action` today names an object by id and changes a value, never the structure.
- **The view builds its objects once,** from the `ModelDescription`, and Init, Reset and going back send a full state over them.

## What would have to change

### 1. Ids for objects that come and go

- **Tombstones:** a removed object stays in the `Vec` as an empty variant that ignores events, and new objects go at the end. Ids stay places, nothing is renumbered, and stale events find the tombstone. *Cost:* one more `match` arm in the dispatch, and memory that never shrinks during a run. A run with many removals keeps its tombstones until Reset. *Rules out:* nothing, but the engine has to treat "an object that isn't there" everywhere it reads by id.
- **A generation per id,** reusing places. Saves memory, but every id carried in an event, `succ` or `blockedlist` grows to carry its generation, which widens the 32-byte heap entry. *Cost:* on the per-event path, so it needs the benchmark first.

### 2. MUs in an object being removed

Choices, each a rule users would have to learn:
- **Refuse** while it holds MUs or has an event queued, so the user empties it first by pausing what feeds it. Simplest, and the run stays sound.
- **Destroy them,** recording them as leaving through a "removed" occurrence, so throughput and lead time leave them out. Statistics have to say how many went this way, or results quietly look better.
- **Move them** to a successor or a chosen object. Needs room there, and the same blocked rules as any move.

### 3. Connections and the blocked lists

Removing a connection while a predecessor waits on that successor has to take the predecessor out of its `blockedlist`, and if that was its only successor, it stays blocked with nowhere to go. Adding a connection to a blocked predecessor should let it try the new successor at once, as a `BlockedRetry` would. Reordering exits resets `succ_pointer`'s meaning. Each is a few lines in `flows.rs`, but the convention in CLAUDE.md that a predecessor gives up its place at every successor as soon as its MU moves was written for a fixed structure, and its tests would need cases for each change.

### 4. The intervention log and going back

Structural changes become `Action`s, such as `AddObject`, `RemoveObject`, `Connect` and `Disconnect`, carrying what replay needs to do them again: for `AddObject`, the whole object's description and stream. *Cost:* the log stops being a list of small `Copy` values, so its memory grows with what is built, and `Action` stops being `Copy`. Going back across a structural change is the hard part:
- **Checkpoints hold the old structure,** since a checkpoint is a clone of the run. Restoring one before an `AddObject` gives a run without that object, and replay adds it again at its time. This works with no new mechanism, as long as every structural change is logged.
- **The view's objects change with shown time.** Going back across an `AddObject` has to remove the object from the canvas, and going forward again has to add it. Today the view builds its objects once per model, so it would need the structure at shown time with each full state, or the structural `Action`s as outputs. *Cost:* the full state grows by the structure, which is sent at Init, Reset, going back and on request, not per event.
- **Marks:** each structural change leaves a mark like any intervention, and going back past one asks first, as today.

### 5. The model file

A run's structure would no longer be the file's. Choices:
- **Changes made during a run stay in the run,** and Reset drops them, unless "Keep changes" saves them to the file in one `edit_model` batch, as it does for process times today. The file never changes under a running session.
- **Each change goes to the file at once,** and the run follows the file. Agents and the window would see the same model, but a run going back would disagree with the file, and the results store's `model_text` would have to be the model at the start.

### 6. Results and reproducibility

- A headless run (`start_run`) never builds during a run, so its results and their reproducibility are untouched.
- A live session that built during its run isn't a model file's run any more. Comparing it with `compare_runs` would need the log stored beside the result, or would be refused.
- Statistics of a removed object stop where it went. Shares of an added one start from its own beginning, so its measured time differs from the run's.

## The performance cost, in short

| Part | Per event | Per change | Memory |
|---|---|---|---|
| Tombstones | one more dispatch arm, likely unmeasurable | none | removed objects kept until Reset |
| Generations | wider ids in every event, needs measuring | none | none |
| Structural `Action`s | none | logging the change | the log grows with what is built |
| View following structure | none | a structure in each full state | small |

A headless run with nothing built during it costs nothing more with tombstones and structural actions, which fits "live sessions cost nothing when absent". Generations would cost every run and are the one choice that touches the per-event path.

## What it rules out or puts off

- Tombstones put off reclaiming memory until Reset, which a long run with constant rebuilding would feel.
- Saving each change to the file at once rules out going back freely during a run, or makes the file and the run disagree.
- Destroying MUs on removal rules out results that count every MU created, unless removals are reported separately.
