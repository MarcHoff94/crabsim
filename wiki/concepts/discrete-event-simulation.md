---
title: Discrete-Event Simulation
section: concepts
related:
  - simulator-and-event-controller
  - event-types
---

# Discrete-Event Simulation

## What it is

In a discrete-event simulation (DES), time does not tick forward continuously. Instead, the simulation jumps directly from one event to the next. Between events, nothing changes. This makes DES extremely efficient for systems where meaningful activity is sparse relative to real time — like a factory floor where a machine might run for 30 seconds before anything interesting happens.

## How time advances in crabsim

The `EventController` holds a priority queue (`BinaryHeap<ScheduledEvent>`) of future events sorted by their scheduled time. The run loop is:

1. Pop the earliest event from the queue.
2. Advance the simulation clock to that event's time.
3. Dispatch the event to the object it targets (looked up by `object_id` in the `Simulator`).
4. The object processes the event and returns zero or more new `EventTemplate`s, each with a relative `exec_time` offset.
5. Schedule those new events at `current_time + exec_time`.
6. Repeat until the queue is empty or `endtime` is reached.

No time passes inside step 3. The state at any point is exactly the state at the most recently processed event.

## FIFO tiebreaking

Events scheduled at the same time are processed in the order they were created. The `EventController` assigns a monotonically increasing `seq` number to every scheduled event. When two events have identical times, the one with the lower `seq` fires first. This makes the simulation deterministic and the test suite reproducible.

## Why DES for a production line

A manufacturing simulation has natural events: a machine finishes processing, a part transfers to the next station, a failure begins. These happen at well-defined moments with nothing to compute in between. DES captures this cleanly without the overhead of a time-step loop that would spin through thousands of idle ticks.

## Code references

- `EventController` — `src/simulator.rs`
- `ScheduledEvent`, `EventTemplate` — `src/event.rs`
- `BinaryHeap` ordering (time + seq) — `src/event.rs` (`Ord` impl on `ScheduledEvent`)
