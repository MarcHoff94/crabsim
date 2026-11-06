# Restore and replay prototype

Step 5 of the build order, measured on 2026-11-06 on the owner's PC: Ryzen 5 7600X3D, 31 GB, Windows 11, Rust 1.91.1. The code is throwaway and lives on the branch [`prototype/restore-replay`](https://github.com/MarcHoff94/crabsim/tree/prototype/restore-replay), never to be merged.

**Result:** the mechanism in ADR 0002 works, and it costs less than estimated. Step 6 can go ahead. A checkpoint is a clone of the run.

## Is it exact?

- **Going back:** 9 runs across 7 seeds, on the demo model and a 2,800-object model, went back to 8 times each. The state there, every occurrence from there on and the final state matched a run that never went back, statistics included. A checkpoint landing part way through an instant made no difference, and in the densest run about a third of them did.
- **Interventions:** in 4 scenarios, a session computed ahead, intervened at earlier times, went back behind interventions and started new futures. Each result matched a fresh run with the same seed and log.

## What it costs

| | Demo, 14 objects | 2,800 objects | ADR 0002 estimate |
|---|---|---|---|
| One checkpoint | 17.1 KB | 3.4 MB | 10 to 20 KB for the demo |
| Taking a checkpoint that is kept | 5.4 µs | About 600 µs | |
| Going back with a replay of 1,000 events | 0.08 ms | 0.74 ms | 0.2 ms |
| Going back with a replay of 100,000 events | 7.3 ms | 13.3 ms | |
| Going back with a replay of 1,000,000 events | 74 ms | 126 ms | |

Going back is one restore plus the replay, so short replays on large models are mostly the restore. An entry takes about 1,200 bytes per object, mostly random number generator state and statistics.

| A checkpoint every | Run at full speed, demo | Run at full speed, 2,800 objects | Reach of 1 GB, 2,800 objects |
|---|---|---|---|
| 1,000,000 events | No slower | 0.6% slower | 34.6 simulated days |
| 100,000 events | 1.8% slower | 6.5% slower | 3.5 simulated days |
| 10,000 events | 3.5% slower | 60.6% slower | About 8 simulated hours, worked out from the row above |

- **The demo's slowdowns** are about the size of the benchmark's noise, so read them as small rather than exact.
- **A checkpoint every 100,000 events** keeps going back within a frame at 60 fps on both models, so it is the place to start.
- **Memory reach only limits large models.** On the demo, 100 MB reaches back about 38 simulated years at that spacing.
- **A headless run pays nothing,** because it takes no checkpoints.

## What it changes in the design

1. **Save and restore are a clone.** Since the storage change, `#[derive(Clone)]` on the run is all it took, with no code per object, and a new field can't be forgotten. User code has to be cloneable.
2. **A checkpoint records how much of the log is in it.** Time alone can't say, because a checkpoint can land at the very time of an intervention, on either side of it.
3. **Replay applies the logged actions it passes,** at their times. Without this, going back to a time after an intervention, from a checkpoint before it, would lose the intervention.
4. **Going back drops what it abandons:** logged actions after that time, and every checkpoint from that future.
5. **`clear_statistics()` can be logged like an intervention.** That gave exact results, including when going back behind it.

## Not covered

- **Pause isn't built,** so `StartFailure` stood in for it. It takes effect at once like a Pause, but it also draws random numbers.
- **The read-ahead buffer, the session's thread and rebuilding the picture from occurrences** weren't tested. The last is the biggest remaining risk in ADR 0002.
- **Live statistics at shown time** weren't tried.
- **Other machines:** this CPU has 96 MB of L3 cache, which may flatter the large model.
