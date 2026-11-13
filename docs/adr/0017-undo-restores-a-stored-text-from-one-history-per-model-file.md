# Undo restores a stored copy of a model file's text, from one history per file that holds everyone's changes

Every change crabsim makes to a model file is kept in that file's change history: an `edit_model` or `run_commands` batch, the file written over with `write_model`, and a change from outside accepted with `adopt_model`, by the user or an agent, in order, each with its author, time and wording. An entry points at the file's whole text before and after its change. Undo writes the earlier text back, and redo the later one. The history is one line with no branches: a new change drops what could be redone. It lives in `<models folder>/.crabsim/history.sqlite`, so it survives restarts and the window, the CLI and agents share it, and a file keeps its last 200 changes or 50 MB of text, whichever is less.

The user undoes anyone's changes. An agent undoes and redoes only its own latest changes, while it holds its claim, and stops at the first change by someone else.

We decided it on 2026-11-13 in a design round (spec issue 107), after building in the window (spec issue 87) had made every building action one batch, the unit undo steps through.

## Considered Options

- **Reverse changes,** an `edit_model` batch that undoes each batch. Small to store, but a removed instance wouldn't return as it was: its place in the file, its stream (ADR 0012) and its connections' order would have to be rebuilt by hand, change kind by change kind, and the file's hash would differ from before. A file written whole or changed outside can't be written as changes at all.
- **A history per user or agent,** so undo skips other people's changes. It breaks when an agent's change builds on the user's, such as a connection to a Station the user's undo would remove.
- **A history in the window's memory.** Nothing to store, but it can't hold what an agent changed while the window was closed, and it is gone on restart.
- **Letting agents undo anything.** Simpler, but an agent could take back the user's work. The user watches agents through their claims (ADR 0013), not the other way round.

## Consequences

- An undo is exact by construction: the text and its hash are what they were, so a stored run matches its model again, and runs before and after stay paired.
- It costs disk space: each change stores the file's text after it, once. On the 6,006-object model (2.3 MB), 7 changes took 18.8 MB, so its history holds about 20 changes, where a demo-sized model keeps all 200.
- It costs one write per change, and nothing in a run. Measured on 2026-11-13 on the 6,006-object model, median of 7 in release: one `edit_model` change kept in the history 741 ms, against 709 ms for one that changed nothing, an undo 55 ms and a redo 56 ms. An undo must never take longer than the edit it reverses, which the timing test checks.
- The history is only as good as its match with the file. One that doesn't end at the file's text any more, such as one restored from a backup, is dropped rather than used, and a history that can't be written never fails a change already saved.
- An undo is a change by whoever made it: an agent holding the file reads the model again before its next change, and the user's undo past an agent's claim is a take-over, as any change is.
- Deleting a file drops its history. A recoverable delete, and agents reading a model as it was at an entry, are left for later.
- Everything one command made is one step: entries carry a group, which a front-end opens around a command it carries out as several changes, such as a macro run with a count.
