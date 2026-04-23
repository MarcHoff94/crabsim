---
title: Visualization & Renderer (AI context)
for: architecture/visualization-and-renderer.md
section: architecture
---

## Facts
- Three modes: plain console (new table per event), in-place ANSI (`live_inplace=true`), external renderer (TCP)
- Topology sent once: `print_network_topology()` — before first event
- State after every event: `print_materialflow_state()` — collects `StateSnapshot` from all objects
- External renderer is a separate binary (`crabsim-renderer`) co-located with the main binary

## Renderer JSON protocol
```
{"kind": "topology", "text": "..."}       // sent once at connection
{"kind": "snapshot", "snapshot": {...}}   // sent after each event
```

## Renderer launch (non-Windows)
```
Command::new(renderer_path).arg("--connect").arg("ip:port").arg("--speed").arg(speed_factor)
```

## Speed factor
- `speed = N` → 1 sim-second = `1/N` wall-seconds
- `speed = -1` → unpaced, renders immediately, then holds open

## In-place mode
- First snapshot: prints ANSI anchor
- Subsequent snapshots: cursor back to anchor, overwrite

## Code locations
- `Simulator::print_materialflow_state`, `print_network_topology` — `src/simulator.rs`
- `StateSnapshot`, `StateRow` — `src/visualization/state.rs`
- `TopologyVisualizer` — `src/visualization/topology.rs`
- Renderer spawn logic — `src/lib.rs` (`run_demo`)
- `Simulator.renderer_stream: Option<TcpStream>` — `src/simulator.rs`
