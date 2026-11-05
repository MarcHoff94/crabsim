# Usage Guide

This guide walks through building, running, and extending CrabSim with practical examples.

## Building the Project
```powershell
cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
```

## Running the Demo
The repository ships a demo binary that constructs a simple network of one source, two stations and a drain.
```powershell
$env:RUST_LOG='info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim
```
The output shows an ASCII table that updates after each event. To enable the external renderer, set `CRABSIM_RENDERER=1`.

## Simple Example – 3‑Station Flow
Create a new Rust file `examples/simple_flow.rs`:
```rust
use crabsim::{Simulator, Source, SingleStation, Drain};
use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;

fn main() {
    let mut sim = Simulator::new();
    let src = Rc::new(RefCell::new(Source::new(Duration::from_secs(1))));
    let st1 = Rc::new(RefCell::new(SingleStation::new(Duration::from_millis(500))));
    let st2 = Rc::new(RefCell::new(SingleStation::new(Duration::from_millis(700))));
    let drain = Rc::new(RefCell::new(Drain::new()));

    // Wire successors
    src.borrow_mut().succ = vec![st1.clone()];
    st1.borrow_mut().succ = vec![st2.clone()];
    st2.borrow_mut().succ = vec![drain.clone()];

    sim.register_material_flow(src);
    sim.register_material_flow(st1);
    sim.register_material_flow(st2);
    sim.register_material_flow(drain);

    sim.run();
}
```
Run with `cargo run --example simple_flow`.

## Analyzing Results
After the simulation finishes, CrabSim prints throughput statistics:
- Total MUs processed
- Average processing time per station
- Idle times
These metrics are derived from timestamps recorded by each object during the event loop.

---

For advanced usage (e.g., custom scheduling policies or parallel execution), consult the [Extension Guide](06-EXTENSION-GUIDE.md).