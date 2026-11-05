---
title: Simulator & EventController (AI context)
for: architecture/simulator-and-event-controller.md
section: architecture
---

## Facts
- `Simulator::register_material_flow(obj)`: assigns next `object_id`, stores `Rc` clone, **returns the assigned id** — store it when you need to manually schedule events
- `Simulator::add_drain(drain)`: also required for drains to appear in `sim.drains` after the run
- `EventController::add_event(time, template)`: wraps in `ScheduledEvent`, assigns next `seq`, pushes to heap
- `EventController::run(simulator)` consumes simulator, returns it after the run
- `simulator.current_seq` is updated per event for logging

## Invariants
- Registration order determines object IDs — tests that manually schedule by id depend on this
- Both `register_material_flow` AND `add_drain` must be called for each `Drain`
- `seq` is monotonically increasing; do not reset or reuse
- `endtime = None` → run until queue empty; `endtime = Some(d)` → stop when `time >= d`

## Borrow rules
- `execute_event` receives `&mut Simulator` — do not hold a `RefCell` borrow on any object while calling it
- Objects are accessed via `simulator.material_flows.get(&id).unwrap().borrow_mut()`

## Code locations
- `Simulator`, `EventController` — `src/simulator.rs`
- `register_material_flow`, `add_drain`, `init` — `src/simulator.rs`
- `EventController::add_event`, `run` — `src/simulator.rs`
