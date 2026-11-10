# First-time experience review

A walkthrough of crabsim's window as a first-time user who doesn't know crabsim, from the README to a first result. It's the third part of the first item in the handoff's "Possible next steps", "Building models in the UI, for a first-time user", and is input for its design round.

Done on 2026-11-09 by an agent driving the window, at `master` after PR 77 (`f322257`), on the owner's PC, with the window at its default size of about 800 by 790 pixels. The window ran on the repository's `models` folder, as `--help` suggests. Attributes weren't changed and objects weren't dragged, because `models/demo.json` is tracked: that path wants a scratch models folder.

**Result:** a first result can be reached. A 30-day run of `demo.json` showed 131,478 MUs in at the Drain, 182.6 an hour. The newcomer stumbles about a dozen times on the way, and nothing in the window helps them build a model yet. The findings are below, in the order a newcomer meets them.

## Before the window opens

1. **The README's commands fail.** `cargo run -p crabsim-ui --profile ui` stops with "--models <MODELS> required", and `cargo run --release --bin crabsim` and its `--seed 7 --length 365d` form stop with "--model <MODEL> required". These are the first things a newcomer types. The README's status paragraph still says the UI "plays the demo model live" and that building models comes later, with no mention of models folders.

## First look

2. **The JSON Schema is listed as a model.** `model-file.schema.json` sits in the model list marked "⚠ (8)". Clicking it shows 8 problems such as "unknown field `$defs` in a model file", which looks like something is broken. The list could leave out files that aren't model files, such as anything with `$schema`'s own fields or the schema's name.
3. **A confirmation appears when there's nothing to lose.** Clicking a model with no model open asks "Open model-file.schema.json? The run in progress ends, with everything it has done.", though no run exists.
4. **The top bar doesn't fit the default window.** At about 800 pixels it wraps onto a second row, and the speed slider and its readout are cut off at the right edge. Opening a model's problems widens the inspector, which moves the top bar again.
5. **Nothing says what to do first.** The canvas says "Choose a model from the list on the left", which helps. After that, Init and Start are both enabled and Start works without Init, which is fine, but a newcomer can't tell whether Init is needed or what it does.

## Running

6. **The colours have no legend.** Orange, green, red and grey make sense only after clicking objects and reading their state. A small legend, or the state named on hover, would do.
7. **Getting to a useful length is hard to find.** At the starting 60 times real time a simulated day takes 24 minutes. Dragging the slider to the right end gives 200,000 times, about 7 days in 3 seconds, and with the slider cut off the speed can't be read. What works is End at, then Start, which stops at "Reached the end time", but nothing points there.
8. **An end time typed while running didn't take,** and the run went past it to 19 days. **Not a defect, found later:** the walkthrough's input tool sends typed text that never reaches the window, so the field stayed empty and Set disabled. With real key presses it works.

## Reading the result

9. **Throughput is buried.** It's "MUs an hour" in the Drain's inspector. On a Drain, Working, Blocked, Idle, Failed, Paused and Availability are always 0% or 100%, and "MUs out 0" next to "MUs in 131478" reads like a fault. There's no summary of the whole model, such as throughput, lead time or the busiest Station, which the catalogue's `run_results` already works out.
10. **A Station shows its process time four times:** the intervention field, "Process time", "Configured" and the attribute. The hint that tells attributes and interventions apart is too faint and small to read at the default size.
11. **Reset throws the result away.** The 30-day figures were gone with no record, and the window has no way to compare before and after a change. Only the CLI's `run` and `compare` can, through the results store.

## Building

12. **There's no way to build or connect anything.** Right-clicking and double-clicking the canvas do nothing. This is expected: the class library, drag and drop and connecting are the work this review feeds. The Editing tab, "Nobody holds it." with a Take button, means nothing to a newcomer who has no agent.

## Decided

The owner went through findings 1 to 11 in a grilling session on 2026-11-09, and [spec issue 78](https://github.com/MarcHoff94/crabsim/issues/78) holds the details. Finding 12 stays with the building design round.

- **The goal:** a newcomer who hasn't read the docs opens a model, runs it as long as they choose, and can say its throughput and which Station is the bottleneck, within 5 minutes. Changing one thing and comparing is the next goal after it.
- **Order:** the defects first (findings 1, 3, 4 and 8) in one PR with no design round, then this slice in five PRs, then the building design round.
- **Starting (1, 2):** without `--models` the window opens `Documents\crabsim-models`, created on first start with a copy of the demo. A folder dialog comes later. The catalogue lists a `.json` file as a model only if it has `format_version`, so the window and agents list the same files.
- **Inspector (9, 10):** each object type shows only the statistics that can vary for it, the window says Busy as the glossary does, and a Station's process time shows once during a run. A "Keep changes on Reset" checkbox, off by default and not remembered, writes changed values to the model file through `edit_model`. With it off, Reset names the changes it drops.
- **Explaining (5, 6):** hover text from the glossary on every control. The Editing tab becomes Agents, with a line saying what shows there. One fixed colour per state for the whole app, so charts can use them later: Busy green, Blocked orange, Idle grey, Failed red, Forced failure dark red, Paused blue. No canvas legend: hovering an object shows its name and state, and a **?** in the inspector opens a panel with its states, their colours and what they mean. A new `state_notes` attribute lets a class or instance add its own note per state, inherited like any attribute and shown above crabsim's definition, never instead of it.
- **Speed (7):** an Animate toggle. Off, the session records nothing and computes with no speed limit, while the window shows the time and the states a few times a second. Going back and stepping still work, since checkpoints count events. Go to works in both directions, and animates at the chosen speed while Animate is on.
- **Results (9):** a Results tab with each Drain's throughput and lead time and every Station's shares, marking the likely bottleneck by a rule that lives in the catalogue as an operation, so agents get the same figure.
- **Deferred (11):** comparing before and after a change in the window, with experiments and `compare_runs`, gets its own round after building. Reset keeps nothing for now.

## Not covered

- Changing attributes in the inspector and dragging objects, which write to the model file.
- A larger window or other scale factors.
- Errors from a model, interventions, stepping and going back.
- The agent's side: a first-time user who starts from an agent rather than the window.

## Second walkthrough, after spec 78

On 2026-11-10, at `master` after PR 85 (`736134e`), the same way: the window started as the README says, with no arguments, on the owner's PC at the default window size. `Documents\crabsim-models` already existed from trying PR 81, holding only the demo, so the window opened it rather than making it, which shows the same.

**Result:** the goal is met in time. About a minute and a half after the window opened, the Results tab gave the demo's throughput, 185.0 an hour, and a likely bottleneck, Station 2, busy or stopped 89.3% of the time. Nothing on the way needed the docs: the canvas says to choose a model, the README says to press Start, hover text explains every control, and the tab names were enough to find the result.

**But the answer is the wrong one.** It came from 18 simulated minutes. Raced to 30 days with Animate off the day before (PR 85), the same tab named Station 7, busy or stopped 86.7%. A newcomer reading the tab early gets a confident answer that a longer run reverses, and nothing in the window says the run is too short to trust, that the start of a run is unlike the rest (warm-up), or that one run of a model that fails at random is one sample.

Findings:

13. **Early results look as final as late ones.** The Results tab shows a likely bottleneck after any length of run, with no sign of how sure it is. Options for the next round: say how much simulated time the figures cover beside them, a note when it is short against the model's failure and repair times, a warm-up, or offering an experiment of several observations, which the catalogue already has, with `find_bottleneck` on it.
14. **Going to a far time while animated is a trap.** Go to 30d with Animate on runs at the chosen speed, 12 hours at the starting 60 times, though the hover text says so. A newcomer who doesn't know to untick Animate first waits. The window could offer to race when the time gone to would take more than a minute or so.
15. **The run controls jump sideways when a model opens.** "No model open" is replaced by the model's name, and the controls after it move by the difference, so a pointer aimed at Start before can land on Halt. Giving the name a fixed width, or putting it last, would keep them still.
16. **Results show shown time to the millisecond,** such as "At 00:18:24.151", where seconds are enough while a run is going.

Not covered this time: changing anything, the Agents tab, a model of the newcomer's own, and a true first start that makes the folder (PR 81's test covers that).

The owner decided findings 13 to 16 on 2026-11-10, in the design round for building models: [spec issue 87](https://github.com/MarcHoff94/crabsim/issues/87). Findings 14 to 16 are its PR 1, and finding 13 its PR 2: confidence intervals in the Results tab from measuring periods within the run, shown once 5 periods are complete, "too close to call" while the bottleneck's interval overlaps the runner-up's, and a note while periods are short against the model's repair times. A warm-up and checking with an experiment go to the comparing round.
