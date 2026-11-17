# Blocked predecessors wait in line by how long their MU has been ready to leave

When a successor can't take an MU, the predecessor offering it joins that successor's line. The line is ordered by when each predecessor's offered MU became ready to leave, longest ready first, and those ready at the same instant in the order they joined. A predecessor whose MU moves gives up its place wherever it was listed, and if it has another MU ready, it waits on in the same lines by that MU's time. So the MU that has waited longest goes first, whichever object holds it.

With one MU per object this is the order crabsim had before: a predecessor joined the line when its MU became ready, at the back. It matters from the moment an object holds several MUs, a Buffer first and a Station with a capacity next.

We decided it on 2026-11-13 and 2026-11-15 in the design round for build step 7 (spec issue 110).

## Considered Options

- **A predecessor keeps its place until it's empty.** A full Buffer at the front of a line would send every MU it holds before anyone else gets a turn, and the others starve.
- **Taking turns per object:** after a predecessor's MU moves, it rejoins at the back. Simple and close to the old rule, but an MU that has been ready for an hour in a Buffer would wait behind one that became ready a second ago in a Station.
- **By MU**, chosen: it is what a modeller expects of a line in front of a machine, and it needs no setting.

## How it works

- **Ready to leave** means: for a Source, its interval has ended. For a Station, its work is finished. For a Buffer, the MU its exit strategy would pass on has dwelt its time. A Station that was stopped while blocked releases nothing during the stop, so its MU counts as ready again from the end of the stop, which is what happened before too.
- **A place has its own time.** A predecessor waiting at two successors can hold a different place in each line. When a successor has room, it sends a retry to everyone in its line, longest ready first. A predecessor whose retry is refused, because someone ahead took the room, is back in the place it had. One whose MU went in, or whose retry found it stopped, has lost that place.
- **A last in, first out Buffer keeps the place it has.** The MU it would pass on changes whenever a newer one becomes ready, but while it waits, its place stays that of the MU it was first refused with. Otherwise a stack that keeps receiving MUs would fall back in every line each time.
- **Retries still come first** at an instant, and still offer only the successor that sent them, so nothing reaches a free successor ahead of those waiting for it.

## Consequences

- An insert in order into a short list when a predecessor is refused, which is nearly always at the back, and one more small call when an MU moves. Nothing on other events. The first version made headless runs 3 to 4% slower on the owner's second PC. Part of that was the line's bookkeeping, made cheaper since, and part a Station grown by the 8 bytes of when its MU became ready, which every object pays because each takes the largest one's size. A Station now keeps that time in the field that holds when its work completes, since the two are never needed at once. The fixed version is still to be measured against `master`, and the ±2% the spec asks for to be confirmed, before merging.
- A model without Buffers gives exactly the results it gave before: every occurrence of the demo and of two other models, over 20 simulated days on three seeds, hashed the same as on `master`.
- Whether a stopped predecessor should keep its place, instead of rejoining at the back after its repair, is decided with stops in the second round of build step 7.
- Pull, where the successor chooses the MU, would replace this line with the successor's own rule.
