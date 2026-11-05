# crabsim handoff

Written 2026-11-06 at the end of a working session. Read this first, then check it against the code and git, because it describes a moment in time.

The project owner returned to this project after about six months away and didn't remember the state of the work, so this also covers what was found.

## Start here

1. Read `CLAUDE.md`.
2. Read `GLOSSARY.md` and `docs/adr/`.
3. Read "Agreed design" below. Most of it isn't recorded anywhere else.
4. Ask the owner what they want to do next. "Possible next steps" lists the options.

## Current state

Checked on 2026-11-06: `master` matches GitHub and passes 52 of 52 tests (`cargo test --all-targets`). The demo ran to the end in both modes on the same source code before it was merged.

| Branch | Where | State |
|---|---|---|
| `master` | Local and GitHub, level | Has the repair, timing fix, PR 1, PR 2, the glossary and this handoff |
| `docs/glossary` | Local only | Merged into `master`, can be deleted |
| `feat/simulation-run` | Local and GitHub | Merged through PR 2, can be deleted |
| `chore/agent-skills-setup` | Local and GitHub | Merged through PR 1, can be deleted |
| `fix/repair-build` | Local only | Merged into `master`, can be deleted |

Merged PRs: [#1 agent skills setup](https://github.com/MarcHoff94/crabsim/pull/1) and [#2 simulation run](https://github.com/MarcHoff94/crabsim/pull/2). The owner merges PRs on GitHub themselves, so fetch before assuming local `master` is current. The repo has no CI, so a local test run is the only check.

## What happened in this session

1. **Skills set up:** `docs/agents/` and the `## Agent skills` section in `CLAUDE.md`. Issues live in GitHub Issues, using the default triage labels, which were created on GitHub. `gh` was installed with winget and signed in as MarcHoff94.
2. **Found `master` broken:** three lines of work had forked from `d78c53f` and been merged badly, leaving 21 compile errors. The last good commit was `0e4db6f`.
3. **Domain design sessions:** see "Agreed design".
4. **Build repaired** (`3d75057`): 20 of 20 original tests passing again.
5. **Failure timing fixed** (`068d2f4`): a short failure used to raise throughput, because a queued `MUExit` from before the failure still fired. A 5s failure gave exits at 70, 75, 130, 140, 190, and now gives 75, 135, 195.
6. **Architecture review:** 8 candidates, listed below.
7. **Candidates 1 and 2 built** (`408b632`, PR 2): `SimulationRun`, occurrences, typed state, per-object reset, view adapters. `crabsim-renderer`, its TCP code and `serde` were removed.

## Agreed design

The owner decided all of this. Don't reopen it without a reason, and flag it if you need to. **Built** means it's in the code now, and **not built** means it's agreed for later.

### Model
- **Material-flow objects** are Source, Station, Buffer and Drain. Each handles MUs and represents a real thing. "Station" means only the processing type. This is a general simulator for any material flow, such as a factory line.
- **Every material-flow object can fail**, so all of them get `StopCtrl`, even ones that consume no time, like a Buffer. Each gets a failure profile, and availability 1.0 means it never fails. *Not built: only Station has failures, and `StopCtrl` is `todo!()` everywhere.*
- **Station capacity** replaces a separate ParallelStation (ADR 0001). It's one machine working on several MUs at once, **not several parallel machines**. Capacity 1 is the default. *Not built.*
  - **Start modes:** batch start (work begins when full) and individual start (each MU starts on entry).
  - **Partly filled batch start:** waits indefinitely, with the waiting MUs visible in state. A timeout can come later if needed.
  - **Batch entrance:** closed once full, until the Station is completely empty.
  - **Individual entrance:** open whenever a slot is free. A finished MU still waiting to leave occupies its slot.
- **Buffer** holds MUs and stays a separate concept from Station even though it can behave alike (ADR 0001). *Buffer's movement is still broken.*
  - **Dwelling time:** one fixed value per Buffer, default 0, counted individually from each MU's entry.
  - **Stops and dwelling time:** a stop doesn't affect dwelling time by default, and this is configurable.
  - **What a stopped Buffer does:** entrance and exit are closed, and it keeps its contents.

### Stops
- **Interruption** is the cause (a failure or a pause). **Stop** is the resulting state of the object. A failure from t=20 to 30 plus a pause from t=25 to 35 is two interruptions and one stop from t=20 to 35.
- **Failure** is unplanned, with random timing. **Pause** is planned and commanded from outside, and doesn't count against availability. They are independent of each other. *Pause isn't built, and overlap isn't built: a second interruption is currently discarded.*
- **Completion time** = entry + process time + time spent stopped. Overlapping stops count once. *Built for failures, without overlap.*
- **A stop by type** (*not built, except Station failures*):
  - **Station:** takes no MU, releases no MU, work is frozen. A whole Station stops at once, whatever its capacity.
  - **Source:** creates no MUs, and its next creation moves later by the stop's duration.
  - **Buffer:** as above.
  - **Drain:** refuses MUs, so predecessors become `Blocked` and retry when it's back.
- **Failure time bases**, configurable, with not every object needing all three (*not built, behaves as simulation time*):
  - **Simulation time (default):** all time counts, whatever the object is doing, including during a pause.
  - **Working time:** idle and blocked count, and only a pause freezes the clock.
  - **Processing time:** only time actively working on at least one MU counts, once however many MUs are in work. A finished MU waiting to leave doesn't count.
- **Availability** is measured against the chosen time basis, so a rarely busy Station on a processing-time basis is available more of real time than its setting. That's expected.

### Moving MUs
- **Push** is the current concept. **Pull** is planned, as a setting on the *receiving* object, where the successor chooses which MU it wants from its predecessor. Only build a seam for "which MU leaves" now, not pull itself. Mixed push and pull cases get decided when pull is designed.
- **Exit strategy** picks which MU leaves, and **distribution strategy** picks which successor gets it. They're two separate rules. The default exit strategy is one at a time, earliest finished first, ties broken by entry order. More strategies may come later. *Not built as a seam.*
- **Under push, MUs can't overtake within a Station**, because successors belong to the Station rather than to one MU. Only pull could change that.
- **Blocked predecessors:** the longest-waiting one retries first, and each is listed once. *Not built: retries run newest-first, and the runtime `add_blocked` doesn't dedupe.*
- **A move takes no simulation time.** Any gliding is visual only. Real transport time would be a new material-flow object, such as a conveyor.

### Running a simulation (built, except where noted)
- **Controls:** Init, Start, Halt, Reset. Halt avoids clashing with Stop and Pause, which happen to objects.
- **Init is its own step**, the first step of every run: t=0, every object's init in id order, first events queued but not processed, even those due at t=0. Start does Init if needed, then advances. Start and Halt are UI controls, and aren't in the code yet.
- **Reset** returns objects to their default state and keeps them in the model. Every run MU is deleted, and configured starting MUs come back as fresh copies. The same seed replays the same run.
- **The default state is what the user configured**, and can be changed. Configuration changes are only allowed after Reset, for now.
- **Control order:** Init only from the default state, advancing requires Init, Reset allowed at any time. Violations return errors rather than panicking.
- **A run advances only when asked** and has no end time of its own. `advance_to(t)` includes events at t. Speed control belongs to the caller.
- **A run stays single-threaded on `Rc`**, created entirely on one dedicated thread. Commands and observations cross threads; objects don't.
- **Occurrences** are returned by each advance. Views get no callbacks. A run can also be asked for its full typed state. "We might add more states later."
- **MU ids** are unique across the model.

### User code (not built)
The owner wants users to add their own source code later. The decisions so far:
- Each object has fixed init and reset parts that **users can't change**. For example, Reset always deletes all MUs.
- A second user phase runs after the fixed phase **has finished for all objects**, in id order, for both Init and Reset.
- How the code gets in (scripting, plugins, WebAssembly) is open, and deserves its own ADR when decided.
- Don't build the user phase until there's real user code.

## Build order

The original repair plan, with status:
1. ✅ Get it building and back to the original tests passing.
2. ✅ Fix the failure timing bug, with stale-event checks.
3. ⬜ Finish `StopCtrl` on all four object types, including overlapping interruptions and failure profiles.
4. ⬜ Fix `Buffer`, test it, and add dwelling time.
5. ⬜ Add the time bases.
6. ⬜ Add Station capacity and start modes, with the exit strategy seam.

Also agreed: a small retry-order fix (longest waiting first, deduped), as its own commit. The owner agreed that candidates 1 and 2 come before step 3, and those are now done.

`ExitTicket` was designed with capacity in mind, because a whole Station stops at once, but that's untested until step 6.

## Architecture review

Candidates from the review. "Strong" and "worth exploring" were the review's own ratings.

| # | Candidate | Rating | Status |
|---|---|---|---|
| 1 | Simulation run you can step, pause and reset | Strong | ✅ Done |
| 2 | Observation seam, presentation out of the engine | Strong | ✅ Done |
| 3 | Model as data, with one module that validates and builds it | Strong | Open. Needed for drag and drop, load and save |
| 4 | Objects that report themselves | Strong | Partly done: state and connections. Open: trim traits, remove the 8 downcasts in `topology.rs` |
| 5 | One MU movement module | Strong | Open. The same exit logic exists in about 6 slightly different copies |
| 6 | Live session seam for the UI | Worth exploring | Open. Next step towards the UI |
| 7 | One shared stop module | Worth exploring | Open. Worth deciding before writing step 3 |
| 8 | Reproducible runs and real errors | Worth exploring | Partly done: init order, event errors. Open: one time representation, stuck-timestamp guard |

ADR 0001 must not be contradicted without flagging it. The review's HTML report was in a temporary folder and may be gone.

## The UI goal

The owner wants to completely replace the CLI with a real UI.
- **First:** a visualization of the code running, as a basis to build on.
- **End goal:** drag and drop objects, configure them, start, stop, init and reset the simulation, animate and render it, watch it run, and control how fast simulation time passes.

What each stage needs:
- **Watching a run:** the steppable run and occurrences (done), connection data (done), plus a view adapter.
- **Controls and speed:** candidate 6.
- **Drag and drop:** candidate 3.

**Undecided, and the owner's call:** UI technology, and desktop or browser. Node isn't installed on this PC, and neither are `wasm-pack` or `trunk`. Rust is cargo 1.91.1. The UI will probably need `serde` back. `crabsim-renderer` is gone, so there's currently no window at all.

## Known problems

- **`Buffer` is broken** (from reading, not run): `process_mu_exit` takes an MU and then the move takes another, so a lone MU never leaves, and with two or more, one is lost per move. It also never notifies blocked predecessors. Its test only proves it replays consistently.
- **Source with several successors** (from reading): a `BlockedRetry` can move an MU before its interval ends, because `is_exit_allowed` is always true.
- **Zero-time loops can hang a run** (from reading), such as a zero-interval Source into a Drain, or constant zero-length failures. This would freeze a UI.
- **Illegal connections panic** (from reading): an edge into a Source, or a self-loop. There's no validation until candidate 3.
- **`pred` is never read**, but constructors still take it, and `succ`/`pred` `Rc` cycles leak when a model is dropped.
- **Dead code** hidden by `#![allow(dead_code)]`: `MUEntrance`, `Finished`, `exitlocked`, `Paused`, `FailureProfile.mtbf`, and much of `topology.rs`, including `print_network_topology` and its helpers.

## Open housekeeping

- **Stale docs:** the README mentions the removed renderer. 29 files across `wiki/`, `docs/` and the README still name `Simulator`, `EventController` or the renderer. Run `/update-wiki`. `wiki.html` is generated from the wiki.
- **Duplicate wiki:** both `wiki/` and `docs/wiki/` exist. Not decided.
- **Wiki hook is off on this PC:** `core.hooksPath` is unset. Turning it on with `git config core.hooksPath .githooks` makes every `src/` commit run `claude` in the background.
- **`crabsim.log` is tracked** in git, and running the demo from the repo root may change it.
- **[Dependabot alert](https://github.com/MarcHoff94/crabsim/security/dependabot/1):** one low-severity alert, not looked into.
- **Finished branches** can be deleted (see the table above).

## How to work with the owner

- **Plain language and domain precision matter.** The owner knows material-flow simulation well and corrects loose models, for example that a ParallelStation is one machine rather than several. Listen for this and update `GLOSSARY.md` as terms settle.
- **Design before code.** The `grilling` and `domain-modeling` skills worked well, in rounds with recommendations where "yes" accepts. The owner often adds a requirement to a "yes", so read the whole answer.
- **Commit, push and merge only when asked.** The owner merges PRs on GitHub.
- **Test-first at agreed seams.** Behaviour tests go through `SimulationRun` in `tests/`.
- **Check that tests can fail.** Deliberately breaking the code exposed real gaps: a same-seed replay can't detect events left over after Reset, because they land on the replay's own event times. See `CLAUDE.md`.
- **The shell is Windows PowerShell 5.1.** `&&` doesn't work there, so give commands one per line. Git Bash is available to agents through the Bash tool.

## Possible next steps

In the order I'd suggest:
1. Run `/update-wiki` so the docs stop describing removed code.
2. Decide where to go next:
   - **UI first:** grill candidate 6 and the UI technology, then build the first read-only visualization.
   - **Domain first:** continue with step 3, deciding candidate 7 first, since it shapes how `StopCtrl` gets written.

The owner's stated priority at the end of this session was the UI and overall architecture, so the UI path is probably next. Ask rather than assume.
