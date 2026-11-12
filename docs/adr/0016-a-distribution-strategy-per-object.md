# Each object chooses its distribution strategy, and weighted ones rank their successors ahead of time

A Source or Station has a `distribution` attribute: which successor receives the MU it passes on. It is `round_robin`, each successor in turn and the default, or `weighted`, which ranks the successors by a weight and offers the MU to the best one that can take it, those ranking equal taking turns. A weighted object weighs by a numeric attribute (`process_time`, `interval`, `starting_mus` or `capacity`), smallest or largest first, and ranks each successor either by its best route onward to a Drain, itself included, or by its own weight. It may name the Drains a route counts. The attribute is inherited as a whole. The run works out the rankings when it is built, at Reset and whenever a value they weigh changes, and moving an MU only reads them. The model file stays format version 2, since the attribute is new and optional.

We decided it on 2026-11-12 in a design round (spec issue 96), after the owner tried weighted paths (PR 95) and wanted them to steer MUs, not just to be read.

## Considered Options

- **Live state as the weight,** such as how busy each successor is at the moment of each exit. A rule that adapts during a run, but one that costs work on every exit, on the path the benchmark measures. Kept for its own round.
- **A stored run's figures as the weight,** frozen into the model. Reproducible, but the model would depend on the results store.
- **Ranking at every exit.** No state to keep up to date, but a route search per MU moved.
- **Waiting for the best successor** even while others are free. A strict routing rule, but it changes the blocked-list rules: an MU would wait at one successor rather than listing itself at every one.
- **Updating Drain lists when a Drain is renamed or removed.** The owner chose not to have crabsim change a model behind the user's back: a name that isn't a Drain is ignored and is a warning that `validate_model`, and the window's Validate button, show.

## Consequences

- Moving an MU checks the object's strategy, one comparison on the per-event path. Measured on 2026-11-12 against `master`, the benchmark's round-robin scenarios stayed within noise. Each object carries its strategy, and the run its rules, a little memory per object.
- A model of round robin only behaves exactly as before, MU by MU.
- A Station's process time changed during a run, an intervention, ranks the successors again, and replaying it after going back does the same, so going back stays exact.
- Switching an object's strategy is allowed only before Init or after Reset, until it becomes an intervention with the live-session operations (issue 28, step 7).
- Weights on connections, such as a transport time, and how many MUs passed each connection come later, with distribution shares on connections after build step 7. The owner wants both, the first as a tool for control strategies.
