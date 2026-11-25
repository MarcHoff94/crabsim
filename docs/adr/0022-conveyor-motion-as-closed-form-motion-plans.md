# Conveyor motion as closed-form motion plans, never time steps

Amended on 2026-11-25 (ADR 0028): a conveyor-driven conveyor has no acceleration, deceleration or jerk, so its MUs' plans stay phases of constant speed, and an MU-driven trip is trapezoidal, constant acceleration up to a top speed and constant deceleration, without jerk, so no phase of constant jerk and no root-finder are needed.

An MU on a conveyor moves by a motion plan: its object, the time it starts, where it starts along the path, and its phases of constant jerk, acceleration or speed up to a time. The engine schedules events only where MUs interact: an MU's front reaching the end, an MU closing up to the one ahead, a phase changing, an interruption beginning or ending. When something changes, such as an MU entering or the end being blocked, only the plans that change are computed again, which is the MUs behind the one that changed, since a change never reaches those ahead. There is no time step anywhere. Where a jerk-limited catch-up time has no closed form, it is found by a numeric root-finder at the event, to the millisecond, never by stepping. We decided it on 2026-11-21 in the design round for conveyors, since performance is the first pillar: a ticking conveyor would make a long run around a thousand times slower, and results that depended on a step size would weaken "the same seed gives the same result".

## Considered Options

- **A fixed time step,** moving every MU each tick. Easy to write and to extend with physics, but its cost grows with simulated time rather than with what happens, and every result would depend on the step chosen.
- **Constant speed only,** as a Buffer with a dwelling time. Cheap, but it can't model acceleration, jerk or MUs with their own drive, which the owner needs.

## Consequences

- **Rendering is decoupled from the engine.** Each new plan is an occurrence (`MotionPlanned` or similar), recorded like any other, which the Picture applies at its time, and a view works out an MU's position at shown time from it. A plan computed ahead of shown time only takes effect when the clock reaches it, so no separate notice of recomputing is needed, and nothing a view does can reach a result.
- **The full state holds every MU's current plan,** so a Picture can be rebuilt after Init, Reset or going back, and a checkpoint, being a clone of the run, holds them too.
- **Headless runs record no plans:** they pay only for the interaction events.
- **Stale events** are voided by a ticket on the event, as `MUExit` already does with `stop_count`, since the heap can't cancel events, so `ScheduledEvent` stays at 32 bytes.
- **The cost to measure:** each recompute walks the MUs behind a change, so a long accumulating conveyor costs time in the number of MUs it holds per change. The benchmark gets a conveyor scenario when the first conveyor is built, and this decision is looked at again if that cost shows up.
- **A plan can span two objects** while an MU straddles them (ADR 0023): at the slower of their two speeds, and, while either is interrupted, at the speed of the one it belongs to.
