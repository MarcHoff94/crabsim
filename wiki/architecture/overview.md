---
title: Architecture Overview
section: architecture
related:
  - simulator-and-event-controller
  - material-flow-objects
  - discrete-event-simulation
---

# Architecture Overview

## The four layers

```
┌─────────────────────────────────────────────────┐
│  EventController  (time engine)                 │
│  BinaryHeap<ScheduledEvent>  ·  seq counter     │
├─────────────────────────────────────────────────┤
│  Simulator  (registry)                          │
│  HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>    │
├─────────────────────────────────────────────────┤
│  Material flow objects  (domain logic)          │
│  Source · SingleStation · Drain                 │
├─────────────────────────────────────────────────┤
│  Visualization  (output)                        │
│  Console (ANSI) · External renderer (TCP/JSON)  │
└─────────────────────────────────────────────────┘
```

## Control flow during a simulation run

```
eventcontroller.init(&mut simulator)
  └─ calls init() on every registered object
  └─ objects return initial EventTemplates
  └─ EventController schedules them

eventcontroller.run(simulator)
  loop:
    pop earliest ScheduledEvent from BinaryHeap
    advance clock to event.time
    look up object by event.object_id in simulator.material_flows
    call object.execute_event(eventtype)
    object returns Vec<EventTemplate>
    schedule each at current_time + template.exec_time
  until queue empty or endtime reached
```

## Data flow for a single MU

```
Source::create_mu()
  → schedules MUExit after proc_time

Source::execute_event(MUExit)
  → try_to_move_to_successor → SingleStation::process_mu_entrance(mu)
  → SingleStation schedules MUExit after its proc_time
  → Source creates next MU

SingleStation::execute_event(MUExit)
  → try_to_move_to_successor → next station or Drain
  → on success: unlocks entrance, notifies blocked predecessors
  → on failure: goes Blocked, records self in successors' blockedlists

Drain::process_mu_entrance(mu)
  → records current_time in exits, mu.id in exit_ids
```

## Ownership model

Every material flow object is heap-allocated behind `Rc<RefCell<T>>`. This allows multiple owners (a station can appear in both its predecessor's `succ` list and its successor's `pred` list) while still providing mutable access at event dispatch time. The `Simulator` holds the canonical `HashMap` reference; `succ`/`pred` vectors hold additional clones of the same `Rc`.

## Module map

```
src/
  lib.rs              public API, run_demo(), unit tests
  main.rs             CLI parsing, calls run_demo()
  simulator.rs        Simulator, EventController
  event.rs            EventType, EventTemplate, ScheduledEvent
  error.rs            error types
  random_numbers.rs   Distribution, DistributionSampler
  materialflow_objects/
    mod.rs            re-exports
    flows.rs          traits, MoveableUnit, FailureProfile, movement helpers
    source.rs         Source
    station.rs        SingleStation
    drain.rs          Drain
  visualization/
    mod.rs
    topology.rs       network topology printer
    state.rs          state snapshot structs and renderer
```
