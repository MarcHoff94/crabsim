# Each instance keeps its own random-number stream

An instance's random numbers come from the run's seed and the instance's **stream**, a number it gets when it's placed in the model and keeps, written in the model file as `"stream": 7`. Changing the run's seed changes every instance's random numbers. Nothing else does: not inserting, removing, reordering or renaming other instances, and not changing the instance's own values. The user may change a stream by hand or with `set_stream`, which isn't advised and which agents can't do. Before this, an instance's random numbers came from the seed and its position among the instances, so removing or inserting an instance changed the random numbers of every instance after it, and two variants of a model compared with the same seed lost their common random numbers without anyone noticing. The second harness trial ran into exactly that.

## Considered Options

- **Position, as before.** Nothing to store, but every insertion or removal reshuffles the random numbers of the instances after it, which breaks the pairing of runs that comparing variants relies on, and drag and drop in the UI will insert and remove all the time.
- **The instance's name, hashed.** Nothing to store, and insertion and removal are harmless, but renaming an instance would change its random numbers, and two models could only pair by name. The owner wanted no operation but changing the seed to influence an object's random numbers.

## Consequences

- **Old models and results are unchanged.** A file without streams numbers its instances by position when read (0, 1, 2…), and the formula is the one used before with the position replaced by the stream, so every existing model gives exactly the results it gave, and stored runs stay reproducible. `models/demo.json` was saved once with its streams written in.
- **Giving out streams:** a new or duplicated instance, or one written without a stream, gets one more than the highest in the model. Removing an instance leaves a gap and renumbers nothing. Two instances with one stream are a validation error. Copying a model file keeps its streams, so a variant made from a copy stays paired with its original.
- **`compare_runs`** pairs every figure by observation when both runs used the same seed and number of observations, which is valid whatever the models, and reports per object whether it drew the same random numbers (the same stream in both). `edit_model`'s warning that removing an instance breaks pairing is gone.
- **The cost:** one stream per object on the model description and the run, read once at Init. Nothing per event. A model file grows by one line per instance, about 8% for the benchmark's 6,006-object model, and `edit_model` on it slows by as much.
- **What can still break pairing:** changing a stream by hand, or `write_model` with a text whose streams differ, such as a model written afresh. A model built from scratch in two separate files doesn't share streams unless they were copied.
