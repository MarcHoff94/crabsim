# A conveyor's path is a chain of segments, and only its length and limits count

A conveyor's path is a list of segments, each a straight, a circular arc, a transition curve (clothoid) or a helix, any of them rising or falling, written in the model file relative to the instance's own position and rotation, such as `{"straight": 2.0}` or `{"helix": {"radius": 1.2, "turns": 2, "rise": 3.0}}`. Segments join without kinks, and validation checks each conveyor's tightest bend and steepest incline against its limits, which is what "physically possible" means. The engine reads only the path's length and those limits: slope and curvature change neither speed nor physics. We decided it on 2026-11-21 in the design round for conveyors, so that any physically possible curve, including spirals, can be built in readable JSON that people and agents can write, while the engine stays one-dimensional along each conveyor and as fast as before.

## Considered Options

- **Free splines (Bézier, B-spline) through control points.** Any shape at all, but hard to type, to validate and to read, and their length needs numeric integration. A spline segment can be added later as an escape hatch.
- **Absolute 3D points,** with segments fitted through them. Easy to place on a layout image, but moving a conveyor would rewrite its path.
- **Slope and curvature in the physics,** such as gravity on an incline or slip in a bend. More realistic, but it is no longer a discrete-event model, and would cost speed on every conveyor. A per-conveyor speed limit in tight bends can come later, as a limit, not as physics.

## Consequences

- Each segment has a closed-form length, so a path's length costs nothing at run time.
- A conveyor connects only at its two ends, its start and its end along its direction. Side feeds and diverts go through a cross conveyor. Connections attach to the computed pose of the path's ends.
- The first UI edits the segment list in the object window and draws the path's plan view on the 2D canvas, with rises as labels. Drag handles come later.
