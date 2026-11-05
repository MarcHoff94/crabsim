---
title: Configuration Flags (AI context)
for: reference/configuration-flags.md
section: reference
---

## CLI flags → env vars they set
- `--inplace` → `CRABSIM_LIVE_INPLACE=1`
- `--renderer-window` → `CRABSIM_RENDERER=1`
- `--speed <f>` → `CRABSIM_RENDERER_SPEED=<f>` (default `5.0`; `-1` = unpaced)
- `--hold <s>` → `CRABSIM_RENDERER_HOLD=<s>` (default `1`)
- `--renderer-launcher <l>` → `CRABSIM_RENDERER_LAUNCHER=<l>` (Windows only: `powershell`|`cmd`)

## Direct env vars (bypass CLI)
Same variables as above. CLI flags overwrite env vars (they set them before `run_demo()`).

## RUST_LOG
Parsed by `env_logger`. Values: `info`, `debug`, `crabsim=debug`, `crabsim=info`.

## Hardcoded values (change in src/lib.rs)
- Simulation endtime: `3_600.0` seconds in `EventController::new(Some(...))`
- Default renderer speed: `"5"` string in `run_demo()`

## Code locations
- CLI parsing — `src/main.rs` (clap `Opts` struct)
- Env var reading — `src/lib.rs` (`run_demo`)
