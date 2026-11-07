---
title: Configuration Flags (AI context)
for: reference/configuration-flags.md
section: reference
---

## Facts
- CLI (`src/main.rs`): `--seed <N>` (default 0), `--length <TIME>` (default `1d`, UI time syntax)
- `RUST_LOG` controls `env_logger`; per-event `info` logging is slow
- Profiles: `dev` (deps optimised), `release` (thin LTO, 1 codegen unit, `panic = unwind` for `LiveSession`), `ui` (release without LTO)
- Rust version pinned in `rust-toolchain.toml`
- `SessionSettings`: `buffer_instants` 100,000; `checkpoint_spacing` 100,000 events; `checkpoint_budget_bytes` 256 MB; `time_mode` Continuous; `reset_keeps_changes` off
- A session starts halted at speed 1; the UI uses 60x

## Code locations
- `src/main.rs`, `src/live_session.rs`, `Cargo.toml`
