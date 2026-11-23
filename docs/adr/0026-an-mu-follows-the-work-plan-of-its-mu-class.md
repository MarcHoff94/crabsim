# An MU follows the work plan of its MU class

Each MU class has a work plan: a list of steps, each a destination (ADR 0025) and optionally a process time, which a Source may replace for its own MUs as it may an MU class's other values. An MU's next destination is its plan's current step. Entering a Station that is the step's instance, or one of its class, the MU is worked on for the step's process time, or the Station's own without one, and its step moves on when it leaves; reaching any other object the step names, such as a Drain, moves it on too. At a Station its step doesn't name, the MU is worked on for the Station's own process time and keeps its step. A Station may also have a **detour**: a destination its MUs go to before carrying on with their plan, such as Rework, cleared when reached. A plain destination is a plan of one step, so one idea covers both. We decided it on 2026-11-23 in the design round for cross conveyors (spec issue 148), on the owner's proposal, because products in real lines follow routings of operations, and a plan on the MU class lets the model say so without a routing object per product.

## Considered Options

- **A destination alone,** set by a Source and changed by Stations as MUs leave. Simpler, but a routing of several operations needs a Station per change, and process times per product can't be said at all.
- **A plan per MU, stored with it.** Any plan for any MU, such as one changed mid-run, but an MU would carry a list, against ADR 0004, and memory would grow with MUs.
- **A detour that replaces the rest of the plan.** Rework would then need its own plan to rejoin the line.

## Consequences

- **An MU carries only where it is in its plan:** its step, a place in one table of every plan's steps for the run, and its detour, each 16 bits (ADR 0027). Its plan comes from its kind.
- **Off plan:** a Station counts the MUs it worked on for their step and those it didn't match, so MUs reaching a Station their plan doesn't name show in its statistics.
- **Per-step lead times** would need a list per MU (ADR 0004), so they are not measured.
- **Up to 65,535 steps** across all plans, and 65,535 destinations, are checked in the model file.
