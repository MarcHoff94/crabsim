# Conveyors move at constant speed, and MU-driven trips are trapezoidal, without jerk

A conveyor-driven conveyor, such as a belt or roller conveyor, has no acceleration, deceleration or jerk: its drive runs at constant speed, and an MU on it moves at that speed or stands, switching between the two at once. An MU-driven trip, such as a shuttle's or a carrier's, accelerates, cruises and brakes with constant acceleration and deceleration up to a top speed: a trapezoidal profile, with no jerk. An optional start and stop time, added once per trip, stands for what the profile leaves out: jerk, brake release and positioning. We decided it with the owner on 2026-11-25, before the design round for increment 4 of conveyors (spec issue 133), because crabsim is a discrete-event simulator, not a physics engine, and both answers keep its motion plans cheap while losing less accuracy than the uncertainty in the data a model is built from.

## Considered Options

- **Acceleration and jerk on every conveyor,** as ADR 0022 first planned. A belt's drive keeps running while an MU on it is blocked, so only the MU's restart would be modelled, which at conveyor speeds takes a fraction of a second against process, switch and repair times of seconds to minutes. Its cost is in the engine: catch-up times between MUs become cubic or quartic equations, found by a root-finder at each event.
- **Jerk on MU-driven trips.** Jerk adds about acceleration divided by jerk to a trip from standstill to standstill, 0.2 to 0.4 s for a typical shuttle, about 4% of a 10 m trip and 2 to 5% of a cycle with loading and unloading. That is nearly the same time for any trip long enough to reach full acceleration, so a constant per trip captures it. A lone trip's time with jerk still has a closed form, but one MU following another on the same track would need a root-finder, and jerk is seldom known: datasheets give top speed and acceleration, and a jerk guessed is no better than a time fitted to measured trips.
- **A trapezoidal profile on MU-driven trips,** chosen: without it, a shuttle at 4 m/s and 2 m/s² would cross a 2 m trip in half the time it really takes, since it never reaches its top speed, and such vehicles are often a system's bottleneck.

## Consequences

- **A conveyor's motion plans stay linear in time,** as they are now: an MU's position is its front plus its speed times the time since, and every catch-up is a division. The root-finder ADR 0022 allowed for isn't needed for conveyors.
- **An MU-driven trip's time is a closed formula,** worked out once when the trip starts: a triangle when the trip is too short to reach top speed, a trapezoid otherwise, plus the start and stop time. One MU following another needs at most a quadratic, solved exactly.
- **A restart delay on a conveyor** (an MU that stood waiting a fixed time before it moves again) is noted for later, as a constant like the gap, if a model of zero-pressure accumulation at high rates needs it. Nothing is built for it now.
- **Exact jerk-limited trips** for a lone MU could be added later as a closed form, without changing how MUs follow each other.
- ADR 0022 and the glossary's Drive entry are amended to match.
