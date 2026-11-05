# Core Architecture

This document describes the high-level design of CrabSim and how its components interact.

## System Overview

```
┌─────────────────────────────────────────────────────┐
│                   Simulator                          │
│  • Registry of all material-flow objects             │
│  • Assigns object IDs on registration                │
│  • Coordinates state snapshots & visualization       │
└─────────────────────────────────────────────────────┘
                         ▲
                         │ controls
                         │
┌─────────────────────────────────────────────────────┐
│              EventController                         │
│  • Priority queue of scheduled events (BinaryHeap)  │
│  • Current simulation time (Duration)                │
│  • FIFO disambiguation via sequence counter (seq)    │
└─────────────────────────────────────────────────────┘
                         ▲
                         │ executes
                         │
┌─────────────────────────────────────────────────────┐
│           Material-Flow Objects                      │
│  • Source: Creates MUs                               │
│  • SingleStation: Processes MUs                      │
│  • Drain: Consumes MUs                               │
└─────────────────────────────────────────────────────┘
```

## Component Responsibilities

### Simulator

**File**: `src/simulator.rs`

The `Simulator` is the **authoritative registry** of all material-flow objects in the system.

**Key responsibilities:**
- Register material-flow objects via `register_material_flow()` → assigns sequential `object_id`
- Maintain `material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>` for all registered objects
- Store `drains: Vec<Rc<RefCell<Drain>>>` for exit analysis
- Coordinate visualization output:
  - Call `print_materialflow_state()` after each event
  - Send JSON snapshots to external renderer (if connected)
- Track `current_seq` for FIFO tiebreaking

**Example initialization:**
```rust
let mut simulator = Simulator::new();
let source = Rc::new(RefCell::new(Source::new(...)));
let st1 = Rc::new(RefCell::new(SingleStation::new(...)));
simulator.register_material_flow(source.clone());
simulator.register_material_flow(st1.clone());
```

### EventController

**File**: `src/simulator.rs`

The `EventController` is the **discrete-event engine** that advances time and executes scheduled events.

**Key characteristics:**
- Holds `BinaryHeap<ScheduledEvent>` (priority queue, min-heap)
- Events are ordered by time (ascending), then by sequence number (ascending) for FIFO tiebreaking
- `current_time: Duration` tracks wall-clock simulation time
- `seq: u64` monotonically increases; assigned to each event for deterministic ordering

**Typical flow:**
```
1. init() → Initialize simulator, collect initial events
2. run()  → Event loop:
   a. Pop next event from heap
   b. Advance simulator time
   c. Call execute_event() on the target object
   d. Collect returned events, assign seq, re-queue
3. return → Final simulator state
```
