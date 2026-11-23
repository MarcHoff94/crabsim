# The conveyor

Spec issue 133, increment 1. The decisions are in ADR 0021 to ADR 0024. This note records how the engine carries them out, for PR 2 onwards. PR 2 builds a straight conveyor at constant speed between objects without geometry. PR 3 adds conveyor to conveyor and non-accumulating conveyors with interruptions while an MU is on two at once, and PR 4 adds the conveyor's statistics (both built).

## Inside

- **One dimension.** An MU's place on a conveyor is where its front is, in metres from the start along the path. The conveyor holds its MUs front first. Each MU has a position at a time and a speed: the conveyor's, the slower speed of an MU on two conveyors, or 0 standing, so its position at any later time is a formula (ADR 0022). An MU's rear is its front less its length, and its booking point its front less its booking point's share of its length.
- **Its MU kinds.** A conveyor has its own copy of the model's table of MU kinds, given when the run is built, so it can read an MU's length, width and booking point without shared ownership.
- **Who moves.** Every MU moves at its own speed unless it has closed up to the one ahead, when it moves at that one's speed and no faster, keeping the gap. The front MU stands at the end while no successor has taken it. So a standing front MU brings the MUs behind it to a stop one by one. Each pass (`settle`) walks the MUs from the front, gives each its speed, and works out the first that is faster than the one ahead and when it will close up. An event rounded up to a millisecond may find an MU a little past where it closes up, so one that has closed up is put back at the gap.
- **Its events.** At most five are queued at a time (the fifth, `Clear`, for its statistics: see below), each worked out again whenever anything on the conveyor changes, with a count on each so that an older one is ignored when it fires, as `MUExit` does with `stop_count`:
  - the front MU's front reaching the end, or once a successor has taken it, its booking point crossing the end;
  - the first MU that is faster than the one ahead closing up to it, never sooner than a millisecond;
  - the last MU, handed over by a conveyor before this one, passing the point where that conveyor stops holding it back, and then where its rear is clear by the gap (`Straddle`);
  - room at the start for the first predecessor waiting that can fit, when the last MU's rear has passed the start by that MU's length ahead of its booking point plus the gap, or by the gap alone for a conveyor's MU, which enters with its front at the start.
  A change replans these few events and the MUs whose movement changed. Times are rounded up to the next millisecond, so a position is never short of where an event says it is.

## Taking an MU in

- **Whether it fits** depends on the MU, so a predecessor now says which kind of MU it offers (`SuccessorContainer::offered_kind`) and asks `MaterialFlow::accepts(kind, now)`, which for objects without geometry is `is_entrance_allowed`. A conveyor refuses an MU wider than itself or longer than its path, and otherwise takes it when the length from its front to its booking point, plus the gap, is free at the start.
- **Placed by its booking point.** It enters with its booking point at the start, the rest of it behind the start, still taking that space.
- **A predecessor refused** waits in the conveyor's line (ADR 0018). Listing a new one wakes the conveyor at once, through `add_blocked`, which now gets the event context, so it can work out when room will come. When it comes, every predecessor waiting gets a retry, longest waiting first, and those whose MU still doesn't fit keep their place. Retries go out once for each arrangement of the conveyor, so a predecessor that can never fit can't make a zero-time loop.

## Passing an MU on

- **Reserving.** When the front MU's front reaches the end, the conveyor offers its MU to its successors, in the order of its distribution strategy. A successor that can take it reserves a place for it (`MaterialFlow::reserve`) and is Receiving until the MU's booking point crosses the end. Then the conveyor hands it over through `process_mu_entrance`, and the `MuMoved` occurrence happens. With the booking point at the front, it is handed over at once.
- **Refused,** the front MU stands at the end, ready since its front arrived, in each successor's line, and is offered again on a retry.
- **Receiving** is a process state of its own (`ProcessState::Receiving`) and a statistic of its own (`time_receiving`). A Station of capacity 1 locks its entrance for it. A Station with capacity, a Buffer and a Drain count the places reserved. A Station working while it receives counts as working. A transfer once begun always finishes: an object without geometry that is interrupted while receiving still takes the MU, and a Station holds it with its work frozen until it is no longer interrupted.

## Conveyor to conveyor

- **Reserving.** An MU reaching the end of a conveyor `A` is offered to the conveyor `B` after it as to any successor, but `B` accepts it when the last MU on `B` has its rear a gap past the start, since it enters front first (`accepts` is asked with `from_belt`). `B` reserves the space under the MU's front from when it touches the start: nothing else may enter `B` until `A` has handed the MU over.
- **Until the booking point crosses the end,** the MU is `A`'s, in `A`'s belt, its front already on `B`. It moves at the slower of the two speeds, but while `B` is interrupted at `A`'s own. It must not run into `B`'s last MU either, so `A` asks `B` for its rear and speed (`tail_lead`) as for an MU ahead, and stands if `B`'s last MU stands too close to the start.
- **From then on** the MU is `B`'s, with its rear still on `A` if it is longer than what its booking point has left behind. It moves at `B`'s speed or, while its rear is on `A` and `A` is running, the slower of the two. `A`'s MUs behind it close up to its rear as to an MU ahead (`led_by`, `lead`), until `B` says its rear is a gap past the start.
- **Neighbours are woken, never polled.** What one conveyor does to the other's MUs changes only when the other's MUs move differently: an interruption beginning or ending, the straddling MU passing a point, `B`'s last MU moving differently while `A`'s MU waits to enter. Each wakes the other once through an ordinary event, and a woken conveyor wakes nobody, so there is no loop.
- **Non-accumulating** conveyors move every MU as one: at the slowest speed any MU on it is held to, which is 0 while the front MU waits at the end or stands behind another's. They start again together.

## Interruptions

An interrupted conveyor stops every MU it holds, takes no MU and passes none on. A retry that finds it interrupted is answered as refused, so it keeps its place. When it is no longer interrupted, its MUs move again by the rules above, and the front MU at the end is offered again. An MU handed over part way stops with the conveyor, and its successor stays Receiving. An MU on two conveyors follows the one it belongs to (ADR 0023): with its booking point still on the stopped one it stops there; having crossed to a stopped one it stands; having crossed to a running one it goes on over the stopped one, at the running one's speed.

## Statistics (PR 4)

- **Busy in three parts,** as a Station's with capacity (`MachineState`): moving MUs while the MU at the end waits for a successor is partly blocked, which outranks the other two; otherwise moving with its start taken, the last MU's rear less than the gap past the start or a conveyor before it holding a reservation, so that no MU of any size could enter, is busy and full (`time_busy_full`); and moving with room at its start is busy, not full. Every MU standing is blocked, and empty is idle. A conveyor is never Receiving.
- **One more event.** The start becoming free changes the state without anything else happening, so while the last MU moves with its rear short of the gap a fifth event (`Clear`) is queued for when it gets there. It costs one event per MU that enters.
- **Fill** is a Buffer's: the same `FillKeeper`, fed from the stretches the `StatisticsKeeper` hands back, so `ObjectState::fill` and everything that reads a Buffer's histogram read a conveyor's too.
- **Length taken** is the whole footprints of the MUs it holds, which change only when an MU enters or leaves, so it needs no event of its own: its time-weighted sum (`BeltStatistics::length_time`) and the most for any length of time. An MU entering or leaving counts whole, so shares are capped at 1.
- **Crossing time** is from an MU's booking point crossing the start, when it enters, to crossing the end, when it is handed over: its count, total and longest, the longest including the MU on it longest, as a Drain's exit interval includes the one in progress.
- All of it is boxed in the conveyor (`BeltKeeper`), fixed in size, and reported as `ObjectState::belt`.
- **Measured** on the benchmark's `conveyor-line` scenario (`demo::conveyor_line`), with the `counters` feature: a conveyor works out its events again about once every two events, walking about 9 MUs each time.

## What a view sees

- **`MotionPlanned`** is an occurrence of an MU's new movement on a conveyor: where its front is at that time, its speed (0 standing) and its MU kind. It comes when an MU enters, stops or moves again. The Picture keeps each MU's latest one, and a view works out where it is at shown time.
- **A full state** holds every MU's movement now (`ObjectState::positions`), so the Picture is right after Init, Reset and going back.
- A headless run records neither, and pays only for the events.
