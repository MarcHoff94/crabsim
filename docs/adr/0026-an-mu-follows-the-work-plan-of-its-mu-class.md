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

## Amended on 2026-11-28, in the design round on process times by MU (ADR 0030)

- **A step is known by what it names,** the class or instance in its `at`, and needs no name of its own: an expectation or a rule line lists steps as classes and instances (`Mill.step always in Welding, Deburr`), which `load` checks as it does every destination, a listed class standing for its derived classes and their instances. Considered and dropped: a step's place in its plan, which shifts when a step is inserted and would change what an accepted expectation means, and free names such as `OP20`, which nothing could check, against a plan changed during a run bringing steps no file names. Steps may still name any destination, Drains included, which routing by destination needs for a plan ending at a Drain.
- **An MU's current step** is what its detour names while it has one, from getting it until it reaches it, otherwise what its plan's current step names, and none without a plan or past its last step. So `Rework.step always in Rework` says that Rework only gets MUs going there, on plan or on a detour; telling the two apart is left to `mus_on_plan` and `mus_off_plan`.
- **The work plan stays as it is,** its steps' own times winning over a Station's time by MU (ADR 0030, amended).
- **Later, a design round of its own:** a detour that is a small work plan of several steps, slotted into an MU's plan during a run, such as an inspection sending a failed weld back to welding and inspection. Today's detour is one destination a Station sets on every MU it takes, so several stations in turn need each to have its own detour. The round settles what decides one (a rate, rule lines or code), where it is slotted in, how an MU carries it in its 16 bytes (ADR 0027), what a deterministic verification run does with a random branch, and what the bounds and the decision record make of it.
