# The booking point decides every transfer

Each MU class has a booking point, a point in the MU's footprint, its centre by default, the same on every object. Wherever an MU passes to or from a geometry-aware object, the booking point is the one rule: the MU belongs to the object its booking point is on, which decides whose contents and statistics it is in, when its `MuMoved` occurrence happens, and which object's interruption stops it. Its footprint may meanwhile take space on two objects at once, so transfers between conveyors are smooth. We decided it on 2026-11-21 in the design round for conveyors, on the owner's proposal, because one rule for entering, leaving and straddling is easier to explain, test and read in results than a rule for each.

## Considered Options

- **A free length check:** an MU enters a conveyor only when its whole length plus a gap is free, and leaves whole and at once. Simple, but transfers between conveyors would jump, and a conveyor's capacity would be too small by part of an MU at each end.
- **A booking point per conveyor or per connection.** More flexible, but the same MU would change hands at different points on different conveyors, and lead times would be hard to read.

## Consequences

- **Between two geometry-aware objects:** the receiving one reserves the space under the MU's front from when the front touches it. A straddling MU moves at the slower of the two speeds. While one of them is interrupted, the one the MU belongs to decides: if its booking point is already on the next, it carries on at that one's speed over the stopped one; if not, it carries on onto the stopped one and stops once it belongs to it.
- **An MU never runs into the one ahead.** When the one ahead stops, the one behind closes up to its gap and stops too, whatever its own conveyor is doing.
- **Into a conveyor from an object without geometry,** such as a Station: the length from the MU's front to its booking point plus the conveyor's gap must be free at the start, and the MU is placed with its booking point at the start, the rest of it behind the start, still taking that space.
- **Out of a conveyor into an object without geometry:** when the front reaches the end, the successor is offered the MU. Refused, the MU waits there with its front at the end, in the successor's blocked line (ADR 0018). Accepted, it drives on until its booking point crosses the end, then belongs to the successor whole, and the conveyor's end is free at once.
- **Receiving:** between accepting and the booking point arriving, the successor is receiving, a state of its own, with its own colour and its own share of time in the statistics, its place taken from the acceptance. A transfer once begun always finishes, even if the object without geometry is interrupted meanwhile: an exception to an interrupted Station's or Drain's refusing MUs, for MUs it has already accepted only. Models without conveyors never enter it and pay nothing.
- **The booking point sets how fast handover is.** At the front, an accepted MU belongs to the successor at once and needs no free length to enter, as today's objects behave. At the rear, the whole MU drives in first and needs its whole length free to enter.
