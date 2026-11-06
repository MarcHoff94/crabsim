# Statistics of material-flow objects

Agreed with the owner on 2026-11-06. **Not built.** Terms are defined in `GLOSSARY.md`. The reasoning behind the two hardest decisions is in ADR 0003 and ADR 0004.

## Principles

- Statistics are always on and fixed-size. Memory doesn't grow with simulated length, and they don't depend on occurrences.
- Statistics store raw whole-number values. Means, shares and MUs/h are methods, so nothing is calculated that nobody reads and same-seed runs can be compared with `==`.
- Everything of fixed size is read through `run.state()`, including after a run error.
- Per-MU data exists only through recording occurrences.

## What each object keeps

| | Source | Station | Buffer | Drain |
|---|---|---|---|---|
| MUs in / out | created / out | in / out | in / out | in / – |
| Busy time | waiting out its interval | working on at least one MU | holding at least one MU | – |
| Blocked time | yes | yes | yes | – |
| Idle time | – (it always holds an MU) | yes | yes | yes |
| Failed time, paused time | yes | yes | yes | yes |
| Number of failures, pauses, stops | yes | yes | yes | yes |
| Max contents, MU time | – | yes | yes | – |
| Time full | – | yes | yes | – |
| Lead time, exit intervals | – | – | – | yes |

Fields that don't apply to a kind stay at zero.

### Machine level: one state at a time

Each moment belongs to exactly one state, so the buckets add up exactly to the time since `statistics_since`. Overlaps are settled by priority, highest first:

- **Station and Source:** failed, paused, busy, blocked, idle.
- **Buffer:** failed, paused, blocked, busy, idle.
- **Drain:** failed, paused, idle.

Failed wins over paused, so failed time always holds the whole repair. A failure from t=20 to 30 and a pause from t=25 to 35 give failed 10s, paused 5s, and one stop from 20 to 35. Stopped time is failed plus paused and isn't stored.

Counts stay separate because a stop can't be worked out from the others: that example is one failure, one pause and one stop.

A Station with several MUs is busy while at least one is in work, which matches the Processing time basis. It's blocked when none is in work and a finished MU is waiting. It's idle otherwise, including a partly filled batch waiting to start.

### MU level: what each MU is doing

Machine-level time hides what's happening inside a multi-MU object, so utilisation and bottlenecks are judged from MU time. Each MU is in one state at any moment:

| Field | MU state | Station | Buffer |
|---|---|---|---|
| `mu_time_busy` | Being handled | In work | Dwelling |
| `mu_time_blocked` | Ready, can't leave | Yes | Yes |
| `mu_time_waiting` | Waiting to start | Partly filled batch | – |
| `mu_time_stopped` | Object is stopped | Yes (stop wins) | Yes (stop wins) |

- Average contents is the sum of the four, divided by measured time.
- Empty share is capacity × measured time, minus the four. So `ObjectState` also reports capacity.
- With capacity 1, both levels show the same picture.

Example: a capacity 2 Station over 100s has one slot working throughout, and the other holding a blocked finished MU for 60s and empty for 40s. Machine level says busy 100%, blocked 0%. MU level says busy 50%, blocked 30%, empty 20%.

`time_full` is the time spent at capacity. It describes how full the object is rather than what it's doing, so it isn't one of the state buckets and doesn't add up with them.

### Finding a bottleneck

No bottleneck figure is stored. Walk along the flow and compare blocked and starved shares. The bottleneck is where upstream objects are mostly blocked or full, and downstream objects are mostly starved.

- **Shifting bottlenecks:** read `state()` at two times and subtract, which gives the numbers for that period at no extra cost.
- **Branching models:** the comparison is harder to read there than on a line. A bottleneck report for those can come later and use the same numbers.

### Lead time

From the MU leaving its Source until it enters a Drain. A starting MU counts from Init (t=0). MUs still in the model at the end aren't counted, which the MUs-in and MUs-out counts make visible.

A Source creates an MU at the start of its interval, so counting from creation would add one interval to every lead time. Time waiting in a blocked Source shows up as the Source's blocked time instead.

`MoveableUnit` needs a new field for the time it left the Source.

Each Drain always keeps min, mean and max, stored as total, min and max, with the count being MUs in. Standard deviation isn't kept, because comparing seeds gives a better measure of spread.

**Percentiles:** exact ones are available from recording. If very long runs need them, add a log-scale histogram as an opt-in Drain setting: a few KB per Drain, within about 2%. It's stored on `ObjectState` only if asked for, since `state()` is copied after every slice of a live session. Not to be built until needed.

### Exit intervals and MUs/h

A Drain keeps its first and last exit time, and shortest and longest exit interval.

**MUs/h = (exits − 1) / (now − first exit) × 3600s**

- It runs up to the current time, so the rate falls while no MUs arrive and a jammed line can't look healthy.
- Exits at 70, 130 and 190 give 60 MUs/h at t=190, 55 at t=200, 40 at t=250 and about 2 at t=3,600.
- The rate dips between exits and recovers at each one. The effect shrinks as exits accumulate.
- Start-up before the first exit is left out.
- There's no value before two exits.
- The longest interval includes the one still in progress when read, so a jam shows there too. The shortest doesn't, because an unfinished interval could still grow.

### Achieved availability

**up / (up + failed)**, where "up" is the time counted on the time basis while not failed:

| Time basis | Up | Over 1000s: failed 100s, paused 200s, no overlap |
|---|---|---|
| Simulation time | Total − failed | 900 / 1000 = 90% |
| Working time | Total − failed − paused | 700 / 800 = 87.5% |
| Processing time | Busy time | With busy 400s: 400 / 500 = 80% |

The configured value is set as MTBF / (MTBF + MTTR), so this converges to it on every basis, which tests can check. Time bases aren't built, so only the Simulation time row applies at first.

## Warm-up

There's no warm-up setting inside the engine. The caller does `advance_to(warm_up)`, then `run.clear_statistics()`, then carries on.

- It costs nothing if never called, and can't affect random numbers or event order.
- It applies to the whole run. `RunState` gains `statistics_since`.
- It can become a live session command, and a headless entry point can accept a `warm_up` value that does these same steps.
- MUs already inside when statistics are cleared count towards MUs out and lead time when they leave, but not towards MUs in. So MUs in minus MUs out no longer equals contents after a warm-up.
- An MU's lead time is counted in full if it exits after the warm-up, even if it entered before.
- Clearing forgets the first and last exit, so no exit interval spans the warm-up and MUs/h runs from the first exit after it.

## Per-MU data

`exits` and `exit_ids` are removed from the Drain. The run gets a setting for which occurrence kinds to record: none, all or chosen kinds. A live session records everything, and a headless run records nothing by default. `MuLeft` gains `lead_time`.

Recorded occurrences come back from each advance, and `AdvanceFailure` carries them through an error. See ADR 0004.

The uses of `exits` and `exit_ids` in `tests/simulation_run.rs` and `src/demo.rs` move to a helper that reads `MuLeft`.

## On `ObjectState`

`mus_left` is replaced by `statistics`. `ObjectState` also gains the object's capacity.

```rust
pub struct ObjectStatistics {
    pub mus_in: u64,               // Source: created. A starting MU counts at Init.
    pub mus_out: u64,
    pub time_busy: Duration,
    pub time_blocked: Duration,
    pub time_idle: Duration,
    pub time_failed: Duration,
    pub time_paused: Duration,
    pub time_full: Duration,
    pub failures: u64,
    pub pauses: u64,
    pub stops: u64,
    pub max_contents: u64,
    pub mu_time_busy: Duration,
    pub mu_time_blocked: Duration,
    pub mu_time_waiting: Duration,
    pub mu_time_stopped: Duration,
    pub drain: Option<DrainStatistics>,
}

pub struct DrainStatistics {
    pub lead_time_total: Duration, // count is mus_in
    pub lead_time_min: Option<Duration>,
    pub lead_time_max: Option<Duration>,
    pub first_exit: Option<Duration>,
    pub last_exit: Option<Duration>,
    pub exit_interval_min: Option<Duration>,
    pub exit_interval_max: Option<Duration>,
}
```

The struct is `Copy` and needs no allocation. Methods that depend on the run's time, such as MUs/h, take it as a parameter.

## Init, Reset and run errors

- **Init:** measuring starts at t=0, with `statistics_since` = 0. Starting MUs count towards MUs in. Before Init, everything is zero.
- **Reset:** clears all statistics and `statistics_since`. Extend the reset tests in `tests/simulation_run.rs`.
- **Reading:** statistics include the state in progress at the run's current time, without changing anything. `report(&self, now)` already takes that time. So the numbers at a given time are the same however the advance was split up.
- **After a run error:** the numbers run up to the clock's time at the error, including any partial effect of the failing event, like the rest of `state()`.

## Building it

State changes are currently scattered: some go through `set_state`, others assign `self.state` directly. All of them need to go through one place that closes the time interval, or the buckets will drift.

Failure statistics can be built now. Pause, overlapping interruptions, capacity, dwelling time and the other time bases aren't built, so their numbers arrive with the domain work in step 7 of the build order.

## Fit with the rest of the agreed design

This design was written alongside ADR 0002 rather than after it.

- **Time type:** `Duration` above stands for simulation time. It becomes the millisecond time type from step 3 of the build order.
- **Recording:** the setting has to be changeable at any time, because replay turns it off and then back on.
- **Checkpoints:** statistics are part of an object's state, so save and restore cover them, and the restore-and-replay test should compare them too.
- **ADR 0004 after step 6:** a read-ahead live session stops sending state after every slice, which removes one of that ADR's reasons. The decision still stands, because a list on the Drain would also be copied into every checkpoint.

### Open questions for step 6

1. **Live statistics at shown time.** In a read-ahead session `run.state()` describes computed time, so its statistics are ahead of what's on screen. Occurrences must be enough to rebuild the state, but rebuilding statistics from them would duplicate the engine's logic in the UI. How does a UI show statistics for shown time?
2. **Warm-up and going back.** If a checkpoint from before `clear_statistics()` is restored, replaying past that point wouldn't clear them again unless the call is remembered, as interventions are. How should it be remembered, and what happens when going back behind it?

## Tests

Through `SimulationRun`:

- The machine-level buckets add up to the time since `statistics_since`.
- The same seed gives identical statistics.
- The numbers don't change when an advance is split into slices.
- Average contents = throughput × lead time (Little's law), on a model at steady state.
- Achieved availability converges to the configured value.
- Reset and `clear_statistics` clear what they should, and nothing else.
