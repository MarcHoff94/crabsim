---
title: Writing Tests (AI context)
for: guides/writing-tests.md
section: guides
---

## Facts
- `tests/simulation_run.rs`: run behaviour via Init, advance, Reset, occurrences, state; helpers `secs`, `no_failures`, `station`, `preloaded_station`, `recorded`, `exit_ids`, `exit_times`, `statistics_of`
- `tests/live_session.rs`: commands and observations with `FakeClock` and `sync`; Picture checked against the run
- `tests/cli.rs`: the binary end to end
- Unit tests in `src/`: failures part way through an event (`RecordsThenFails`), session buffer limit and thinning
- `ui/tests/`: view state against a real session, no window
- Build models through `ModelDescription`; `availability = 1.0` unless testing failures; `Recording::ALL` to read occurrences

## Invariants
- Check every test can fail by breaking the code on purpose
- Every new field gets a reset test
- No speed or timing assertions
- Refactors of scheduling must keep every occurrence identical (hash against master)

## Code locations
- `tests/simulation_run.rs`, `tests/live_session.rs`, `tests/cli.rs`
- Long run: `cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model`
