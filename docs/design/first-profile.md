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

**Make the storage change: one `Vec` indexed by id, an `enum` for the object type, successors as ids.** Storage does matter, though less than the rest of the path:

1. **Speed:** from C to D is 1.20 times on the demo model and 1.14 times on the large one, with everything else already gone. That's real, but most of the gain doesn't depend on storage.
2. **It removes the costs anyway:** with successors as ids, the lookup and the list clone have nothing left to do. Keeping today's structure means working around the borrow checker to avoid that clone.
3. **Later steps need it:** a checkpoint becomes close to one clone, the `Rc` cycles and their leak go away, and a run can move between threads. Step 5's prototype would otherwise have to save objects through `Rc<RefCell<dyn ...>>`.
4. **It only gets dearer:** the design pillars settle expensive-to-undo decisions up front, and step 7 adds more object code.

The ADR has to answer one thing this measurement can't: an `enum` closes the set of object types, and how user code gets in is still open. If user code ever needs its own object types, one enum variant could hold a boxed trait object, which costs only those objects a dispatch.

**In step 3 or next to it, whichever way storage goes:**

- Remove the `order` `Vec`, which needs no storage change and could be its own small PR. On the demo model that's 16% more speed.
- Replace the new-events `Vec`. An output buffer the run reuses should beat the inline list measured here.
- Shrink the queue entry when the millisecond time type goes in. A 64-bit time and dropping the duplicate `seq` and `exec_time` fields gets it to 32 bytes.
- Leave the `log` calls alone.
- Step 4's switch for recording is worth 6 to 8% of speed, and far more in memory.

Show before and after numbers from `cargo bench --bench reference`, and check that the events and MUs left don't change unless behaviour is meant to. The millisecond type is expected to change them slightly.
