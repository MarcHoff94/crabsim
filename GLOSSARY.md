# crabsim

A discrete-event simulator for modelling any material flow, such as a factory line, as real things that handle moveable units.

## Language

### Running a simulation

**Simulation run**:
One execution of a model, from its Init until it is reset or discarded.

**Init**:
The first step of every simulation run: time is set to 0, every object's init is called and the first events are generated. This fixed part cannot be changed, and users may later add their own behaviour after it.

**Start**:
The control that begins or continues advancing time, doing Init first if it has not happened yet.

**Halt**:
The control that stops shown time advancing while everything the user sees stays exactly as it is. Start continues from there.
_Avoid_: Pause, Stop (those happen to a material-flow object)

**Reset**:
The control that returns every object to its default state, ready for a new simulation run. The objects stay in the model. This fixed part cannot be changed, and users may later add their own behaviour after it.
_Avoid_: Init, delete, clear

**Headless run**:
A simulation run advanced as fast as the machine allows, with no live session or view attached. It is not a separate mode: live sessions and views are optional layers on top of a simulation run.
_Avoid_: Fast mode, batch mode

**Replication**:
One simulation run of a model that differs from the model's other runs only in its seed.

**Distributed simulation**:
Running many replications at the same time across CPU cores. Each simulation run stays on one thread; a single run is never split up.

**Live session**:
A simulation run controlled while it goes, on its own thread. It takes Start, Halt, Reset and a speed, and reports occurrences, state, its status and errors.

**Instant**:
One point in simulation time and everything that happens at it. Instants are as far apart as the events make them.
_Avoid_: Time step, tick

**Computed time**:
How far a simulation run has got. In a live session it can be ahead of shown time.

**Shown time**:
How far a live session has released results to the user. Everything the user experiences follows it, including Halt and errors.

**Checkpoint**:
A saved copy of a simulation run's complete state at one instant, from which the run can continue exactly as it would have.
_Avoid_: Undo, snapshot

**Intervention**:
A change made from outside to a simulation run in progress, such as switching a Pause on or off. It takes effect at shown time and discards any future already computed. The same seed and the same interventions give the same result.

**Speed**:
How much shown time passes per unit of wall time in a live session. Changing it never makes shown time jump.

**Zero-time loop**:
Events that keep happening without simulation time advancing. A simulation run reports it as an error rather than looping forever.

**Default state**:
The state a material-flow object is in before a simulation run's Init, as configured by the user, including any starting MUs.

**Occurrence**:
Something that happened in the model at a point in simulation time, such as an MU moving or a stop beginning.
_Avoid_: Event (events are the simulator's internal schedule)

### Material flow

**Moveable Unit (MU)**:
An item that travels through the modelled material flow. Its identity is unique across the whole model.
_Avoid_: Part, product, item

**Material-flow object**:
Anything in the model that handles MUs and represents a real thing: a Source, Station, Buffer or Drain.
_Avoid_: Station (when meaning any object), node

**Source**:
A material-flow object that creates MUs.

**Station**:
A material-flow object that works on MUs for a process time. It is one machine, even when its capacity lets it work on several MUs at once.
_Avoid_: ParallelStation, parallel machines

**Buffer**:
A material-flow object that holds MUs without working on them, optionally for a dwelling time.

**Drain**:
A material-flow object that takes MUs out of the model.

**Capacity**:
How many MUs a Station or Buffer can hold at the same time.

### Starting work

**Batch start**:
A Station's work begins only once it is full, and all its MUs are worked on together.

**Individual start**:
A Station's work on each MU begins when that MU enters.

### Moving MUs

**Move**:
An MU passing from one material-flow object to the next. It takes no simulation time.

**Push**:
The object holding a finished MU sends it to a successor that can accept it. This is the current flow concept.

**Pull**:
A successor that can receive an MU chooses which MU it wants from its predecessor. Planned, not yet part of the model.

**Exit strategy**:
The rule for which MU leaves an object next. By default, the earliest finished MU leaves first, with ties broken by entry order.

**Distribution strategy**:
The rule for which successor receives the MU that is leaving.

### Timing

**Process time**:
How long a station needs to work on one MU if nothing interrupts it.

**Completion time**:
The moment a station finishes working on an MU: its entry time plus the process time, moved later by the time the station spends stopped during that work.
_Avoid_: Exit time (an MU can finish but still have to wait before it can leave)

**Dwelling time**:
How long a Buffer holds each MU, counted individually from that MU's entry. By default a stop of the Buffer does not affect it, and this can be configured.
_Avoid_: Process time (a Buffer does no work)

### Interruptions

**Interruption**:
A cause that stops a material-flow object: a failure or a pause. Interruptions can overlap.

**Failure**:
An unplanned interruption whose timing is random. Failures and pauses are independent of each other.
_Avoid_: Breakdown, fault

**Pause**:
A planned interruption commanded from outside, such as a shift break or scheduled maintenance. It does not count against availability.
_Avoid_: Planned failure

**Stop**:
The state of a material-flow object while at least one interruption is active. A whole Station stops at once, whatever its capacity.

**Availability**:
The share of an object's time basis during which it is not failed. Pauses do not count against it.

**Time basis**:
The kind of time that counts down to an object's next failure: simulation time, working time or processing time. Not every object needs all three.

**Simulation time**:
The default time basis: all time counts, whatever the object is doing.

**Working time**:
A time basis where all time counts except while the object is paused, including time spent idle or blocked.

**Processing time**:
A time basis where only time spent actively working on at least one MU counts, once however many MUs are in work.
_Avoid_: Process time (how long one MU takes, not a basis for failures)
