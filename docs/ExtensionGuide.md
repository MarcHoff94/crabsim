# Extension Guide

This guide shows how to plug a custom routing strategy into an existing simulation.

## 1. Define the strategy

```rust
use crate::materialflow_objects::flows::{DistributionStrategy, EventInterface};
use std::vec::Vec;

pub struct RandomStrategy;
impl DistributionStrategy for RandomStrategy {
    fn order(&self, n: usize, start: usize) -> Vec<usize> {
        // Simple round‑robin but shuffled each call.
        let mut indices: Vec<usize> = (0..n).collect();
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        indices.shuffle(&mut rng);
        indices
    }
}
```

## 2. Use the strategy in a simulation

```rust
use crate::materialflow_objects::{flows::{try_to_move_to_successor_with_strategy, RoundRobinStrategy}, MaterialFlow};
use crate::simulator::Simulator;
use std::rc::Rc;
use std::cell::RefCell;

// Assume `source`, `station` and `drain` are already created.
let mut simulator = Simulator::new();

// Choose the strategy you want to test.
let strategy = RandomStrategy;

// Schedule an event that will trigger movement. In a real demo you would
// use the simulator's scheduling API; here we just call the helper
// directly for illustration.
let result = try_to_move_to_successor_with_strategy::<
    impl SuccessorContainer + EventInterface,
    _>,  // Rust infers the generic type.
    &mut source, &simulator, &strategy);
```

## 3. Observe the logs

With `RUST_LOG=info` the simulator prints lines like:

```text
seq=42 MU 5 moved from 1 to 2 (no immediate event)
seq=42 MU 6 moved from 1 to 3
```

Changing the strategy changes how the unit is routed without touching the source, station or drain implementations.

---

Feel free to extend this pattern with more sophisticated strategies (shortest queue, priority‑based, etc.). The core movement logic in `flows.rs` already supports any type that implements `DistributionStrategy`.