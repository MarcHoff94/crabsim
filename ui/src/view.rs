//! The view state: from the model, a live session's outputs and wall time, it works out what
//! the window draws and which controls it offers. Bevy and egui only draw what it says.

use crate::camera::Camera;
pub use crabsim::time::{format_time, format_time_exact, parse_time};
use crate::marks::Marks;
pub use crate::marks::{Intervention, Mark};
use crabsim::{
    ModelDescription, ObjectKind, ObjectStatistics, SessionOutput, Picture, ProcessState, RunState, SessionStatus,
    SimTime, StopCause, TimeMode,
};
use std::collections::VecDeque;
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
    StepBack,
    StepForward,
}

/// What pressing Step back should do. Stepping back goes to the instant before the one on
/// screen, and drops the interventions after it, so the window asks first when there are any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepBack {
    /// Step back: it drops no intervention.
    Now,
    /// Stepping back goes to this instant and drops interventions after it, so confirm first,
    /// then go back to it.
    Confirm(SimTime),
    /// The view doesn't know where stepping back goes, and it might drop interventions. Halt if
    /// advancing, ask the session with `locate_step_back`, note it with `locating_step_back`, and
    /// take the answer from `located_step_back`.
    Locate,
}

/// Why a time typed in to jump to can't be gone to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpProblem {
    /// There is no run to go back in until Init.
    NotInitialised,
    NotATime,
    /// Going back goes to shown time or earlier.
    NotEarlier,
}

/// Why a time typed in as the end time can't be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndTimeProblem {
    NotATime,
    /// An end time has to be later than shown time.
    NotLater,
}

/// How much simulation time passes per unit of wall time to start with: 60 times real time, which
/// shows the demo at about 30 events a second.
pub const DEFAULT_SPEED: f64 = 60.0;
pub const MIN_SPEED: f64 = 0.1;
/// A simulated year in about half a minute.
pub const MAX_SPEED: f64 = 1_000_000.0;

/// How far back the achieved speed is measured, in wall time.
const ACHIEVED_WINDOW: Duration = Duration::from_secs(2);
/// The least wall time measured before the achieved speed is shown.
const ACHIEVED_MINIMUM: Duration = Duration::from_secs(1);
/// Playback counts as falling behind below this share of the speed asked for.
const BEHIND: f64 = 0.95;

/// How tall names under objects are on the floor, in metres, so they shrink and grow with the
/// model as the view zooms.
const LABEL_METRES: f64 = 0.5;
/// Names smaller than this, in pixels, can't be read, so they aren't drawn.
pub const MIN_LABEL_PIXELS: f64 = 5.0;
/// Names grow no bigger than this when zoomed in close, the size of the panels' text.
pub const MAX_LABEL_PIXELS: f64 = 16.0;

/// How big an MU is drawn, in metres. Visual only: MUs have no size in the model yet.
pub const MU_METRES: f64 = 0.5;
/// MUs are drawn from this zoom on, where one is 3 pixels across.
pub const MU_MIN_PIXELS_PER_METRE: f64 = 6.0;
/// The fastest speed at which MUs are drawn while advancing. Faster, they jump about too quickly
/// to follow, so holders show a count instead.
pub const MU_MAX_SPEED: f64 = 120.0;

/// One MU as drawn, in the object that holds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuMarker {
    pub mu: u32,
    pub holder: u32,
    pub at: Point,
}

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
    speed: f64,
    time_mode: TimeMode,
    /// Wall time and shown time while advancing, oldest first, over the last `ACHIEVED_WINDOW`.
    pace: VecDeque<(Duration, SimTime)>,
    selected: Option<u32>,
    /// The last full state, for what only a full state says, such as statistics.
    details: Option<RunState>,
    /// Whether something since the last full state may have changed it at the same shown time.
    details_stale: bool,
    /// When the window last asked the session for a full state.
    last_request: Option<Duration>,
    marks: Marks,
    /// Whether the window asked the session where stepping back goes and awaits the answer.
    locating: bool,
    /// Where the session halts by itself, as last set.
    end_time: Option<SimTime>,
    /// The model file changed during a run, so Reset will use the new model.
    model_changed: bool,
}

/// What the inspector says about the selected object.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub id: u32,
    pub kind: ObjectKind,
    pub name: String,
    /// Working, Blocked, Idle or Stopped.
    pub state: &'static str,
    pub contents: Vec<u32>,
    /// Time left on the work in progress, frozen while stopped.
    pub remaining: Option<SimTime>,
    /// Why it's blocked: the MU it holds and what each successor is doing.
    pub blocked: Option<String>,
    /// Why it's stopped: each active interruption and since when, most serious first.
    pub stopped: Vec<String>,
    /// What only a full state says. `None` until the session has sent one.
    pub details: Option<Details>,
    /// The interventions on offer, and which are on. `None` where there are none.
    pub interventions: Option<Offered>,
}

/// What only a full state says about an object, as of `as_of`.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub as_of: SimTime,
    pub process_time: Option<SimTime>,
    pub configured_process_time: Option<SimTime>,
    /// The names of the predecessors waiting for it, longest waiting first.
    pub waiting: Vec<String>,
    pub statistics: ObjectStatistics,
    /// When the statistics began: at Init, or when they were last cleared.
    pub statistics_since: SimTime,
}

/// The interventions a Station offers, and which are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offered {
    pub paused: bool,
    pub forced: bool,
}

/// How often, in wall time, a full state is asked for while one is wanted.
const DETAILS_INTERVAL: Duration = Duration::from_millis(500);

impl ViewState {
    /// A view of `model`, before any output. An object the model hasn't named is called by
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
            speed: DEFAULT_SPEED,
            time_mode: TimeMode::Continuous,
            pace: VecDeque::new(),
            selected: None,
            details: None,
            details_stale: false,
            last_request: None,
            marks: Marks::default(),
            locating: false,
            end_time: None,
            model_changed: false,
        }
    }

    /// A view of `model` that keeps the window's settings from `old`: its speed, time mode and
    /// end time, which the window gives the new session too.
    pub fn with_settings_of(model: &ModelDescription, old: &ViewState) -> ViewState {
        let mut view = ViewState::new(model);
        view.speed = old.speed;
        view.time_mode = old.time_mode;
        view.end_time = old.end_time;
        view
    }

    /// Takes in one output from the session, read at wall time `wall`.
    pub fn observe(&mut self, output: &SessionOutput, wall: Duration) {
        let shown_before = self.picture.shown();
        self.picture.observe(output);
        match output {
            SessionOutput::State(state) => {
                self.initialised = state.initialised;
                // A state asked for while failed changes nothing: the error goes once the
                // session is going again.
                if self.status != SessionStatus::Failed {
                    self.error = None;
                }
                // Shown time jumped, as after Reset or going back. A state asked for comes at
                // shown time.
                if state.time != shown_before {
                    self.pace.clear();
                }
                self.marks.saw_state(state.time, state.initialised);
                self.details = Some(state.clone());
                self.details_stale = false;
            }
            SessionOutput::Status(status) => {
                let recovered = self.status == SessionStatus::Failed && *status != SessionStatus::Failed;
                if *status == SessionStatus::Advancing || recovered {
                    self.error = None;
                }
                self.status = *status;
                self.pace.clear();
                if *status == SessionStatus::Advancing {
                    self.locating = false;
                }
            }
            SessionOutput::Error(error) => {
                self.error = Some(error.to_string());
                self.locating = false;
            }
            SessionOutput::Occurrences(occurrences) => self.marks.saw_occurrences(occurrences),
            SessionOutput::ShownTime(_) => {}
            SessionOutput::StepBackTarget { shown, to } => self.marks.saw_step_back_target(*shown, *to),
        }
        if self.status == SessionStatus::Advancing {
            self.note_pace(wall);
        }
        self.settle_until = self.settle_until.max(wall + SETTLE);
    }

    /// Notes that the window sent the session a command at wall time `wall`, so the answer gets
    /// drawn. A refused command's error goes, while a run error stays until Reset or going back.
    /// Any command also ends waiting to know where stepping back goes, since the user has moved on.
    pub fn commanded(&mut self, wall: Duration) {
        if !self.run_failed() {
            self.error = None;
        }
        self.locating = false;
        self.settle_until = self.settle_until.max(wall + SETTLE);
    }

    /// The speed asked for: simulation time per unit of wall time.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Asks for `speed`, kept within `MIN_SPEED` and `MAX_SPEED`, and returns what it became, for
    /// the session.
    pub fn set_speed(&mut self, speed: f64) -> f64 {
        self.speed = if speed.is_nan() { MIN_SPEED } else { speed.clamp(MIN_SPEED, MAX_SPEED) };
        self.pace.clear();
        self.speed
    }

    pub fn time_mode(&self) -> TimeMode {
        self.time_mode
    }

    pub fn set_time_mode(&mut self, mode: TimeMode) {
        self.time_mode = mode;
        self.pace.clear();
    }

    /// The speed playback actually reached over the last 2 s of wall time, when it fell behind
    /// the speed asked for. `None` while it keeps up, while halted, and until a second has passed
    /// since the speed or anything else changed.
    pub fn achieved_speed(&self) -> Option<f64> {
        let (&(first_wall, first_shown), &(last_wall, last_shown)) = (self.pace.front()?, self.pace.back()?);
        let wall = last_wall.saturating_sub(first_wall);
        if self.status != SessionStatus::Advancing || wall < ACHIEVED_MINIMUM {
            return None;
        }
        let achieved = last_shown.saturating_sub(first_shown).as_secs_f64() / wall.as_secs_f64();
        (achieved < BEHIND * self.speed).then_some(achieved)
    }

    fn note_pace(&mut self, wall: Duration) {
        let shown = self.picture.shown();
        match self.pace.back_mut() {
            // Everything read in one frame shares its wall time.
            Some(last) if last.0 == wall => last.1 = shown,
            _ => self.pace.push_back((wall, shown)),
        }
        while self.pace.front().is_some_and(|&(oldest, _)| oldest + ACHIEVED_WINDOW < wall) {
            self.pace.pop_front();
        }
    }

    /// Whether the run reported an error, so that only Reset or going back gets it going again.
    pub fn run_failed(&self) -> bool {
        self.status == SessionStatus::Failed
    }

    /// The end time, as last set.
    pub fn end_time(&self) -> Option<SimTime> {
        self.end_time
    }

    /// Notes the end time the window gave the session.
    pub fn set_end_time(&mut self, end: Option<SimTime>) {
        self.end_time = end;
    }

    /// The end time for `typed`, as the end time field takes it: later than shown time, in the
    /// form shown time is written, such as `30d`.
    pub fn end_time_target(&self, typed: &str) -> Result<SimTime, EndTimeProblem> {
        let end = parse_time(typed).ok_or(EndTimeProblem::NotATime)?;
        if end <= self.picture.shown() {
            return Err(EndTimeProblem::NotLater);
        }
        Ok(end)
    }

    /// Whether a changed model file is used at once: only with no run in progress, before Init or
    /// after Reset. Otherwise the run carries on with its model, and Reset uses the new one.
    pub fn reloads_now(&self) -> bool {
        !self.initialised && self.status == SessionStatus::Halted
    }

    /// Whether the model file changed during the run, so Reset will use the new model.
    pub fn model_changed(&self) -> bool {
        self.model_changed
    }

    pub fn set_model_changed(&mut self, changed: bool) {
        self.model_changed = changed;
    }

    /// Whether the seed can change now: only before Init or after Reset, since a run draws its
    /// random numbers from the seed from Init on.
    pub fn allows_seed(&self) -> bool {
        !self.initialised && self.status == SessionStatus::Halted
    }

    /// The time to go back to for `typed`, as the jump field takes it: earlier than or at shown
    /// time, in the form shown time is written.
    pub fn jump_target(&self, typed: &str) -> Result<SimTime, JumpProblem> {
        if !self.initialised {
            return Err(JumpProblem::NotInitialised);
        }
        let to = parse_time(typed).ok_or(JumpProblem::NotATime)?;
        if to > self.picture.shown() {
            return Err(JumpProblem::NotEarlier);
        }
        Ok(to)
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

    /// Whether there is a model to run: the window shows none until one is chosen.
    pub fn has_model(&self) -> bool {
        !self.objects.is_empty()
    }

    /// Whether a control would do anything now. None does without a model.
    pub fn allows(&self, control: Control) -> bool {
        if !self.has_model() {
            return false;
        }
        let failed = self.status == SessionStatus::Failed;
        match control {
            Control::Init => !self.initialised && !failed,
            Control::Start => self.status == SessionStatus::Halted,
            Control::Halt => self.status == SessionStatus::Advancing,
            Control::Reset => self.initialised || failed,
            Control::StepForward => self.initialised && self.status == SessionStatus::Halted,
            // Also out of a run error, and while advancing, which halts there.
            Control::StepBack => self.initialised,
        }
    }

    /// Shows an error that didn't come from the session, such as a refused operation, until the
    /// next command.
    pub fn show_error(&mut self, error: String) {
        self.error = Some(error);
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

    // ---------- MUs ----------

    /// Whether MUs are drawn: when zoomed in far enough, and while halted or advancing slowly
    /// enough to follow them. Otherwise objects show their state, and a count where readable.
    pub fn shows_mus(&self, camera: &Camera) -> bool {
        camera.pixels_per_metre >= MU_MIN_PIXELS_PER_METRE
            && (self.status != SessionStatus::Advancing || self.speed <= MU_MAX_SPEED)
    }

    /// Every MU to draw, each in the object that holds it, or none when MUs aren't shown. A move
    /// takes no time, so an MU is drawn in its new holder at once.
    pub fn mu_markers(&self, camera: &Camera) -> Vec<MuMarker> {
        if !self.shows_mus(camera) {
            return Vec::new();
        }
        self.picture
            .objects()
            .iter()
            .filter(|pictured| !pictured.contents().is_empty())
            .flat_map(|pictured| {
                let holder = pictured.id();
                let spots = self.objects[holder as usize].mu_spots(pictured.contents().len());
                pictured.contents().iter().zip(spots).map(move |(&mu, at)| MuMarker { mu, holder, at })
            })
            .collect()
    }

    /// How many MUs each object holds, for the objects holding any, where MUs aren't drawn and a
    /// count can be read.
    pub fn counts(&self, camera: &Camera) -> Vec<(u32, usize)> {
        if self.shows_mus(camera) || label_size(camera).is_none() {
            return Vec::new();
        }
        self.picture
            .objects()
            .iter()
            .filter(|pictured| !pictured.contents().is_empty())
            .map(|pictured| (pictured.id(), pictured.contents().len()))
            .collect()
    }

    // ---------- Selecting and inspecting ----------

    /// The object whose footprint holds `floor`, if any. Where footprints overlap, the one added
    /// to the model last, which is drawn on top.
    pub fn object_at(&self, floor: Point) -> Option<u32> {
        self.objects.iter().rev().find(|object| object.holds(floor)).map(|object| object.id)
    }

    pub fn select(&mut self, id: Option<u32>) {
        self.selected = id.filter(|&id| (id as usize) < self.objects.len());
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    /// What the inspector says about the selected object.
    pub fn inspection(&self) -> Option<Inspection> {
        let id = self.selected?;
        let object = &self.objects[id as usize];
        let pictured = self.picture.object(id);
        let state = match pictured {
            None => "No state yet",
            Some(pictured) if pictured.is_stopped() => "Stopped",
            Some(pictured) => match pictured.process() {
                ProcessState::Processing => "Working",
                ProcessState::Blocked => "Blocked",
                ProcessState::Idle => "Idle",
            },
        };
        let blocked = pictured.filter(|pictured| pictured.process() == ProcessState::Blocked).map(|pictured| {
            let holding = match pictured.contents().first() {
                Some(mu) => format!("Holding MU {mu}"),
                None => "Holding nothing".to_string(),
            };
            let successors: Vec<String> = pictured
                .successors()
                .iter()
                .map(|&successor| format!("{} {}", self.objects[successor as usize].name, doing(self.shade(successor))))
                .collect();
            format!("{holding}: {}", successors.join(", "))
        });
        let stopped = pictured
            .map(|pictured| pictured.interruptions())
            .unwrap_or_default()
            .iter()
            .map(|interruption| format!("{} since {}", cause_name(interruption.cause), format_time(interruption.since)))
            .collect();
        let offered = self.initialised && !self.run_failed() && object.kind == ObjectKind::Station;
        Some(Inspection {
            id,
            kind: object.kind,
            name: object.name.clone(),
            state,
            contents: pictured.map(|pictured| pictured.contents().to_vec()).unwrap_or_default(),
            remaining: pictured.and_then(|pictured| pictured.remaining(self.picture.shown())),
            blocked,
            stopped,
            details: self.details_of(id),
            interventions: pictured.filter(|_| offered).map(|pictured| {
                let on = |cause| pictured.interruptions().iter().any(|interruption| interruption.cause == cause);
                Offered { paused: on(StopCause::Pause), forced: on(StopCause::ForcedFailure) }
            }),
        })
    }

    fn details_of(&self, id: u32) -> Option<Details> {
        let state = self.details.as_ref().filter(|state| state.initialised)?;
        let object = state.objects.iter().find(|object| object.id == id)?;
        Some(Details {
            as_of: state.time,
            process_time: object.process_time,
            configured_process_time: object.configured_process_time,
            waiting: object.blocked_predecessors.iter().map(|&id| self.objects[id as usize].name.clone()).collect(),
            statistics: object.statistics,
            statistics_since: state.statistics_since,
        })
    }

    /// Whether the window should ask the session for a full state at wall time `wall`: only while
    /// something is selected, at most twice a second, and only when the last one may be out of
    /// date. So a halted window asks once, and again only after something changes.
    pub fn wants_state(&self, wall: Duration) -> bool {
        let out_of_date = self.details_stale
            || self.details.as_ref().is_none_or(|state| state.time != self.picture.shown() || !state.initialised);
        self.selected.is_some()
            && self.initialised
            && out_of_date
            && self.last_request.is_none_or(|last| wall >= last + DETAILS_INTERVAL)
    }

    /// Notes that the window asked the session for a full state at wall time `wall`, so the
    /// answer gets drawn.
    pub fn state_requested(&mut self, wall: Duration) {
        self.last_request = Some(wall);
        self.settle_until = self.settle_until.max(wall + SETTLE);
    }

    // ---------- Interventions ----------

    /// Notes that the window sent `intervention` on object `object` at wall time `wall`. It takes
    /// effect at shown time, and leaves a mark there.
    pub fn intervened(&mut self, object: u32, intervention: Intervention, wall: Duration) {
        self.marks.add(self.picture.shown(), object, intervention);
        self.details_stale = true;
        self.commanded(wall);
    }

    /// Every intervention still in the run, oldest first.
    pub fn marks(&self) -> &[Mark] {
        self.marks.all()
    }

    /// What a mark stands for, such as "Pause on Station 1".
    pub fn describe(&self, mark: &Mark) -> String {
        let what = match mark.intervention {
            Intervention::Pause => "Pause".to_string(),
            Intervention::Resume => "Resume".to_string(),
            Intervention::ForceFailure => "Forced failure".to_string(),
            Intervention::EndForcedFailure => "End of forced failure".to_string(),
            Intervention::SetProcessTime(time) => format!("Process time {}", format_time_exact(time)),
        };
        let name = self.objects.get(mark.object as usize).map_or("an unknown object", |object| object.name.as_str());
        format!("{what} on {name}")
    }

    /// How many interventions going back to `to` would drop: those after it.
    pub fn lost_going_back(&self, to: SimTime) -> usize {
        self.marks.lost_going_back(to)
    }

    /// What pressing Step back should do now. It asks only when stepping back drops an
    /// intervention, and has to find out where it goes only when there are interventions and the
    /// view didn't see that instant arrive, or shown time is still moving.
    pub fn step_back(&self) -> StepBack {
        if self.marks.all().is_empty() {
            return StepBack::Now;
        }
        let known = match self.status {
            SessionStatus::Advancing => None,
            SessionStatus::Halted | SessionStatus::ReachedEnd | SessionStatus::Failed => self.marks.step_back_target(self.picture.shown()),
        };
        match known {
            Some(to) => self.decide_step_back(to),
            None => StepBack::Locate,
        }
    }

    /// Notes that the window asked the session where stepping back goes, after `commanded`.
    pub fn locating_step_back(&mut self) {
        self.locating = true;
    }

    /// Once the session has said where stepping back goes, what to do: `Now` or `Confirm`. `None`
    /// while still waiting, or if nothing is awaited, as after any other command.
    pub fn located_step_back(&mut self) -> Option<StepBack> {
        if !self.locating || self.status == SessionStatus::Advancing {
            return None;
        }
        let to = self.marks.step_back_target(self.picture.shown())?;
        self.locating = false;
        Some(self.decide_step_back(to))
    }

    fn decide_step_back(&self, to: Option<SimTime>) -> StepBack {
        match to {
            Some(to) if self.lost_going_back(to) > 0 => StepBack::Confirm(to),
            _ => StepBack::Now,
        }
    }
}

/// What an object is doing, as said after its name.
fn doing(shade: Shade) -> &'static str {
    match shade {
        Shade::Unknown => "has no state yet",
        Shade::Idle => "is idle",
        Shade::Working => "is working",
        Shade::Blocked => "is blocked",
        Shade::Paused => "is paused",
        Shade::Failed => "has failed",
        Shade::ForcedFailure => "has a forced failure",
    }
}

fn cause_name(cause: StopCause) -> &'static str {
    match cause {
        StopCause::Failure => "Failure",
        StopCause::ForcedFailure => "Forced failure",
        StopCause::Pause => "Pause",
    }
}

impl ObjectView {
    fn corners(&self) -> [Point; 4] {
        self.outline(0.0)
    }

    /// The footprint's corners, grown by `margin` metres on every side, going round.
    pub fn outline(&self, margin: f64) -> [Point; 4] {
        let (half_length, half_width) = (self.length / 2.0 + margin, self.width / 2.0 + margin);
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(sx, sy)| self.from_local(Point { x: sx * half_length, y: sy * half_width }))
    }

    /// Where `count` MUs sit in the footprint: spread over a grid, in the order held, starting
    /// at the top left. One sits in the middle.
    pub fn mu_spots(&self, count: usize) -> Vec<Point> {
        let columns = (count as f64).sqrt().ceil().max(1.0) as usize;
        let rows = count.div_ceil(columns).max(1);
        let (step_x, step_y) = (self.length / columns as f64, self.width / rows as f64);
        (0..count)
            .map(|index| {
                let (column, row) = (index % columns, index / columns);
                self.from_local(Point {
                    x: (column as f64 + 0.5) * step_x - self.length / 2.0,
                    y: self.width / 2.0 - (row as f64 + 0.5) * step_y,
                })
            })
            .collect()
    }

    fn holds(&self, floor: Point) -> bool {
        let local = rotate(Point { x: floor.x - self.centre.x, y: floor.y - self.centre.y }, -self.rotation_degrees);
        local.x.abs() <= self.length / 2.0 && local.y.abs() <= self.width / 2.0
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

/// How big names under objects are drawn at this camera's zoom, in pixels, or `None` when they
/// would be too small to read.
pub fn label_size(camera: &Camera) -> Option<f64> {
    let size = LABEL_METRES * camera.pixels_per_metre;
    (size >= MIN_LABEL_PIXELS).then_some(size.min(MAX_LABEL_PIXELS))
}
