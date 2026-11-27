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
_Avoid_: Pause, Stop (Paused and Stopped are interruptions of a material-flow object)

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
_Avoid_: Stop time (Stopped is an interruption by hand)

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

**Inspector**:
The short summary of the selection docked beside the canvas: what it is, what it's doing and its key figures, each attribute in one line. Everything else about an instance is in its object window.
_Avoid_: Properties panel

**Object window**:
A window with everything about one instance: its state, what it holds, its statistics and every attribute, which can be changed there. Any number can be open, one per instance, and each stays with its instance through renames, moves and runs until the instance is removed.
_Avoid_: Popup, detail view

**Statistics window**:
A window with a run's results at shown time: each Drain's throughput and lead time, the likely bottleneck, and every Station's and Buffer's figures.
_Avoid_: Results tab (what it was before it got a window of its own), dashboard

**Layout image**:
An image of the real site, placed under the model at a known scale, so objects can be positioned on it. It has no effect on results.
_Avoid_: Picture (what the user knows about a run)

**Dimensions**:
An object's real size in metres: length, width and height, plus its rotation around the vertical axis. For a Source, Station, Buffer or Drain they are visual only and have no effect on results: MU moves stay instant whatever the distance. A geometry-aware object, such as a conveyor, reads its own dimensions and path, and what it can take depends on them.

**Default state**:
The state a material-flow object is in before a simulation run's Init, as configured by the user, including any starting MUs.

**Occurrence**:
Something that happened in the model at a point in simulation time, such as an MU moving or an interruption beginning.
_Avoid_: Event (events are the simulator's internal schedule)

### Material flow

**Moveable Unit (MU)**:
An item that travels through the modelled material flow. Its identity is unique across the whole model.
_Avoid_: Part, product, item

**MU class**:
What an MU is, kept in the class library beside the object classes. It gives the MU its size, a footprint of length and width in metres, and later other values, such as its own motion limits (a self-driven MU) or what it can carry (a pallet). Every Source is set to one MU class, and every MU it creates is of that class. The Source may change values of the MUs it creates, but never picks no class.
_Avoid_: MU type

**Booking point**:
A point in an MU's footprint. When it crosses from one geometry-aware object to the next, the MU officially moves: until then it belongs to the first, though it may already take up space on both.
_Avoid_: Transfer point

**Destination**:
Where an MU is meant to go next: an instance, or a class standing for any of its instances. It is the current step of the MU's work plan, or a Station's detour while the MU is on it. An object whose distribution strategy is by destination sends the MU towards a successor from which the destination can be reached (ADR 0025).

**Work plan**:
The steps an MU of an MU class goes through, in order, each a destination and optionally a process time, which a Source may replace for its own MUs. An MU entering a Station its current step names is worked on for the step's process time, or the Station's own without one, and moves on to its next step when it leaves. At a Station its step doesn't name, it is worked on for the Station's own time and counts as off plan (ADR 0026).
_Avoid_: Routing (a conveyor's path), recipe

**Detour**:
A destination a Station sends its MUs to before they carry on with their work plan, such as Rework. It is cleared when the MU reaches it.

**Geometry-aware object**:
A material-flow object whose results depend on real sizes and paths, such as a conveyor, which cannot take an MU wider than itself. It reads the MU's size. A Source, Station, Buffer or Drain is not one: they take any MU.

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
A material-flow object that holds MUs without working on them, optionally for a dwelling time, up to its capacity or without limit. It passes on one MU at a time, chosen by its exit strategy among those whose dwelling time is over. It is idle when empty, holding, or blocked.

**Drain**:
A material-flow object that takes MUs out of the model.

**Capacity**:
How many MUs a Station or Buffer can hold at the same time. A Station's is a whole number from 1, and 1 by default: one machine working on that many MUs at once. A Buffer's may be unlimited, which is what a new Buffer has: it then takes every MU and grows as far as it's filled (ADR 0019).

**Start**:
When a Station with a capacity above 1 begins work on its MUs: individual, each MU on its entry, or batch, all of them once the Station is full, after which it takes no MU until it is completely empty.

**Connection**:
The way from one material-flow object to another along which MUs pass. It is a thing of its own in a model, with its own properties: its exit, its place in the order its object offers MUs to its successors, its entrance, its place among the connections into the object it leads to, and a description. Moving along it takes no time: travel time belongs to objects such as a conveyor.
_Avoid_: Edge (the graph view's word), link

**Successor**:
An object a connection leads to from a given object. The object it comes from is the successor's predecessor.

### Conveyors

**Conveyor**:
A geometry-aware object that carries MUs along its path from its start to its end, holding as many as fit along its length, as wide as its width allows. MUs enter only at its start and leave only at its end. Its direction is set before a run. An MU never runs into the one ahead of it: when that one stops, it stops too, whatever its own conveyor is doing.
_Avoid_: Belt, track (a conveyor of either drive)

**Accumulating**:
What a conveyor does with the MUs behind one that stops. **Accumulating**, the default: they close up to it, keeping the gap, and stop one by one while the rest move on. **Non-accumulating** (`accumulating` false): every MU on the conveyor stops as a whole when its first can't leave, wherever it is, and they all start again together, as far apart as they were.

**Drive**:
What moves the MUs on a conveyor. **Conveyor-driven**: the conveyor's own drive moves every MU on it at its constant speed, as a belt or roller conveyor does, an MU standing while it is held back and moving at that speed again at once (ADR 0028). **MU-driven**: each MU moves by its own top speed, acceleration and deceleration, a trapezoidal profile without jerk, plus an optional start and stop time per trip, as a shuttle or carrier does, and the conveyor is only the way it follows.

**Path**:
The line a conveyor's MUs follow, made of segments joined without kinks, any of them rising or falling. Only its length, and the limits it sets, such as its tightest bend and steepest incline, affect results: a path that breaks its conveyor's `min_radius` or `max_incline` can't run.

**Segment**:
One piece of a path: a straight, an arc, a transition curve or a helix, each of which may rise or fall. Its length is in plan, as it is laid out from above, and the conveyor's length counts each segment in 3D, rise included. A path starts at its conveyor's position, heading along its rotation.

**Transition curve**:
A segment whose bend changes evenly along its length (a clothoid), from the bend the segment before it ended with to a radius it names, or back to straight, so a curve can begin and end without a jolt. Optional: a straight may also run directly into an arc.

**Direction**:
Which way a conveyor's MUs travel along its path: forward, from its first segment to its last, or backward. Its start and end are where its MUs enter and leave, so they swap with its direction. It is set before a run.

**Attach**:
To move and turn an object, and set its height, so that its end meets a connected neighbour's: a conveyor's start onto the end of the one before it, or a Station's centre half its length past a conveyor's end. Nothing requires neighbours to meet: attaching is how the user lines them up when they want to.

**Straddle**:
An MU straddles two geometry-aware objects while its footprint is on both. It belongs to the one its booking point is on, and moves at the slower of the two speeds unless one is interrupted, when the one it belongs to decides.

**Cross conveyor**:
A geometry-aware routing object with four sides, holding one MU at a time. Each exit side lists the destinations it takes. An MU whose next destination one or more exit sides list leaves by one of them, chosen by the cross conveyor's own distribution strategy: it stops at the centre for the switch time and leaves sideways, keeping its orientation. Any other MU passes straight through to the side opposite the one it came in by, without stopping.
_Avoid_: Diverter, transfer unit

**Crosswise**:
How an MU lies after a cross conveyor turned it: across its way, its width along it and its length across. It stays so on the conveyors after it until another cross conveyor turns it again, or an object without geometry takes it and passes it on lengthwise.

**Side**:
One of a cross conveyor's four physical sides: front, back, left or right. Connections attach to sides. A side with incoming connections is an entry side, one with outgoing connections an exit side, never both.
_Avoid_: Entrance, exit (a connection's place in an order)

**Receiving**:
What an object without geometry, such as a Station, is doing between accepting an MU from a conveyor and that MU's booking point arriving. It is a state of its own, with its own colour and share of time. Its place is taken from the acceptance. A transfer once begun always finishes, even if that object is interrupted meanwhile.

**Reversible conveyor**:
A conveyor whose direction can change during a run, its MUs slowing, stopping and moving back, and its two ends trading places as start and end.

**Switch time**:
How long an MU turning at a cross conveyor stands at its centre before leaving sideways, such as a lift raising it onto cross rollers.

**Release time**:
How long a cross conveyor takes no MU after one that turned has left, such as its lift going back down. 0 by default.

**Turntable**:
A cross conveyor that turns the MU with it as it changes direction, so the MU leaves in a new orientation.

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
The rule for which MU leaves an object next. By default, the earliest finished MU leaves first, with ties broken by entry order. A Buffer's is **first in, first out**, the default, or **last in, first out**, each among the MUs whose dwelling time is over.

**Distribution strategy**:
The rule for which successor receives the MU that is leaving, set per Source or Station as its `distribution` (ADR 0016). **Round robin**, the default, offers each MU to the successors in turn. **Weighted** ranks the successors by a weight, such as each one's best route onward to a Drain by process time, and offers the MU to the best one that can take it, those ranking equal taking turns. **By destination** narrows the successors to those from which the MU's next destination can be reached along the connections, and then round robin or weighted chooses among them. An MU with no destination left, or one no successor reaches, is offered to every successor. An object may instead have a distribution hook, user code choosing one successor, holding the MU or leaving the choice to its strategy (ADR 0029).

**Macro**:
A named list of commands in the command line's language, run as `@name` with arguments for its `$1`, `$2` and so on. Macros live in `macros.json` beside the models, each with its author. An agent's macros last for its work and are then archived, unless the user keeps one.

**Warning**:
Something in a model that can run but is probably a mistake, such as an object no Source reaches. `validate_model` and the window's Validate button list warnings beside problems. Unlike a problem, a warning never stops a run.

### Timing

**Process time**:
How long a station needs to work on one MU if nothing interrupts it.

**Completion time**:
The moment a station finishes working on an MU: its entry time plus the process time, moved later by the time the station spends interrupted during that work.
_Avoid_: Exit time (an MU can finish but still have to wait before it can leave)

**Dwelling time**:
How long a Buffer holds each MU, counted individually from that MU's entry. By default it keeps running while the Buffer is interrupted, and `stop_freezes_dwelling` freezes it instead.
_Avoid_: Process time (a Buffer does no work)

### Interruptions

**Interruption**:
A cause that interrupts a material-flow object: a failure, a stop by hand or a pause, from most to least serious. Interruptions can overlap. Every type of object can be interrupted (ADR 0020).

**Failure**:
An unplanned interruption whose timing is random, drawn from one of the object's failure profiles. Its state is Failed. Failures, stops and pauses are independent of each other.
_Avoid_: Breakdown, fault

**Failure profile**:
One named source of failures on an object, such as "Tool wear" or "Jam", with its own availability, repair time, time basis and random stream. An object can have several, each name unique within it, and they are independent: their clocks and their failures don't affect each other.

**Stopped**:
An interruption a person starts and ends by hand, as an intervention, to see what happens if an object breaks down now. It counts as a failure against availability and in the statistics, and random failures carry on underneath it.
_Avoid_: Forced failure (its old name), Pause (planned, and does not count against availability)

**Pause**:
A planned interruption commanded from outside, such as a shift break or scheduled maintenance. It does not count against availability.
_Avoid_: Planned failure

**Interrupted**:
The state of a material-flow object while at least one interruption is active. A whole Station is interrupted at once, whatever its capacity. A Station's work freezes, a Source's next creation moves later, a Buffer takes and releases nothing, and a Drain refuses MUs.
_Avoid_: Stop, Down

**Availability**:
The share of time an object is not failed, counting only time on its time basis plus time failed, for each failure profile on its own basis. Stopped time counts as failed, and pauses do not count against it.

**Achieved availability**:
The availability a simulation run actually produced, as opposed to the value configured for the object.

**Time basis**:
The kind of time that counts down to a failure profile's next failure: simulation time, working time or processing time. Stations and Sources offer all three, Buffers and Drains simulation and working time.

**Simulation time**:
The default time basis: all time counts, whatever the object is doing.

**Working time**:
A time basis where all time counts except while the object is paused, including time spent idle or blocked.

**Processing time**:
A time basis where only time spent busy counts, once however many MUs are in work. A Source is busy while it waits out its interval.
_Avoid_: Process time (how long one MU takes, not a basis for failures)

### Measuring

**Statistics**:
The fixed-size numbers every material-flow object keeps about a simulation run. They are always on, and their size does not depend on how long the run is.

**Busy**:
The state of an object doing its work: a Station working on at least one MU, a Source waiting out its interval. A Station with a capacity above 1 is busy in one of three ways that don't overlap: full, with every place in work; not full, with free places and nothing waiting to leave; and partly blocked, working while a finished MU can't leave, which outranks the other two. A conveyor is busy in the same three ways, since what fits on it is its capacity: full when no MU could enter at its start, not full when one could, and partly blocked when its first MU waits at the end while others still move. The likely bottleneck, a Station or a conveyor, counts only busy and full.
_Avoid_: Processing (Processing time is a time basis)

**Fill histogram**:
How long a Buffer held each number of MUs, in 64 bars. Each bar covers one number of MUs until more come than the bars reach, and then twice as many, or as many as the model sets, the last bar then taking everything beyond. It gives a Buffer's median fill.

**Holding**:
The state of a Buffer with at least one MU in it, none of which is kept from leaving: each is still in its dwelling time. A Buffer does no work, so it is holding rather than busy, with its own colour. In the statistics its holding time is kept where other objects keep busy time.

**State note**:
A model's own explanation of what one state means for a class or instance, such as Blocked meaning "the robot waits for the conveyor". It is inherited like any attribute and shown beside crabsim's fixed definition of the state, never instead of it. It cannot change what is measured or the state's colour.

**Blocked**:
The state of an object holding an MU that is ready to leave but that no successor can accept. It waits in line at each of its successors, and the line is ordered by how long each predecessor's MU has been ready to leave, whichever object holds it (ADR 0018).

**Idle**:
The state of an object that is not busy, blocked or interrupted. A partly filled batch start Station waiting to start is idle.

**MU time**:
MU count multiplied by time, kept per MU state: busy, blocked, waiting to start or interrupted. Unlike an object's state, which is one at a time, it shows what each MU in a multi-MU object is doing. Judge utilisation and bottlenecks from it.

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

### User code

**User code**:
Code a user, or an agent they ask, attaches to a model to decide and act where crabsim's own rules don't reach, such as which successor an MU goes to. It runs inside crabsim's sandbox, sees the run through addresses and changes it only through actions (ADR 0029).
_Avoid_: Script, plugin, macro (a list of commands that changes a model file)

**Code module**:
A compiled WebAssembly file in the models folder's `code/` folder, holding functions that hooks name. The user compiles it, from Rust with the `crabsim-guest` crate, or later any language that compiles to WebAssembly, and crabsim reads it and never writes it. It declares its functions and their parameters, and the version of crabsim's interface it was built against. Its sources may sit beside it, so people and agents can read what ran.
_Avoid_: Library, plugin, DLL

**Hook**:
A point in a simulation run where a function of a code module runs, named on a class or instance with its parameters and inherited as a whole. Every object that holds an MU has three: **on entry**, as an MU comes in; **on exit**, as it leaves; and **distribution**, which chooses the successor. The model has four more: before and after Init, and before and after Reset, around crabsim's own. A hook runs alone, and any hook its actions set off runs after it, at the same time.
_Avoid_: Callback, event (an engine internal), trigger

**Trigger point**:
Where along an MU a hook runs: its **front**, its **booking point**, the default, or its **rear**, entering or leaving, chosen by flags on the hook. At an object without geometry the three are the start of Receiving, the handover and the MU's rear leaving the conveyor before it, and all three are the same instant between two objects without geometry. A code distribution may be moved to another trigger point than its object's, at the user's own risk.

**Hold**:
What a distribution hook may answer instead of a successor: the object keeps the MU until code moves it. Its other answers are one connected successor, which the MU then waits for alone, and **default**, which leaves the choice to the object's distribution strategy.

**Action**:
A change user code asks crabsim to make to the run, such as moving an MU, setting a process time, pausing an object or calling the code again at a later time. crabsim checks it and carries it out with its own code, as if the engine had done it, or refuses it and says why. Code changes nothing in a run any other way.
_Avoid_: Command (a line in the command language), intervention (a change from outside the run)

**Address**:
How user code, the command line and `e` name a value, such as `Station 1.process_time`, `self.successors[2]` or `mu.destination`. In user code, an object is given as a hook's parameter, an instance's name checked when the model is built, and its values are read through typed functions such as `process_time()`, each giving a copy at that moment; writing goes through an action.
_Avoid_: Path (a conveyor's line)

**Fuel**:
How much work a call of user code may do, counted in WebAssembly instructions, the same on every machine. A call that runs out ends the run with an error naming its hook, object and MU. A run reports the fuel its code used, apart from its results.

### Agreeing and checking

**Request**:
What a user wants from the model, in their own words, with an agent's restatement in precise terms, the expectations that check it and its state: open, built, met or broken. The user accepts the restatement and its expectations together (ADR 0030).
_Avoid_: Requirement (SysML's word for an expectation), task, ticket

**Expectation**:
A check of a model that crabsim judges on its own, agreed with the user: a figure at the end of a run, such as "Station A busy at least 70%", or a limit during it, such as "Buffer X never holds more than 20". It names an object, a class with `every` or `mean`, or the model, and belongs to a stage. An agent proposes it, the user accepts it, and only the user changes an accepted one; an agent may question it, with its reason.
_Avoid_: Requirement, test, assertion, constraint

**Reference figure**:
An expectation from the real line, such as "Drain takes 4,100 parts a day, ± 3%", with where it came from, checked at the validation stage.

**Verdict**:
What crabsim says of an expectation: **met**, **not met**, **undecided**, when an experiment's 95% interval straddles its limit and more observations would tell, or **error**, when it couldn't be judged. In a single run or a live session, met so far.

**Stage**:
How far a model is checked: **draft**; **verified**, when a deterministic run meets its own expectations; **validated**, when runs with randomness also meet its reference figures. crabsim sets it, and drops it back when a change breaks a check.
_Avoid_: Phase, status

**Verification**:
Checking that the model does what was specified, in a deterministic run: no failures, every random time at its mean.

**Validation**:
Checking that the model behaves like the real line, in runs with randomness, against reference figures.

**Rule**:
A decision written as when/then lines that crabsim reads in order, such as "when the MU's class is Frame, to the one of Line A and Line B holding fewest MUs". Between the built-in strategies and user code: readable and checkable as the built-in ones are, without a program.
_Avoid_: Script, code, policy

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

**Change history**:
A model file's changes in order, by the user and by agents, each with its author, its time and what it did. It is kept beside the models, so it is still there after a restart. A file keeps its newest changes, and a deleted file's history goes with it.
_Avoid_: Edit log, version history

**Undo**:
Taking back the latest change in a model file's change history, after which the file is exactly as it was before it. **Redo** brings an undone change back, until the next change to the file. The user undoes anyone's changes, and an agent only its own latest ones. Never used for a simulation run, where an earlier instant is reached by going back.
_Avoid_: Revert, go back (what a live session does), checkpoint

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
