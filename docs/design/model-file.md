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
  "connections": [ ... ],
  "hooks": { ... },
  "expectations": [ ... ]
}
```

- `$schema` is optional: where the JSON Schema is, so an editor checks the file as it's typed. crabsim keeps it when saving and otherwise ignores it.
- `format_version` is required, and is 2. Version 1, where each instance listed its `successors`, is refused with one problem saying connections are their own list since version 2: no code reads or converts it (ADR 0014). Any other version is refused too.
- `description` is optional, for the model as a whole.
- `classes` is optional.
- `instances` is in Init order: ids follow it.
- `connections` is optional, and its order means nothing.
- `hooks` is optional: the model's hooks around crabsim's own Init and Reset, `before_init`, `after_init`, `before_reset` and `after_reset`, each `{"code": {"function": "<module>.<function>", "with": {…}}}` (ADR 0029, see "User code" below).
- `expectations` is optional: what the model must achieve, checks crabsim judges on its own after a run (ADR 0030, see "Expectations" below).
- `requests` and `stages` are optional: what the user asked for, with the expectations that check it, and how each stage's checks run (ADR 0030, see "Requests, agreeing and stages" below).

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
| `process_time` | Station | 0, a dummy Station | A time, 0 or more, or a time by MU, `{"rules": [...], "default": time}`, see "Process time by MU" below (ADR 0030). Inherited as a whole |
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
| `distribution` | Source, Station, Buffer, Conveyor, CrossConveyor | `"round_robin"` | `"round_robin"`, weighted, by destination or code, see below (ADR 0016, ADR 0025, ADR 0029) |
| `on_entry` | any | none | User code run as an MU comes in, see "User code" below. Inherited as a whole |
| `on_exit` | any but Drain | none | User code run as an MU leaves, after it has moved on. Inherited as a whole |
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
- **Code** chooses the successor in user code (ADR 0029), answering one successor, hold or default, which hands the choice to `default`, round robin when left out:
  ```json
  "distribution": { "code": { "function": "routing.every_nth", "with": { "n": 3, "left": "Left", "right": "Right" } }, "default": "round_robin", "asked_on_entry": "booking_point" }
  ```
  `asked_on_entry`, optional, asks the code at that trigger point as the MU comes in rather than when it wants to leave; a cross conveyor's distribution always runs as it lets the MU in, so it can't have one.
- **Starting MUs** are a count. MU ids belong to a run, and are handed out when a run is built, in file order. They are of the built-in MU class, with the model's own values for it.
- **A conveyor's position** is where its path starts, and its rotation the way its path heads there (spec issue 141), so editing its segments never moves its start. A view draws it centred on its path's bounds (`Model::drawn_position`). In files from before increment 2 it was the centre of a straight conveyor, so those conveyors now start half their length back. A bend tighter than half its width or its `min_radius`, or a segment steeper than its `max_incline`, is a problem, and a transition that ends at the bend it starts at is one too. Nothing checks that its ends meet its neighbours: the engine treats every connection as touching. `Model::warnings` also warns about MUs that a Source sends to a conveyor whose corners stick out past its edge in its tightest bend, which only a view sees. The geometry worked out from a path is in `crabsim_model::path`, with the short form `4, arc 1.5 90, helix 1.2 2 rise 3` that the command language reads and the window shows.
- **A conveyor's size** is its path's length in 3D and its width: its `dimensions` give only its height and rotation. A conveyor can lead straight into another. Its `accumulating` is true by default: MUs behind one that stops close up and stop while the rest move, and with false every MU on it stops as a whole when the first can't leave. `Model::warnings` warns about a Source whose MUs are too wide or too long for a conveyor it reaches through objects without geometry, which would refuse them for good.
- **An MU's values** reach the engine as its MU kind: each different set of values a Source makes is one entry in the run's table of them, and every MU carries its entry's number. Only objects whose geometry counts read them, such as conveyors.
- **Visual only:** `position`, `dimensions`, `description` and `state_notes` never reach a run, so they can't change a result. The 3D model path will be one more inheritable attribute when there is a 3D view, a path and never embedded.

## User code

A hook names a function of a code module with values for its parameters (ADR 0029, spec issue 156):

```json
"on_entry": { "code": { "function": "routing.report", "with": { "drain": "Drain", "every": "1:00:00" } }, "at": ["front", "rear"] }
```

- `function` is `<module>.<function>`, the module being `code/<module>.wasm` in the models folder, built from Rust with the `crabsim-guest` crate (`docs/design/code-interface.md`). Its sources may sit in `code/<module>/`, which the check for source code in a models folder leaves alone.
- `with` gives each parameter the function declares, by name: a whole number, a number, true or false, a time as text such as `"1:30"`, an instance's name, or text. It is left out when empty.
- `at`, for `on_entry` and `on_exit`, lists the trigger points the hook runs at, each once, front first: `"front"`, `"booking_point"` and `"rear"`. The booking point alone by default, and then left out.
- **Checked in two steps.** Reading the file checks a hook's shape: `function`'s form, `with` an object, `at` and `asked_on_entry` trigger points, a Drain without `on_exit`. Building it with its code (`Model::to_description_in`, `load_code`, `code_problems`) checks the rest against the modules in `code/`: a module that's missing or isn't a crabsim module of this interface version, a function it doesn't have, a parameter it doesn't declare or one missing, and a value that doesn't fit its type, such as an instance's name that isn't in the model. Each is a problem where it is written, a class's on the class. The CLI, `start_run` and the window build every model this way, so a model with code never runs without it.
- **A model without code** builds exactly as before. `to_description` builds a model's structure without its code, for what only reads it, such as `query_model`.

## Expectations

What a model must achieve, as checks crabsim judges itself after every run (ADR 0030, spec issue 162, PR 1):

```json
"expectations": [
  { "id": "presses-busy", "text": "every Press.working_share since start >= 60%", "assumptions": "one shift", "source": "the planner" },
  { "id": "buffer-limit", "text": "Buffer X.mus always <= 20" },
  { "id": "output", "text": "model.throughput_per_hour last 8h >= 400/h", "stage": "validation" },
  { "id": "measured", "text": "Drain.throughput_per_hour since start = 4100/d ± 3%", "stage": "validation", "source": "the line's MES, October" }
]
```

- `id` is required, unique among the expectations and without spaces. `edit_model`'s `add_expectation` gives the first free of `e1`, `e2` and so on when it's left out.
- `text` is required: one line, `[every|mean] <subject>.<figure> <scope> <comparison> <value>`, parsed by `crabsim_model::expect::Check` and saved in its canonical form once it parses, such as `every Press.working_share since start >= 60%`.
  - **The subject** is an instance by its name, a class with `every` (each of its instances must meet it) or `mean` (their mean), a class counting every instance whose chain holds it and a built-in class every instance of it, or `model`, its Drains together. The address ends at the last `.` followed by a figure's name, so a name may hold dots.
  - **The figure** is one of the vocabulary's (`crabsim::figures`, the operation `list_figures`), whose names are the results store's, such as `working_share` or `mean_lead_time_s`, and it must be one of its subject's built-in class: a Drain has no `working_full_share`. The model's figures are `throughput_per_hour`, `mean_lead_time_s` and `lead_time_s`. An MU's figures (`next_destination`, `plan_step`) are for rules and code, not expectations, but `class` and `step`, with `always`, are a limit on which MUs an object takes (spec issue 162, PR 5): `Mill.step always in Welding, Deburr` or `every Press.class always not in Tote`, on any object MUs enter, never a Source or the model. `class` lists MU classes, each standing for those derived from it; `step` lists classes of objects or instances, the MU's current step as it enters (what its detour names while it has one, otherwise its plan's current step), a listed class standing for itself, the classes derived from it and their instances, an instance for itself alone, and an MU without a step breaking it whether `in` or `not in`. Its value is how many MUs broke it, met when none did, and the first that did is reported with its time, MU, class and step. A run judging expectations watches only the objects such a limit names (`Model::entry_watches`, `SimulationRun::watch`); the window's live session watches nothing. No bounds are worked out for them.
  - **The scope** is required, one the figure can be read over: `since start` (since statistics began, after the warm-up), `now` (at the run's end), `last <time>` (the stretch before the end, such as `last 1h`) or `always`, a limit at every moment. A limit is upper (`<=`, `<`) on `mus`, the most MUs held, which Stations, Buffers, Conveyors and cross conveyors keep, and either way on a Drain's `lead_time_s`, each MU's; the fewest MUs held is `min_mus since start`. `mean` takes no `always`.
  - **The comparison** is `>=`, `<=`, `>`, `<`, `=` or `!=`, a state only `=` or `!=`, and `class` and `step` only `in` or `not in`, followed by a comma-separated list.
  - **A tolerance** may follow an `=` value after `±` (typed `±`, `+-` or `+/-`, saved as `±`), for a reference figure, measured on the real line (spec issue 162, PR 4): in the figure's own unit (`± 100/d`, `± 30s`, `± 5%` for a share, in percentage points) or, for a figure not counted in shares, as a percentage of the value (`= 4100/d ± 3%`), more than 0. Not on a limit or a state, nor with any other comparison.
  - **The value** has an explicit unit: a share as a percentage (`60%`), a rate with its unit (`4000/h`, `70/min`, `96000/d`), a time with its unit (`90s`, `30min`, `2h`, `1d`) or as the UI writes time (`1:30`, saved as `00:01:30`), MUs and counts plainly (`20`), a state by its name, one its subject's class can be in. A bare number for a share or a time is a problem, so a line never means something other than it says.
- `stage` is `verification`, the default and then left out, or `validation`.
- `assumptions` and `source` are optional text: what it takes for granted, and where it comes from, such as a SysML element, a document or a person, so a later import of SysML v2 requirements maps onto the record.
- **Bounds** (`model/src/bounds.rs`, `Model::expectation_bounds`, spec issue 162, PR 4) are warnings at the line, among `validate_model`'s and in `get_status`'s `cannot_be_met`, of an expectation crabsim works out without a run can't be met, or contradicts another. Each is certain, from the model alone: a share above 100% or a value below 0; more MUs than an object holds; a failure at the verification stage, or of an object without a failure profile, a pause, or receiving where no conveyor leads in; more MUs in or out, a higher throughput or a higher Station busy share than a maximum flow lets through (each object passing at most its own pace in the stretch, a Source one MU an interval, a Station its capacity a process time, its shortest of its own and its work plans', a conveyor one MU at its speed per the MUs' least spacing, a cross conveyor one MU per its shorter side, from Sources and what each object can hold as the stretch begins); a lead time or a conveyor's crossing time shorter than the work, dwelling and crossing on the shortest way; and two expectations reading the same figure of the same instance in the same check with no value both accept, or shares of one object's time that can't add up to 100%. A since-start throughput, which counts from the Drain's first MU, is bounded as if that came as early as it can: the one bound resting on how a run goes, since MUs arriving together just before the end could make the figure anything. An object in a loop is bounded by its own pace alone, since its MUs pass it more than once. A model with code gets only the range, capacity and contradiction bounds, since code can move MUs and change times (ADR 0029). Warnings never stop a check, whose verdict is what counts. `validate_model` also warns of a validation expectation with `=` and no tolerance, and of a reference figure without a `source`.
- **Problems** point at the expectation's `id` or `text`: a line that doesn't parse says at which character and what it expected there, and one that doesn't fit the model names what's missing, such as "there is no instance `Press`: `Press` is a class, which takes `every` or `mean` before it". Renaming an instance or a class through `edit_model` renames it in the lines naming it. Removing one an expectation names is refused until the expectation goes too.
- **Judged outside the run,** from its states: since start and now from the state at its end, `last` from the difference of the statistics at the end and at the stretch's start, where the run stops on its way, and a limit from the extreme the statistics keep (the most MUs held, the longest and shortest lead time). Only an observation that broke a limit runs again, with the same seed, recording MUs coming and going until it finds the moment and the MU that first broke it. A run with no expectations stops nowhere and runs nothing again.
- **Verdicts** (`crabsim_model::expect::judge`): over an experiment's observations, met when the mean's 95% interval lies wholly on the right side of the value, not met when wholly on the wrong side, and undecided when it straddles it, with about how many observations would decide it. One observation, or a deterministic run, decides by its value, a random run's single observation saying "so far". `=` with a tolerance is met by a value within it, and over an experiment when the mean's 95% interval lies within it, not met when wholly outside it, and undecided otherwise, with about how many observations would bring the interval inside or outside the nearer edge. `=` without one, `!=` and states are judged observation by observation: met when every one holds, not met when none does, undecided otherwise. A limit is not met if any observation broke it. A figure with no value, such as a mean lead time with no MU in the Drain, or a stretch longer than the run, is an error. `every` takes its worst instance: not met, then error, then undecided, then met.

## Requests, agreeing and stages

What the user agreed with an agent, and how crabsim checks it (ADR 0030, spec issue 162, PR 2):

```json
"stages": { "verification": { "length": "2d" }, "validation": { "length": "30d", "warm_up": "1d", "observations": 20 } },
"requests": [
  { "id": "r1", "words": "the presses must earn their keep", "restatement": "both Presses working at least 60% of the time", "expectations": ["presses-busy"], "accepted": true, "state": "built" }
],
"expectations": [
  { "id": "presses-busy", "text": "every Press.working_share since start >= 60%", "state": "accepted" },
  { "id": "buffer-limit", "text": "Buffer X.mus always <= 20", "state": "questioned", "question": "Press B is slower than the Source, so the Buffer fills without end", "length": "8:00:00" }
]
```

- **An expectation's `state`** is `proposed`, the default and then left out, for one an agent wrote; `accepted`, for one the user accepted or wrote, which only the user changes; or `questioned`, with its `question`, an agent's reason, required then and only then. A questioned expectation is still in force until the user answers it: by accepting it again, changing it, which accepts it, or removing it.
- **A request** has an `id` (unique, no spaces, `r1`, `r2` and so on from `add_request`), the user's own `words` (required), an agent's `restatement`, the `expectations` that check it by id (each one there, none twice), `accepted` (false by default, left out) and `state`, `open` (the default, left out) or `built`, which whoever builds it sets. Whether it is met is never written here: crabsim works it out from the checks it keeps, `met` when every expectation of it was met in its latest check of the model as it is, `broken` when one wasn't, `built` until then. So a check never changes the model file, its hash, its history or anyone's claim.
- **`stages`** give `verification` and `validation` each a `length`, `warm_up` and `observations`, every part left out taking its default: a day, no warm-up, one observation for verification, which runs deterministically, and ten for validation, which runs with randomness. An expectation may give its own `length`, `warm_up` and `observations`, which its check runs with instead: a stage is checked with one run for each set of settings its expectations have.
- **Only the user agrees:** the catalogue refuses an agent anything that accepts an expectation or a request, or changes or removes an accepted one, or changes how an accepted expectation is checked, whichever operation it tries (`edit_model`, `write_model`, `adopt_model`, `delete_model`, `undo_model`, `redo_model`). Renaming what an accepted expectation names keeps it accepted: an instance is the same when it keeps its stream (ADR 0012). An agent may change its own proposals, question an accepted expectation and mark a request built.

## Rules

A distribution by when/then rules (ADR 0030, spec issue 162, PR 3), on a Source, Station, Buffer, Conveyor or cross conveyor, inherited as a whole like any distribution:

```json
"distribution": {
  "rules": [
    "when class = Frame then smallest working_share last 30min of Line A, Line B among those free",
    "when time of day >= 22:00 or time of day < 06:00 then to Night shift",
    "otherwise default"
  ],
  "default": "round_robin"
}
```

- **The lines** are read in order as the object passes an MU on; the first that holds decides. A line is `when <conditions> then <action>` or `otherwise <action>`, saved in canonical form (`crabsim_model::rules::Line`): `among those free` written out, a time of day as `06:00`, values as an expectation writes them.
- **Conditions,** joined by `and`, which binds first, and `or`: `class = <MU class>` or `!=`, with a comma-separated list for any of them, each standing for the MU classes derived from it too; `destination = <destination>` or `!=`, an instance or a class of objects, the MU's next one; `step = <class or instance>` or `!=`, a list for any, the MU's current step as an expectation reads it (PR 5), an MU without one on none; `time of day <comparison> <hh:mm>`, from the run's start, a day long; `<instance>.state now = <state>` or `!=`, a state its class can be in, a stop by hand reading as failed; and `<instance>.<figure> <scope> <comparison> <value>`, or another `<instance>.<figure> <scope>` in the same unit. The scope is `now` (for `mus`), `since start` or `last <time>`; the figures are those the statistics keep as the run goes (`crabsim::rules::readable`). A rule never names the object deciding.
- **Actions:** `to <successor>`, the MU waiting for it alone; `smallest` or `largest <figure> <scope> [of <successors>] [among those free|and wait]`, each successor's figure, the best free one taking the MU, or only the best, waited for; `round robin [among <successors>]`; `weighted among <successors>`, by the object's own ranking, so its `default` must be weighted; and `default`.
- **`default`** is round robin when left out, or weighted, or by destination, never code or rules: it decides when no line holds and when a line says `default`.
- **Problems** point at the line where it is written: the instance's `/attributes/distribution/rules/<n>`, or the class's, naming the instance it doesn't fit. A line that doesn't read says why; one that doesn't fit names the successor, class, destination, figure or state that's wrong. Renaming an instance or class through `edit_model` renames it in the lines, as a destination either way.
- **Built** into the engine's `crabsim::rules::Rules` by id (`ModelDescription::set_rules`), a class's MU kinds those of its own Sources and every MU class derived from it.

## Process time by MU

A Station's `process_time` can be when/then lines (ADR 0030, spec issue 162, PR 5), the same lines as a distribution's rules but each giving a time:

```json
"process_time": {
  "rules": ["when class = Frame then 30s", "when step = Deburr, Grind then 1:20", "otherwise default"],
  "default": "20"
}
```

- **The lines** are read in order as an MU enters the Station; the first that holds gives its time, with its unit (`30s`, `2min`, `1:30`, saved as `00:01:30`), or `default`. With none holding, the Station's own time. A bare number is a problem.
- **`default`** is required: the Station's own time, as a plain `process_time` is. It is what an intervention changes, Keep changes saves, a weighted strategy and the Paths view weigh, and what `get_attributes` and the bounds call its own. The whole value is inherited as one: a plain time set below a class's lines cuts them off.
- **Conditions** are a distribution rule's, without `destination`: `class`, `step`, `time of day`, another object's `state now`, and figures. A line reading another object reads it as the MU is handed over, the giver included, which has already counted the MU out.
- **A work plan step's own time wins** for an MU on plan at the Station; the lines decide for MUs off plan, without a plan, or on a step without a time. A Station with capacity decides each MU's time as it enters, even in batch mode, where the Station's own time is otherwise taken as work starts. Starting MUs, which never enter, take the Station's own time.
- **Problems** point at the line where it is written, `/attributes/process_time/rules/<n>` of the instance or the class; renaming an instance or class through `edit_model` renames it in the lines.
- **Bounds** take the shortest and longest time the lines can give, with the Station's own and its plan steps'. The warning of a Buffer that fills faster than it empties counts such a Station as unknown.
- **Built** into the engine's `crabsim::rules::TimeRules` by id (`ModelDescription::set_time_rules`).

An MU class alike another in every value still gets an MU kind of its own once a Source makes it (`ModelDescription::add_separate_mu_kind`), so rules and expectations tell their MUs apart by class. Before PR 5 such a class shared the first one's kind, and a rule naming it never held.

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
