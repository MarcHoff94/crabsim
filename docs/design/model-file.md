# The model file

Agreed with the owner on 2026-11-07 in a design round, as step 2 of the AI-native order of work (issue 28). The code is in `model/` (`crabsim-model`, ADR 0009). It's built in three slices: today's objects with the built-in classes only, then derived classes and inheritance, then the published JSON Schema. This document describes all three, and says where a slice isn't built yet.

## What it is

A model as a readable JSON file that people, agents and scripts can write, read, diff and keep in git. Most people will build models in the UI, which offers only valid choices, so the file's validation errors are mainly for agents and scripts.

A file holds the model only: classes, instances, their attributes, names, positions, dimensions, descriptions and connections. Seed, length, warm-up and the number of observations are inputs to a run and never appear in it.

## Top level

```json
{
  "format_version": 1,
  "description": "A line with two branches",
  "classes": [ ... ],
  "instances": [ ... ]
}
```

- `format_version` is required. Only 1 exists, and any other is refused with a clear error. When the format has a reason to change, the version goes up and older files are migrated.
- `description` is optional, for the model as a whole.
- `classes` is optional, and empty until slice 2.
- `instances` is in Init order: ids follow it.

## Instances

```json
{
  "name": "Mill 1",
  "class": "CNC mill",
  "position": { "x": 4, "y": 0 },
  "successors": ["Station 2", "Station 3"],
  "attributes": { "process_time": "0:45" }
}
```

- An instance owns `name` and `class`, both required, `position` and `successors`.
- `position` is in metres, the centre of the object's footprint: `x` and `y`, and `z` optional, 0 until there's a 3D view. Without a position, the UI places the object where it puts unplaced objects.
- `successors` names the instances an object passes MUs to, in the order they are tried. Optional, none by default.
- `attributes` holds only the values set on this instance, where its inheritance is cut. Anything not there comes from its class. It is left out when empty, so an instance that changes nothing is a name, a class, a position and its successors.

## Classes (slice 2)

```json
{ "name": "CNC mill", "parent": "Station", "attributes": { "process_time": "0:45", "description": "5-axis" } }
```

- The built-in classes are Source, Station, Buffer and Drain. A derived class names its `parent`, a built-in or another derived class, so chains are allowed.
- Classes can appear in any order. A cycle in the class tree is a validation error.
- A class writes only the attributes it changes from its parent, the same way an instance does.
- Until slice 2, an instance's class must be a built-in.

## Attributes

Which attributes an instance has depends on its built-in class at the root of its chain. Any other attribute is a validation error.

| Attribute | On | Built-in default | Rule |
|---|---|---|---|
| `interval` | Source | none: must be set somewhere in the chain | A time, 0 or more |
| `process_time` | Station | 0, a dummy Station | A time, 0 or more |
| `failures` | Station | none: never fails | See below |
| `starting_mus` | Station | 0 | 0 or 1, until Station capacity exists |
| `capacity` | Buffer | none: must be set somewhere in the chain | 1 or more. Buffer is known to be broken until step 7 of issue 3 |
| `dimensions` | all | by type: Station 2 × 1.5 × 1.5 m, Buffer 2 × 1 × 1 m, Source and Drain 1 m each way | `length`, `width`, `height` above 0, `rotation_degrees` optional, 0 by default |
| `description` | all | none | Free text |

- **Times** are strings in the UI's syntax, read by the same `parse_time` as the UI and the CLI: `"10"`, `"1:30"`, `"2:00:00"`, `"2d 06:00:00"`, `"0:00:12.5"`. Exact to the millisecond.
- **A dummy Station** has a process time of 0. It takes no time, and exists so that something can happen as an MU passes, such as user code at a hook later. It can still fail, like any Station.
- **A Source with an interval of 0** creates an MU and offers it at once. It holds one MU at a time and creates the next only once that MU has left, so it waits whenever its successor is busy.
- **Failures:**
  ```json
  "failures": { "availability": 0.9, "repair_time": { "erlang": { "mean": "1:00", "k": 2 } } }
  ```
  Availability is strictly between 0 and 1. The repair time is one of `{"constant": t}`, `{"uniform": {"min": t, "max": t}}`, `{"normal": {"mean": t, "std": t}}` or `{"erlang": {"mean": t, "k": n}}`, with its mean above 0. The time between failures is derived from the availability, as today.
- **Starting MUs** are a count. MU ids belong to a run, and are handed out when a run is built, in file order.
- **Visual only:** `position`, `dimensions` and `description` never reach a run, so they can't change a result. The 3D model path will be one more inheritable attribute when there is a 3D view, a path and never embedded.

## Names

- Instance names and class names are each unique, compared exactly (case-sensitive).
- A name can't be empty or have spaces at either end, and may otherwise contain anything.
- Instances and classes are separate namespaces. A class can't take a built-in's name.

## Validation

Loading returns every problem at once and never panics. Each problem has:

- a JSON Pointer into the file, such as `/instances/3/attributes/process_time`,
- the name of the instance or class it is about, when there is one,
- a plain message.

They come in file order, as JSON for agents, and one per line on the CLI:

```
/instances/3/attributes/process_time "Station 3": "1:75" is not a time
```

What it catches:

- JSON that doesn't parse, a missing or unsupported `format_version`, wrong types, and unknown fields, such as a typo like `proces_time`, which would otherwise leave a value silently inherited.
- Names that are missing, empty, padded or used twice, and a class named like a built-in.
- Unknown classes and, from slice 2, cycles among classes.
- Connections to an unknown name, into a Source, out of a Drain, from an object to itself, or to the same successor twice.
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

- `models/demo.json` is the demo model as a file. A test checks it builds a run identical to `demo::model()` for the same seed.
- The benchmark's 6,006-object model is generated in code and checked to round-trip as a file in a test, rather than committed, since it is about 2 MB.

## The CLI

```bash
cargo run --release --bin crabsim -- --model models/demo.json --seed 7 --length 30d
```

Without `--model` it runs the built-in demo. An invalid file prints every problem and exits with failure, without running.
