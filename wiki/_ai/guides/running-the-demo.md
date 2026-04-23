---
title: Running the Demo (AI context)
for: guides/running-the-demo.md
section: guides
---

## Commands
```bash
cargo run --bin crabsim                                          # plain
cargo run --bin crabsim -- --inplace                            # in-place ANSI
cargo run --bin crabsim -- --renderer-window --speed 5 --hold 1 # external renderer
RUST_LOG=info cargo run --bin crabsim                           # with logs
cargo test                                                       # run all tests
cargo test <name>                                               # single test
RUST_LOG=info cargo test -- --nocapture                         # tests with logs
```

## Env vars (alternative to CLI flags)
- `CRABSIM_LIVE_INPLACE=1` — in-place mode
- `CRABSIM_RENDERER=1` — external renderer
- `CRABSIM_RENDERER_SPEED=5` — speed factor (`-1` = unpaced)
- `CRABSIM_RENDERER_HOLD=1` — hold seconds after finish
- `RUST_LOG=info|debug|crabsim=debug` — log level

## Notes
- `env_logger::init()` is called inside `run_demo()` — RUST_LOG must be set before launch
- Endtime is hardcoded at 3600s in `run_demo()` — change in `src/lib.rs` to adjust
