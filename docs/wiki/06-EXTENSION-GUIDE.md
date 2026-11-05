# Extension Guide

CrabSim is designed to be extended with custom material‑flow objects. This guide explains how to implement a new type and integrate it into the simulator.

## 1. Implementing a New Material‑Flow Object
All concrete types must implement two traits defined in `src/materialflow_objects/flows.rs`:
- `MaterialFlow`
- `EventInterface`

### Example: A Buffer Station
A buffer holds up to *N* MUs before forwarding them.
```rust
use crabsim::materialflow_objects::{MaterialFlow, EventInterface};
use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;
use crabsim::MoveableUnit;

pub struct Buffer {
    id: u32,
    capacity: usize,
    queue: Vec<MoveableUnit>,
    succ: Vec<Rc<RefCell<dyn MaterialFlow>>>,
}

impl Buffer {
    pub fn new(id: u32, capacity: usize) -> Self { /* ... */ }
}

impl MaterialFlow for Buffer {
    // Implement state queries and MU handling
}

impl EventInterface for Buffer {
    // Handle on_enter/on_exit events
}
```

## 2. Registering the Object
Create an `Rc<RefCell<Buffer>>`, register it with the simulator, and wire successors:
```rust
let buffer = Rc::new(RefCell::new(Buffer::new(id, cap)));
simulator.register_material_flow(buffer.clone());
buffer.borrow_mut().succ = vec![next_obj.clone()];
```

## 3. Scheduling Events
Your object should schedule events by returning `Vec<ScheduledEvent>` from its `on_enter` or `on_exit` callbacks. Use the simulator’s `seq` counter to assign sequence numbers for deterministic ordering.

## 4. Testing Your Extension
Add unit tests in `tests/` that:
- Instantiate the new object
- Wire it into a small network
- Run the simulator and assert expected MU counts or timestamps

Run with `cargo test -- --nocapture` to see debug logs.

---

For more advanced patterns (e.g., dynamic topology changes), refer to the source code in `src/materialflow_objects/flows.rs` and the existing concrete types for reference.