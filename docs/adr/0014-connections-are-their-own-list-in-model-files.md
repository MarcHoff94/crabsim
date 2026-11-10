# Connections are their own list in model files, from format version 2

A model file lists its connections at the top level, beside its classes and instances, each with both ends, its exit (its place among the connections out of its object, the order MUs are offered to successors), its entrance (its place among the connections into the object it leads to) and an optional description. Both orders are explicit numbers, 1 to n per object with no gaps or repeats, and the list's own order means nothing. Instances no longer carry `successors`. This is format version 2. Version 1 is refused, with no code kept to read or convert it: the few files there were are converted once by hand, and the runs stored from them are deleted.

We changed it while designing how a model is built in the window, on 2026-11-10. The owner wants connections to be things of their own, edges with properties, because a model in crabsim is meant to be a graph that people and agents query and run graph algorithms on, such as weighted paths. A connection already has two orders that can matter: the exit order today, and the entrance order once an entrance rule such as pull or priorities reads it, which is coming soon. The two can't both come from one list's order: with A and B both connected to X and Y, A's exits X then Y, B's Y then X, X taking B before A and Y taking A before B, no order of the four connections gives all four at once. So the entrance order has to be written on the connection, and the exit order is written the same way, so each connection reads complete on its own and sorting or editing the list by hand can't change a result.

## Considered Options

- **Connections inside the instances,** as `successors` lists whose entries may grow into objects such as `{ "to": "Pack", "description": "via the lift" }`. No new version and nothing to convert, an instance and its outgoing connections read as one, and copying an instance carries them. But a connection belongs to two objects and would be written in one: the entrance order and any property of the connection as a whole sit with the "from" side, incoming connections are found only by searching every instance, and a graph has to be flattened out of the instances before it can be used.
- **The exit order implicit, from the list's order,** with only the entrance written. Shorter, but sorting the list by hand would silently change results, and the two orders would be written in two different ways.
- **Reading version 1 alongside version 2.** It would keep older files and the runs stored from them readable. The owner chose not to carry that code: every file and run so far was a demo for building crabsim, and the few files are quick to convert.

## Consequences

- Every model file is converted once, every model's `model_hash` changes with it, and the results stores' runs are deleted, so no stored run can be read back with its model.
- An instance read alone doesn't show its connections. Removing or renaming an instance updates the connection list, which `edit_model` does in the same change.
- Adding, removing or reordering a connection renumbers the exits or entrances of the objects at its ends. `edit_model` and the window do this, so people never number connections themselves, and the file's check refuses gaps and repeats from anyone writing by hand.
- `query_model` and `export_model` read connections as the file has them, edges with properties, and properties added later, such as a distribution share, have one place to go.
- The entrance order is stored and edited before anything reads it, labelled as not affecting results until an entrance rule does.
