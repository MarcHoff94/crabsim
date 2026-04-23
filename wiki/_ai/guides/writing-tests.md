---
title: Writing Tests (AI context)
for: guides/writing-tests.md
section: guides
---

## Minimal test pattern
```rust
let mut sim = Simulator::new();
let mut ec = EventController::new(Some(Duration::from_secs_f64(1000.0)));
let fp = FailureProfile::new(100.0, Duration::ZERO, Distribution::CONSTANT{value:0.0}, Distribution::CONSTANT{value:0.0}, 0);
// create objects, wire succ/pred, register (register_material_flow + add_drain for Drain)
ec.init(&mut sim);
let sim = ec.run(sim).unwrap();
let exit_ids = &sim.drains[0].borrow().exit_ids;
```

## Manual event scheduling
```rust
let b_id = simulator.register_material_flow(station_b.clone()); // store returned id
eventcontroller.add_event(Duration::from_secs_f64(100.0), EventTemplate {
    eventtype: EventType::MUExit, exec_time: Duration::ZERO, object_id: b_id, seq: 0
});
```

## FIFO assertion
```rust
for (i, &id) in exit_ids.iter().enumerate() { assert_eq!(id as usize, i + 1); }
```

## Tests that must not regress
`drain_fifo_order`, `ordering_across_stations_preserved`, `ordering_across_parallel_stations_preserved`, `station_becomes_blocked_when_no_successor_accepts`, `blocked_retry_while_predecessor_processing_does_not_move_mu`, `blocked_retry_full_flow`, `blockedlist_dedup`, `source_continues_producing_after_blocked_retry`
