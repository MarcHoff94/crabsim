//! The view state: from the model, a live session's observations and wall time, it works out what
//! the window draws and which controls it offers. Bevy and egui only draw what it says.

use crate::camera::Camera;
use crabsim::{ModelDescription, ObjectKind, Observation, Picture, ProcessState, SessionStatus, SimTime, StopCause};
use std::time::Duration;

/// A point on the floor in metres, or on screen in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

/// One material-flow object as the 2D view draws it: a rectangle on the floor, seen from above.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectView {
    pub id: u32,
    pub kind: ObjectKind,
    pub name: String,
    pub centre: Point,
    /// Along the object's own x axis, before rotation.
    pub length: f64,
    pub width: f64,
    /// Counterclockwise, seen from above.
    pub rotation_degrees: f64,
}

/// A connection drawn from the edge of one object to the edge of its successor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Connection {
    pub from: u32,
    pub to: u32,
    pub start: Point,
    pub end: Point,
}

/// What an object's colour says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shade {
    /// No state has arrived yet.
    Unknown,
    Idle,
    Working,
    Blocked,
    Paused,
    Failed,
    ForcedFailure,
}

/// The run controls in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Init,
    Start,
    Halt,
    Reset,
}

/// How tall names under objects are on the floor, in metres, so they shrink and grow with the
/// model as the view zooms.
const LABEL_METRES: f64 = 0.5;
/// Names smaller than this, in pixels, can't be read, so they aren't drawn.
pub const MIN_LABEL_PIXELS: f64 = 5.0;
/// Names grow no bigger than this when zoomed in close, the size of the panels' text.
pub const MAX_LABEL_PIXELS: f64 = 16.0;

/// How long the window keeps drawing frames after something happens while halted, so that the
/// answer to a command reaches the screen. After that a halted window draws nothing.
const SETTLE: Duration = Duration::from_secs(1);

/// How long the window draws frames after it opens. On the owner's PC, with frames stopped after
/// 1 s, a thread started during startup kept a core busy until a burst of frames, such as from
/// moving the mouse, every time. With 3 s it still happened in 2 of 15 launches. The cause,
/// somewhere in Bevy or the graphics driver, wasn't found.
const STARTUP: Duration = Duration::from_secs(3);

/// Space between objects placed by the fallback, for objects the model hasn't placed.
const FALLBACK_SPACING: f64 = 3.0;

pub struct ViewState {
    objects: Vec<ObjectView>,
    connections: Vec<Connection>,
    picture: Picture,
    status: SessionStatus,
    initialised: bool,
    error: Option<String>,
    /// Wall time until which frames are wanted, however the session stands.
    settle_until: Duration,
}

impl ViewState {
    /// A view of `model`, before any observation. An object the model hasn't named is called by
    /// its type and id, and one it hasn't placed stands in a row below the origin.
    pub fn new(model: &ModelDescription) -> ViewState {
        let objects: Vec<ObjectView> = (0..model.object_count())
            .map(|id| {
                let kind = model.kind(id);
                let size = model.dimensions(id);
                let centre = match model.position(id) {
                    Some(position) => Point { x: position.x, y: position.y },
                    None => Point { x: id as f64 * FALLBACK_SPACING, y: -FALLBACK_SPACING },
                };
                ObjectView {
                    id,
                    kind,
                    name: model.name(id).map_or_else(|| format!("{kind:?} {id}"), str::to_owned),
                    centre,
                    length: size.length,
                    width: size.width,
                    rotation_degrees: size.rotation_degrees,
                }
            })
            .collect();
        let connections = objects
            .iter()
            .flat_map(|from| model.successors(from.id).iter().map(move |&to| (from, to)))
            .map(|(from, to)| {
                let to = &objects[to as usize];
                Connection {
                    from: from.id,
                    to: to.id,
                    start: from.edge_towards(to.centre),
                    end: to.edge_towards(from.centre),
                }
            })
            .collect();
        ViewState {
            objects,
            connections,
            picture: Picture::new(),
            status: SessionStatus::Halted,
            initialised: false,
            error: None,
            settle_until: STARTUP,
        }
    }

    /// Takes in one observation from the session, read at wall time `wall`.
    pub fn observe(&mut self, observation: &Observation, wall: Duration) {
        self.picture.observe(observation);
        match observation {
            Observation::State(state) => {
                self.initialised = state.initialised;
                self.error = None;
            }
            Observation::Status(status) => {
                self.status = *status;
                if *status == SessionStatus::Advancing {
                    self.error = None;
                }
            }
            Observation::Error(error) => self.error = Some(error.to_string()),
            Observation::Occurrences(_) | Observation::ShownTime(_) => {}
        }
        self.settle_until = self.settle_until.max(wall + SETTLE);
    }

    /// Notes that the window sent the session a command at wall time `wall`, so the answer gets
    /// drawn.
    pub fn commanded(&mut self, wall: Duration) {
        self.settle_until = self.settle_until.max(wall + SETTLE);
    }

    /// Every object, indexed by id.
    pub fn objects(&self) -> &[ObjectView] {
        &self.objects
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn picture(&self) -> &Picture {
        &self.picture
    }

    /// The smallest rectangle on the floor holding every object whole.
    pub fn bounds(&self) -> Rect {
        let mut bounds = Rect { min: Point { x: f64::MAX, y: f64::MAX }, max: Point { x: f64::MIN, y: f64::MIN } };
        for corner in self.objects.iter().flat_map(ObjectView::corners) {
            bounds.min.x = bounds.min.x.min(corner.x);
            bounds.min.y = bounds.min.y.min(corner.y);
            bounds.max.x = bounds.max.x.max(corner.x);
            bounds.max.y = bounds.max.y.max(corner.y);
        }
        if self.objects.is_empty() {
            bounds = Rect { min: Point::default(), max: Point::default() };
        }
        bounds
    }

    pub fn shade(&self, id: u32) -> Shade {
        let Some(object) = self.picture.object(id) else { return Shade::Unknown };
        match object.stop() {
            Some(StopCause::Failure) => Shade::Failed,
            Some(StopCause::ForcedFailure) => Shade::ForcedFailure,
            Some(StopCause::Pause) => Shade::Paused,
            None => match object.process() {
                ProcessState::Processing => Shade::Working,
                ProcessState::Blocked => Shade::Blocked,
                ProcessState::Idle => Shade::Idle,
            },
        }
    }

    pub fn status(&self) -> SessionStatus {
        self.status
    }

    pub fn shown_time(&self) -> String {
        format_time(self.picture.shown())
    }

    /// Whether a control would do anything now.
    pub fn allows(&self, control: Control) -> bool {
        let failed = self.status == SessionStatus::Failed;
        match control {
            Control::Init => !self.initialised && !failed,
            Control::Start => self.status == SessionStatus::Halted,
            Control::Halt => self.status == SessionStatus::Advancing,
            Control::Reset => self.initialised || failed,
        }
    }

    /// The last error the session reported, until the run is reset, goes back or starts again.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the window should keep drawing frames at wall time `wall`: always while shown time
    /// advances, and for a moment after anything happens. Otherwise nothing on screen changes,
    /// so a halted window costs nothing.
    pub fn wants_frames(&self, wall: Duration) -> bool {
        self.status == SessionStatus::Advancing || wall < self.settle_until
    }
}

impl ObjectView {
    fn corners(&self) -> [Point; 4] {
        let (half_length, half_width) = (self.length / 2.0, self.width / 2.0);
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(sx, sy)| self.from_local(Point { x: sx * half_length, y: sy * half_width }))
    }

    /// Where a line from the centre towards `target` crosses the object's outline.
    fn edge_towards(&self, target: Point) -> Point {
        let along = Point { x: target.x - self.centre.x, y: target.y - self.centre.y };
        let local = rotate(along, -self.rotation_degrees);
        let reach = |half: f64, d: f64| if d == 0.0 { f64::INFINITY } else { half / d.abs() };
        let scale = reach(self.length / 2.0, local.x).min(reach(self.width / 2.0, local.y));
        if !scale.is_finite() {
            return self.centre;
        }
        Point { x: self.centre.x + along.x * scale, y: self.centre.y + along.y * scale }
    }

    fn from_local(&self, local: Point) -> Point {
        let turned = rotate(local, self.rotation_degrees);
        Point { x: self.centre.x + turned.x, y: self.centre.y + turned.y }
    }
}

fn rotate(point: Point, degrees: f64) -> Point {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Point { x: point.x * cos - point.y * sin, y: point.x * sin + point.y * cos }
}

/// Simulation time as hours, minutes and seconds, with days in front once there are any.
pub fn format_time(time: SimTime) -> String {
    let secs = time.as_secs();
    let (days, hours, minutes, seconds) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60, secs % 60);
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// How big names under objects are drawn at this camera's zoom, in pixels, or `None` when they
/// would be too small to read.
pub fn label_size(camera: &Camera) -> Option<f64> {
    let size = LABEL_METRES * camera.pixels_per_metre;
    (size >= MIN_LABEL_PIXELS).then_some(size.min(MAX_LABEL_PIXELS))
}
