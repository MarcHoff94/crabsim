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
