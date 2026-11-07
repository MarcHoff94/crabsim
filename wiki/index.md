---
title: crabsim Wiki
section: root
---

# crabsim Wiki

A discrete-event simulator for any material flow, such as a factory line, written in Rust. Sources create moveable units (MUs), Stations work on them, Buffers hold them and Drains take them out. crabsim exists to simulate long periods as fast as possible, and a live session can also play a run for a UI to show.

`CLAUDE.md` describes what is built, `GLOSSARY.md` defines the words, `docs/adr/` records the decisions, and `docs/agents/handoff.md` holds the agreed design, including what isn't built yet. This wiki explains how the parts work.

## Start here

Read these in order if you are new to the codebase:

1. [Discrete-Event Simulation](concepts/discrete-event-simulation.md): how time moves
2. [Material Flow Model](concepts/material-flow-model.md): objects, connections and moves
3. [Blocked Retry Semantics](concepts/blocked-retry.md): the key non-obvious mechanic
4. [Architecture Overview](architecture/overview.md): how the Rust code is organised

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
- [Simulation Run](architecture/simulation-run.md)
- [Material Flow Objects](architecture/material-flow-objects.md)
- [Live Session & UI](architecture/live-session-and-ui.md)

### Guides
How to do common tasks.

- [Running crabsim](guides/running-crabsim.md)
- [Adding a New Object Type](guides/adding-a-new-object-type.md)
- [Writing Tests](guides/writing-tests.md)

### Reference
Quick-lookup facts.

- [Event Types](reference/event-types.md)
- [Configuration Flags](reference/configuration-flags.md)

## AI context

Compact summaries for model context live in `_ai/`.

- Full project context: `@wiki/_ai/all.md`
- Per topic: `@wiki/_ai/<section>/<page>.md`
