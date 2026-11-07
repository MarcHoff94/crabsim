# UI prototype

Step 2 of the UI's order of work, measured on 2026-11-07 on the owner's PC: Ryzen 5 7600X3D, 31 GB, AMD Radeon RX 9070 XT, Windows 11, Rust 1.99.1, Bevy 0.20.1, `bevy_egui` 0.43.1 (egui 0.36.2). The window was 1600 by 900 logical pixels at a scale factor of 1.75. The code is throwaway and lives on the branch `prototype/ui-bevy`, never to be merged. The pass marks are in the handoff's "UI design".

**Result:** Bevy with `bevy_egui` holds up. The scale and session marks pass with a wide margin. The real session failed at first, and the cause was a bug in the live session, not the technology: shown time lost about 30% at high load. A two-line fix makes it hold the asked speed. Panels and canvas work together, and a halted window uses almost no CPU once nothing on screen changes. The owner tried it and it felt good. Two things changed as a result, both under "The owner's check" below: objects can now be dragged on the canvas, and MUs no longer glide.

## What was built

One program with three modes, all drawing the demo line repeated 429 times, 6,006 objects, at hand-written positions in metres:

- **Scale test:** synthetic, with no simulation. Every frame about 5% of objects change state colour, and 5,000 markers glide along connections without stopping.
- **Session:** a real `LiveSession` on a wall clock. Every frame the window drains the channel and rebuilds the picture from occurrences, using a copy of the session tests' rebuild helper. MUs are drawn in the object that holds them when zoomed in at 6 pixels per metre or more and running at 120 times real time or slower. At first they glided between objects instead: see "The owner's check".
- **Windowless session:** the same session with a loop reading observations and no window, to tell the window's cost from the session's.

Each draws either with sprites or with flat meshes under an orthographic 3D camera, the route to a 3D view. Every frame also draws all 7,722 connections as gizmo lines. egui panels sit over the canvas: a top bar, a palette whose buttons drag into the canvas, and an inspector for the object clicked. Labels come from egui's painter when zoomed in to 25 pixels per metre or more.

Figures are single runs of 15 to 30 seconds, with frame rates uncapped. Asking for vsync didn't cap the frame rate here, and that wasn't looked into. A frame time is the wall time between frames, after the first second.

## 1. Scale

6,006 objects, 5% of them changing colour each frame, 5,000 gliding markers:

| Renderer | View | Frames per second | Frame time p50 | p99 | Worst | Frames over 33 ms |
|---|---|---|---|---|---|---|
| Sprites | Whole model | 484 | 2.17 ms | 2.81 ms | 16.7 ms | 0 |
| Flat meshes | Whole model | 367 | 2.80 ms | 3.33 ms | 17.4 ms | 0 |
| Sprites | Zoomed in, with labels | 652 | 1.50 ms | 2.02 ms | 19.0 ms | 0 |
| Flat meshes | Zoomed in, with labels | 498 | 1.98 ms | 2.47 ms | 14.1 ms | 0 |

**Passes,** with about five times the 60 frames per second asked for, on either renderer. Flat meshes cost about 30% more frame time than sprites, which is a small price for drawing 2D the way 3D will be drawn. Zoomed in is faster, because Bevy draws only what is in view. The worst frames, around 17 ms, are single frames. They can't be told apart from the operating system, because the median stays near 2 ms.

## 2. A real session

6,006 objects at 3,600 times real time, 1 simulated hour per second, after the session fix below:

| Renderer | Achieved speed | Occurrences per second | Frame time p50 | p99 | Worst | Frames over 33 ms |
|---|---|---|---|---|---|---|
| Flat meshes | 3,599 times | 1,869,000 | 1.91 ms | 5.68 ms | 16.0 ms | 0 |
| Sprites | 3,600 times | 1,870,000 | 1.96 ms | 3.70 ms | 16.9 ms | 0 |

- **Draining and applying** the channel's occurrences to the picture takes 0.18 ms per frame at p99, and 1.2 ms at worst, for up to 132,000 occurrences in one frame. So rebuilding the picture in Rust each frame is cheap.
- **The occurrence rate matches the calculation** in the handoff, about 1.9M per second.
- **Zoomed in at 60 times real time,** with MUs drawn in their objects, the median frame was 1.74 ms, p99 2.31 ms and the worst 13.1 ms. With MUs gliding instead, as first built, the median was 2.35 ms, but that run had a build beside it.
- **Passes** on the numbers. Whether playback looks smooth is for the owner's eyes.

### The session bug this found

Before the fix, the session reached only about 72% of the asked speed, at 1,800 and at 3,600 times real time, and with no window at all. A windowless session run flat out reached 8.6M occurrences per second, about 6.5 times what 3,600 times needs, so computing wasn't short. Counters in a throwaway copy of the session showed all of the loss came from the rule that shown time waits, without debt, while computing is behind: 2.9 s lost in 10 s, in 1,460 waits. Yet the buffer held about 96,000 of its 100,000 instants when each wait began.

**The cause:** `release` reads the clock once to decide what to release, and again afterwards through `target()` to decide whether it is waiting. Instants that fell due between the two readings look as if computing hasn't reached them, so the session starts waiting. With its buffer full it then sleeps on a 16 ms timeout, which averaged 25 ms here, and that whole sleep comes off shown time.

**The fix:** use one clock reading for the whole release. With it, the windowless session held exactly 360, 3,600 and 7,200 times real time, the last at 3.7M occurrences per second, with no waits. A test can catch this with a fake clock that moves on every time it is read. The fix belongs on `master` as its own change, with that test.

## 3. Panels and canvas together

- **Halted:** with the window in Bevy's reactive mode, the process used 0.006 CPU seconds per second and drew 3 frames in 16 seconds. **But only once nothing on screen changed.** The first attempt still showed the frame rate in the top bar. Each frame changed that text, egui asked to repaint, and `bevy_egui` turned that into a redraw. That kept it at about 70 frames per second and a third of a core. A halted UI must show nothing that changes by itself.
- **Advancing:** 2.1 to 2.4 CPU cores at 3,600 times real time, with the frame rate uncapped, the session thread included.
- **Input to the right layer:** canvas input runs only when egui doesn't want the pointer, through `bevy_egui`'s run condition.
- **Selecting:** a click finds the nearest object by scanning all 6,006 positions. It wasn't timed.
- **Dragging from the palette:** a ghost follows the pointer, and letting go over the canvas adds an object at that point in the world.
- **Moving an object:** a left press on an object selects it and starts a drag, and its connections follow. Added after the owner's check.

## 4. Build times

| Build | Time |
|---|---|
| Clean release, with the engine's thin LTO and one codegen unit | About 4 min 17 s, for the dependencies |
| Incremental release, thin LTO and one codegen unit, one line changed | 1 min 53 s |
| Clean release, Cargo's default profile | 4 min 54 s |
| Incremental release, Cargo's default profile | 7 to 8 s |
| Clean debug, with dependencies optimised | 8 min 9 s, while other runs used the CPU, so it overstates |
| Incremental debug, one line changed | 9 s |

**Judgement:** the clean builds are a one-off cost of 4 to 8 minutes. Day to day, a change rebuilds in 7 to 9 s, in debug or in a release profile without LTO, but not with the engine's tuned release profile. The engine keeps that profile for the benchmark. How fast the debug build runs wasn't measured.

## The owner's check

The owner ran it by hand. It felt good, with two findings:

- **Objects dropped from the palette couldn't be moved.** The prototype only built dropping. Dragging any object on the canvas was added afterwards, and the owner confirmed it works.
- **MUs shouldn't glide.** A move takes no simulation time, and a glide suggests travel time that isn't there. So MUs are now drawn in the object that holds them and jump straight to the next one. Gliding is kept for a future conveyor, which has real transport time. This replaces the gliding agreed in the UI design, and the handoff, glossary and spec now say so.

## Other findings

- **The engine logs every event at info level.** Bevy's logger shows `log` output, so the first session run printed a line per event. The prototype filters it with `crabsim=warn`. The real UI needs the same filter, or the engine should log events at a quieter level.
- **Init's occurrences come before Init's full state,** and the state already includes them. A picture has to ignore occurrences until the first state arrives, or it fails on an empty object list. The Picture in the library should handle this itself.
- **The 2D-to-3D route held:** the flat-mesh path draws through Bevy's 3D pipeline with an orthographic camera and unlit materials, one material per state colour.
- **Bevy 0.20's API** gave little trouble. The program compiled after one borrow-checker fix, written from the crates' own sources and examples.

## Limits

- **Single runs** of 15 to 30 seconds each, on one PC with a fast GPU, and the frame rate wasn't capped.
- **Synthetic load:** the scale test's colour changes and glides are random, not from a simulation.
- **Not tried:** text labels drawn by Bevy rather than egui, and a model with real layout and dimensions.
- **The session fix was tested only in the prototype,** not with the session's own tests.
