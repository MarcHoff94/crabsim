# The conveyor

Spec issue 133, increment 1. The decisions are in ADR 0021 to ADR 0024. This note records how the engine carries them out, for PR 2 onwards. PR 2 builds a straight conveyor at constant speed between objects without geometry. PR 3 adds conveyor to conveyor and non-accumulating conveyors with interruptions while an MU is on two at once, and PR 4 adds the conveyor's statistics.

## Inside

- **One dimension.** An MU's place on a conveyor is where its front is, in metres from the start along the path. The conveyor holds its MUs front first. Each MU has a position at a time and whether it moves: it moves at the conveyor's speed or stands, so its position at any later time is a formula (ADR 0022). An MU's rear is its front less its length, and its booking point its front less its booking point's share of its length.
- **Its MU kinds.** A conveyor has its own copy of the model's table of MU kinds, given when the run is built, so it can read an MU's length, width and booking point without shared ownership.
- **Who moves.** The front MU moves unless it has reached the end and no successor has taken it. Every MU behind moves unless it has closed up to the gap behind a standing MU. A conveyor at constant speed moves every moving MU alike, so a moving MU only ever reaches a standing one, and the standing MUs are a block at the front. When the front MU moves again, the whole block moves with it at once.
- **Its events.** At most three are queued at a time, each worked out again whenever anything on the conveyor changes, with a count on each so that an older one is ignored when it fires, as `MUExit` does with `stop_count`:
  - the front MU's front reaching the end, or once a successor has taken it, its booking point crossing the end;
  - the first moving MU behind the block closing up to it;
  - room at the start for the first predecessor waiting that can fit, when the last MU's rear has passed the start by that MU's length ahead of its booking point plus the gap.
  A change replans these few events and the MUs whose movement changed, never the whole conveyor. Times are rounded up to the next millisecond, so a position is never short of where an event says it is, and positions are set exactly when the event fires.

## Taking an MU in

- **Whether it fits** depends on the MU, so a predecessor now says which kind of MU it offers (`SuccessorContainer::offered_kind`) and asks `MaterialFlow::accepts(kind, now)`, which for objects without geometry is `is_entrance_allowed`. A conveyor refuses an MU wider than itself or longer than its path, and otherwise takes it when the length from its front to its booking point, plus the gap, is free at the start.
- **Placed by its booking point.** It enters with its booking point at the start, the rest of it behind the start, still taking that space.
- **A predecessor refused** waits in the conveyor's line (ADR 0018). Listing a new one wakes the conveyor at once, through `add_blocked`, which now gets the event context, so it can work out when room will come. When it comes, every predecessor waiting gets a retry, longest waiting first, and those whose MU still doesn't fit keep their place. Retries go out once for each arrangement of the conveyor, so a predecessor that can never fit can't make a zero-time loop.

## Passing an MU on

- **Reserving.** When the front MU's front reaches the end, the conveyor offers its MU to its successors, in the order of its distribution strategy. A successor that can take it reserves a place for it (`MaterialFlow::reserve`) and is Receiving until the MU's booking point crosses the end. Then the conveyor hands it over through `process_mu_entrance`, and the `MuMoved` occurrence happens. With the booking point at the front, it is handed over at once.
- **Refused,** the front MU stands at the end, ready since its front arrived, in each successor's line, and is offered again on a retry.
- **Receiving** is a process state of its own (`ProcessState::Receiving`) and a statistic of its own (`time_receiving`). A Station of capacity 1 locks its entrance for it. A Station with capacity, a Buffer and a Drain count the places reserved. A Station working while it receives counts as working. A transfer once begun always finishes: an object without geometry that is interrupted while receiving still takes the MU, and a Station holds it with its work frozen until it is no longer interrupted.

## Interruptions

An interrupted conveyor stops every MU it holds, takes no MU and passes none on. A retry that finds it interrupted is answered as refused, so it keeps its place. When it is no longer interrupted, its MUs move again by the rules above, and the front MU at the end is offered again. An MU handed over part way stops with the conveyor, and its successor stays Receiving.

## Not in PR 2

A conveyor's successor can't be another conveyor yet, which a model file says is a problem until PR 3. Non-accumulating conveyors, the busy time in three parts, fill and crossing times come with PR 3 and PR 4.

## What a view sees

- **`MotionPlanned`** is an occurrence of an MU's new movement on a conveyor: where its front is at that time, its speed (0 standing) and its MU kind. It comes when an MU enters, stops or moves again. The Picture keeps each MU's latest one, and a view works out where it is at shown time.
- **A full state** holds every MU's movement now (`ObjectState::positions`), so the Picture is right after Init, Reset and going back.
- A headless run records neither, and pays only for the events.
