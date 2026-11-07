---
title: Running crabsim (AI context)
for: guides/running-crabsim.md
section: guides
---

## Facts
- CLI: `cargo run --release --bin crabsim -- --seed 7 --length 365d` runs the demo headless (or a model file with `--model models/demo.json`) and prints per-object statistics, Drain throughput and lead time, events and wall time
- `--length` uses the UI's time syntax (`90`, `1:30`, `8:00:00`, `30d 12:00:00`); defaults: seed 0, length `1d`
- UI: `cargo run -p crabsim-ui --profile ui` (first build several minutes)
- In code: `demo::model().build(seed)`, `init`, `advance_to`, `state`; `set_recording(Recording::ALL)` for occurrences
- Benchmark: `cargo bench --bench reference` (about a minute; demo and 6,006 objects)
- `RUST_LOG=info` logs every event (about 42,000 a simulated day for the demo)

## Pitfalls
- Debug builds are about 10x slower; use release for long lengths
- A run error still prints statistics, then exits with failure

## Code locations
- CLI — `cli/src/main.rs`; model files — `model/`, `models/demo.json`; demo — `src/demo.rs`; benchmark — `benches/reference.rs`
