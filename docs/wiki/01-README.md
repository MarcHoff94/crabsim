# CrabSim Wiki

Welcome to the CrabSim documentation! This wiki provides comprehensive guidance on understanding, using, and extending the discrete-event material-flow simulator.

## What is CrabSim?

CrabSim is a **compact Rust discrete-event material‑flow simulator** designed to model production systems where:
- **Moveable Units (MUs)** flow through a network of processing stations
- **Sources** create MUs at configured intervals
- **Stations** process MUs sequentially with configurable processing times
- **Drains** consume MUs and record exit metrics

The simulator uses a **priority queue‑based event controller** with FIFO tiebreaking to advance time deterministically and supports real‑time visualization.

## Quick Start (5 minutes)

### Building
```powershell
cargo build --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml
```

### Running the Demo
```powershell
# Basic demo (prints state snapshots)
$env:RUST_LOG = 'info'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim

# Live in-place terminal updates
$env:CRABSIM_LIVE_INPLACE = '1'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim

# External renderer window (5x speed)
$env:CRABSIM_RENDERER = '1'
$env:CRABSIM_RENDERER_SPEED = '5'
cargo run --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml --bin crabsim
```

### Running Tests
```powershell
# Run all tests
test cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml

# Run tests with log output
$env:RUST_LOG = 'info'
cargo test --manifest-path C:\Users\marcu\projects\crabsim\Cargo.toml -- --nocapture
```

## Key Features

- **Deterministic Simulation**: Events at the same time are resolved via FIFO sequence numbers
- **Blocked Retry Semantics**: When a station can't accept an MU, it retries when space frees up
- **Parallel Branches**: Support for complex network topologies with splits and merges
- **Real-time Visualization**: ASCII terminal output + optional external renderer window
- **Extensible Architecture**: Implement custom material‑flow types via traits
- **Comprehensive Logging**: Debug via `env_logger` with `RUST_LOG` environment variable

## Core Concepts at a Glance

| Concept | Purpose |
|---------|---------|
| **Simulator** | Registers all material-flow objects and manages state snapshots |
| **EventController** | Advances simulation time via priority queue; manages event scheduling |
| **Source** | Creates MUs at intervals; first object in the flow |
| **SingleStation** | Processes one MU at a time; core building block |
| **Drain** | Consumes MUs; records exit times for analysis |
| **MoveableUnit (MU)** | Entity flowing through the system; has id and location |
| **ProcessState** | Idle, Processing, or Blocked; affects entrance/exit availability |

## Wiki Structure

1. **README** (this file) - Overview and quick start
2. **[Core Architecture](02-ARCHITECTURE.md)** - System design and event flow
3. **[Module Reference](03-MODULES.md)** - API documentation for each module
4. **[Core Concepts](04-CORE-CONCEPTS.md)** - Deep dive into Simulator, EventController, and flow mechanics
5. **[Usage Guide](05-USAGE-GUIDE.md)** - How to build and run simulations
6. **[Extension Guide](06-EXTENSION-GUIDE.md)** - Creating custom material-flow types

## Common Tasks

- **Build a simple 3-station flow**: See [Usage Guide > Simple Example](05-USAGE-GUIDE.md#simple-example)
- **Understand blocked retry**: See [Core Concepts > Blocked Retry Semantics](04-CORE-CONCEPTS.md#blocked-retry-semantics)
- **Create a custom station type**: See [Extension Guide > Custom Material Flow](06-EXTENSION-GUIDE.md)
- **Analyze performance**: See [Usage Guide > Analyzing Results](05-USAGE-GUIDE.md#analyzing-results)

## Logging & Debugging

Set `RUST_LOG` to control verbosity:
```powershell
$env:RUST_LOG = 'info'              # Info level
$env:RUST_LOG = 'crabsim=debug'     # Debug for crabsim only
$env:RUST_LOG = 'crabsim=trace'     # Trace (very verbose)
```

## External Resources

- **Rust Book**: https://doc.rust-lang.org/book/
- **Project README**: See `README.md` in repository root
- **Copilot Instructions**: See `.github/copilot-instructions.md` for development conventions

---

Next: [Core Architecture →](02-ARCHITECTURE.md)