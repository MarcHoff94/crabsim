# A model changes a built-in class's values under the built-in's own name

A model file's classes may include one named like a built-in class (Source, Station, Buffer or Drain) with no `parent`. It holds this model's own values for that built-in class: every class and instance built on the built-in inherits them, after its own chain and before crabsim's defaults. A model has at most one for each built-in class. It stays format version 2, since every earlier file means the same as before, and a file using it is refused by a crabsim built before it, as an unknown parent would be.

We decided it on 2026-11-11, while building models in the window (spec issue 87). The owner wants everything in the window's class library editable, Station, Source and Drain included, and doesn't want crabsim's own built-in classes exposed or changed: a model's Station is something the user can change at once, and a new Station works 10 s on each MU, while the built-in Station's default stays 0. So the library's Station is the model's own, and the file needs somewhere to keep its values.

## Considered Options

- **A separate `defaults` section,** an object keyed by built-in class beside the classes. It keeps the class list free of special cases, but the values that every Station inherits would live in a second place with its own reading, checking, editing and showing, though they behave exactly like a class's.
- **Setting the 10 s on each placed Station,** as before this decision. It needs no format change, but changing what a Station is in a model would mean changing every Station, and the library's Station couldn't be edited.

## Consequences

- `Model::chain` ends with the model's class for the built-in, if it has one, so `resolve` and `get_attributes` show its values as from class `Station`, for example.
- `edit_model`'s `set_attribute` and `restore_inheritance` on `{"class": "Station"}` make and remove it. `remove_class` on the built-in's name removes only it and nothing built on it. Renaming it, changing its parent or duplicating it are refused, since the built-in class keeps its name and parent.
- The window gives the model's Station 10 s when it makes a model, and when the first Station is placed or the first class derived from Station in a model with none yet. A model already holding Stations without it keeps their values: a Station placed there gets the 10 s on itself instead, so no earlier result changes.
- `query_model`'s class tree and `export_model` show its values on the built-in class's node.
