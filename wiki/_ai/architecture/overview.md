---
title: Architecture Overview (AI context)
for: architecture/overview.md
section: architecture
---

## Facts
- Four layers: EventController (time) → Simulator (registry) → material flow objects (domain) → visualization (output)
- `Simulator.material_flows: HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>` — keyed by object_id
- `EventController` owns the `BinaryHeap`; `Simulator` owns the objects
- After `eventcontroller.run(simulator)` returns the simulator — inspect `sim.drains[i].borrow().exit_ids`
- All objects behind `Rc<RefCell<T>>` — multiple owners, runtime borrow checking

## Control flow
```
init() → all objects return initial EventTemplates → scheduled
run(): pop event → find object by id → execute_event → schedule returned templates → repeat
```

## Module map
```
src/simulator.rs          Simulator, EventController
src/event.rs              EventType, EventTemplate, ScheduledEvent
src/materialflow_objects/
  flows.rs                traits, MoveableUnit, FailureProfile, movement helpers
  source.rs / station.rs / drain.rs
src/visualization/
  topology.rs / state.rs
src/lib.rs                public API, run_demo(), unit tests
src/main.rs               CLI (clap), calls run_demo()
src/random_numbers.rs     Distribution, DistributionSampler
```
