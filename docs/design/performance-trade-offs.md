# Performance trade-offs

Every performance trade-off crabsim makes, and every idea that could make it faster, in one place. The owner asked for it on 2026-11-27. `CLAUDE.md` says to keep it up to date.

**Keeping it:** a change that costs speed or memory on any model, or that leaves speed on the table on purpose, gets an entry under "Trade-offs made", newest first, in the same commit. Each entry says:
- what was chosen, and what it was chosen over;
- what it costs, on which scenarios, measured how and against what;
- who decided, and when.

An idea for something faster goes under "Ideas", with what it would gain and what it would cost, and moves to the ledger once it's built or turned down. An entry that turns out wrong is corrected, not deleted.

**Measuring** (`CLAUDE.md`'s "Measure", the method of `first-profile.md`):
- **Wall time:** `cargo bench --bench reference`, alternating the two builds round by round, comparing medians. On the owner's first PC, rounds of one build differ by about ±1.5%. In the cloud container (4 cores) they differ by ±3% and sometimes 20%, so differences under about 3% there are noise.
- **Instructions:** callgrind, `valgrind --tool=callgrind`, which is installed in the container where `perf` isn't. It counts instructions deterministically, whatever else is running, so it settles small differences that wall time can't. `--cache-sim=yes --branch-sim=yes` adds simulated cache misses and mispredicts. The simulated caches aren't the real processor's, but they point to code layout when instructions barely change and time does.
- **Results:** a performance change must not change results. Hash every occurrence and the final state of the reference models against `master`, as the user code PRs did.

## Trade-offs made

### Process times by MU and which MUs an object takes, spec issue 162's PR 5 (2026-11-28)
- **Chosen:** a Station's time rules decided by whoever hands it an MU, on a cold path entered only when the run has them (`EventContext::decides_times`, one check at each move, `model_object::enter_deciding_time`), so a Station's entry and `ModelObject::enter` are as they were, kept inlined with `#[inline(always)]` on `SingleStation::take_in`, `MultiStation::take_in` and `ModelObject::process_mu_entrance`. The time rules and the watches live in the run's event context behind one pointer (`Extras`, and the run's `RunExtras`), not on the Stations, so no object grows (592 bytes) and the context stays 256 bytes. A watch is checked inside the check recorded moves already make, one flag for user code and watches (`beyond_recording`), so a run that watches nothing pays nothing for them.
- **Cost on models without them:** one check at each move. Every occurrence and the final state of the demo, the buffered, conveyor and cross lines (seeds 0 and 7, 2 days, and the demo deterministic) and the model files in `models/` hash the same as `master`, `conveyor-line.json` once MU kind ids are masked: its Tote, the built-in MU's size, now has a kind of its own. Instructions (cachegrind, the benchmark's own scenarios, 1 year): demo-headless -0.05%, demo-kept -0.24%, buffered-headless +0.67%. Wall time, the benchmark's own binaries alternating with `master`'s in the container, medians of 3 repetitions, means of 3 pairs: demo-headless +0.3%, large-headless -2.0% (pairs -2.4, -5.8, +2.3; -0.1% in the round before, with the same code on the move path), demo-kept -1.0%, demo-dropped -0.6%, large-dropped -1.7%, buffered-headless -1.6%, conveyor-line +1.0%, cross-line +4.2%, crowded-buffer -1.1%. `master` itself varied by about 4% between runs of large-dropped.
- **Tried first, and dropped:** a box for the time rules on every Station, which grew `ModelObject` from 592 to 600 bytes (the size test caught it before anything was measured); then the time rules in the context with the decided time read back by every Station's entry, which lost the inlining of a Station's entry and of `ModelObject::process_mu_entrance` into the move loop: about 1% more instructions on the demo, and 1.5 to 5.4% more wall time on most scenarios (demo-kept -4.4%, large-dropped -5.4%); `cg_diff` of two cachegrind runs put it in `take_in`, `ModelObject::process_mu_entrance` and the move loops. With the inlining back, large-dropped was still about 5% slower, which the context growing from 248 to 264 bytes and the run from 576 to 672 explained: one pointer for both brought it back within noise.
- **Cost of time rules:** at each move into a Station that has them, its lines, each condition reading statistics as they stand, as a distribution rule's do; and a stretch's buckets rolled with the rules'. **Cost of a watch:** a binary search among the watched objects and a list comparison at each move into one, only in runs judging expectations. Not measured: no scenario has either.
- **Decided by** Claude while building, against the spec's ±2%.

### Bounds and reference figures, spec issue 162's PR 4 (2026-11-28)
- **Chosen:** bounds worked out on request outside any run (`Model::expectation_bounds`, among `Model::warnings` and in `get_status`), from the model alone: a maximum flow (Dinic's, in whole MUs, without recursion) through what lies upstream of each figure's object, a shortest way (Dijkstra's) for lead times, loops by Tarjan's algorithm, and contradictions by a hash of what each expectation reads. A tolerance is one more field of a parsed line, judged after the run as any verdict is.
- **Cost on runs: none.** Nothing on the per-event path changed, and a run reads no bound.
- **Cost of working them out,** release build in the container, measured once each: 429 copies of the demo (6,006 objects) with five expectations, three of them `every` over a class of 429 to 3,003 instances, 69 ms for the bounds and 177 ms for all of `validate_model`'s warnings; a single line of 3,000 Stations with `every Station.working_share`, about 1.1 s, since each Station's flow is over everything before it. `get_status` keeps the bounds of its last 8 models by their hash, so the window, reading the status every few seconds, works them out once per change.
- **Decided by** Claude while building.

### Rules, spec issue 162's PR 3 (2026-11-28)
- **Chosen:** rules as one more `Distributing` kind on code's cold path, deciding without allocating (`Ruled::order` and `values` reused), and a stretch's 60 buckets kept by the rules that read it, rolled by the run before an event that reaches the next boundary. The statistics keeper is as it was.
- **Cost on models without rules: one comparison an event** (`stretch_next`, `SimTime::MAX`), and the new match arms. Every occurrence and the final state of the demo, the buffered, conveyor and cross lines over 2 days hash the same as `master`. Instructions (cachegrind, 10 simulated days): about +0.7% on the demo and +0.4% on the buffered line with the buckets in the keeper and the check in the run loop. Wall time with the buckets in the rules, the benchmark's own binaries alternating with `master`'s in the container, medians of 9 repetitions (3 for the cross line), means of the pairs: demo headless -0.6% (5 pairs), buffered line +1.6% (5), demo kept +1.0% (3), 6,006 objects -1.0% (3), cross line -1.8% (3). The container's noise is a few percent between runs of one build, so these are within it, the cross line nearest the ±2% the spec allows.
- **Tried first, and dropped:** the buckets in `StatisticsKeeper`, behind a pointer, with its capacity narrowed to 32 bits so no object grew. A check on each change of state cost 2.2 to 2.5% of instructions (callgrind put it all in `update_statistics`); moved to the run loop, instructions came to +0.4 to +0.7%, but the demo still ran about 4% slower in five alternating pairs, which only the keeper's changed layout explains. Putting the keeper back as it was brought it within ±2%.
- **Cost of a rule:** its lines per decision, each condition reading statistics as they stand (a `read` of the keeper, a copy of its totals), and `smallest`/`largest` one read per successor and a sort. Not measured: no scenario has rules.
- **Decided by** Claude while building, against the spec's ±2%.

### WebAssembly hooks, PR 2 (2026-11-27, spec issue 156)
- **Chosen:** a plain WebAssembly module with a hand-written interface, run by wasmtime, rather than a component under a WIT world (`code-interface.md`). The reason was checkpoints, not speed: a component's memory can't be read through wasmtime's API, so it couldn't be copied.
- **Cost on models without code: none.** The engine's per-event path is unchanged: PR 2 added only `HookRun::seed` and `stream` and made `TriggerPoint::ALL` public. A model without code builds exactly as before (a test compares every occurrence and the final state), and the host crate is never on its path.
- **Cost of a call,** `cargo bench -p crabsim-code --bench calls`, the demo for 30 days with a distribution hook at each of its 3 forks answering default, against the same model without code, best of 3 in the container: about 190 ns a call in WebAssembly and 178 ns natively, so simulated time a wall second falls from 26.4M to 17.4M. Two and a half events' worth, where the spec expected about one.
- **Where it goes:** callgrind puts most of it in the engine's path for code (`move_by_code`, the hook checks at every move once a model has any hook, `run_calls`), and about a sixth in the host's `Code::call`. wasmtime's own call is about 15 ns, the difference between WebAssembly and native.
- **Decided by** Claude while building; the owner confirmed on 2026-11-27, dropping the component model, after a measurement in wasmtime 51 (scratch benchmark, 10 million calls, median of 5): an empty call 14 ns as a plain module against 43 ns as a component, each read from crabsim about 2 ns against about 26 ns, so a component would cost about 15% more on a hook that reads nothing and 50 to 100% more on real ones. A component's checkpoint through `save_state`/`restore_state` was 15 µs against 26 µs at 100 bytes, but 57 µs against 26 µs at 64 KB and about 1.05 ms against 350 µs at 1 MB, and would lose any state its author forgot to save.

### Every run knows where each MU is (2026-11-27, spec issue 156, PR 1b)
- **Chosen:** a table of the object holding each MU (`src/locations.rs`), in every run, so that user code, the window, the catalogue and agents find an MU by its id alone (`SimulationRun::location_of`). The owner chose it over a table kept only in runs with user code, which would cost other runs nothing, "for now, to see what it costs".
- **How it's built:** a window over the MU ids, a move one bounds check and one write. It grows while at least half its slots hold an MU, so a crowded model stays in it at most 8 bytes per MU. Stray long-stayers and starting MUs go to a map. A Drain takes its MUs out, so memory stays flat.
- **The first version moved every MU past 4,096 ids to the map.** The `crowded-buffer` benchmark scenario (about 50,000 MUs in a Buffer at once) showed that cost 22% more instructions and a third of the speed there, against 4.3% for the growing window.
- **Cost against the branch without the table** (callgrind instructions):

  | scenario | instructions |
  |---|---|
  | demo, 30 days | +2.36% |
  | buffered line, 30 days | +2.61% |
  | conveyor line, 7 days | +0.39% |
  | `crowded-buffer`, 30 days | +4.26% |

  Wall time on `crowded-buffer` was −1.6% (6.91M against 7.02M events/s). Peak memory there was 4.2 MB against 3.7 MB, and on other scenarios unchanged.
- **Where the cost is:** mostly in the move itself (`try_to_move_to_successor`) and in a Drain taking each MU out. Models with conveyors pay less because their events do more work.

### Conveyors' and cross conveyors' hooks (2026-11-27, spec issue 156, PR 1b)
- **Chosen:** hooks on conveyors and cross conveyors at their Front, Booking point and Rear trigger points, with a rear event queued only for hooks that ask for the rear.
- **Cost on models without hooks:** within ±2% of `master`, alternating, medians of 3 rounds in the container: conveyor line −0.8%, cross line −1.8%, demo −0.9%, buffered line +3.8%. Results are unchanged.
- **The first version was 4 to 8% slower on the conveyor scenarios,** for code layout rather than work: only 0.7% more instructions, but 57% more simulated instruction-cache misses, in `offer`, `replan` and `hand_over`. Fixed by:
  - the code path's copies of `reserve_with_strategy` wrapped in `ByCode`, so the hot copy keeps one caller and stays inlined into `offer`;
  - what conveyors do for code moved into cold functions behind one `has_hooks` check;
  - a conveyor's state for code in one boxed `ForCode`;
  - the rear event checked in the arm for stale tickets.
- **Decided by** the owner's rule from PR 1a: the spec's ±2%.

### User code's engine, PR 1a (2026-11-26, spec issue 156)
- **Chosen:** hooks on objects without geometry, with calls made right after the event rather than through the queue.
- **Cost:** models without hooks run 4 to 8% slower than before (alternating on the first PC: demo headless −7.9%, 6,006 objects −3.6%), outside the spec's ±2%. The owner accepted it for now, to be looked at once PR 2's WebAssembly path gives the hot path its final shape.
- **What was found, fixed and ruled out** is in the handoff's "User code (PR 1a)". The cost was spread evenly over every hot function rather than in one place.

### Conveyors' paths, PR 135 (2026-11-22, spec issue 133)
- **Cost:** models without conveyors run 2 to 4% slower than before it (demo headless 12.46M against 12.87M events/s), and no single change explains it. Each suspect taken out alone left the gap, which points to code layout.
- **What was fixed:** the first version was 11.5% slower, because every move asked which MU it offered. Only a conveyor asks now, in a cold function.
- The handoff's "Conveyors (spec issue 133)" has the details.

### Smaller decisions recorded elsewhere
- **A conveyor's start coming free has no event of its own** (`clears_at`, `short_until`). An event per MU entering made `conveyor-line` 20% slower in simulated time (`conveyor.md`).
- **Each MU's length along the belt is kept on the belt** (`OnBelt::along`), so a replan reads no MU kind. Reading it each time had made the conveyor line slower in every round (handoff, "Conveyors").
- **A Station of capacity 1 stays a `SingleStation`,** the cheaper path, so models without capacity can't get slower (handoff, build step 7).
- **An MU is 16 bytes** (ADR 0027), its departure time and kind packed in one word. That limits a run to about 8,900 years.
- **Models without work plans pay one comparison per MU entering an object** (`MoveableUnit::is_routed`) and a table lookup per MU made (handoff, "Conveyors").
- **Measuring periods cost one comparison per event** (`docs/design/statistics.md`).
- **Scheduling without a `Vec` per event** (PR 126) took allocations per event to 0 and made demo headless 45% faster and 6,006 objects 23% (handoff, build step 7).

## Ideas

- **Bounds of a line in one pass:** where nothing upstream of an object splits its MUs between successors, the maximum flow into it is the least of the paces on the way, plus what is held, which one pass in flow order gives for every object at once. It would take `every` over a long line from about 1.1 s for 3,000 Stations to a few milliseconds (spec issue 162, PR 4). Worth it if a real model is that large and the status or validation feels slow.

- **Pooling or copy-on-write memory for code's instances,** asked to be explored by the owner on 2026-11-27.
  - A checkpoint of a run with code makes a fresh instance and copies the module's memory into it. Measured with wasmtime 51 in the container, most of that is first-touch page faults on the fresh memory, about 1.4 µs per 4 KB page: 26 µs for 64 KB and about 350 µs for 1 MB, where the copy alone is 1.3 µs and 40 to 50 µs. A Rust module's default 1 MB stack puts real modules near the 1 MB figure.
  - wasmtime's pooling allocator keeps memories mapped between instances, and a copy-on-write snapshot would share pages until written. Either would make checkpoints, and Reset, with code much cheaper.
  - Cost: the pooling allocator reserves address space up front and caps instances per engine; copy-on-write needs wasmtime's memory images or our own mapping.
  - Measure with a live session's checkpoints on a model with code, before and after.

- **Make a code distribution's call cheaper in the engine** (PR 2's measurement above).
  - Check for hooks only at objects that have them, rather than at every move of a model that has any (`note_entry`, `moved_hooked`).
  - Skip the `Decided` event's second pass through the move for a default answer, which tries the built-in strategy straight away.
  - Gain: up to most of the 190 ns a call, measured with `cargo bench -p crabsim-code --bench calls` and callgrind.
- **Don't clone the host's shared `Arc` on every call** in `Code::call`, a pair of atomic operations per call. Small.

- **Keep the MU table only in runs with user code.**
  - Gain: about 2.4% of instructions on models without conveyors, 0.4% with them, back for models without code.
  - Cost: `location_of` for the window, the catalogue and agents searches the run's state, slower the more MUs the run holds.
  - The owner chose to keep it in every run for now.
- **Find PR 1a's 4 to 8% and PR 135's 2 to 4%.**
  - Both look like code layout.
  - A sampling profiler with hardware counters would settle them: samply and the Windows Performance Toolkit are installed on the first PC.
  - In the container, callgrind's simulated caches point the way, as they did for PR 1b.
  - Planned once PR 2 has given the hot path its final shape.
- **Statistics cost about 7 to 12% of a headless run** and haven't been tuned (handoff, "What's left on the per-event path").
- **A Buffer's exits that find it already blocked** are 11% of the buffered line's events (same place).
- **Order each object's fields** so that those every event reads share cache lines. Needs a profile counting cache misses first.
- **Put the event context's fields for user code behind one boxed `Option`.** PR 1a and 1b grew `EventContext` (`calls`, `rears`). Its size was ruled out alone in PR 1a, but together with other changes it may still matter.
- **The MU table without its live count.**
  - The count steers only when the window moves MUs to the map. It costs about 0.1% of instructions on the demo, one increment per MU made and one decrement per MU leaving.
  - A rule based on the window's length alone could drop it, at the risk of moving MUs to the map too early in a crowded model.
