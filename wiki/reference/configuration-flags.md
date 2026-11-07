---
title: Configuration Flags
section: reference
related:
  - running-crabsim
  - live-session-and-ui
---

# Configuration Flags

## CLI

Parsed by `clap` in `cli/src/main.rs`.

| Flag | Default | Effect |
|---|---|---|
| `--model <FILE>` | the built-in demo | A model file to run. An invalid one prints every problem and runs nothing |
| `--seed <N>` | `0` | Seed for the run's random numbers. The same seed gives the same results |
| `--length <TIME>` | `1d` | Simulated length, written as the UI shows time: `90`, `1:30`, `8:00:00`, `30d 12:00:00`. Anything else is refused |


## Environment

| Variable | Effect |
|---|---|
| `RUST_LOG` | `env_logger` level, such as `info`, `debug` or `crabsim=debug`. Per-event logging slows a run down a lot |

## Cargo profiles

| Profile | Use |
|---|---|
| `dev` | Tests. Dependencies are optimised even here, because Bevy is slow without |
| `release` | The CLI and the benchmark: thin LTO and one codegen unit, about 11% faster than the default. `panic` stays `unwind`, because `LiveSession` catches panics |
| `ui` | The UI day to day: release without LTO, so it rebuilds in seconds |

The Rust version is pinned in `rust-toolchain.toml`.

## Live session settings

`SessionSettings`, fixed when a session is created except the time mode and `reset_keeps_changes`, which have commands:

| Setting | Default | Effect |
|---|---|---|
| `buffer_instants` | 100,000 | How many instants the run may compute ahead of what has been released |
| `checkpoint_spacing` | 100,000 events | How far apart checkpoints are |
| `checkpoint_budget_bytes` | 256 MB | Memory for checkpoints. Past it, every other one in the older half goes |
| `time_mode` | `Continuous` | Or `Skipping`, where no gap between instants takes longer than a set wall time (`TimeMode::SKIPPING`: 0.25 s) |
| `reset_keeps_changes` | off | Whether Reset keeps changed process times as the new default state |

A session starts halted at speed 1. The UI sets 60 times real time.

## Code references

- `Opts` — `cli/src/main.rs`
- `SessionSettings`, `TimeMode` — `src/live_session.rs`
- Profiles — `Cargo.toml`
