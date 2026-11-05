# Core Concepts

This document dives deeper into the key abstractions that make CrabSim work.

## Simulator
- **Purpose**: Central registry and orchestrator. All material‑flow objects register here, receive unique `object_id`s, and are stored in a `HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>`.
- **State snapshot**: After each event the simulator calls `print_materialflow_state()` which outputs an ASCII table of all stations, sources, drains, and current MU locations. When an external renderer is connected it also serialises a JSON snapshot.

## EventController
- **Priority queue**: Uses a `BinaryHeap<ScheduledEvent>` where `ScheduledEvent` contains the execution time, target object id, event type, and a monotonically increasing sequence number (`seq`).
- **Determinism**: Events with identical timestamps are ordered by `seq`, ensuring reproducible runs.
- **Lifecycle**:
  1. `init()` collects initial events from all registered objects.
  2. `run()` repeatedly pops the earliest event, updates simulation time, dispatches it to the target object via its `EventInterface` implementation, and enqueues any new events returned.

## Material‑Flow Objects
All concrete types implement two traits:
- `MaterialFlow`: provides methods for querying state (`get_state`, `get_successors`) and handling MU entrance/exit.
- `EventInterface`: callbacks invoked by the event controller (`on_enter`, `on_exit`).

### Source
Creates new `MoveableUnit`s at a configurable interval. It schedules its first creation during initialization and then reschedules itself after each MU is emitted.

### SingleStation
Processes one MU at a time. When an MU arrives it records the start time, schedules a completion event after the configured processing duration, and attempts to forward the MU to successors once finished. If all successors are blocked, the station becomes `Blocked` and retries when space frees up.

### Drain
Simply consumes MUs. It records the exit timestamp for each MU, which is later used by the simulator to compute throughput statistics.

## MoveableUnit (MU)
- Holds a unique id and the current location (`object_id`).
- Carries no payload; all logic is in the owning material‑flow object.

---

For code‑level details, inspect `src/simulator.rs`, `src/materialflow_objects/flows.rs`, and the concrete modules under `src/materialflow_objects`.