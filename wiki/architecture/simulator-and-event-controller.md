---
title: Simulator & EventController
section: architecture
related:
  - discrete-event-simulation
  - material-flow-objects
  - event-types
---

# Simulator & EventController

These two structs in `src/simulator.rs` are the engine of the simulation. They are intentionally separated: `Simulator` is state, `EventController` is behaviour.

## Simulator

`Simulator` is the authoritative registry of all material flow objects in the simulation.

```rust
pub struct Simulator {
    pub material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
    pub drains: Vec<Rc<RefCell<Drain>>>,
    pub object_id: u32,          // increments with each registration
    pub current_seq: u64,        // current event sequence number
    pub live_inplace: bool,      // in-place ANSI terminal rendering
    pub renderer_stream: Option<TcpStream>,  // external renderer connection
    // ...
}
```

### Registration

`register_material_flow(obj)` assigns the next `object_id` to the object (via `set_object_id`), stores a clone of the `Rc` in `material_flows`, and returns the assigned ID. Registration order determines IDs — this matters for tests that manually schedule events by `object_id`.

`add_drain(drain)` registers a drain separately so the simulator can access typed drain data (exit records) after the run. Drains must be registered with both methods.

### Simulation init

`Simulator::init(&mut eventcontroller)` calls `init()` on every registered object. Each object returns initial `EventTemplate`s which the EventController schedules at `current_time + exec_time`. The `Source` uses this to schedule the creation of the first MU at time 0.

## EventController

`EventController` owns the event queue and drives time forward.

```rust
pub struct EventController {
    pub time: Duration,                   // current simulation clock
    pub endtime: Option<Duration>,        // stop condition
    event_queue: BinaryHeap<ScheduledEvent>,
    seq: u64,                             // monotonic counter for FIFO order
}
```

### Scheduling: add_event

```rust
eventcontroller.add_event(scheduled_time, event_template)
```

Wraps the template in a `ScheduledEvent` (time + template + seq), assigns the next `seq`, and pushes onto the heap.

### Run loop

`run(simulator)` consumes the simulator, processes events until the queue is empty or `endtime` is exceeded, then returns the simulator so callers can inspect drain results.

For each event:
1. Pop from heap (earliest time, lowest seq for ties).
2. Look up the object in `simulator.material_flows` by `object_id`.
3. Call `execute_event(&mut simulator, &eventtype)`.
4. Schedule any returned `EventTemplate`s at `current_time + exec_time`.
5. Update `simulator.current_seq` for logging.
6. Call `simulator.print_materialflow_state()` (console or renderer output).

### FIFO tiebreaking

The `seq` counter is the mechanism that makes same-time events deterministic. `ScheduledEvent::cmp` orders first by time (ascending), then `BlockedRetry` events before all others, then by `seq` (ascending). Because `BinaryHeap` is a max-heap and the time and `seq` comparisons are reversed, the event with the smallest time and smallest seq in its group is popped first.

Preserve `seq` assignment order when refactoring scheduling code — deterministic tests depend on it.

## Code references

- `Simulator`, `EventController` — `src/simulator.rs`
- `ScheduledEvent` ordering — `src/event.rs`
- `Simulator::init` — `src/simulator.rs`
- `EventController::run` — `src/simulator.rs`
