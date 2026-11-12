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
The control that returns every object to its default state, ready for a new simulation run. The objects stay in the model. If the user chose to keep changes, settings changed by interventions, such as a process time, first become the new default state, while momentary interventions such as a Pause are dropped. This fixed part cannot be changed, and users may later add their own behaviour after it.
_Avoid_: Init, delete, clear

**Headless run**:
A simulation run advanced as fast as the machine allows, with no live session or view attached. It is not a separate mode: live sessions and views are optional layers on top of a simulation run.
_Avoid_: Fast mode, batch mode

**Model description**:
The objects of a model and how they connect. One description can build any number of simulation runs.

**Model file**:
A model description written as text, which people and agents can read and write directly. It holds the model only: settings of a simulation run, such as its seed and length, are not part of it.

**Canonical form**:
The one way crabsim writes a model file, so that saving, loading and saving again gives the same file. A file written by hand may be reformatted the first time crabsim saves it.

**Experiment**:
A model with chosen attribute values and a defined random seed, run as one or more observations.

**Observation**:
One simulation run of an experiment. Observations of one experiment differ only in their seeds, which all follow from the experiment's seed.
_Avoid_: Replication

**Paired runs**:
Two runs with the same seed and the same number of observations, so that observation *k* of one can be compared with observation *k* of the other. An instance with the same stream in both models then drew the same random numbers, which gives the difference a far narrower interval than either run alone, and is what makes comparing two variants of a model cheap.
_Avoid_: Matched runs

**Stream**:
The number an instance gets when it's placed in a model, which together with the run's seed gives its random numbers. It stays whatever else changes in the model, so only changing the seed changes an instance's random numbers, unless the user changes its stream on purpose.
_Avoid_: Object seed (the seed belongs to the run)

**Simulation study**:
Several experiments of one model, each changing different attribute values, each run as a defined number of observations. By default, observation *k* of every experiment uses the same seed, so the experiments differ by their values and not by chance.

**Distributed simulation**:
Running many simulation runs, such as observations, at the same time across CPU cores. Each simulation run stays on one thread; a single run is never split up.

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

**End time**:
A simulation time set on a live session at which the run halts by itself, exactly there, such as 30d. Start is refused there until it is moved or cleared. It isn't an intervention: it changes when a run stops, not what happens in it.
_Avoid_: Stop time (a stop is an object's state when interrupted)

**Time mode**:
How shown time passes between instants: continuously at the session's speed, or skipping, where shown time jumps over the rest of a long gap. It changes nothing in the simulation run.

**Checkpoint**:
A saved copy of a simulation run's complete state at one instant, from which the run can continue exactly as it would have.
_Avoid_: Undo, snapshot

**Intervention**:
A change made from outside to a simulation run in progress, such as switching a Pause on or off or changing an attribute. Every attribute a user can set can also be changed by an intervention. It takes effect at shown time and discards any future already computed. The same seed and the same interventions give the same result.

**Speed**:
How much shown time passes per unit of wall time in a live session. Changing it never makes shown time jump.

**Animation**:
A live session releasing what happens at the chosen speed, so a view can draw every move and change of state. With animation off, the session computes as fast as the machine allows with no speed limit, and a view shows only the time and the objects' states a few times a second. Going back and stepping still work, and results are the same either way.
_Avoid_: Headless run (that has no live session at all)

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

**Dummy Station**:
A Station with a process time of 0. It takes no time, and is there so that something can happen as an MU passes, such as user code at a hook.

**Buffer**:
A material-flow object that holds MUs without working on them, optionally for a dwelling time.

**Drain**:
A material-flow object that takes MUs out of the model.

**Capacity**:
How many MUs a Station or Buffer can hold at the same time.

**Connection**:
The way from one material-flow object to another along which MUs pass. It is a thing of its own in a model, with its own properties: its exit, its place in the order its object offers MUs to its successors, its entrance, its place among the connections into the object it leads to, and a description. Moving along it takes no time: travel time belongs to objects such as a conveyor.
_Avoid_: Edge (the graph view's word), link

**Successor**:
An object a connection leads to from a given object. The object it comes from is the successor's predecessor.

### Classes

**Attribute**:
Anything about a class or instance that a user can set, such as a process time, a capacity or the choice of exit strategy. How an object works inside is hidden: users change attributes, and later add behaviour only at defined hooks.
_Avoid_: Parameter

**Class**:
What a material-flow object is. Source, Station, Buffer and Drain are built-in classes, and users make their own by deriving from them. A model can also change a built-in class's values for itself alone, with a class of the built-in's name and no parent, which everything built on it inherits.

**Instance**:
A material-flow object placed in a model, made from a class. Its name, position and connections are its own, and every other attribute, including its dimensions and look, it can inherit.

**Derived class**:
A class made from another class, which stays linked to it: it inherits every value it hasn't set itself, including any later change to its parent. Classes can be derived from derived classes.

**Duplicate**:
A copy of a class or instance with no link to the original.
_Avoid_: Derived class (that stays linked)

**Inheritance**:
Taking a value from the parent class rather than setting it, decided separately for each attribute. Setting a value cuts the inheritance of that one value. A change to a class reaches every instance and derived class whose inheritance of that value isn't cut, during a simulation run too.

### Starting work

**Batch start**:
A Station's work begins only once it is full, and all its MUs are worked on together.

**Individual start**:
A Station's work on each MU begins when that MU enters.

### Moving MUs

**Move**:
An MU passing from one material-flow object to the next. It takes no simulation time, so it is shown as an instant jump. Transport that takes time would be its own material-flow object, such as a conveyor.

**Push**:
The object holding a finished MU sends it to a successor that can accept it. This is the current flow concept.

**Pull**:
A successor that can receive an MU chooses which MU it wants from its predecessor. Planned, not yet part of the model.

**Exit strategy**:
The rule for which MU leaves an object next. By default, the earliest finished MU leaves first, with ties broken by entry order.

**Distribution strategy**:
The rule for which successor receives the MU that is leaving, set per Source or Station as its `distribution` (ADR 0016). **Round robin**, the default, offers each MU to the successors in turn. **Weighted** ranks the successors by a weight, such as each one's best route onward to a Drain by process time, and offers the MU to the best one that can take it, those ranking equal taking turns.

**Warning**:
Something in a model that can run but is probably a mistake, such as an object no Source reaches. `validate_model` and the window's Validate button list warnings beside problems. Unlike a problem, a warning never stops a run.

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

**State note**:
A model's own explanation of what one state means for a class or instance, such as Blocked meaning "the robot waits for the conveyor". It is inherited like any attribute and shown beside crabsim's fixed definition of the state, never instead of it. It cannot change what is measured or the state's colour.

**Blocked**:
The state of an object holding an MU that is ready to leave but that no successor can accept.

**Idle**:
The state of an object that is not busy, blocked or stopped. A partly filled batch start Station waiting to start is idle.

**MU time**:
MU count multiplied by time, kept per MU state: busy, blocked, waiting to start or stopped. Unlike an object's state, which is one at a time, it shows what each MU in a multi-MU object is doing. Judge utilisation and bottlenecks from it.

**Bottleneck**:
The object limiting a material flow: what is upstream of it is mostly blocked, and what is downstream of it is mostly starved.

**Upstream**:
Every object that can pass MUs to a given object, directly or through any number of others.

**Downstream**:
Every object a given object can pass MUs to, directly or through any number of others.

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

**Measuring period**:
A stretch of simulated time whose figures count as one sample, so that one simulation run gives a confidence interval from the spread between its periods. Its length is a setting of the run, not part of the model, and an hour by default.
_Avoid_: Batch (a batch start is a Station starting work on several MUs at once)

**Recording**:
Keeping the occurrences of chosen kinds from a simulation run. It is opt-in, and it is the only way to get data per MU.

### Working with agents

**Agent**:
An AI that uses crabsim through its operations, either from a harness outside crabsim, such as a coding assistant, or inside crabsim itself. It acts from outside a simulation run, as a person does.

**Operation**:
One thing crabsim can do for a person or an agent, such as validating a model or starting a simulation run. Anything a person can do is an operation an agent can use too. Several changes to one model, such as adding an instance and connecting it, can be made together as one operation, all or nothing.
_Avoid_: Tool (the word one agent protocol uses for an operation), command (what a live session takes)

**Editor**:
The user or the one agent currently allowed to change a model file through crabsim's operations. An agent becomes one by a claim, and the user can take the editor right at any time, which pauses the agent's claim.
_Avoid_: Lock, owner, text editor (a program such as a code editor)

**Claim**:
An agent's hold on one model file, which it needs for every change: granted at once if nobody holds the file, refused otherwise. It says the agent's **goal** and what it is **doing** now, which the agent updates as it goes. It ends when the agent releases it, its process ends, it stays unused for too long or the user revokes it. While the user has taken the right, it is **paused**.
_Avoid_: Edit queue (there is none), lock

**User**:
The one person working with a models folder, through either the UI or the CLI, who controls who edits its model files. Only one UI can have a models folder open at a time: working in a second window means working on an independent copy of the folder.
_Avoid_: Agent (never a user)

**Models folder**:
The one folder whose model files crabsim's operations may touch, given when the MCP server or the CLI starts. It also holds crabsim's own `.crabsim` folder, with the results store. A folder holding source code is refused.

**Job**:
A simulation run started through the operation catalogue: the observations of one experiment, run in the background with an id, such as `run-3`, that can be asked for its status, cancelled and read.
_Avoid_: Task, batch

**Results store**:
The database in a models folder that keeps every job's settings, the model it ran and its results, for people and agents alike, until they are forgotten.

**Skill**:
Written instructions that teach an agent one kind of task with crabsim's operations, such as finding the bottleneck of a model.
