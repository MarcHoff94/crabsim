---
title: Writing Tests
section: guides
related:
  - simulation-run
  - material-flow-objects
  - blocked-retry
---

# Writing Tests

Tests go through the engine's seams, never into an object's insides:

| File | What it tests |
|---|---|
| `tests/simulation_run.rs` | Behaviour at the run's interface: Init, advance, Reset, then read occurrences and state |
| `tests/live_session.rs` | Commands in and outputs out, with a fake clock, and the `Picture` against the run's own state |
| `model/tests/model_file.rs` | Model files: text in, a model, its problems or a run out. Validation tests feed broken files and check every problem comes back with its location |
| `cli/tests/cli.rs` | The CLI's binary, end to end |
| `src/` unit tests | What can't be seen from outside: a failure part way through an event (`RecordsThenFails`), and in `live_session.rs` the buffer limit and thinning |
| `ui/tests/` | The UI's view state against a real live session on a fake clock, with no window |

## A run test

Build the model through `ModelDescription`, turn recording on if you read occurrences, and assert on occurrences and state:

```rust
let mut model = ModelDescription::new();
let source = model.add_source(secs(5));
let station = model.add_station(secs(10), no_failures(0.0));
let drain = model.add_drain();
model.connect(source, station);
model.connect(station, drain);

let mut run = model.build(0);
run.set_recording(Recording::ALL);
run.init().unwrap();
let occurrences = run.advance_to(secs(100)).unwrap();

assert_eq!(exit_ids(&occurrences), vec![1, 2, 3, ...]);
assert_eq!(statistics_of(&run, drain).mus_in, 9);
```

`tests/simulation_run.rs` has helpers for this: `secs`, `no_failures`, `station`, `preloaded_station`, `recorded`, `exit_times`, `exit_ids`, `statistics_of`. Tests use `availability = 1.0` unless they exercise failures.

To set up a particular moment, give a Station a starting MU (`add_station_holding`) or queue an event from outside with `schedule_event`, such as `EventType::PauseOn` at a known time.

## A session test

Use the fake clock: move it, then `sync`, which blocks until the session has handled earlier commands and shown time has reached the clock. Read outputs from `session.outputs()` and feed them to a `Picture` to compare with the run's own state.

## Rules

- **Check the test can fail.** Break the code on purpose and watch the test go red. This keeps finding tests that never exercised the rule they were named after.
- **Reset tests:** every new field needs one. A same-seed replay can't detect events left over after Reset, because they land on the replay's own event times.
- **No speed or timing assertions.** The benchmark measures speed. Tests check what can be observed, such as a headless run returning no occurrences.
- **Determinism:** the same seed gives identical results. `seq` order is load-bearing, so a refactor of scheduling must keep every occurrence identical; compare hashes of all occurrences and the final state against `master` before trusting it.

## Behaviours to keep

| Test | What it guards |
|---|---|
| `a_source_feeds_a_drain_in_order`, `order_is_preserved_across_stations` | MUs leave in the order they were created |
| `a_station_blocks_when_no_successor_accepts` | Refused everywhere means Blocked |
| `a_retry_while_the_predecessor_is_working_does_not_move_its_mu` | A retry to a busy object does nothing |
| `the_longest_waiting_predecessor_retries_first` | Retry order |
| `a_retry_only_offers_the_successor_that_sent_it` | Nothing overtakes those waiting |
| `failure_moves_completion_time_later` | Completion time includes time stopped |
| `a_pause_is_not_a_failure_and_counts_as_paused_time` | Pause and failure are separate |
| `reset_returns_the_exact_state_from_before_init` | Reset is complete |
| `a_seed_gives_identical_results_every_time` | Determinism |
| `machine_level_times_add_up_at_any_time_on_the_demo_model` | Statistics stay consistent |
| `the_demo_model_runs_far_past_fifteen_simulated_years` | No false zero-time loop. Ignored by default: `cargo test --release --test simulation_run -- --ignored --nocapture the_demo_model` |

## Code references

- Run tests and helpers — `tests/simulation_run.rs`
- Session tests — `tests/live_session.rs`
- Model file tests — `model/tests/model_file.rs`
- CLI tests — `cli/tests/cli.rs`
- `RecordsThenFails` — `src/materialflow_objects/records_then_fails.rs`
