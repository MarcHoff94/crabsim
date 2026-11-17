# A Buffer may have no limit, the one case where a run's memory isn't flat

A Buffer's capacity is a whole number from 1 or unlimited, and a new Buffer is unlimited. An unlimited Buffer takes every MU it's offered and grows as far as it's filled, 24 bytes for each MU it holds, and up to as much again in spare room, which it keeps until Reset (201 MB at 4.3M MUs, measured on 2026-11-17). It is the one case where a run's memory can grow with simulated length without the user having asked for recording, and it is the user's choice.

In a model file it is written `"capacity": "unlimited"`. `-1` is read as unlimited wherever a capacity is entered (a file written by hand, `edit_model`, the `:` line and the inspector) and always saved as `"unlimited"`.

We decided it on 2026-11-15 in the design round for build step 7 (spec issue 110). The owner wants unlimited knowingly: sketching a line shouldn't need a number for every Buffer, and a Buffer that never fills is often what is meant.

## Considered Options

- **A capacity is always required,** as before. Safe for memory, but every sketch needs numbers nobody has yet, and a large number typed to mean "enough" hides the same growth.
- **A hard limit on unlimited,** such as an error at a million MUs. It would stop a runaway model, but at a number nobody chose, in the middle of a long run.
- **Unlimited with warnings**, chosen.

## Consequences

- Memory stays flat as simulated length grows only when every unlimited Buffer empties at least as fast as it fills. The performance pillar's wording in `CLAUDE.md` holds with that exception.
- `validate_model`, and the window's Validate button, warn about an unlimited Buffer that can get MUs faster than it can pass them on, judged from the intervals of the Sources and the process times of the Stations directly before and after it. It is a rough rule: anything else beside it, such as another Buffer or a Drain, and it says nothing, and failures aren't counted.
- A Buffer's highest number of MUs shows in the inspector, so growth can be seen in a run.
- A Source with an interval of 0 that reaches an unlimited Buffer through nothing but zero-time objects is a problem in the model file, since it would create MUs without end at one instant.
- A checkpoint is a clone of the run, so it holds every MU in every Buffer. A live session counts them in a checkpoint's size against its memory budget.
- An unlimited Buffer is never full, so it has no full share, and it has no number to be weighed by in a weighted distribution or a route.
