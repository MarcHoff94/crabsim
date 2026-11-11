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
- Saving writes the list in one order: by the place of the instance each connection leaves, then by exit, so it reads as each object's exits in turn.
- `edit_model`'s `connect`, `disconnect` and `set_connection`, and removing or renaming an instance, number the exits and entrances at both ends again, so people never number connections themselves.

## Classes

```json
{ "name": "CNC mill", "parent": "Station", "attributes": { "process_time": "0:45", "description": "5-axis" } }
```

- The built-in classes are Source, Station, Buffer and Drain. A derived class names its `parent`, a built-in or another derived class, so chains are allowed.
- Classes can appear in any order. A cycle in the class tree is a validation error.
- A class writes only the attributes it changes from its parent, the same way an instance does.
- A class's attributes must suit the built-in class at the root of its chain, as an instance's must, and are checked the same way, with problems reported on the class.

## Where a value comes from

An instance's value of an attribute is the first one set along its chain: on the instance itself, then on its class, then on that class's parent, and so on, ending at the built-in class's default, if it has one. Setting a value cuts inheritance of that attribute only, and removing it from the file restores it.

`Model::resolve(instance)` gives every value an instance ends up with, and its origin: `Instance`, `Class(name)` of the nearest class that sets it, or `BuiltIn`. This is what the operation catalogue's `get_attributes` will show, so a person or agent can see why an instance behaves as it does.

A class change reaches every instance that inherits that attribute, through any number of derived classes, and leaves alone those that set their own. Duplicating a class or an instance, as an independent copy, and restoring inheritance as an intervention during a run are operations, so they come with the catalogue.

## Attributes

Which attributes an instance has depends on its built-in class at the root of its chain. Any other attribute is a validation error.

| Attribute | On | Built-in default | Rule |
|---|---|---|---|
| `interval` | Source | 0, so every Source is valid when placed | A time, 0 or more |
| `process_time` | Station | 0, a dummy Station | A time, 0 or more |
| `failures` | Station | none: never fails | See below |
| `starting_mus` | Station | 0 | 0 or 1, until Station capacity exists |
| `capacity` | Buffer | none: must be set somewhere in the chain | 1 or more. Buffer is known to be broken until step 7 of issue 3 |
| `dimensions` | all | by type: Station 2 × 1.5 × 1.5 m, Buffer 2 × 1 × 1 m, Source and Drain 1 m each way | `length`, `width`, `height` above 0, `rotation_degrees` optional, 0 by default |
| `description` | all | none | Free text |
| `state_notes` | all | none | A note per state, see below |

- **Times** are strings in the UI's syntax, read by the same `parse_time` as the UI and the CLI: `"10"`, `"1:30"`, `"2:00:00"`, `"2d 06:00:00"`, `"0:00:12.5"`. Exact to the millisecond.
- **A dummy Station** has a process time of 0. It takes no time, and exists so that something can happen as an MU passes, such as user code at a hook later. It can still fail, like any Station.
- **A Source with an interval of 0** creates an MU and offers it at once. It holds one MU at a time and creates the next only once that MU has left, so it waits whenever its successor is busy.
- **Failures:**
  ```json
  "failures": { "availability": 0.9, "repair_time": { "erlang": { "mean": "1:00", "k": 2 } } }
  ```
  Availability is strictly between 0 and 1. The repair time is one of `{"constant": t}`, `{"uniform": {"min": t, "max": t}}`, `{"normal": {"mean": t, "std": t}}` or `{"erlang": {"mean": t, "k": n}}`, with its mean above 0. The time between failures is derived from the availability, as today.
- **State notes** say what a state means for this class or instance, beside crabsim's own meaning of it, which the window shows in its help on an object:
  ```json
  "state_notes": { "blocked": "waits for the conveyor to clear", "failed": "the gripper needs changing" }
  ```
  The states are `busy`, `blocked`, `idle`, `failed`, `forced_failure` and `paused`, and a note may name only states its built-in class can be in: a Source is busy or blocked, a Station any of them, a Buffer busy, blocked or idle, and a Drain only idle. A note can't be empty. Like every attribute, the notes are inherited as a whole, so an instance that sets its own replaces its class's. A note never changes what is measured or a state's colour.
- **Starting MUs** are a count. MU ids belong to a run, and are handed out when a run is built, in file order.
- **Visual only:** `position`, `dimensions`, `description` and `state_notes` never reach a run, so they can't change a result. The 3D model path will be one more inheritable attribute when there is a 3D view, a path and never embedded.

## Names

- Instance names and class names are each unique, compared exactly (case-sensitive).
- A name can't be empty or have spaces at either end, and may otherwise contain anything.
- Instances and classes are separate namespaces. A class can't take a built-in's name.

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
- Names that are missing, empty, padded or used twice, and a class named like a built-in.
- Unknown classes, a class deriving from itself, and classes deriving from each other in a circle, named in the order they go round.
- Connections to an unknown name, into a Source, out of a Drain, from an object to itself, or the same connection twice, each with the connection's pointer, such as `/connections/3/to`, and its ends as its name, such as `In → Mill`.
- Exits or entrances with gaps or repeats, such as an exit of 3 out of an object with 2 connections out, or two connections out of one object with exit 1.
- Attributes the class doesn't have, values out of range, and required values set nowhere in the chain.
- **A Source that would create MUs forever:** a Source with an interval of 0 that can reach a Drain through nothing but zero-time Stations and Buffers. Nothing ever fills, so it would create MUs without time passing. The run's zero-time-loop error stays as the backstop. Once Buffer is fixed and Station capacity exists, a full Buffer can block a Source, and this check gets refined.

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
