# An MU packs its departure time and kind into one word

An MU keeps when it left its Source as 48 bits of milliseconds and its kind (ADR 0021) as 16 bits in one 64-bit word, so that its plan step and its detour (ADR 0026) fit in 16 bits each and an MU stays 16 bytes: its id, the packed word, its step and its detour. 48 bits of milliseconds reach about 8,900 years of simulated time, and a run is refused at that length. We decided it on 2026-11-23 in the design round for cross conveyors (spec issue 148), because adding the step and detour as fields of their own would make an MU 24 bytes, half again the memory for every MU a Buffer holds, and steps and detours of 8 bits each would allow too few steps for real routings.

## Considered Options

- **Fields of their own,** 24 bytes an MU: no packing, but 50% more memory per MU held.
- **8 bits each for step and detour:** 16 bytes without packing, but at most 255 steps and destinations, which the owner judged too few.
- **A table of detours on the run, by MU id:** no per-MU cost for the rare detour, but objects own their MUs, and a shared table would break ADR 0005's single ownership.

## Consequences

- **A shift and a mask** wherever the departure time or the kind is read: at a Drain, for lead time, and on conveyors, for an MU's footprint. The benchmark is measured before and after the change, on every scenario.
- **Simulated time past about 8,900 years** is refused when a run is asked to advance there, with a run error saying why.
- `MoveableUnit` keeps its size test, now with the step and detour in it.
