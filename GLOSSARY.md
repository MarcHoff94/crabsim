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
The control that stops shown time advancing while everything the user sees stays exactly as it is. Start continues from there. An MU already gliding to its new object finishes the glide, since the move itself has already happened.
_Avoid_: Pause, Stop (those happen to a material-flow object)

**Reset**:
The control that returns every object to its default state, ready for a new simulation run. The objects stay in the model. If the user chose to keep changes, settings changed by interventions, such as a process time, first become the new default state, while momentary interventions such as a Pause are dropped. This fixed part cannot be changed, and users may later add their own behaviour after it.
_Avoid_: Init, delete, clear

**Headless run**:
A simulation run advanced as fast as the machine allows, with no live session or view attached. It is not a separate mode: live sessions and views are optional layers on top of a simulation run.
_Avoid_: Fast mode, batch mode

**Model description**:
The objects of a model and how they connect. One description can build any number of simulation runs.

**Replication**:
One simulation run of a model that differs from the model's other runs only in its seed.

**Distributed simulation**:
Running many replications at the same time across CPU cores. Each simulation run stays on one thread; a single run is never split up.

**Live session**:
A simulation run controlled while it goes, on its own thread. It takes Start, Halt, Reset and a speed, and reports occurrences, state, its status and errors.

**Picture**:
What the user currently knows about a simulation run at shown time, rebuilt from a full state and occurrences. It says what is where and since when, and nothing about how that is drawn.
_Avoid_: Layout image (a picture of the real site)

**Instant**:
One point in simulation time and everything that happens at it. Instants are as far apart as the events make them.
_Avoid_: Time step, tick

**Computed time**:
How far a simulation run has got. In a live session it can be ahead of shown time.

**Shown time**:
How far a live session has released results to the user. Everything the user experiences follows it, including Halt and errors.

**Time mode**:
How shown time passes between instants: continuously at the session's speed, or skipping, where shown time jumps over the rest of a long gap. It changes nothing in the simulation run.

**Checkpoint**:
A saved copy of a simulation run's complete state at one instant, from which the run can continue exactly as it would have.
_Avoid_: Undo, snapshot

**Intervention**:
A change made from outside to a simulation run in progress, such as switching a Pause on or off. It takes effect at shown time and discards any future already computed. The same seed and the same interventions give the same result.

**Speed**:
How much shown time passes per unit of wall time in a live session. Changing it never makes shown time jump.

**Zero-time loop**:
Events that keep happening without simulation time advancing. A simulation run reports it as an error rather than looping forever.

**View**:
One way of drawing a model and its picture. The 2D view is for building and debugging, and the 3D view is for showing and watching. Both show the same model.

**Layout image**:
An image of the real site, placed under the model at a known scale, so objects can be positioned on it. It has no effect on results.
_Avoid_: Picture (what the user knows about a run)

**Dimensions**:
An object's real size in metres: length, width and height, plus its rotation around the vertical axis. They are visual only and have no effect on results. MU moves stay instant whatever the distance.

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
An unplanned interruption whose timing is random, unless the user forces it. Failures and pauses are independent of each other.
_Avoid_: Breakdown, fault

**Forced failure**:
A failure the user starts and ends by hand, as an intervention, to see what happens if an object breaks down now. It counts as a failure against availability, and random failures carry on underneath it.
_Avoid_: Pause (planned, and does not count against availability)

**Pause**:
A planned interruption commanded from outside, such as a shift break or scheduled maintenance. It does not count against availability.
_Avoid_: Planned failure

**Stop**:
The state of a material-flow object while at least one interruption is active. A whole Station stops at once, whatever its capacity.

**Availability**:
The share of time an object is not failed, counting only time on its time basis plus time failed. Pauses do not count against it.

**Achieved availability**:
The availability a simulation run actually produced, as opposed to the value configured for the object.

**Time basis**:
The kind of time that counts down to an object's next failure: simulation time, working time or processing time. Not every object needs all three.

**Simulation time**:
The default time basis: all time counts, whatever the object is doing.

**Working time**:
A time basis where all time counts except while the object is paused, including time spent idle or blocked.

**Processing time**:
A time basis where only time spent actively working on at least one MU counts, once however many MUs are in work.
_Avoid_: Process time (how long one MU takes, not a basis for failures)

### Measuring

**Statistics**:
The fixed-size numbers every material-flow object keeps about a simulation run. They are always on, and their size does not depend on how long the run is.

**Busy**:
The state of an object doing what it is there for: a Station working on at least one MU, a Source waiting out its interval, a Buffer holding at least one MU.
_Avoid_: Processing (a Buffer does no work, and Processing time is a time basis)

**Blocked**:
The state of an object holding an MU that is ready to leave but that no successor can accept.

**Idle**:
The state of an object that is not busy, blocked or stopped. A partly filled batch start Station waiting to start is idle.

**MU time**:
MU count multiplied by time, kept per MU state: busy, blocked, waiting to start or stopped. Unlike an object's state, which is one at a time, it shows what each MU in a multi-MU object is doing. Judge utilisation and bottlenecks from it.

**Bottleneck**:
The object limiting a material flow: what is upstream of it is mostly blocked, and what is downstream of it is mostly starved.

**Starved**:
Having empty capacity because no MU has arrived.

**Lead time**:
How long an MU spends in the model: from leaving its Source until entering a Drain. A starting MU counts from Init.
_Avoid_: Cycle time, throughput time

**Exit interval**:
The time between two consecutive MUs entering the same Drain.
_Avoid_: Takt (a target, not a measurement)

**Throughput**:
How many MUs per hour a Drain takes, measured from its first exit up to now, so it falls while no MUs arrive.

**Warm-up**:
The start of a simulation run, left out of the statistics by clearing them when it ends. Clearing statistics changes nothing else in the run.
_Avoid_: Reset (that returns every object to its default state)

**Recording**:
Keeping the occurrences of chosen kinds from a simulation run. It is opt-in, and it is the only way to get data per MU.
