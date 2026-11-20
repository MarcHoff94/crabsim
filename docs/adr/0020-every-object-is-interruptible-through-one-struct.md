# Every object is interruptible, through one struct and a thin trait

Every material-flow type can be interrupted: failed, stopped by hand or paused. The bookkeeping lives once, in a plain struct, **`Interruptions`**: which interruptions are active, the most serious one, the failure profiles and their clocks, `stop_count`, and when the interruption began. It is handed each stop event and says what changed: the object went down, came back up, its most serious cause changed, or nothing. Each type implements a thin trait, **`Interruptible`**, with `stop_began` and `stop_ended` saying what being interrupted means for it (a Station freezes its work, a Source moves its next creation later, a Buffer holds on to its MUs, a Drain refuses MUs), and one provided method that passes an event to `Interruptions` and calls them on the result. The cause is an **`Interruption`**: `Failed`, `Stopped` or `Paused`, from most to least serious. `StopCtrl`, which nothing called, goes, and so do `StopCause` and `InterruptionState`.

We decided it with the owner on 2026-11-20 in the design round for build step 7's second round (spec issue 122). The owner's intent for `StopCtrl` was one way to interrupt any object for any cause, a shift calendar's off-shift time included later, with several failure profiles per object.

## Considered Options

- **A trait with default methods doing the bookkeeping,** through an accessor to shared state in each type. One place for the rules, but the control flow bounces between the trait and the type, and the rules can only be tested through an object.
- **A thin trait over each type's own state.** Each of the four types would repeat the overlap rules that are tested only on Stations.
- **Keep the Station's own state and copy it.** The same repetition, and no common way in for a new cause.
- **A struct that keeps the bookkeeping and a thin trait tying it to each type,** chosen: the rules are written and tested once, and each type's code says plainly what an interruption means for it.

## Consequences

- Station and MultiStation move onto `Interruptions` first, with occurrences and state hashed identical before and after, so the move alone changes nothing.
- Source, Buffer and Drain grow by their `Interruptions`, about 24 bytes with no profiles, since the profile list allocates nothing when empty.
- Off shift, when a shift calendar is designed, is another cause in `Interruption`, not a change to four types.
- The glossary's names follow: the combined state is **Interrupted** (it was *Stop*) and the cause a person sets by hand is **Stopped** (it was *forced failure*). Operations, the results store and the window use the same words, the store's columns renamed by a version bump.
- Adding a material-flow type means implementing `Interruptible`, in place of `StopCtrl`.
