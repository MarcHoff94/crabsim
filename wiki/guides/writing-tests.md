---
title: Writing Tests
section: guides
related:
  - simulator-and-event-controller
  - material-flow-objects
  - blocked-retry
---

# Writing Tests

Tests live in `src/lib.rs` under `#[cfg(test)]`. All tests use the same pattern: build a small network, run the simulation, assert on drain exits.

## Minimal setup pattern

```rust
let mut simulator = Simulator::new();
let mut eventcontroller = EventController::new(Some(Duration::from_secs_f64(1000.0)));

let default_fp = FailureProfile::new(
    100.0,
    Duration::ZERO,
    Distribution::CONSTANT { value: 0.0 },
    Distribution::CONSTANT { value: 0.0 },
    0,
);

let station = Rc::new(RefCell::new(SingleStation::new(
    0,
    Duration::from_secs_f64(10.0),
    default_fp.clone(),
    None,
    Vec::new(),  // succ — wire after creation
    Vec::new(),  // pred
)));
let source = Rc::new(RefCell::new(Source::new(0, Duration::from_secs_f64(5.0), vec![station.clone()])));
let drain = Rc::new(RefCell::new(Drain::new(0, vec![station.clone()])));

station.borrow_mut().succ = vec![drain.clone()];

simulator.register_material_flow(source.clone());
simulator.register_material_flow(station.clone());
simulator.register_material_flow(drain.clone());
simulator.add_drain(drain.clone());

eventcontroller.init(&mut simulator);
let sim = eventcontroller.run(simulator).unwrap();
```

## Asserting on exits

```rust
let exits = &sim.drains[0].borrow().exits;      // Vec<Duration> — timestamps
let exit_ids = &sim.drains[0].borrow().exit_ids; // Vec<u32> — MU ids in arrival order

assert!(!exit_ids.is_empty());

// FIFO check: IDs should be sequential from 1
for (i, &id) in exit_ids.iter().enumerate() {
    assert_eq!(id as usize, i + 1);
}

// Throughput check
assert!(exits.len() >= expected_count);
```

## Manually scheduling an event

Use this to set up a specific mid-simulation state, e.g., a busy station that finishes at a known time:

```rust
let b_id = simulator.register_material_flow(station_b.clone());

eventcontroller.add_event(
    Duration::from_secs_f64(100.0),
    EventTemplate {
        eventtype: EventType::MUExit,
        exec_time: Duration::ZERO,
        object_id: b_id,
        seq: 0,
    },
);
```

Always store the return value of `register_material_flow` when you need the assigned `object_id` for manual event scheduling.

## Testing blocked/retry behaviour

Pre-configure a station as busy and locked to test the blocked/retry path:

```rust
let mut busy = SingleStation::new(0, Duration::from_secs_f64(75.0), fp.clone(), None, Vec::new(), Vec::new());
busy.content = Some(MoveableUnit { id: 9999, location_id: 0 });
busy.entrancelocked = true;
busy.state = ProcessState::Processing;
let station_b = Rc::new(RefCell::new(busy));
```

Then schedule its `MUExit` manually to trigger the retry flow at a known time.

## Behaviours tests must preserve

When refactoring core scheduling or movement logic, keep these test guarantees intact:

| Test | What it verifies |
|---|---|
| `drain_fifo_order` | MU IDs arrive at drain in creation order |
| `ordering_across_stations_preserved` | FIFO preserved across a linear chain |
| `ordering_across_parallel_stations_preserved` | FIFO preserved across parallel branches |
| `station_becomes_blocked_when_no_successor_accepts` | Predecessor goes Blocked when all successors deny |
| `blocked_retry_while_predecessor_processing_does_not_move_mu` | BlockedRetry is no-op when station is Processing |
| `blocked_retry_full_flow` | End-to-end blocked/retry reaches the drain |
| `blockedlist_dedup` | Inherent `add_blocked` method deduplicates |
| `source_continues_producing_after_blocked_retry` | Source creates ≥3 MUs after a blocked-retry cycle |
