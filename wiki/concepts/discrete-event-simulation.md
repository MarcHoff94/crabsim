---
title: Discrete-Event Simulation
section: concepts
related:
  - simulation-run
  - event-types
---

# Discrete-Event Simulation

## What it is

In a discrete-event simulation, time doesn't tick forward continuously. The simulation jumps from one event to the next, and nothing changes in between. That makes it efficient for systems where activity is sparse, like a factory floor where a machine works for 30 seconds before anything else happens.

## How time advances in crabsim

A `SimulationRun` holds a priority queue (`BinaryHeap<ScheduledEvent>`) of future events. Processing one event:

1. Pop the earliest event from the queue.
2. Move the clock to its time.
3. Hand it to the object it names, by id, with an `EventContext` (the time, a place to record occurrences, new MU ids) and `OtherObjects` (every other object, by id).
4. The object returns zero or more `EventTemplate`s, each due `exec_time` from now.
5. Queue them at `now + exec_time`.

No time passes inside step 3, so a move between objects takes no simulation time. A run advances only when asked: one event (`step`), up to a time (`advance_to`), or a number of events. It has no end time of its own.

## Time

Simulation time is a `SimTime`: a 64-bit count of whole milliseconds, for both points in time and lengths of it. Sampled times, such as a repair time, are rounded to the nearest millisecond. Wall time stays a `std::time::Duration`, so the compiler keeps the two apart.

## Order at one time

Several events can fall on the same millisecond. They run in a fixed order, so the same seed always gives the same result:

1. `BlockedRetry` events run before all others at that time, so predecessors already waiting get a free successor before anything else can reach it.
2. Within each of those two groups, events run in the order they were queued, by a monotonically increasing `seq`.

Too many events at one time (`ZERO_TIME_EVENT_LIMIT`, a million) return `RunError::ZeroTimeLoop` rather than looping forever.

## Why discrete events for material flow

A line has natural events: a Station finishes its work, an MU moves on, a failure begins or ends. They happen at well-defined moments with nothing to compute in between, so jumping between them costs far less than a loop that ticks through idle time.

## Code references

- `SimulationRun`, `ZERO_TIME_EVENT_LIMIT` — `src/simulation_run.rs`
- `ScheduledEvent` and its `Ord`, `EventTemplate`, `EventContext` — `src/event.rs`
- `SimTime` — `src/time.rs`
