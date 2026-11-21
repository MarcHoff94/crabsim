# First profile

Measured on 2026-11-06 for step 2 of the build order, on the owner's PC: Ryzen 5 7600X3D, 6 cores, 31 GB, Windows 11, Rust 1.91.1. Step 3 uses this to decide object storage. It describes the code as of this date, so re-measure before relying on it later.

**Result:** the per-event path spends most of its time on work it doesn't need. Removing four per-event costs in today's structure makes it 1.55 to 1.69 times faster. Going on to no recording and a smaller queue entry gives 2.1 to 2.4 times. One `Vec` of an enum on top of that gives 2.5 to 2.7 times.

## Baseline

`cargo bench --bench reference`, tuned release profile, seed 0, median of 7 runs:

| Scenario | Objects | Simulated | Events/s | Simulated/wall | Wall | Peak heap | Events |
|---|---|---|---|---|---|---|---|
| `demo-kept` | 14 | 1 year | 5.68M | 11.6M times | 2.73 s | 2,189 MB | 15,501,648 |
| `demo-dropped` | 14 | 1 year | 6.67M | 13.6M times | 2.32 s | 42 MB | 15,501,648 |
| `large-dropped` | 2,800 | 2 days | 3.56M | 36,000 times | 4.78 s | 102 MB | 16,998,832 |

- **The scenarios:** `kept` is one advance that returns every occurrence, as a run does today. `dropped` advances an hour at a time and drops what comes back, the nearest thing to a headless run until step 4. The large model is 200 copies of the demo line, because the target scale is a few thousand objects.
- **Every event costs** 2.38 allocations and records 2.46 occurrences, and a queue entry is 64 bytes.
- **Peak heap** comes from a counting allocator and agrees with the OS's peak working set: 2,180 MB, 42 MB and 109 MB. `demo-dropped` still grows with simulated length, because the Drain lists every exit. Step 4 removes that.
- **Noise:** runs of one build differ by about 2%, with occasional slow stretches of 10 to 30% when something else is running. Compare medians from fresh runs, and be suspicious of a difference under about 3%.

### After removing the `order` `Vec`

Everything else in this document, apart from the three sections after this one, describes the code before this change. Before and after were each run 7 times in rotation, as a fresh process each time, and the events and MUs left didn't change.

| Scenario | Events/s before | Events/s after | Faster by | Wall after | Allocations per event |
|---|---|---|---|---|---|
| `demo-kept` | 5.70M | 6.43M | 1.13 times | 2.41 s | 2.38 to 1.76 |
| `demo-dropped` | 6.70M | 7.71M | 1.15 times | 2.01 s | 2.38 to 1.77 |
| `large-dropped` | 3.62M | 3.89M | 1.07 times | 4.37 s | 2.38 to 1.77 |

Peak heap didn't change.

### After the storage change

Step 3 moved objects into one `Vec` of an enum with successors as ids, removed the per-move list clone, and shrank a queue entry from 64 to 32 bytes. Three builds were each run 7 times in rotation, as a fresh process each time, and these are medians. It is safe code, so this is the real cost of what the prototype behind D only estimated.

| Scenario | Before | Storage change | With rounded times | Faster by | Peak heap | Allocations per event |
|---|---|---|---|---|---|---|
| `demo-kept` | 2.40 s, 6.46M/s | 1.54 s, 10.06M/s | 1.52 s, 10.20M/s | 1.58 times | 2,189 to 1,636 MB | 1.76 to 0.86 |
| `demo-dropped` | 2.11 s, 7.36M/s | 1.21 s, 12.85M/s | 1.21 s, 12.83M/s | 1.74 times | 42 to 25 MB | 1.77 to 0.87 |
| `large-dropped` | 5.53 s, 3.07M/s | 2.29 s, 7.41M/s | 2.29 s, 7.43M/s | 1.9 to 2.4 times | 102 to 67 MB | 1.77 to 0.86 |

- **Same behaviour:** the storage change alone gives the same events and MUs left as before. A fingerprint of every occurrence and the final state also matched `master` on 5 demo seeds over 60 days, a Buffer model and a model with starting MUs and heavy blocking.
- **Rounded times change results slightly, as agreed:** `demo-dropped` goes from 15,501,648 to 15,501,859 events and from 1,598,620 to 1,598,659 MUs left. They make no measurable difference to speed.
- **`large-dropped` before was unsteady:** its runs ranged from 4.39 s to 5.56 s, with a median of 5.53 s, and the earlier baseline was 4.37 s. Hence the range against 2.29 s. The new build stayed within 2.26 to 2.29 s.
- **Occurrences got smaller,** because a time is now 8 bytes rather than 16. That is the drop in `demo-kept`'s peak.
- **What's left:** the new-events `Vec` is the likely source of most remaining allocations, and recording is still always on.

### After the headless run and statistics

Step 4 made recording opt-in, removed the Drain's lists, and added always-on statistics and a `lead_time` on `MuLeft`. Medians of 7 runs in rotation, as a fresh process each time, with the events and MUs left unchanged.

| Scenario | Objects | Simulated | Events/s | Simulated/wall | Wall | Peak heap |
|---|---|---|---|---|---|---|
| `demo-headless` | 14 | 1 year | 13.20M | 26.9M times | 1.17 s | Under 0.05 MB |
| `demo-headless-10y` | 14 | 10 years | 13.13M | 26.7M times | 11.82 s | Under 0.05 MB |
| `large-headless` | 2,800 | 2 days | 7.52M | 76,000 times | 2.26 s | 8.1 MB |
| `demo-dropped` | 14 | 1 year | 11.11M | 22.6M times | 1.40 s | 0.3 MB |
| `demo-kept` | 14 | 1 year | 8.97M | 18.2M times | 1.73 s | 2,148 MB |
| `large-dropped` | 2,800 | 2 days | 6.34M | 64,000 times | 2.68 s | 37 MB |

**Memory is flat:** the benchmark reports heap to 0.1 MB, and a headless demo run rounds to zero over both 1 and 10 years. `cargo test --all-targets` checks this more finely over 5 and 20 days, which gave exactly 35,320 bytes each.

**Against the code before step 4:** a headless run is about 1.04 times as fast as the old nearest thing, one-hour slices that recorded and dropped everything (1.22 s to 1.17 s), and its peak heap goes from 25 MB to under 0.05 MB on the demo and from 67 MB to 8 MB on the large model. Turning recording off gained about what the profile predicted, and statistics spent most of it. A run that records all of it got slower: `demo-kept` from 1.54 s to 1.73 s, and `demo-dropped` from 1.22 s to between 1.28 and 1.40 s across two rotations.

**What each of the two additions costs.** Throwaway builds with one thing removed, 7 rounds in rotation:

| Scenario | As built | Without statistics | Without `lead_time` |
|---|---|---|---|
| `demo-headless` | 1.19 s | 1.11 s, 7% less | 1.18 s, no difference |
| `large-headless` | 2.22 s | 1.99 s, 10% less | 2.22 s, no difference |
| `demo-dropped` | 1.28 s | 1.22 s, 5% less | 1.28 s, no difference |
| `large-dropped` | 2.64 s | 2.40 s, 9% less | 2.54 s, 4% less |
| `demo-kept` | 1.73 s, 2,148 MB | 1.65 s, 5% less | 1.59 s, 8% less, 1,611 MB |

- **Statistics** cost about 7 to 12% of a headless run's time, counting against the build without them. No attempt has been made to reduce this yet. They add 264 bytes to each object, taking an entry to 1,168 bytes, and that includes a Drain's fields on every Station, and most events change an object's state and so update its totals.
- **`lead_time`** costs a headless run nothing. It takes an occurrence from 24 to 32 bytes, so recording everything costs up to 9% more time. The 33% in peak memory is that growth landing on a `Vec` that doubles: 38M occurrences take 1.2 GB where they took 0.9 GB.

### After the read-ahead live session

Step 6 rewrote the live session, added Pause on Stations and made a run cloneable. Medians of 7 runs in rotation, as a fresh process each time, with the events and MUs left unchanged.

| Scenario | Before | After | Peak heap |
|---|---|---|---|
| `demo-headless` | 1.17 s, 13.28M/s | 1.20 s, 12.93M/s | Under 0.05 MB |
| `large-headless` | 2.23 s, 7.61M/s | 2.23 s, 7.62M/s | 8.2 MB |
| `demo-kept` | 1.69 s, 9.17M/s | 1.75 s, 8.85M/s | 2,148 MB |

The headless differences are within the benchmark's noise. None of the session runs without a session. `demo-kept` is about 3.5% slower, just past the noise, and nothing in the engine obviously explains it.

**A live session at full speed.** A throwaway program started a session at a speed too high to matter and read every observation to the end. These are single runs, so treat them as rough.

| Model | Simulated | Before | After | Peak heap after |
|---|---|---|---|---|
| Demo | 30 days | 0.21 s | 0.31 s | 1 MB |
| 2,800 objects | 6 hours | 0.94 s | 0.52 s | 79 MB |
| 2,800 objects, past the 256 MB budget | 3 days | | 6.40 s | 251 MB |

The large model got faster because a session no longer reports a full state after every slice. The small one got slower, because buffering per instant and taking checkpoints outweigh that there.

**Going back in a real session,** with the default settings. These are single measurements.

| Going back from the end | Demo, 30 days | 2,800 objects, 6 hours | 2,800 objects, 3 days, past the budget |
|---|---|---|---|
| A state on request | 7.4 ms | 6.1 ms | 2.0 ms |
| 1 second | 7.4 ms | 5.7 ms | 1.5 ms |
| 1 more hour | 7.0 ms | 13.4 ms | 8.9 ms |
| 1 more day | 3.1 ms | | 139 ms |
| To near the start | 5.9 ms | 12.5 ms | 386 ms |

So recent history stays within a frame even past the budget, and thinning makes only far jumps slower, as intended.

### After the Rust update and the 6,000-object model

This is the current baseline. Measured on 2026-11-07, for step 1 of the UI's order of work. Rust went from 1.91.1 to 1.99.1, now pinned in `rust-toolchain.toml`, and the large model went from 200 to 429 copies of the demo line, from 2,800 to 6,006 objects, because the target scale is now up to 6,000 objects.

**The compiler alone,** before the large model changed. One benchmark run each, with 5 repetitions per scenario, so treat small differences as noise:

| Scenario | Rust 1.91.1 | Rust 1.99.1 |
|---|---|---|
| `demo-headless` | 1.21 s, 12.77M/s | 1.19 s, 13.02M/s |
| `demo-headless-10y` | 12.12 s, 12.81M/s | 12.04 s, 12.89M/s |
| `large-headless`, 2,800 objects | 2.25 s, 7.54M/s | 2.26 s, 7.52M/s |
| `demo-kept` | 1.83 s, 8.49M/s | 1.71 s, 9.08M/s |
| `demo-dropped` | 1.28 s, 12.14M/s | 1.27 s, 12.18M/s |
| `large-dropped`, 2,800 objects | 2.74 s, 6.20M/s | 2.67 s, 6.38M/s |

Every scenario gave the same events and MUs left on both compilers, so the update changed no result. Only `demo-kept` moved past the noise, about 7% faster. The flat-memory check in `cargo test --all-targets` now reads 35,800 bytes over both 5 and 20 days, where it read 35,320 on 1.91.1, so it is still flat.

**The new baseline,** on Rust 1.99.1. One benchmark run with 7 repetitions per scenario, medians:

| Scenario | Objects | Simulated | Events/s | Simulated/wall | Wall | Peak heap | Events |
|---|---|---|---|---|---|---|---|
| `demo-headless` | 14 | 1 year | 13.23M | 26.9M times | 1.17 s | Under 0.05 MB | 15,501,859 |
| `demo-headless-10y` | 14 | 10 years | 13.19M | 26.8M times | 11.77 s | Under 0.05 MB | 155,213,269 |
| `large-headless` | 6,006 | 2 days | 6.75M | 32,000 times | 5.41 s | 16.9 MB | 36,488,865 |
| `demo-kept` | 14 | 1 year | 9.15M | 18.6M times | 1.69 s | 2,148 MB | 15,501,859 |
| `demo-dropped` | 14 | 1 year | 11.96M | 24.3M times | 1.30 s | 0.3 MB | 15,501,859 |
| `large-dropped` | 6,006 | 2 days | 5.63M | 27,000 times | 6.48 s | 75 MB | 36,488,865 |

- **The large model is slower per event than at 2,800 objects:** 6.75M against 7.52M events/s headless, about 10% less. Its objects take about 7 MB at 1,168 bytes each, which still fits in this CPU's 96 MB of L3 cache, so the cause isn't known. It wasn't profiled.
- **A simulated year of the large model** would take about 16 minutes headless, from 36.5M events per 2 simulated days.
- **Not measured here:** a live session at 6,000 objects. The UI prototype measures that, on a real clock.

### After Buffer and waiting by MU

Measured on 2026-11-17 on the first PC (Ryzen 5 7600X3D, 96 MB of L3, a desktop on mains), after PR 111 merged Buffer and waiting by MU (ADR 0018, ADR 0019). PR 111 itself was measured only on the second PC, a laptop with 12 MB of L3, where the version that merged was never timed on mains. The method, which suits any before and after: build the other commit's benchmark in a `git worktree` with its own `CARGO_TARGET_DIR` (`cargo bench --bench reference --no-run`), copy both `reference-*.exe` binaries aside, run each as `--bench --reps 1 --scenario <name>`, alternating the two for ten rounds and swapping which goes first each round, then compare medians and count the rounds in which each was slower. A single run can be far off: the first `buffered-headless` here gave 8.6M events/s, and every later one 12 to 14M.

**PR 111 against the code before it** (`da3b522` against `a553f03`):

| Scenario | Before | After PR 111 | Change | After slower in |
|---|---|---|---|---|
| `demo-headless` | 13.22M/s | 12.85M/s | −2.8% | 9 of 10 |
| `large-headless` | 6.71M/s | 6.61M/s | −1.5% | 9 of 10 |
| `demo-dropped` | 11.52M/s | 11.41M/s | −0.9% | 6 of 10 |
| `large-dropped` | 5.75M/s | 5.58M/s | −2.9% | 8 of 10 |
| `demo-kept` | 9.01M/s | 8.98M/s | −0.3% | 8 of 10 |

Events, MUs left and peak heap were identical in every round. So the size fix in PR 111 worked as the second PC suggested (the large model is within ±2% here too, where its 7.7 MB of objects fit easily in L3), but the demo was still 2.8% slower, outside the spec's ±2%.

**Where the demo's loss came from: one allocation.** The handoff named two suspects, the `retried` mark keeping entries in lines and `offered_since` on every move, and a throwaway build without each settled it:

- **Without `offered_since`:** 12.71M against 12.32M in one paired run, within the noise. A Station's `offered_since` is trivially `None` after its MU is taken.
- **Emptying the line on retry, as before PR 111:** 14.31M/s and 0.62 allocations per event instead of 0.86, but different results, so not a fix in itself. The drop in allocations was the clue.
- **The cause:** `retries_for_blocked` made its `Vec` with `with_capacity(len)`, and a Station that frees up pushes its own exit onto the retries it gets back (`station.rs`, `be.push(event_template)`). An exactly full `Vec` reallocates on that push, so nearly every move to a Station with someone waiting cost an allocation and a copy. The code before PR 111 collected to an exact size too, so it paid the same, and the loss in PR 111 is the rest of the line's bookkeeping on top. `Vec::new()` makes room for four on the first push, and the Station's push fits.

**After the fix** (`Vec::new()` in `retries_for_blocked`), against the code before PR 111:

| Scenario | Before PR 111 | With the fix | Change | Fix slower in | Allocations per event |
|---|---|---|---|---|---|
| `demo-headless` | 12.89M/s | 14.57M/s | +13.0% | 0 of 10 | 0.86 to 0.62 |
| `large-headless` | 6.68M/s | 7.04M/s | +5.4% | 0 of 10 | 0.86 to 0.62 |
| `demo-dropped` | 11.23M/s | 13.11M/s | +16.6% | 0 of 10 | |
| `large-dropped` | 5.75M/s | 6.03M/s | +4.9% | 0 of 10 | |
| `demo-kept` | 8.93M/s | 9.63M/s | +8.0% | 0 of 10 | |
| `buffered-headless`, against PR 111 | 13.20M/s | 14.02M/s | +6.2% | 0 of 10 | 0.87 to 0.74 |

Every occurrence and the final state, of the demo and the buffered line over 20 simulated days on seeds 0, 7 and 123, hashed identical with and without the fix, and the events and MUs left matched in every round. The large model gains less because its time goes more to memory than to the allocator.

**The buffered line, counted.** A throwaway build counting events by object and kind, over one simulated year on seed 0:

| | Demo | Buffered line |
|---|---|---|
| Objects | 14 | 7 |
| Events | 15.5M | 39.0M |
| MUs left | 1.60M | 5.09M |
| Events per MU left | 9.7 | 7.7 |
| Busiest object | Station 1, 21% of events | Store, 26% of events |
| `relist_blocked` calls per event | 0.92, a third of them finding an entry | 0.78, a quarter finding one |
| Longest line | 3 | 1 |
| Inserted in the middle of a line | never | never |

- **A Buffer's exits that do nothing:** 4.35M a year, 11% of the buffered line's events, are `MUExit`s for a Buffer that is already blocked: Store sits full in front of Mill, its bottleneck, and every MU entering it schedules an exit at the end of its zero dwelling time that finds it waiting in Mill's line. Each is cheap: a pop from the queue, a match and a statistics update. Scheduling an exit only for the MU at the front, or none while blocked, would remove them, at the cost of more logic per entry. Not tried.
- **Store gets a retry for almost every MU** (4.9M a year), because Mill sends one each time it frees up. That is the movement working as designed, not waste.
- **`relist_blocked`** is a dispatch through `ModelObject` and a scan of a line of at most 3 for every successor on every move, mostly finding nothing. It was there before PR 111 as `remove_blocked`, so it isn't new cost.

**Buffer variants**, headless, one simulated year, median of 5, a throwaway program on `master` before the fix:

| Model | Events/s | Events per MU left | Peak heap |
|---|---|---|---|
| The buffered line (`buffered-headless`) | 12.65M | 7.66 | Under 0.05 MB |
| Cooling first in, first out | 13.30M | 7.66 | Under 0.05 MB |
| Cooling without its dwelling time | 14.03M | 7.66 | Under 0.05 MB |
| The same line without its Buffers | 15.58M | 5.12 | Under 0.05 MB |
| Source, Buffer of 1, Station, Drain | 16.96M | 3.00 | Under 0.05 MB |
| Buffer of 1,000 kept full, dwelling 1 h, FIFO | 13.07M | 5.00 | 0.1 MB |
| Unlimited, dwelling 1 h, about 3,600 held, FIFO | 11.80M | 3.00 | 0.2 MB |
| The same, LIFO | 9.59M | 3.00 | 0.2 MB |

- **Without its Buffers** the line passes 9% fewer MUs (4.61M against 5.09M), so the Buffers buy throughput with 2.5 more events per MU and 1.6 s more wall time per simulated year.
- **Last in, first out with a dwelling time** is the dearest case: with the newest MU still dwelling, `offered` does a binary search over everything held, and it is asked several times per move. At 3,600 MUs held that's 19% slower than first in, first out. In the buffered line, where Cooling holds about a dozen, it's 5%. Without a dwelling time the newest MU is always ready and no search happens.

**An unlimited Buffer filling**, a Source every second into an unlimited Buffer before a Station of 2 s, single runs:

| Simulated | MUs held at the end | Peak heap | Bytes per MU held |
|---|---|---|---|
| 1 day | 43,200 | 1.6 MB | 37 |
| 10 days | 432,000 | 12.6 MB | 29 |
| 100 days | 4,320,000 | 201 MB | 47 |

Speed doesn't change as it fills (17.6M events/s, LIFO 16.7M). Memory is 24 bytes per MU held plus up to as much again in spare room, since a `VecDeque` doubles as it grows, so between 24 and 48 bytes per MU. It never shrinks: a Buffer that once held a million MUs keeps their room until Reset, and a checkpoint's clone holds only what's in it.

**Object sizes** are as PR 111 left them: `ModelObject` 1,280 bytes, set by `SingleStation`, with `Buffer` 496, `Source` 448 and `Drain` 312.

### After Station capacity

Measured on 2026-11-17 on the first PC, for PR 2 of build step 7: the branch against `master` after PR 112, by the alternating method above, ten rounds.

| Scenario | `master` | With Station capacity | Change | Slower in |
|---|---|---|---|---|
| `demo-headless` | 14.61M/s | 14.49M/s | −0.9% | 7 of 10 |
| `large-headless` | 7.12M/s | 7.00M/s | −1.8% | 10 of 10 |
| `demo-dropped` | 13.15M/s | 13.42M/s | +2.1% | 3 of 10 |
| `large-dropped` | 6.11M/s | 6.04M/s | −1.1% | 8 of 10 |
| `demo-kept` | 9.75M/s | 9.73M/s | −0.2% | 5 of 10 |

- **Within the spec's ±2%,** with identical events and MUs left, and every occurrence of the demo and a split model with a Buffer hashed identical over 20 simulated days on three seeds.
- **Why so little changes:** a Station of capacity 1 stays a `SingleStation`, and every object stays 1,280 bytes. The statistics' two new fields would have added 16 bytes, which `SingleStation` won back by keeping the work left during a stop in its `Finish` field, and a `MultiStation` boxes its failure profile, so at 544 bytes it needs no tag of its own in `ModelObject`.
- **The 6,006-object model's −1.8% is steady** (slower in all ten rounds), so it is likely real: the extra `ModelObject` variant in every dispatch, or the statistics' two new buckets. Not measured apart.
- **`buffered-headless` changed model:** the line now has an Oven of capacity 6 with individual start. 44.1M events a year at 12.6M events/s, one more event per MU than without it, passing the same 5,087,928 MUs. With batch start the Oven passed only 3.2M: a batch closed until empty waits idle while it fills.

### After fill statistics

Measured on 2026-11-18 on the first PC, for PR 3 of build step 7, against `master` after PR 113, ten alternating rounds per set.

| Scenario | Change | Slower in |
|---|---|---|
| `demo-headless` | −0.7% | 8 of 10 |
| `large-headless` | +0.2% | 4 of 10 |
| `demo-kept` | +0.6% | 4 of 10 |
| `demo-dropped` | −2.4%, then −3.6% | 7, then 10 of 10 |
| `buffered-headless` | −1.1%, −2.3%, then −4.3% | 7, 9 and 9 of 10 |

- **The first version cost the buffered line 6.1%.** Its fill keeper kept its own clock, comparing and storing the MUs held on every Buffer event. A throwaway build keeping the fields but never calling it was within noise (−0.9%), so the size of a Buffer, 560 bytes more, costs nothing here. Feeding the histogram from the stretches the `StatisticsKeeper` already measures (`observe_stretch`), and finding a bar by a shift with a fast path for an automatic width, brought it down to the range above.
- **The noise was high:** `master` itself ran `buffered-headless` between 9.7M and 12.3M events/s in one set, and the same binary measured −1.1% and −4.3% in two sets.
- **`demo-dropped` runs none of the new code:** the demo has no Buffer, and no function on its path changed. Its loss is likely where the compiler placed code. Not proven with a control build.
- Events and MUs left were identical in every round, and every occurrence and object state of the demo and the buffered line hashed identical to `master` over 20 simulated days on three seeds.

## Release profile

Each build was run 12 times in rotation, as a fresh process each time. Clean build time includes dependencies.

| Profile | `demo-dropped` | `large-dropped` | Clean build |
|---|---|---|---|
| Default release | 2.59 s | 5.07 s | 10 s |
| Thin LTO, 1 codegen unit (**chosen**) | 2.30 s, 11% less | 4.73 s, 7% less | 13 s |
| Fat LTO, 1 codegen unit | 2.31 s | 4.76 s | 21 s |

Fat LTO gains nothing measurable here, so it isn't worth the build time. A smaller rotation of one codegen unit with no LTO landed between the default and thin LTO, but it was too noisy to give a figure. `panic` stays `unwind`, because `LiveSession` relies on `catch_unwind`.

## Where the time goes

**Measured by removing each cost.** Each row is a throwaway copy of the code with that cost removed, run in rotation for 5 rounds of fresh processes. Every copy produced the same event count and MUs left as the base. Times are medians for `demo-dropped` and `large-dropped`.

| Removed | Demo | Large | Faster by |
|---|---|---|---|
| Nothing (base) | 2.31 s | 4.89 s | |
| `log` calls, compiled out | 2.32 s | 4.99 s | Nothing measurable |
| Recording occurrences | 2.17 s | 4.53 s | 1.06, 1.08 times |
| `BTreeMap` lookup and `Rc` clone per event | 2.24 s | 4.31 s | 1.03, 1.13 times |
| Successor list clone on each move | 1.92 s | 4.45 s | 1.20, 1.10 times |
| `order` `Vec` on each move | 1.99 s | 4.59 s | 1.16, 1.07 times |
| `Vec` for new events (up to 4 inline instead) | 2.12 s | 4.73 s | 1.09, 1.03 times |
| Queue entry from 64 to 32 bytes | 2.17 s | 4.44 s | 1.06, 1.10 times |
| **A:** lookup, clone, both move `Vec`s, new events `Vec` | 1.37 s | 3.15 s | 1.69, 1.55 times |
| **B:** A, plus no recording and no `log` | 1.20 s | 2.63 s | 1.93, 1.86 times |
| **C:** B, plus the 32-byte queue entry | 1.10 s | 2.05 s | 2.10, 2.39 times |
| **D:** C, with one `Vec` of an enum and successors as ids | 0.92 s | 1.80 s | 2.51, 2.72 times |

A takes allocations per event from 2.38 to about 0.01. Recording also costs memory: without it, `large-dropped` peaks at 68 MB rather than 102 MB, and `demo-kept` at 42 MB rather than 2,189 MB.

The `order` `Vec` wasn't on the list of suspects, and it is the second-largest single cost on the demo model. `RoundRobinStrategy` and `OnlySuccessor` build it on every move attempt.

**Estimated by sampling.** These are shares of samples, so treat them as estimates. The sampler slows the run by about 10%, and key steps were kept out of line to name them, which shifts the numbers slightly.

| Share of time | Demo | Large |
|---|---|---|
| Heap allocation and freeing | 34% | 28% |
| Event queue, pop and push | 20% | 25% |
| Object logic, including dispatch and `RefCell` | 25% | 19% |
| `BTreeMap` lookup | 2% | 10% |
| `Rc` clone and drop per event | 1% | 2% |
| Recording occurrences, without their heap use | 5% | 4% |
| `log` level checks | 0.5% | 0.4% |
| Engine loop, and the rest of the two move `Vec`s | 13% | 11% |

The two methods agree on the main points. Heap work is the largest cost, the lookup only matters at scale, and `log` doesn't matter.

**Limits of these figures:**

- **The prototype behind D is not a design.** It reaches other objects through raw pointers and leaves `Buffer` out. A safe way to borrow two objects at once costs something, so measure it for real. It reaches the same event count and MUs left, but its recording was off, so occurrences weren't compared.
- **Cache effects are probably understated.** This CPU has 96 MB of L3 cache, so a 2,800-object model fits in it. On an ordinary CPU, storage layout may matter more than shown here. That's an expectation, not a measurement.
- **Enum entries are 976 bytes each,** mostly random number generator state. Moving rarely used state out of the entry wasn't tried.
- **The large model's objects are allocated together,** which flatters pointer chasing. A model built in another order may do worse.

**Tools:** the sampler is a small in-process one written for this, using `SuspendThread`, the unwind tables and `dbghelp`, because this session had no elevated shell and WPR needs one. It and the removal scripts aren't in the repo, since they were built to be thrown away.

## Recommendation for step 3

**Make the storage change: one `Vec` indexed by id, an `enum` for the object type, successors as ids.** The owner agreed, and it is done: see "After the storage change" and ADR 0005. Storage does matter, though less than the rest of the path:

1. **Speed:** from C to D is 1.20 times on the demo model and 1.14 times on the large one, with everything else already gone. That's real, but most of the gain doesn't depend on storage.
2. **It removes the costs anyway:** with successors as ids, the lookup and the list clone have nothing left to do. Keeping today's structure means working around the borrow checker to avoid that clone.
3. **Later steps need it:** a checkpoint becomes close to one clone, the `Rc` cycles and their leak go away, and a run can move between threads. Step 5's prototype would otherwise have to save objects through `Rc<RefCell<dyn ...>>`.
4. **It only gets dearer:** the design pillars settle expensive-to-undo decisions up front, and step 7 adds more object code.

The ADR has to answer one thing this measurement can't: an `enum` closes the set of object types, and how user code gets in is still open. If user code ever needs its own object types, one enum variant could hold a boxed trait object, which costs only those objects a dispatch.

**In step 3 or next to it, whichever way storage goes:**

- ~~Remove the `order` `Vec`.~~ Done, and it measured about what the removal above predicted. See "After removing the `order` `Vec`".
- ~~Replace the new-events `Vec`.~~ Done on 2026-11-21: see "Without a `Vec` per event".
- ~~Shrink the queue entry when the millisecond time type goes in.~~ Done: it is 32 bytes.
- Leave the `log` calls alone.
- Step 4's switch for recording is worth 6 to 8% of speed, and far more in memory.

Show before and after numbers from `cargo bench --bench reference`, and check that the events and MUs left don't change unless behaviour is meant to. The millisecond type is expected to change them slightly.

## Without a `Vec` per event (2026-11-21)

An object's event handler returned what it scheduled as a freshly allocated `Option<Vec<EventTemplate>>`, which the run copied into its queue and freed. Objects now call `EventContext::schedule`, into one list the run moves into its queue after each event and hands back emptied, keeping its memory. Every handler schedules in the order its list had, so ties in the queue are unchanged: occurrences and state of the demo and the buffered line, plain and with every Station general, on seeds 0, 7 and 123 over 20 days, with and without pauses and stops, hashed identical to `master`.

Measured on the first PC, master's benchmark binary and the new one alternating, six rounds of 5 repetitions per scenario. The first three rounds were disturbed by something else on the machine (master's demo between 5.65M and 8.67M events/s), so the table gives the means of the last three, in which every round agrees:

| Scenario | `master` | Without the `Vec` | Change | Allocations per event |
|---|---|---|---|---|
| `demo-headless` | 8.84M/s | 12.83M/s | +45% | 0.62 to 0.00 |
| `large-headless` | 5.06M/s | 6.24M/s | +23% | 0.62 to 0.00 |
| `buffered-headless` | 7.49M/s | 11.01M/s | +47% | 0.77 to 0.00 |
| `demo-dropped` | 8.12M/s | 11.52M/s | +42% | 0.63 to 0.01 |

The new binary was faster in all 12 pairs of those rounds, and in 22 of all 24. The gain is far larger than the 10 to 20% expected from an allocation and a free of about 20 to 40 ns each against an event of about 113 ns, so Windows' default allocator costs more than that here. The large model gains least, as before, because more of its time goes to memory than to the allocator.
