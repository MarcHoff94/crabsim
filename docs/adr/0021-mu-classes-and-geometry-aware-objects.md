# MU classes, and objects whose geometry counts

MUs get classes in the class library, beside the object classes, and every Source is set to exactly one MU class, whose values it may change for the MUs it creates but never leaves unset. An MU class gives the MU its footprint, a length and width in metres, and its booking point (ADR 0023), and later its own motion limits and what it can carry. Objects split in two: a Source, Station, Buffer or Drain ignores sizes, as before, and takes any MU, while a geometry-aware object, such as a conveyor or a cross conveyor, reads its own dimensions and path and the MU's footprint, and what it can take depends on them. We decided it on 2026-11-21 in the design round for conveyors, so that conveyors can hold as many MUs as fit without making every existing object, and every model without conveyors, pay for geometry.

## Considered Options

- **Dimensions stay visual everywhere,** with a conveyor holding a number of MUs. Simple, but a conveyor's capacity then has nothing to do with its length, and an MU wider than the conveyor can't be refused.
- **Every object becomes geometry-aware.** One rule for all, but every Station and Buffer would need a size check and space bookkeeping for no gain in most models.
- **Sizes stamped by the Source alone, with no MU class.** Fewer concepts, but pallets and self-driven MUs, which come later, need a class to hang their behaviour on.

## Consequences

- Every MU carries a small class id and a destination id (amending ADR 0004), so the per-MU cost is two small integers. The footprint, booking point and motion limits are looked up in the class's table, never copied into each MU.
- A Source's MUs all have its MU class, with its changes, so a model file can show statically which MU class reaches which conveyor. An MU that doesn't fit is refused at run time like a locked successor, never a run error, and `validate_model` warns when the model file already shows an MU class will never fit a conveyor it can reach.
- A pallet counts only by its own footprint. Geometry-aware objects ask an MU for its footprint and never sum what it carries, and carried MUs never stick out.
- A geometry-aware object is a material-flow object like any other: it has a class, connections, interruptions and statistics, and joins the existing blocked-line rules (ADR 0018).
