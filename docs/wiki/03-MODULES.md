# Module Reference

This section provides a high‑level API overview for each public module in CrabSim.

## `src/lib.rs`
- Re‑exports the main entry point `run_demo()` and unit tests.
- Public items:
  - `pub fn run_demo() -> Result<(), Box<dyn std::error::Error>>` – builds a sample network and runs the simulator.

## `src/simulator.rs`
- **Simulator**: registry, state snapshot, event controller integration.
- **EventController**: priority queue, time advancement, FIFO sequencing.
- Public structs:
  - `pub struct Simulator { … }`
  - `pub struct EventController { … }`

## `src/materialflow_objects/mod.rs`
- Re‑exports the concrete material‑flow types and traits.
- Public items:
  - `pub mod flows;` – trait definitions (`MaterialFlow`, `EventInterface`).
  - `pub mod source;` – `Source` struct.
  - `pub mod station;` – `SingleStation` struct.
  - `pub mod drain;` – `Drain` struct.

## `src/materialflow_objects/flows.rs`
- Trait definitions:
  - `MaterialFlow`: methods for state inspection, entrance/exit handling.
  - `EventInterface`: event callbacks (`on_enter`, `on_exit`).
- Helper structs:
  - `MoveableUnit` – represents an MU with id and location.
  - `ScheduledEvent` – internal event representation.

## `src/materialflow_objects/source.rs`
- Implements a source that periodically creates MUs.
- Configurable via `interval: Duration` and optional `max_units`.

## `src/materialflow_objects/station.rs`
- Implements a single‑station processor.
- Handles processing time, blocked retry logic, and successor wiring.

## `src/materialflow_objects/drain.rs`
- Consumes MUs and records exit timestamps for statistics.

## `src/visualization/mod.rs` & submodules
- Provides optional external renderer support.
- Serialises simulation snapshots to JSON and sends over a channel.

---

For detailed method signatures, refer to the source files or run `cargo doc --open` in the repository root.