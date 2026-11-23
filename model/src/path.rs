//! A conveyor's path (ADR 0024, spec issue 141): its segments, the short form people and agents
//! type them in, and everything worked out from them: where it ends, how long it is, its bounds,
//! its tightest bend and its steepest incline. The engine reads only its length
//! ([`path_length`]); the catalogue and the window read the rest.
//!
//! A path starts at its conveyor's position, heading along its rotation, and each segment starts
//! where the one before ends, heading the same way, so the path has no kinks in plan. A segment's
//! length is in plan, as laid out from above, and its rise climbs evenly over it, so its length
//! in 3D is the hypotenuse of the two.

use std::f64::consts::{PI, TAU};

use serde_json::{Value, json};

/// One piece of a conveyor's path. Angles and turns are positive to the left, the way
/// `rotation_degrees` turns, and negative to the right. Every `rise` is in metres, negative falling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// `length` metres straight on, in plan.
    Straight { length: f64, rise: f64 },
    /// A circular arc of `radius` metres turning through `angle` degrees.
    Arc { radius: f64, angle: f64, rise: f64 },
    /// A clothoid of `length` metres in plan, its bend changing evenly from the bend the segment
    /// before ended with (straight for the first) to `to`.
    Transition { length: f64, to: Bend, rise: f64 },
    /// A spiral of `radius` metres in plan, turning `turns` times while it climbs `rise`, which is
    /// never 0: a flat turn is an arc.
    Helix { radius: f64, turns: f64, rise: f64 },
}

/// What a transition curve eases to: straight, or a radius in metres, positive turning left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bend {
    Straight,
    Radius(f64),
}

impl Bend {
    /// Its curvature, 1 over its radius, positive to the left.
    pub fn curvature(self) -> f64 {
        match self {
            Bend::Straight => 0.0,
            Bend::Radius(radius) => 1.0 / radius,
        }
    }
}

impl Segment {
    /// A flat straight of `length` metres.
    pub const fn straight(length: f64) -> Segment {
        Segment::Straight { length, rise: 0.0 }
    }

    /// What a file and the short form call its kind.
    pub fn kind(self) -> &'static str {
        match self {
            Segment::Straight { .. } => "straight",
            Segment::Arc { .. } => "arc",
            Segment::Transition { .. } => "transition",
            Segment::Helix { .. } => "helix",
        }
    }

    pub fn rise(self) -> f64 {
        match self {
            Segment::Straight { rise, .. } | Segment::Arc { rise, .. } | Segment::Transition { rise, .. } | Segment::Helix { rise, .. } => rise,
        }
    }

    /// Its length in plan, as laid out from above.
    pub fn plan_length(self) -> f64 {
        match self {
            Segment::Straight { length, .. } | Segment::Transition { length, .. } => length,
            Segment::Arc { radius, angle, .. } => radius * angle.to_radians().abs(),
            Segment::Helix { radius, turns, .. } => radius * TAU * turns.abs(),
        }
    }

    /// Its length in 3D, rise included: what its MUs travel.
    pub fn length(self) -> f64 {
        self.plan_length().hypot(self.rise())
    }

    /// How steeply it climbs or falls, in degrees.
    pub fn incline_degrees(self) -> f64 {
        self.rise().abs().atan2(self.plan_length()).to_degrees()
    }

    /// Its curvature at its end: for an arc or a helix all along it.
    fn end_curvature(self) -> f64 {
        match self {
            Segment::Straight { .. } => 0.0,
            Segment::Arc { radius, angle, .. } => angle.signum() / radius,
            Segment::Helix { radius, turns, .. } => turns.signum() / radius,
            Segment::Transition { to, .. } => to.curvature(),
        }
    }

    /// Its tightest radius in plan, given its curvature at its start: none for a straight, or a
    /// transition between two straights.
    fn tightest_radius(self, start_curvature: f64) -> Option<f64> {
        match self {
            Segment::Straight { .. } => None,
            Segment::Arc { radius, .. } | Segment::Helix { radius, .. } => Some(radius.abs()),
            Segment::Transition { to, .. } => {
                let most = start_curvature.abs().max(to.curvature().abs());
                (most > 0.0).then(|| 1.0 / most)
            }
        }
    }
}

/// A path's length in 3D: what a conveyor's MUs travel, and the only thing about it the engine
/// reads.
pub fn path_length(path: &[Segment]) -> f64 {
    path.iter().map(|segment| segment.length()).sum()
}

/// A path's length in plan.
pub fn plan_length(path: &[Segment]) -> f64 {
    path.iter().map(|segment| segment.plan_length()).sum()
}

/// Each segment's curvature at its start: the curvature the segment before it ended with, and
/// straight for the first.
pub fn start_curvatures(path: &[Segment]) -> Vec<f64> {
    let mut curvature = 0.0;
    path.iter()
        .map(|segment| {
            let start = curvature;
            curvature = segment.end_curvature();
            start
        })
        .collect()
}

/// Whether the transition at `index` has a bend to ease from or to: one that ends at the bend it
/// starts at does nothing an arc or a straight wouldn't.
pub fn transition_eases(path: &[Segment], index: usize) -> bool {
    match path[index] {
        Segment::Transition { to, .. } => (start_curvatures(path)[index] - to.curvature()).abs() > 1e-12,
        _ => true,
    }
}

/// A conveyor's path until given one: 5 m straight.
pub const DEFAULT_PATH: [Segment; 1] = [Segment::straight(5.0)];

/// Which way a conveyor's MUs travel along its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// From its first segment to its last, the default.
    #[default]
    Forward,
    /// From its last segment to its first.
    Backward,
}

impl Direction {
    pub fn name(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        }
    }

    pub fn from_name(name: &str) -> Option<Direction> {
        match name {
            "forward" => Some(Direction::Forward),
            "backward" => Some(Direction::Backward),
            _ => None,
        }
    }
}

// ---------- Geometry ----------

/// A place on a path and the way it heads there, in metres and degrees, as a model file writes
/// positions and rotations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Which way the path heads in plan, in degrees, counter-clockwise from the x axis.
    pub heading_degrees: f64,
}

/// The smallest rectangle in plan, along the axes, that holds a conveyor's whole width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    /// Its centre in plan.
    pub fn centre(self) -> (f64, f64) {
        ((self.min_x + self.max_x) / 2.0, (self.min_y + self.max_y) / 2.0)
    }
}

/// Everything worked out about a conveyor's path, with its direction applied: its start is where
/// its MUs enter and its end where they leave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Its length in 3D, which its MUs travel.
    pub length: f64,
    pub plan_length: f64,
    pub start: Pose,
    pub end: Pose,
    pub bounds: Bounds,
    /// Its tightest bend's radius in plan, none if it is straight throughout.
    pub tightest_radius: Option<f64>,
    /// Its steepest climb or fall, in degrees.
    pub steepest_incline_degrees: f64,
}

/// Where a path starts and how it lies: the conveyor's position and rotation, and its width and
/// direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Laid {
    pub origin: Pose,
    pub width: f64,
    pub direction: Direction,
}

/// The state at a point along the path, as the walk carries it on.
#[derive(Debug, Clone, Copy)]
struct At {
    x: f64,
    y: f64,
    z: f64,
    /// Heading in radians.
    theta: f64,
    curvature: f64,
}

impl At {
    fn pose(self) -> Pose {
        Pose { x: self.x, y: self.y, z: self.z, heading_degrees: self.theta.to_degrees() }
    }
}

/// Where `segment`, starting at `at`, has got to after `u` metres in plan.
fn along(segment: Segment, at: At, u: f64) -> At {
    let plan = segment.plan_length();
    let z = at.z + if plan > 0.0 { segment.rise() * u / plan } else { 0.0 };
    match segment {
        Segment::Straight { .. } => At { x: at.x + at.theta.cos() * u, y: at.y + at.theta.sin() * u, z, theta: at.theta, curvature: 0.0 },
        Segment::Arc { .. } | Segment::Helix { .. } => {
            let k = segment.end_curvature();
            let theta = at.theta + k * u;
            At {
                x: at.x + (theta.sin() - at.theta.sin()) / k,
                y: at.y - (theta.cos() - at.theta.cos()) / k,
                z,
                theta,
                curvature: k,
            }
        }
        Segment::Transition { length, to, .. } => {
            let (k0, k1) = (at.curvature, to.curvature());
            let rate = (k1 - k0) / length;
            let heading = |s: f64| at.theta + k0 * s + rate * s * s / 2.0;
            let (dx, dy) = integrate(heading, u, k0.abs().max((k0 + rate * u).abs()) * u);
            At { x: at.x + dx, y: at.y + dy, z, theta: heading(u), curvature: k0 + rate * u }
        }
    }
}

/// The integral of (cos, sin) of `heading` from 0 to `u`, by Gauss-Legendre on panels each
/// turning at most a tenth of a radian, so a clothoid's place is right to well under a
/// micrometre. `turning` bounds how far the heading turns over the whole stretch.
fn integrate(heading: impl Fn(f64) -> f64, u: f64, turning: f64) -> (f64, f64) {
    const NODES: [(f64, f64); 5] = [
        (0.0, 0.568_888_888_888_888_9),
        (-0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (0.538_469_310_105_683_1, 0.478_628_670_499_366_5),
        (-0.906_179_845_938_664, 0.236_926_885_056_189_1),
        (0.906_179_845_938_664, 0.236_926_885_056_189_1),
    ];
    if u <= 0.0 {
        return (0.0, 0.0);
    }
    let panels = (turning / 0.1).ceil().clamp(1.0, 100_000.0) as usize;
    let width = u / panels as f64;
    let (mut x, mut y) = (0.0, 0.0);
    for panel in 0..panels {
        let middle = (panel as f64 + 0.5) * width;
        for (node, weight) in NODES {
            let theta = heading(middle + node * width / 2.0);
            x += weight * theta.cos() * width / 2.0;
            y += weight * theta.sin() * width / 2.0;
        }
    }
    (x, y)
}

/// Walks the path from its first point, giving each segment with where it starts.
fn walk(path: &[Segment], origin: Pose) -> Vec<(Segment, At)> {
    let mut at = At { x: origin.x, y: origin.y, z: origin.z, theta: origin.heading_degrees.to_radians(), curvature: 0.0 };
    path.iter()
        .map(|&segment| {
            let start = at;
            at = along(segment, start, segment.plan_length());
            (segment, start)
        })
        .collect()
}

/// The path's first and last points, as laid, before its direction is applied.
fn ends(path: &[Segment], origin: Pose) -> (Pose, Pose) {
    let walked = walk(path, origin);
    let last = walked.last().map_or_else(
        || At { x: origin.x, y: origin.y, z: origin.z, theta: origin.heading_degrees.to_radians(), curvature: 0.0 },
        |&(segment, start)| along(segment, start, segment.plan_length()),
    );
    (origin, last.pose())
}

/// The same place, heading the other way.
fn turned(pose: Pose) -> Pose {
    Pose { heading_degrees: normal_degrees(pose.heading_degrees + 180.0), ..pose }
}

/// Degrees in (-180, 180].
fn normal_degrees(degrees: f64) -> f64 {
    let turned = degrees.rem_euclid(360.0);
    if turned > 180.0 { turned - 360.0 } else { turned }
}

/// Where a conveyor's MUs enter and leave, with its direction applied: backward, they enter at
/// the path's last point, heading back along it.
pub fn travel_ends(path: &[Segment], laid: Laid) -> (Pose, Pose) {
    let (first, last) = ends(path, laid.origin);
    let normal = |pose: Pose| Pose { heading_degrees: normal_degrees(pose.heading_degrees), ..pose };
    match laid.direction {
        Direction::Forward => (normal(first), normal(last)),
        Direction::Backward => (turned(last), turned(first)),
    }
}

/// Where a point `distance` metres along the path in 3D from where its MUs enter is, heading the
/// way they travel. Clamped to the path's ends.
pub fn pose_at(path: &[Segment], laid: Laid, distance: f64) -> Pose {
    let total = path_length(path);
    let from_first = match laid.direction {
        Direction::Forward => distance,
        Direction::Backward => total - distance,
    }
    .clamp(0.0, total);
    let mut gone = 0.0;
    let walked = walk(path, laid.origin);
    let mut pose = laid.origin;
    for (index, &(segment, start)) in walked.iter().enumerate() {
        let length = segment.length();
        if from_first <= gone + length || index + 1 == walked.len() {
            let into = ((from_first - gone) / length).clamp(0.0, 1.0) * segment.plan_length();
            pose = along(segment, start, into).pose();
            break;
        }
        gone += length;
    }
    match laid.direction {
        Direction::Forward => Pose { heading_degrees: normal_degrees(pose.heading_degrees), ..pose },
        Direction::Backward => turned(pose),
    }
}

/// Points along the path's centreline from its first point to its last, each with its distance
/// along the path in 3D from the first, close enough that the heading turns at most `step_degrees`
/// between two, and with every point where a bend heads along an axis, so the outermost points of
/// arcs and helices are on it exactly. The window draws from it, and the bounds come from it.
pub fn centreline(path: &[Segment], origin: Pose, step_degrees: f64) -> Vec<(f64, Pose)> {
    let step = step_degrees.to_radians().max(1e-3);
    let mut points = vec![(0.0, origin)];
    let mut gone = 0.0;
    for (segment, start) in walk(path, origin) {
        let plan = segment.plan_length();
        let length = segment.length();
        let mut at: Vec<f64> = Vec::new();
        match segment {
            Segment::Straight { .. } => {}
            Segment::Arc { .. } | Segment::Helix { .. } => {
                let k = segment.end_curvature();
                let turn = (k * plan).abs();
                let pieces = (turn / step).ceil() as usize;
                at.extend((1..pieces).map(|piece| plan * piece as f64 / pieces as f64));
                // Where it heads along an axis: a multiple of a quarter turn.
                let (from, to) = (start.theta, start.theta + k * plan);
                let (low, high) = (from.min(to), from.max(to));
                let mut quarter = (low / (PI / 2.0)).ceil();
                while quarter * PI / 2.0 < high {
                    at.push((quarter * PI / 2.0 - from) / k);
                    quarter += 1.0;
                }
            }
            Segment::Transition { length, to, .. } => {
                let turn = start.curvature.abs().max(to.curvature().abs()) * length;
                let pieces = (turn / step).ceil().max(4.0) as usize;
                at.extend((1..pieces).map(|piece| plan * piece as f64 / pieces as f64));
            }
        }
        at.retain(|&u| u > 0.0 && u < plan);
        at.push(plan);
        at.sort_by(f64::total_cmp);
        for u in at {
            let point = along(segment, start, u);
            points.push((gone + length * u / plan, point.pose()));
        }
        gone += length;
    }
    points
}

/// The bounds in plan of a conveyor of `width` along `path`.
fn bounds(path: &[Segment], laid: Laid) -> Bounds {
    let mut bounds = Bounds { min_x: f64::MAX, min_y: f64::MAX, max_x: f64::MIN, max_y: f64::MIN };
    let half = laid.width / 2.0;
    for (_, pose) in centreline(path, laid.origin, 1.0) {
        let theta = pose.heading_degrees.to_radians();
        let (nx, ny) = (-theta.sin() * half, theta.cos() * half);
        for (x, y) in [(pose.x + nx, pose.y + ny), (pose.x - nx, pose.y - ny)] {
            bounds.min_x = bounds.min_x.min(x);
            bounds.min_y = bounds.min_y.min(y);
            bounds.max_x = bounds.max_x.max(x);
            bounds.max_y = bounds.max_y.max(y);
        }
    }
    bounds
}

/// The tightest radius of any bend on the path, none if it is straight throughout.
pub fn tightest_radius(path: &[Segment]) -> Option<f64> {
    path.iter().zip(start_curvatures(path)).filter_map(|(segment, start)| segment.tightest_radius(start)).reduce(f64::min)
}

/// Each segment's tightest radius, none for one that doesn't bend.
pub fn tightest_radii(path: &[Segment]) -> Vec<Option<f64>> {
    path.iter().zip(start_curvatures(path)).map(|(segment, start)| segment.tightest_radius(start)).collect()
}

/// Everything worked out about a conveyor's path as it is laid.
pub fn geometry(path: &[Segment], laid: Laid) -> Geometry {
    let (start, end) = travel_ends(path, laid);
    Geometry {
        length: path_length(path),
        plan_length: plan_length(path),
        start,
        end,
        bounds: bounds(path, laid),
        tightest_radius: tightest_radius(path),
        steepest_incline_degrees: path.iter().map(|segment| segment.incline_degrees()).fold(0.0, f64::max),
    }
}

// ---------- As a file writes it ----------

/// A path as a model file writes it: a flat straight as just its length, every other segment as
/// an object, its rise only when it has one.
pub fn to_json(path: &[Segment]) -> Value {
    Value::Array(path.iter().map(|&segment| segment_json(segment)).collect())
}

/// A path as a model file writes it, if it is one: a list of segments, each read as the model
/// file's reader does, without saying what is wrong. The window reads values from files the
/// reader has already accepted.
pub fn from_json(value: &Value) -> Option<Vec<Segment>> {
    value.as_array()?.iter().map(segment_from_json).collect()
}

fn segment_from_json(value: &Value) -> Option<Segment> {
    let fields = value.as_object().filter(|fields| fields.len() == 1)?;
    let (kind, body) = fields.iter().next()?;
    let number = |key: &str| body.get(key)?.as_f64();
    let rise = number("rise").unwrap_or(0.0);
    let known = |keys: &[&str]| body.as_object().is_some_and(|fields| fields.keys().all(|key| keys.contains(&key.as_str())));
    match kind.as_str() {
        "straight" if body.is_number() => Some(Segment::straight(body.as_f64()?)),
        "straight" if known(&["length", "rise"]) => Some(Segment::Straight { length: number("length")?, rise }),
        "arc" if known(&["radius", "angle", "rise"]) => Some(Segment::Arc { radius: number("radius")?, angle: number("angle")?, rise }),
        "transition" if known(&["length", "to_radius", "rise"]) => {
            let to = match body.get("to_radius")? {
                Value::String(word) if word == "straight" => Bend::Straight,
                radius => Bend::Radius(radius.as_f64()?),
            };
            Some(Segment::Transition { length: number("length")?, to, rise })
        }
        "helix" if known(&["radius", "turns", "rise"]) => Some(Segment::Helix { radius: number("radius")?, turns: number("turns")?, rise: number("rise")? }),
        _ => None,
    }
}

fn segment_json(segment: Segment) -> Value {
    let with_rise = |mut fields: Value, rise: f64| {
        if rise != 0.0 {
            fields["rise"] = json!(rise);
        }
        fields
    };
    match segment {
        Segment::Straight { length, rise: 0.0 } => json!({ "straight": length }),
        Segment::Straight { length, rise } => json!({ "straight": { "length": length, "rise": rise } }),
        Segment::Arc { radius, angle, rise } => json!({ "arc": with_rise(json!({ "radius": radius, "angle": angle }), rise) }),
        Segment::Transition { length, to, rise } => {
            let to = match to {
                Bend::Straight => json!("straight"),
                Bend::Radius(radius) => json!(radius),
            };
            json!({ "transition": with_rise(json!({ "length": length, "to_radius": to }), rise) })
        }
        Segment::Helix { radius, turns, rise } => json!({ "helix": { "radius": radius, "turns": turns, "rise": rise } }),
    }
}

// ---------- The short form ----------

/// A path in the short form the command language reads and the window shows on one line, such as
/// `4, arc 1.5 90, transition 1.2 to 1.5, helix 1.2 2 rise 3, 2 rise 0.5`: segments separated by
/// commas, a bare number a straight, `rise` after any segment.
pub fn short_form(path: &[Segment]) -> String {
    path.iter()
        .map(|&segment| {
            let body = match segment {
                Segment::Straight { length, .. } => number(length),
                Segment::Arc { radius, angle, .. } => format!("arc {} {}", number(radius), number(angle)),
                Segment::Transition { length, to: Bend::Straight, .. } => format!("transition {} to straight", number(length)),
                Segment::Transition { length, to: Bend::Radius(radius), .. } => format!("transition {} to {}", number(length), number(radius)),
                Segment::Helix { radius, turns, .. } => format!("helix {} {}", number(radius), number(turns)),
            };
            match segment.rise() {
                0.0 => body,
                rise => format!("{body} rise {}", number(rise)),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A number as people write it: no trailing zeros.
fn number(value: f64) -> String {
    format!("{value}")
}

/// The words of the short form that can come next in a segment, after the words `before` it, for
/// completion: a kind to begin with, `to` and then `straight` in a transition, and `rise` once the
/// segment is whole.
pub fn next_words(before: &str) -> &'static [&'static str] {
    let words: Vec<String> = before.split_whitespace().map(str::to_ascii_lowercase).collect();
    let whole = |count: usize| if words.len() == count { &["rise"][..] } else { &[][..] };
    match words.first().map(String::as_str) {
        None => &["arc", "transition", "helix", "straight"],
        Some("transition") => match words.len() {
            2 => &["to"],
            3 => &["straight"],
            _ => whole(4),
        },
        Some("arc" | "helix") => whole(3),
        Some("straight") => whole(2),
        Some(number) if number.parse::<f64>().is_ok() => whole(1),
        Some(_) => &[],
    }
}

/// Reads a path in the short form, or says what's wrong with it, naming the segment by its place.
/// Values are only read here: whether they make a sound path is the model file's rules to check.
pub fn read_short_form(text: &str) -> Result<Vec<Segment>, String> {
    let pieces: Vec<&str> = text.split(',').map(str::trim).collect();
    if pieces.iter().all(|piece| piece.is_empty()) {
        return Err("a path needs at least one segment, such as `10` for 10 m straight".to_string());
    }
    pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| read_segment(piece).map_err(|wrong| format!("segment {} (`{piece}`): {wrong}", index + 1)))
        .collect()
}

fn read_segment(piece: &str) -> Result<Segment, String> {
    let mut words: Vec<&str> = piece.split_whitespace().collect();
    let read = |word: Option<&&str>, what: &str| -> Result<f64, String> {
        let word = word.ok_or_else(|| format!("{what} is missing"))?;
        word.parse::<f64>().ok().filter(|value| value.is_finite()).ok_or_else(|| format!("`{word}` should be a number, the {what}"))
    };
    // A rise at the end, after any segment.
    let mut rise = 0.0;
    if let Some(at) = words.iter().position(|word| word.eq_ignore_ascii_case("rise")) {
        if at + 2 != words.len() {
            return Err("`rise` comes last, followed by one number, such as `rise 0.5`".to_string());
        }
        rise = read(words.get(at + 1), "rise")?;
        words.truncate(at);
    }
    let Some(&first) = words.first() else { return Err("it's empty: write a length, or a kind such as `arc 1.5 90`".to_string()) };
    let count = |wanted: usize, say: &str| -> Result<(), String> {
        if words.len() == wanted { Ok(()) } else { Err(format!("write it as `{say}`")) }
    };
    match first.to_ascii_lowercase().as_str() {
        "straight" => {
            count(2, "straight <length>")?;
            Ok(Segment::Straight { length: read(words.get(1), "length")?, rise })
        }
        "arc" => {
            count(3, "arc <radius> <angle>")?;
            Ok(Segment::Arc { radius: read(words.get(1), "radius")?, angle: read(words.get(2), "angle in degrees")?, rise })
        }
        "transition" => {
            count(4, "transition <length> to <radius>, or to straight")?;
            if !words[2].eq_ignore_ascii_case("to") {
                return Err("write it as `transition <length> to <radius>`, or `to straight`".to_string());
            }
            let to = if words[3].eq_ignore_ascii_case("straight") { Bend::Straight } else { Bend::Radius(read(words.get(3), "radius it eases to")?) };
            Ok(Segment::Transition { length: read(words.get(1), "length")?, to, rise })
        }
        "helix" => {
            count(3, "helix <radius> <turns> rise <height>")?;
            Ok(Segment::Helix { radius: read(words.get(1), "radius")?, turns: read(words.get(2), "turns")?, rise })
        }
        _ if first.parse::<f64>().is_ok() => {
            count(1, "<length>, such as `4`")?;
            Ok(Segment::Straight { length: read(words.first(), "length")?, rise })
        }
        other => {
            let guess = crate::suggest::did_you_mean(&other.to_ascii_lowercase(), ["arc", "transition", "helix", "straight"]);
            Err(format!("`{other}` isn't a segment{guess}: write a length, or `arc`, `transition` or `helix`"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: Pose = Pose { x: 0.0, y: 0.0, z: 0.0, heading_degrees: 0.0 };

    fn laid(origin: Pose) -> Laid {
        Laid { origin, width: 0.6, direction: Direction::Forward }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn end(path: &[Segment]) -> Pose {
        travel_ends(path, laid(ORIGIN)).1
    }

    #[test]
    fn a_straight_goes_its_length_along_its_heading() {
        let at = travel_ends(&[Segment::straight(4.0)], laid(Pose { x: 1.0, y: 2.0, z: 0.5, heading_degrees: 90.0 })).1;
        assert!(close(at.x, 1.0) && close(at.y, 6.0) && close(at.z, 0.5) && close(at.heading_degrees, 90.0), "{at:?}");
    }

    #[test]
    fn a_quarter_arc_to_the_left_and_right() {
        let left = end(&[Segment::Arc { radius: 2.0, angle: 90.0, rise: 0.0 }]);
        assert!(close(left.x, 2.0) && close(left.y, 2.0) && close(left.heading_degrees, 90.0), "{left:?}");
        let right = end(&[Segment::Arc { radius: 2.0, angle: -90.0, rise: 0.0 }]);
        assert!(close(right.x, 2.0) && close(right.y, -2.0) && close(right.heading_degrees, -90.0), "{right:?}");
        assert!(close(path_length(&[Segment::Arc { radius: 2.0, angle: 90.0, rise: 0.0 }]), PI));
    }

    #[test]
    fn a_helix_of_whole_turns_ends_above_where_it_began() {
        let path = [Segment::Helix { radius: 1.2, turns: 2.0, rise: 3.0 }];
        let at = end(&path);
        assert!(close(at.x, 0.0) && close(at.y, 0.0) && close(at.z, 3.0) && close(at.heading_degrees, 0.0), "{at:?}");
        let plan = 1.2 * TAU * 2.0;
        assert!(close(path_length(&path), (plan * plan + 9.0).sqrt()));
        assert!(close(geometry(&path, laid(ORIGIN)).steepest_incline_degrees, 3.0f64.atan2(plan).to_degrees()));
    }

    #[test]
    fn a_clothoid_from_straight_ends_where_the_fresnel_integrals_say() {
        // A clothoid of length L from straight to radius R heads s²/2RL after s, so with
        // s = sqrt(pi·R·L)·t it ends at sqrt(pi·R·L)·(C(t), S(t)) for t = sqrt(L / pi·R). With
        // R = L / pi, t is 1 and the scale L, and C(1) = 0.7798934004, S(1) = 0.4382591473
        // (Abramowitz and Stegun, table 7.7).
        let length = 2.0;
        let radius = length / PI;
        let at = end(&[Segment::Transition { length, to: Bend::Radius(radius), rise: 0.0 }]);
        assert!((at.x - length * 0.779_893_400_4).abs() < 1e-9, "{at:?}");
        assert!((at.y - length * 0.438_259_147_3).abs() < 1e-9, "{at:?}");
        // It turns through L / 2R.
        assert!(close(at.heading_degrees.to_radians(), length / (2.0 * radius)), "{at:?}");
    }

    #[test]
    fn a_transition_eases_from_the_bend_before_it() {
        // Into a curve, around it and back out ends heading through the whole turn.
        let path = [
            Segment::Transition { length: 1.0, to: Bend::Radius(2.0), rise: 0.0 },
            Segment::Arc { radius: 2.0, angle: 45.0, rise: 0.0 },
            Segment::Transition { length: 1.0, to: Bend::Straight, rise: 0.0 },
        ];
        let turned = 1.0 / 4.0 + PI / 4.0 + 1.0 / 4.0;
        assert!(close(end(&path).heading_degrees.to_radians(), turned), "{:?}", end(&path));
        assert!(transition_eases(&path, 2));
        let nothing = [Segment::Arc { radius: 2.0, angle: 45.0, rise: 0.0 }, Segment::Transition { length: 1.0, to: Bend::Radius(2.0), rise: 0.0 }];
        assert!(!transition_eases(&nothing, 1));
        assert_eq!(tightest_radius(&path), Some(2.0));
    }

    #[test]
    fn a_rise_climbs_evenly_and_lengthens_the_path() {
        let path = [Segment::Straight { length: 3.0, rise: 4.0 }];
        assert!(close(path_length(&path), 5.0) && close(plan_length(&path), 3.0));
        let middle = pose_at(&path, laid(ORIGIN), 2.5);
        assert!(close(middle.x, 1.5) && close(middle.z, 2.0), "{middle:?}");
    }

    #[test]
    fn backward_enters_at_the_last_point_heading_back() {
        let path = [Segment::straight(4.0), Segment::Arc { radius: 1.0, angle: 90.0, rise: 0.0 }];
        let back = Laid { direction: Direction::Backward, ..laid(ORIGIN) };
        let (start, end) = travel_ends(&path, back);
        assert!(close(start.x, 5.0) && close(start.y, 1.0) && close(start.heading_degrees, -90.0), "{start:?}");
        assert!(close(end.x, 0.0) && close(end.y, 0.0) && close(end.heading_degrees, 180.0), "{end:?}");
        let first = pose_at(&path, back, 0.0);
        assert!(close(first.x, 5.0) && close(first.y, 1.0), "{first:?}");
        let along = pose_at(&path, back, path_length(&path) - 1.0);
        assert!(close(along.x, 1.0) && close(along.y, 0.0) && close(along.heading_degrees, 180.0), "{along:?}");
    }

    #[test]
    fn bounds_hold_the_whole_width_including_a_bends_outermost_point() {
        let straight = geometry(&[Segment::straight(4.0)], laid(ORIGIN)).bounds;
        assert!(close(straight.min_x, 0.0) && close(straight.max_x, 4.0) && close(straight.min_y, -0.3) && close(straight.max_y, 0.3), "{straight:?}");
        // A half circle to the left of radius 2 reaches y = 4 at its end and x = 2 at its middle.
        let half = geometry(&[Segment::Arc { radius: 2.0, angle: 180.0, rise: 0.0 }], laid(ORIGIN)).bounds;
        assert!(close(half.max_x, 2.3) && close(half.max_y, 4.3) && close(half.min_x, -0.0) && close(half.min_y, -0.3), "{half:?}");
    }

    #[test]
    fn the_short_form_reads_and_writes_every_segment() {
        let text = "4, arc 1.5 90, transition 1.2 to 1.5, helix 1.2 2 rise 3, 2 rise 0.5, arc 2 -45, transition 1 to straight";
        let path = read_short_form(text).unwrap();
        assert_eq!(
            path,
            vec![
                Segment::straight(4.0),
                Segment::Arc { radius: 1.5, angle: 90.0, rise: 0.0 },
                Segment::Transition { length: 1.2, to: Bend::Radius(1.5), rise: 0.0 },
                Segment::Helix { radius: 1.2, turns: 2.0, rise: 3.0 },
                Segment::Straight { length: 2.0, rise: 0.5 },
                Segment::Arc { radius: 2.0, angle: -45.0, rise: 0.0 },
                Segment::Transition { length: 1.0, to: Bend::Straight, rise: 0.0 },
            ]
        );
        assert_eq!(short_form(&path), text);
        assert_eq!(read_short_form("straight 3, ARC 1 90").unwrap(), vec![Segment::straight(3.0), Segment::Arc { radius: 1.0, angle: 90.0, rise: 0.0 }]);
    }

    #[test]
    fn the_short_form_says_what_is_wrong_and_where() {
        let wrong = |text: &str| read_short_form(text).unwrap_err();
        assert!(wrong("4, arc 1.5").contains("segment 2") && wrong("4, arc 1.5").contains("arc <radius> <angle>"), "{}", wrong("4, arc 1.5"));
        assert!(wrong("4, ark 1 90").contains("did you mean `arc`?"), "{}", wrong("4, ark 1 90"));
        assert!(wrong("4 rise").contains("rise"), "{}", wrong("4 rise"));
        assert!(wrong("transition 1 into 2").contains("transition <length> to"), "{}", wrong("transition 1 into 2"));
        assert!(wrong("").contains("at least one segment"));
        assert!(wrong("arc x 90").contains("`x` should be a number"), "{}", wrong("arc x 90"));
    }

    #[test]
    fn the_file_form_writes_a_flat_straight_as_its_length_alone() {
        let path = [Segment::straight(4.0), Segment::Straight { length: 2.0, rise: 0.5 }, Segment::Transition { length: 1.0, to: Bend::Straight, rise: 0.0 }];
        assert_eq!(
            to_json(&path),
            json!([{ "straight": 4.0 }, { "straight": { "length": 2.0, "rise": 0.5 } }, { "transition": { "length": 1.0, "to_radius": "straight" } }])
        );
        assert_eq!(from_json(&to_json(&path)), Some(path.to_vec()));
        assert_eq!(from_json(&json!([{ "arc": { "radius": 1 } }])), None);
        assert_eq!(from_json(&json!([{ "availability": 0.9 }])), None);
    }
}
