---
title: crabsim Wiki
section: root
---

# crabsim Wiki

Discrete-event material-flow simulator written in Rust. Models production lines where parts (Moveable Units) are created by a Source, processed by SingleStations, and consumed by a Drain.

## Start here

Read these in order if you are new to the codebase:

1. [Discrete-Event Simulation](concepts/discrete-event-simulation.md) — the core time model
2. [Material Flow Model](concepts/material-flow-model.md) — how objects connect into a network
3. [Blocked Retry Semantics](concepts/blocked-retry.md) — the key non-obvious mechanic
4. [Architecture Overview](architecture/overview.md) — how the Rust code is organized

## Sections

### Concepts
Domain ideas a reader must grasp before the code makes sense.

- [Discrete-Event Simulation](concepts/discrete-event-simulation.md)
- [Material Flow Model](concepts/material-flow-model.md)
- [Moveable Units](concepts/moveable-units.md)
- [Blocked Retry Semantics](concepts/blocked-retry.md)

### Architecture
How the main components are structured and why.

- [Overview](architecture/overview.md)
- [Simulator & EventController](architecture/simulator-and-event-controller.md)
- [Material Flow Objects](architecture/material-flow-objects.md)
- [Visualization & Renderer](architecture/visualization-and-renderer.md)

### Guides
How to do common tasks.

- [Running the Demo](guides/running-the-demo.md)
- [Adding a New Station Type](guides/adding-a-new-station-type.md)
- [Writing Tests](guides/writing-tests.md)

### Reference
Quick-lookup facts.

- [Event Types](reference/event-types.md)
- [Configuration Flags](reference/configuration-flags.md)

## AI context

Compact summaries optimized for model context injection live in `_ai/`.

- Full project context: `@wiki/_ai/all.md`
- Per-topic: `@wiki/_ai/<section>/<page>.md`
