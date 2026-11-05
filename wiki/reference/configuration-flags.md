---
title: Configuration Flags
section: reference
related:
  - running-the-demo
  - visualization-and-renderer
---

# Configuration Flags

## CLI flags

Parsed by `clap` in `src/main.rs`. All flags set the corresponding env var before calling `run_demo()`.

| Flag | Env var set | Default | Effect |
|---|---|---|---|
| `--inplace` / `-i` | `CRABSIM_LIVE_INPLACE=1` | off | Redraw state table in-place using ANSI |
| `--renderer-window` | `CRABSIM_RENDERER=1` | off | Spawn external renderer window |
| `--speed <f>` | `CRABSIM_RENDERER_SPEED=<f>` | `5.0` | Renderer pacing: sim-seconds per wall-second. `-1` = unpaced |
| `--hold <s>` | `CRABSIM_RENDERER_HOLD=<s>` | `1` | Seconds to hold renderer window open after simulation ends |
| `--renderer-launcher <l>` | `CRABSIM_RENDERER_LAUNCHER=<l>` | `powershell` (Windows) | Windows only: `powershell` or `cmd` |

## Environment variables

Can be set directly without using the CLI. `run_demo()` reads these after `clap` may have set them.

| Variable | Values | Effect |
|---|---|---|
| `CRABSIM_LIVE_INPLACE` | `1` (or any non-empty, non-`0` value) | Enable in-place terminal rendering |
| `CRABSIM_RENDERER` | `1` (or any non-empty, non-`0` value) | Spawn and connect external renderer |
| `CRABSIM_RENDERER_SPEED` | float string, e.g. `5` or `-1` | Renderer playback speed factor |
| `CRABSIM_RENDERER_HOLD` | integer string, seconds | Hold renderer open after last event |
| `CRABSIM_RENDERER_LAUNCHER` | `powershell` or `cmd` | Windows renderer launch method |
| `RUST_LOG` | `info`, `debug`, `crabsim=debug`, etc. | Controls `env_logger` output level |

## Env var precedence

CLI flags **set** env vars, so if both a CLI flag and env var are present, the CLI flag wins (it overwrites the env var before `run_demo()` reads it).

## Endtime

The simulation endtime is hardcoded in `run_demo()`:

```rust
EventController::new(Some(Duration::from_secs_f64(3_600.0)))
```

To change the simulated duration, edit this value in `src/lib.rs`. Tests each set their own endtime via `eventcontroller.endtime = Some(...)`.
