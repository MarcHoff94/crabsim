# Destinations are places in the model, reached by its connections

An MU's destination is the name of an instance, or of a class standing for any of its instances, never a label of its own. An object whose distribution is **by destination** narrows its successors to those from which the MU's next destination can be reached along the model's connections, and its own strategy, round robin or weighted, chooses among them. A cross conveyor does the same among its exit sides, with each exit side's list of destinations it takes, and sends an MU no exit takes straight through. Which successors reach which destination is worked out once, when a run is built, as a small table per object, so routing costs a lookup per MU and no search. We decided it on 2026-11-23 in the design round for cross conveyors (spec issue 148), because a destination that names no place leaves an object with two ways to it no means to choose, and a table of ways the user keeps by hand goes wrong whenever the model changes.

## Considered Options

- **A list of destination names of the model's own,** such as "North", with a table per routing object mapping each to a successor. Destinations could stand for regions or order types, but a name reached by two ways gives no way to choose, and every table has to be kept in step with the layout by hand.
- **A number per MU.** Smallest, but unreadable in a model file and in the window.
- **Committing to one instance** of a class at the first decision, kept by the MU. It would stop a later object choosing another instance of the same class, but costs another field per MU. Left for later: with every object weighing by route, a route that is best from one object stays best from every object along it, so the decisions agree.

## Consequences

- **A destination none of an object's successors reaches** is a warning in Validate wherever the model file shows it. In a run, that MU is offered to every successor, so it never waits for good unseen. An MU with no destination left is offered to every successor too.
- **A way that reaches the destination but is blocked is waited for,** as any blocked successor is. Taking another way would send the MU wrong.
- **What reaches where** is found by searching back from each destination's instances along the connections, once per run, in time growing with destinations times connections. Faster ways to find it on very large models are a topic for later: it is a well-known problem on networks.
- **Models without destinations pay nothing:** the table is built only for objects whose distribution is by destination.
