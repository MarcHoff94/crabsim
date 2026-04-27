---
title: Visualization & Renderer
section: architecture
related:
  - simulator-and-event-controller
  - configuration-flags
---

# Visualization & Renderer

## Two output modes

crabsim supports two ways to display the simulation state:

| Mode | How | When |
|---|---|---|
| Console (plain) | Prints a new table after each event | Default |
| Console (in-place) | Rewrites the same table using ANSI escape sequences | `--inplace` |
| External renderer | Sends JSON to a separate terminal window | `--renderer-window` |

## What gets printed

After every processed event the simulator emits two things: a static **topology** (once, at the start) and a **state snapshot** (after each event).

### Topology

`Simulator::print_network_topology()` (or `render_network_topology` for the renderer) prints the directed graph of objects as an ASCII tree. It walks `succ` relationships and draws branches and connectors.

For the external renderer it is sent as:
```json
{ "kind": "topology", "text": "..." }
```

### State snapshot

`Simulator::print_materialflow_state()` collects a `StateSnapshot` from all registered objects — each becomes a `StateRow` containing the object's current state, content, timing info, and interruption status — and renders it as a table with columns: ID, Type, Content, State, Interr (interruption), RemTime, Blocked.

For the external renderer:
```json
{ "kind": "snapshot", "snapshot": { ... } }
```

### StateRow fields

```rust
pub struct StateRow {
    pub id: u32,
    pub typ: String,         // "Station", "Source", "Drain", "Unknown"
    pub content: String,     // "MU<n>" or "-"
    pub state: String,       // "Process", "Blocked", "Idle", or "-"
    pub interruption: String, // "-", "Failed", or "Paused"
    pub rem_time: u128,      // remaining processing time in milliseconds
    pub blocked: String,     // comma-separated blocked predecessor IDs or "-"
}
```

The renderer only counts down `rem_time` when `interruption == "-"`; a failing or paused station's displayed remaining time is frozen until the interruption clears.

## External renderer

The external renderer is a second binary (`crabsim-renderer`) that:

1. Connects to the simulator via TCP on a port chosen by the simulator.
2. Receives newline-delimited JSON messages.
3. Displays: header, topology string, live state table.
4. Paces display: `1 sim-second = 1/speed wall-seconds`. At `speed = -1` (unpaced), renders immediately until the last event then holds open.

### Launch sequence

When `CRABSIM_RENDERER=1`:
1. Simulator binds a random local TCP port.
2. Spawns `crabsim-renderer --connect <ip>:<port> --speed <factor>`.
   - On Linux: direct `Command::new(renderer_prog)`.
   - On Windows: `PowerShell Start-Process` (maximized) or `cmd /C start /MAX`.
3. Accepts the single inbound connection.
4. Immediately sends the topology message.

The renderer binary path is resolved relative to the current executable (same directory). If the renderer binary is missing, the simulator silently continues without it.

## In-place console mode

When `live_inplace = true`, the simulator:
1. Prints an ANSI anchor marker on the first snapshot.
2. On subsequent snapshots, moves the cursor back to the anchor and overwrites the previous table.

This gives a live-updating dashboard in the terminal without scroll.

## Code references

- `Simulator::print_materialflow_state`, `print_network_topology` — `src/simulator.rs`
- `StateSnapshot`, `StateRow`, `StateVisualizer` — `src/visualization/state.rs`
- `TopologyVisualizer`, `render_network_topology` — `src/visualization/topology.rs`
- Renderer spawn logic — `src/lib.rs` (`run_demo`)
