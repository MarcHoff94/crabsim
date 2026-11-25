# The model file

Agreed with the owner on 2026-11-07 in a design round, as step 2 of the AI-native order of work (issue 28). The code is in `model/` (`crabsim-model`, ADR 0009). It's built in three slices: today's objects with the built-in classes only, then derived classes and inheritance, then the published JSON Schema. All three are built. Format version 2 (spec issue 87, ADR 0014) made connections a list of their own and gave a Source's interval a default of 0.

## What it is

A model as a readable JSON file that people, agents and scripts can write, read, diff and keep in git. Most people will build models in the UI, which offers only valid choices, so the file's validation errors are mainly for agents and scripts.

A file holds the model only: classes, instances, their attributes, names, positions, dimensions, descriptions and connections. Seed, length, warm-up and the number of observations are inputs to a run and never appear in it.

## Top level

```json
{
  "$schema": "./model-file.schema.json",
  "format_version": 2,
  "description": "A line with two branches",
  "classes": [ ... ],
  "instances": [ ... ],
  "connections": [ ... ]
}
```

- `$schema` is optional: where the JSON Schema is, so an editor checks the file as it's typed. crabsim keeps it when saving and otherwise ignores it.
- `format_version` is required, and is 2. Version 1, where each instance listed its `successors`, is refused with one problem saying connections are their own list since version 2: no code reads or converts it (ADR 0014). Any other version is refused too.
- `description` is optional, for the model as a whole.
- `classes` is optional.
- `instances` is in Init order: ids follow it.
- `connections` is optional, and its order means nothing.

## Instances

```json
{
  "name": "Mill 1",
  "class": "CNC mill",
  "stream": 7,
  "position": { "x": 4, "y": 0 },
  "attributes": { "process_time": "0:45" }
}
```

- An instance owns `name` and `class`, both required, `stream` and `position`. Its connections are in the list of connections, not in the instance.
- `stream` (ADR 0012) is the number that, with a run's seed, gives the instance's random numbers. crabsim gives one when the instance is placed: one more than the highest in the model, or, for a file from before streams, its position. It stays whatever else changes, so variants of a model keep their common random numbers. Saving always writes it, and two instances with one stream are a problem. The user may change it by hand or with `edit_model`'s `set_stream`, which agents can't use, and which ends pairing with earlier runs.
- `position` is in metres, the centre of the object's footprint: `x` and `y`, and `z` optional, 0 until there's a 3D view. Without a position, the UI places the object where it puts unplaced objects.
- `attributes` holds only the values set on this instance, where its inheritance is cut. Anything not there comes from its class. It is left out when empty, so an instance that changes nothing is a name, a class, a stream and a position.

## Connections

```json
{ "from": "Mill 1", "to": "Station 2", "exit": 1, "entrance": 2, "description": "via the lift" }
```

- A connection (glossary: Connection) is a thing of its own, an edge of the model's graph with properties, so it lives in a top-level list rather than inside the instance it leaves (ADR 0014).
- `from` and `to` name instances. `to` is a successor of `from`.
- `exit` is the connection's place among the connections out of `from`, from 1: the order MUs are offered to successors, round-robin. The engine connects each object's successors in exit order.
- `entrance` is its place among the connections into `to`, from 1. Nothing reads it yet, so it doesn't change results until an entrance rule, such as pull or priorities, does. The inspector will say so on hover once it shows connections, in spec 87 PR 4.
- Both run 1 to n per object, n being how many connections leave it or lead into it, with no gaps or repeats. Both are written on every connection because no single order of the list could give both orders at once, and so that sorting or editing the list by hand can't change a result.
- `description` is optional.
- `from_side` and `to_side` (spec issue 148) name the side of a cross conveyor a connection leaves or comes in by: `"front"`, where its own x axis points, `"back"`, `"left"`, 90° counter-clockwise from the front, or `"right"`. Each is given for an end that is a cross conveyor and only then. A side has one connection, in or out, so it is an entry side or an exit side, never both, and MUs pass at its middle. The format stays version 2.
- Saving writes the list in one order: by the place of the instance each connection leaves, then by exit, so it reads as each object's exits in turn.
- `edit_model`'s `connect`, `disconnect` and `set_connection`, and removing or renaming an instance, number the exits and entrances at both ends again, so people never number connections themselves.

## Classes

```json
{ "name": "CNC mill", "parent": "Station", "attributes": { "process_time": "0:45", "description": "5-axis" } }
```

- The built-in classes are Source, Station, Buffer, Drain, Conveyor (spec issue 133) and CrossConveyor (spec issue 148), and MU for MU classes. A derived class names its `parent`, a built-in or another derived class, so chains are allowed.
- A class named like a built-in class, with no `parent`, is this model's own values for that built-in class (ADR 0015), such as `{ "name": "Station", "attributes": { "process_time": "0:10" } }`. Every class and instance built on the built-in class inherits them, between its own chain and the built-in's defaults, so a model can change what a Station is without a class of its own. A model has at most one for each built-in class, and one that names a `parent` is a validation error. `edit_model` makes it when a value is set on the built-in class's name and removes it when the last is restored, and the window's class library shows it as the built-in class. A new model made in the window starts with one for Station at 10 s, and so does a model when the first Station is placed or a class is first derived from Station.
- **MU classes** (ADR 0021, spec issue 133) derive from the built-in class `MU`, or from each other, and say what a Source's MUs are: `{ "name": "Tote", "parent": "MU", "attributes": { "length": 0.6, "width": 0.4 } }`. They are never placed as instances, which is a validation error, and have only an MU's attributes. A class named `MU` with no parent is the model's own values for the built-in MU, which a Source makes unless set to another. `edit_model`'s `rename_class` renames an MU class wherever a `mu_class` names it, and `remove_class` refuses while a Source or a class of them makes its MUs, unless asked to remove dependents, when they make the MUs they inherit again (`also_removed.mu_classes_from`).
- Classes can appear in any order. A cycle in the class tree is a validation error.
- A class writes only the attributes it changes from its parent, the same way an instance does.
- A class's attributes must suit the built-in class at the root of its chain, as an instance's must, and are checked the same way, with problems reported on the class.

## Where a value comes from

An instance's value of an attribute is the first one set along its chain: on the instance itself, then on its class, then on that class's parent, and so on, then on the model's own values for the built-in class, if it has them, ending at the built-in class's default, if it has one. Setting a value cuts inheritance of that attribute only, and removing it from the file restores it.

`Model::resolve(instance)` gives every value an instance ends up with, and its origin: `Instance`, `Class(name)` of the nearest class that sets it, or `BuiltIn`. This is what the operation catalogue's `get_attributes` will show, so a person or agent can see why an instance behaves as it does.

A class change reaches every instance that inherits that attribute, through any number of derived classes, and leaves alone those that set their own. Duplicating a class or an instance, as an independent copy, and restoring inheritance as an intervention during a run are operations, so they come with the catalogue.

## Attributes

Which attributes an instance has depends on its built-in class at the root of its chain. Any other attribute is a validation error.

| Attribute | On | Built-in default | Rule |
|---|---|---|---|
| `interval` | Source | 0, so every Source is valid when placed | A time, 0 or more |
| `process_time` | Station | 0, a dummy Station | A time, 0 or more |
| `failures` | any | none: never fails | See below |
| `starting_mus` | Station, Buffer | 0 | At most its capacity, and for a Buffer at most 1,000,000: they count as entered at Init, in their order. A Station's full starting batch begins work at Init, and a part of one waits |
| `capacity` | Station, Buffer | 1 for a Station, `"unlimited"` for a Buffer | A Station's is a whole number from 1 to 10,000: one machine working on that many MUs at once. A Buffer's is a whole number from 1, or `"unlimited"` (ADR 0019). `-1` is read as unlimited and saved as `"unlimited"`, so a Station refuses it |
| `start` | Station | `"individual"` | `"individual"`, each MU starting on entry, or `"batch"`, work beginning once the Station is full, its entrance closed until it is empty |
| `dwelling_time` | Buffer | 0 | A time, 0 or more: how long each MU stays before it may leave, counted from its own entry |
| `exit_strategy` | Buffer | `"first_in_first_out"` | That or `"last_in_first_out"`: which of the MUs whose dwelling time is over it passes on next |
| `histogram_bar_width` | Buffer | automatic | A whole number from 1: how many MUs each of its fill histogram's 64 bars covers, the last bar then taking everything beyond, such as 315 or more with 5. Left out, one MU a bar, doubling whenever more MUs come than the bars reach |
| `dimensions` | all | by type: Station 2 × 1.5 × 1.5 m, Buffer 2 × 1 × 1 m, Source and Drain 1 m each way | `length`, `width`, `height` above 0, `rotation_degrees` optional, 0 by default |
| `description` | all | none | Free text |
| `state_notes` | all | none | A note per state, see below |
| `distribution` | Source, Station, Buffer, Conveyor, CrossConveyor | `"round_robin"` | `"round_robin"`, weighted or by destination, see below (ADR 0016, ADR 0025) |
| `mu_class` | Source | `"MU"` | An MU class: the MUs it makes are of it (ADR 0021) |
| `mu_values` | Source | none | Any of `length`, `width`, `booking_point` and `work_plan`, changing its MU class's values for its MUs alone. Inherited as a whole |
| `work_plan` | MU class | none | Steps its MUs follow, each `{"at": name}` with an optional `"process_time"`, see below (ADR 0026). Inherited as a whole |
| `detour` | Station | none | A destination every MU it works on goes to first, before carrying on with its plan, such as `"Rework"` (ADR 0026) |
| `length` | MU class | 0.6 | Metres above 0: the footprint's length along the way it travels |
| `width` | MU class | 0.4 | Metres above 0: the footprint's width across the way it travels |
| `path` | Conveyor | `[{"straight": 5}]` | Its segments from its position, heading along its rotation (ADR 0024, spec issue 141): `{"straight": 4}` or `{"straight": {"length": 4, "rise": 0.5}}`, `{"arc": {"radius": 1.5, "angle": 90}}`, `{"transition": {"length": 1, "to_radius": 1.5}}` (or `"to_radius": "straight"`) and `{"helix": {"radius": 1.2, "turns": 2, "rise": 3}}`, angles and turns positive to the left, any with an optional `rise` in metres, a helix's never 0. Lengths are in plan, and its length counts each segment in 3D |
| `direction` | Conveyor | `"forward"` | `"forward"`, from its first segment to its last, or `"backward"`: which end its MUs enter at |
| `min_radius` | Conveyor | unset | The tightest bend its path may have, in metres above 0. Unset, only half its width limits it |
| `max_incline` | Conveyor | 30 | The steepest its path may climb or fall, in degrees above 0 and below 90 |
| `width` | Conveyor | 0.6 | Metres above 0: an MU wider than this doesn't fit |
| `speed` | Conveyor | 0.5 | Metres per second, above 0 |
| `gap` | Conveyor | 0 | The least space between two MUs on it, in metres, 0 or more |
| `length`, `width` | CrossConveyor | 0.6, 0.6 | Metres above 0: its length along its own x axis, back to front, and its width, right to left. A conveyor's default width each way, so default conveyors meet every side |
| `speed` | CrossConveyor | 0.5 | Metres per second, above 0 |
| `switch_time` | CrossConveyor | `"2"` | How long an MU turning there stands at its centre, such as a lift raising it onto cross rollers |
| `release_time` | CrossConveyor | `"0"` | How long it takes no MU after one that turned has left, such as its lift going back down |
| `exits` | CrossConveyor | none | Each exit side's destinations, such as `{"left": ["Mill"], "right": ["Rework", "Shipping"]}`: see below. Inherited as a whole |
| `booking_point` | MU class | 0.5 | A share of the MU's length from its front, 0 at the front to 1 at the rear: where it passes from one object to the next (ADR 0023) |

- **Times** are strings in the UI's syntax, read by the same `parse_time` as the UI and the CLI: `"10"`, `"1:30"`, `"2:00:00"`, `"2d 06:00:00"`, `"0:00:12.5"`. Exact to the millisecond.
- **A dummy Station** has a process time of 0. It takes no time, and exists so that something can happen as an MU passes, such as user code at a hook later. It can still fail, like any Station.
- **A cross conveyor's exits** (spec issue 148) list, by side, the destinations each exit side takes: instances, or classes standing for any of their instances, as a work plan's steps do. An MU whose next destination an exit side's list stands for (any object both stand for) leaves by that side: it runs to the centre, stands for the switch time and leaves sideways, keeping its orientation, so it lies crosswise on what comes next until another cross conveyor turns it again or an object without geometry takes it. Several sides listing it, the one straight across included, are chosen among by the cross conveyor's `distribution` as the MU is let in, the first in its order whose successor would take it then, otherwise the first, which it then waits for; leaving straight across, it doesn't stop. Every MU no exit side lists goes straight on, through to the side opposite the one it came in by, without stopping. A `by_destination` distribution there chooses by what follows it, since the exits already route by destination. Renaming an instance or class renames it in exits.
- **A Source with an interval of 0** creates an MU and offers it at once. It holds one MU at a time and creates the next only once that MU has left, so it waits whenever its successor is busy.
- **Failures** are a list of named failure profiles, independent of each other (spec issue 122, PR 3):
  ```json
  "failures": [
    { "name": "Tool wear", "stream": 0, "availability": 0.9, "repair_time": { "erlang": { "mean": "1:00", "k": 2 } } },
    { "name": "Jam", "stream": 1, "time_basis": "processing", "availability": 0.98, "repair_time": { "constant": "0:20" } }
  ]
  ```
  Each name is unique within its object. Availability is strictly between 0 and 1. The repair time is one of `{"constant": t}`, `{"uniform": {"min": t, "max": t}}`, `{"normal": {"mean": t, "std": t}}` or `{"erlang": {"mean": t, "k": n}}`, with its mean above 0. The time between failures is derived from the availability. A profile's `stream` is its own within its object, unique there: with the object's random numbers it gives the profile's, so adding or removing another profile changes nothing for it, and a class's profiles fail apart in each of its instances. Without one, its place in the list counts as its stream, and stream 0 is the object's own random numbers. A single `failures` object, as older files have it, reads as a list of one named "Failures" with no stream, so it fails exactly as before, and saves as a list. A profile's `time_basis` is what counts down to its next failure: `"simulation"`, all time, the default and left out when saved; `"working"`, all time but paused; or `"processing"`, only time busy, which only a Station or Source has. The list is inherited as a whole, and an empty list says an instance never fails though its class does. At most 255 profiles.
- **State notes** say what a state means for this class or instance, beside crabsim's own meaning of it, which the window shows in its help on an object:
  ```json
  "state_notes": { "blocked": "waits for the conveyor to clear", "failed": "the gripper needs changing" }
  ```
  The states are `busy`, `holding`, `blocked`, `idle`, `failed`, `stopped` and `paused` (`forced_failure`, its name before ADR 0020, is still read as `stopped`), and a note may name only states its built-in class can be in: a Source is busy or blocked, a Station any of them but holding, a Buffer holding, blocked or idle, and a Drain only idle. A note can't be empty. Like every attribute, the notes are inherited as a whole, so an instance that sets its own replaces its class's. A note never changes what is measured or a state's colour.
- **Distribution** says which successor receives the MU an object passes on (ADR 0016): `"round_robin"`, each successor in turn, or weighted:
  ```json
  "distribution": { "weighted": { "by": "process_time", "order": "smallest", "rank": "route", "drains": ["Out"] } }
  ```
  A weighted object ranks its successors and offers the MU to the best one that can take it, those ranking equal taking turns, and waits only when all are blocked. `by` is `process_time`, `interval`, `starting_mus` or `capacity`, resolved through inheritance on each object weighed, which weighs 0 without it. `order` is `smallest`, the default, or `largest`. `rank` is `route`, the default, a successor's best route onward to a Drain with itself included, or `successor`, its own weight only. `drains`, optional and not empty, names the Drains a route counts: left out, every Drain. A name that isn't a Drain is ignored, and `validate_model` warns about it. A successor with no route to a counted Drain ranks last. Inherited as a whole. Saving writes `order` and `rank` out.
- **Work plans** (spec issue 148, ADR 0026) are a list of steps on an MU class, or in a Source's `mu_values` for its MUs alone:
  ```json
  "work_plan": [{ "at": "Saw", "process_time": "0:40" }, { "at": "Mill", "process_time": "1:30" }, { "at": "Shipping" }]
  ```
  Each `at` is a destination (ADR 0025): an instance's name, or a class's, standing for every instance of it or of a class derived from it, a built-in class such as `Station` included, but never an MU class. A name that is neither is a problem, and so is a `process_time` on a step whose name can't be a Station. An MU's next destination is its detour if it has one, otherwise its plan's current step. Entering a Station its next destination names, it is worked on for the step's time, or the Station's own without one, and moves on to its next step, or clears its detour; entering any other object its next destination names, such as a Drain, moves it on too. At a Station its next destination doesn't name it is worked on for the Station's own time, keeps its step and counts as off plan (`mus_off_plan`). Starting MUs have no plan. Two MU classes, or a class and a Source's own values, alike but for their plans are two MU kinds. A model may name at most 65,535 destinations and its MU kinds may have at most 65,535 steps between them (ADR 0027). Renaming an instance or class renames it in every plan and detour.
- **By destination** routes each MU only to the successors from which its next destination can be reached along the connections, and `then`, round robin or weighted, chooses among them:
  ```json
  "distribution": { "by_destination": { "then": "round_robin" } }
  ```
  `then` is round robin when left out, and saved written out. A way that reaches but is blocked is waited for. An MU with no destination left, or one none of the successors reaches, is offered to every successor by `then`, and `validate_model` warns where it follows a Source's MUs, plans and detours through the model to an object routing by destination none of whose successors leads to where they go next.
- **Starting MUs** are a count. MU ids belong to a run, and are handed out when a run is built, in file order. They are of the built-in MU class, with the model's own values for it.
- **A conveyor's position** is where its path starts, and its rotation the way its path heads there (spec issue 141), so editing its segments never moves its start. A view draws it centred on its path's bounds (`Model::drawn_position`). In files from before increment 2 it was the centre of a straight conveyor, so those conveyors now start half their length back. A bend tighter than half its width or its `min_radius`, or a segment steeper than its `max_incline`, is a problem, and a transition that ends at the bend it starts at is one too. Nothing checks that its ends meet its neighbours: the engine treats every connection as touching. `Model::warnings` also warns about MUs that a Source sends to a conveyor whose corners stick out past its edge in its tightest bend, which only a view sees. The geometry worked out from a path is in `crabsim_model::path`, with the short form `4, arc 1.5 90, helix 1.2 2 rise 3` that the command language reads and the window shows.
- **A conveyor's size** is its path's length in 3D and its width: its `dimensions` give only its height and rotation. A conveyor can lead straight into another. Its `accumulating` is true by default: MUs behind one that stops close up and stop while the rest move, and with false every MU on it stops as a whole when the first can't leave. `Model::warnings` warns about a Source whose MUs are too wide or too long for a conveyor it reaches through objects without geometry, which would refuse them for good.
- **An MU's values** reach the engine as its MU kind: each different set of values a Source makes is one entry in the run's table of them, and every MU carries its entry's number. Only objects whose geometry counts read them, such as conveyors.
- **Visual only:** `position`, `dimensions`, `description` and `state_notes` never reach a run, so they can't change a result. The 3D model path will be one more inheritable attribute when there is a 3D view, a path and never embedded.

## Names

- Instance names and class names are each unique, compared exactly (case-sensitive).
- A name can't be empty or have spaces at either end, and may otherwise contain anything.
- Instances and classes are separate namespaces. A class takes a built-in's name only as the model's own values for it, with no parent.

## Validation

Loading returns every problem at once and never panics. Each problem has:

- a JSON Pointer into the file, such as `/instances/3/attributes/process_time`,
- the name of the instance or class it is about, when there is one,
- a plain message.

- a "did you mean" when a mistyped name is one or two edits from a known one (an insertion, deletion, substitution or two letters swapped each count as one, and no more than a third of the name may change), for attributes, fields, classes and the instances connections name, such as `unknown attribute proces_time: did you mean process_time?`.

An instance or class whose class is unknown can't have its attributes checked, so the message says they weren't: fixing it may bring up more problems.

They come in file order, as JSON for agents, and one per line on the CLI:

```
/instances/3/attributes/process_time "Station 3": "1:75" is not a time
```

What it catches:

- JSON that doesn't parse, a missing or unsupported `format_version`, wrong types, and unknown fields, such as a typo like `proces_time`, which would otherwise leave a value silently inherited.
- Names that are missing, empty, padded or used twice, and a class named like a built-in that names a parent.
- Unknown classes, a class deriving from itself, and classes deriving from each other in a circle, named in the order they go round.
- Connections to an unknown name, into a Source, out of a Drain, from an object to itself, or the same connection twice, each with the connection's pointer, such as `/connections/3/to`, and its ends as its name, such as `In → Mill`.
- Exits or entrances with gaps or repeats, such as an exit of 3 out of an object with 2 connections out, or two connections out of one object with exit 1.
- Attributes the class doesn't have, values out of range, and required values set nowhere in the chain.
- **A Source that would create MUs forever:** a Source with an interval of 0 that can reach a Drain, or a Buffer with no limit, through nothing but zero-time objects: Stations with a process time of 0 and Buffers with a capacity and a dwelling time of 0. Nothing ever fills, so it would create MUs without time passing. A Buffer with a capacity and a dwelling time fills, and then the Source waits. The run's zero-time-loop error stays as the backstop.
- **A Buffer that starts with more MUs than it holds,** wherever the two values come from.
- **Sides** (spec issue 148): a connection to or from a cross conveyor without its side, a side on an end that isn't a cross conveyor, a side that isn't front, back, left or right, and a side with a second connection, in or out. Exits listing a place that is neither an instance nor an object class, or at a side that is none of the four.

A model without problems can still get warnings from `validate_model`, which never stop a run. One of them is a Buffer with no limit that can get MUs faster than it can pass them on, judged from the intervals of the Sources and the process times of the Stations directly before and after it. A cross conveyor gets three (spec issue 148): an entry side with no exit straight across, where an MU going somewhere no exit side lists would wait for good, given only where MUs can arrive going so, found by following each Source's MUs through their plans, their detours and the cross conveyors' exits; exits listed for a side no connection leaves by; and MUs too long or wide for its way across from every side they come in by.

## Saving

Saving writes one canonical form, so a file crabsim saved always reads back identically:

- keys in a fixed order, fields with their default values left out,
- times as `format_time_exact` writes them: hours, minutes and seconds, with days and milliseconds only when needed, so `"90"` is saved as `"00:01:30"`,
- two-space indentation and a final newline.

A file written by hand may be reformatted the first time crabsim saves it. Saving, loading and saving again gives byte-identical files.

## Building a run

A valid file becomes the engine's `ModelDescription`, with every instance's values worked out from its chain of classes, so a run never sees classes and pays nothing for them. Names, positions and dimensions go on the description for views, as today. Descriptions stay in the file's types.

## Files

- `models/demo.json` is the demo model as a file, pointing at the schema with `"$schema": "./model-file.schema.json"`. Its Stations share their failures through one class, `Demo station`, and set only their process times. A test checks it builds a run identical to `demo::model()` for the same seed.
- The benchmark's 6,006-object model is generated in code and checked to round-trip as a file in a test, rather than committed, since it is about 2 MB.

## The JSON Schema

`models/model-file.schema.json` is the published schema, generated from the types in `model/src/schema.rs` with `schemars`, pinned to one version because a new one can change the file. Its descriptions come from those types' doc comments, so an editor shows them on hover and an agent can read them.

The schema checks a file's shape: fields, types, time strings, ranges such as an availability between 0 and 1. crabsim's own validation goes further: names that must exist, classes going round in a circle, seconds below 60, values set nowhere in a chain, a Source that would create MUs forever. So a file the schema accepts may still have problems, but a file crabsim accepts always passes the schema.

The reader reads files by hand, so it can report every problem at once, and the schema comes from separate types. Tests in `model/tests/schema.rs` keep the two from drifting apart:

- every file the reader accepts, including one with every field and kind of value, passes the schema,
- shape mistakes, from unknown fields to a time that isn't one, fail both,
- the schema's attributes are exactly the reader's,
- times crabsim saves and typical typed ones match the schema's pattern,
- the committed file is the generated one. After changing the format, write it again with `cargo test -p crabsim-model --test schema -- --ignored write_the_schema_file`.

The operation catalogue will reuse these types for its own input schemas, so the file's schema and the operations' never disagree.

## The JSON Schema

`models/model-file.schema.json` is the published schema, generated from the types in `model/src/schema.rs` with `schemars`, pinned to one version because a new one can change the file. Its descriptions come from those types' doc comments, so an editor shows them on hover and an agent can read them.

The schema checks a file's shape: fields, types, time strings, ranges such as an availability between 0 and 1. crabsim's own validation goes further: names that must exist, classes going round in a circle, seconds below 60, values set nowhere in a chain, a Source that would create MUs forever. So a file the schema accepts may still have problems, but a file crabsim accepts always passes the schema.

The reader reads files by hand, so it can report every problem at once, and the schema comes from separate types. Tests in `model/tests/schema.rs` keep the two from drifting apart:

- every file the reader accepts, including one with every field and kind of value, passes the schema,
- shape mistakes, from unknown fields to a time that isn't one, fail both,
- the schema's attributes are exactly the reader's,
- times crabsim saves and typical typed ones match the schema's pattern,
- the committed file is the generated one. After changing the format, write it again with `cargo test -p crabsim-model --test schema -- --ignored write_the_schema_file`.

The operation catalogue will reuse these types for its own input schemas, so the file's schema and the operations' never disagree.

## The CLI

```bash
cargo run --release --bin crabsim -- --model models/demo.json --seed 7 --length 30d
```

Without `--model` it runs the built-in demo. An invalid file prints every problem and exits with failure, without running.
