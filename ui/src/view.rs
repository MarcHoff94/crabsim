//! The view state: from the model, a live session's outputs and wall time, it works out what
//! the window draws and which controls it offers. Bevy and egui only draw what it says.

use crate::camera::Camera;
pub use crabsim::time::{format_time, format_time_exact, parse_time};
use crate::marks::Marks;
pub use crate::marks::{Intervention, Mark};
use crabsim_catalogue::bottleneck::{likely_bottleneck, LikelyBottleneck, Shares};
use crabsim_model::State;
use crabsim::{
    ModelDescription, ObjectKind, ObjectStatistics, SessionOutput, Picture, ProcessState, RunState, SessionStatus,
    SimTime, Spread, StopCause, TimeMode,
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
    Busy,
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
}

/// What going on to a later time should do. Animated, a trip far ahead can take hours at the
/// speed asked for, so the window asks first whether to race there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoingOn {
    /// Go there.
    Now,
    /// Getting there animated would take about `wall` of wall time, more than `LONG_TRIP`, so ask
    /// whether to race there, with animation off for the trip, or animate anyway.
    Ask { wall: Duration },
}

/// The longest an animated trip to a later time takes before going there asks first.
pub const LONG_TRIP: Duration = Duration::from_secs(60);

/// A trip raced there after asking, with animation off until it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trip {
    /// Sent, but the session hasn't said it is advancing yet.
    Sent,
    Racing,
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

/// The wait for a trip, as people say it: in minutes, hours or days.
pub fn wall_time_roughly(wall: Duration) -> String {
    let secs = wall.as_secs_f64();
    let (count, unit) = if secs < 3_600.0 {
        ((secs / 60.0).round().max(1.0), "minute")
    } else if secs < 2.0 * 86_400.0 {
        ((secs / 3_600.0).round(), "hour")
    } else {
        ((secs / 86_400.0).round(), "day")
    };
    let count = count as u64;
    format!("about {count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// How far past an object's outgoing side its connect handle sits, in metres.
pub const HANDLE_GAP: f64 = 0.3;

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
    /// Whether advancing is animated. See `LiveSession::set_animation`.
    animate: bool,
    /// Wall time and shown time while advancing, oldest first, over the last `ACHIEVED_WINDOW`.
    pace: VecDeque<(Duration, SimTime)>,
    /// The objects selected, in the order they were. One alone is the object the inspector shows.
    selection: Vec<u32>,
    /// The connection selected, from and to, instead of an object.
    selected_connection: Option<(u32, u32)>,
    /// Whether the Results tab is open, which wants full states as a selection does.
    results_open: bool,
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
    /// A trip raced there with animation off, which goes on again when it ends.
    trip: Option<Trip>,
    /// A raced trip ended, so the window turns animation on in the session too.
    trip_ended: bool,
    /// The model's longest mean time between failures or repair time, which measuring periods
    /// should be long against.
    longest_failure_time: Option<SimTime>,
}

/// What Delete removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deletion {
    Connection { from: String, to: String },
    /// An instance, and its connections with it, as (from, to).
    Instance { name: String, connections: Vec<(String, String)> },
    /// Several instances, in model order, and every connection to or from any of them.
    Instances { names: Vec<String>, connections: Vec<(String, String)> },
}

impl Deletion {
    /// Whether the window asks before removing: when an instance's connections go with it.
    pub fn asks_first(&self) -> bool {
        matches!(self, Deletion::Instance { connections, .. } | Deletion::Instances { connections, .. } if !connections.is_empty())
    }
}

/// How far `point` is from the segment from `start` to `end`.
fn distance_to_segment(point: Point, start: Point, end: Point) -> f64 {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    let length_squared = dx * dx + dy * dy;
    let along = if length_squared == 0.0 { 0.0 } else { (((point.x - start.x) * dx + (point.y - start.y) * dy) / length_squared).clamp(0.0, 1.0) };
    let (x, y) = (start.x + along * dx, start.y + along * dy);
    ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt()
}

/// What the inspector says about the selected object.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub id: u32,
    pub kind: ObjectKind,
    pub name: String,
    /// Busy, Blocked, Idle or Stopped.
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

impl Inspection {
    /// The object's statistics as the inspector's rows, only those that can vary for its type: a
    /// Source only makes MUs and waits to pass them on, and a Drain only takes them in, so their
    /// shares of time would always read 0% or 100%. None until a full state has arrived.
    pub fn statistics_rows(&self) -> Vec<(&'static str, String)> {
        let Some(details) = &self.details else { return Vec::new() };
        let statistics = &details.statistics;
        let measured = statistics.measured_time();
        let share = |time: SimTime| match measured.as_millis() {
            0 => "–".to_string(),
            total => format!("{:.1} %", time.as_millis() as f64 * 100.0 / total as f64),
        };
        let lead_time = statistics.lead_time_mean().map(|mean| ("Mean lead time", format_time_exact(mean)));
        match self.kind {
            ObjectKind::Source => vec![("MUs out", statistics.mus_out.to_string()), ("Blocked", share(statistics.time_blocked))],
            ObjectKind::Drain => {
                let throughput = statistics.mus_per_hour(details.as_of).map_or_else(|| "–".to_string(), |per_hour| format!("{per_hour:.1} an hour"));
                [("MUs in", statistics.mus_in.to_string()), ("Throughput", throughput)].into_iter().chain(lead_time).collect()
            }
            ObjectKind::Station | ObjectKind::Buffer => {
                let mut rows = vec![
                    ("MUs in", statistics.mus_in.to_string()),
                    ("MUs out", statistics.mus_out.to_string()),
                    ("Busy", share(statistics.time_busy)),
                    ("Blocked", share(statistics.time_blocked)),
                    ("Idle", share(statistics.time_idle)),
                    ("Failed", share(statistics.time_failed)),
                    ("Paused", share(statistics.time_paused)),
                    ("Failures", statistics.failures.to_string()),
                    ("Pauses", statistics.pauses.to_string()),
                ];
                if let Some(availability) = statistics.achieved_availability() {
                    rows.push(("Availability", format!("{:.1} %", availability * 100.0)));
                }
                rows
            }
        }
    }
}

impl Details {
    /// The process time in force, with the configured one beside it only when they differ, such as
    /// "00:00:20 (configured 00:00:10)". `None` for an object without one.
    pub fn process_time_text(&self) -> Option<String> {
        let now = self.process_time?;
        Some(match self.configured_process_time {
            Some(configured) if configured != now => format!("{} (configured {})", format_time_exact(now), format_time_exact(configured)),
            _ => format_time_exact(now),
        })
    }
}

/// A value an intervention changed during the run, which Reset either keeps, by saving it to the
/// model file, or drops.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub object: u32,
    /// The instance's name, as the model file has it.
    pub name: String,
    /// The attribute, as a model file names it, such as `process_time`.
    pub attribute: &'static str,
    pub value: SimTime,
}

impl Change {
    /// Such as "Station 2's process time, 00:00:20".
    pub fn describe(&self) -> String {
        let attribute = match self.attribute {
            "process_time" => "process time",
            other => other,
        };
        format!("{}'s {attribute}, {}", self.name, format_time_exact(self.value))
    }
}

/// A Drain's line in the Results tab.
#[derive(Debug, Clone, PartialEq)]
pub struct DrainResult {
    pub id: u32,
    pub name: String,
    pub mus_in: u64,
    pub throughput_per_hour: Option<f64>,
    pub mean_lead_time: Option<SimTime>,
    /// MUs an hour, from the spread between measuring periods, once `MIN_PERIODS` are complete.
    pub throughput_interval: Option<Interval>,
}

/// A Station's line in the Results tab: its shares of the time measured, from 0 to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct StationResult {
    pub id: u32,
    pub name: String,
    pub busy: f64,
    pub blocked: f64,
    pub idle: f64,
    pub failed: f64,
    pub paused: f64,
    /// From the spread between measuring periods, once `MIN_PERIODS` are complete.
    pub intervals: Option<StationIntervals>,
}

/// A figure's 95% confidence interval from the spread between measuring periods, each period one
/// sample: `mean` plus or minus `half_width`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    pub mean: f64,
    pub half_width: f64,
}

impl Interval {
    fn of(spread: Spread) -> Option<Interval> {
        Some(Interval { mean: spread.mean()?, half_width: spread.half_width()? })
    }

    pub fn low(&self) -> f64 {
        self.mean - self.half_width
    }

    pub fn high(&self) -> f64 {
        self.mean + self.half_width
    }

    /// Whether the two can't be told apart: their intervals share a value.
    pub fn overlaps(&self, other: &Interval) -> bool {
        self.low() <= other.high() && other.low() <= self.high()
    }
}

/// A Station's shares of time as intervals, as the Results tab's columns have them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StationIntervals {
    pub busy: Interval,
    pub blocked: Interval,
    pub idle: Interval,
    /// Failed or paused.
    pub stopped: Interval,
    /// Busy or stopped, which the likely-bottleneck rule ranks by.
    pub not_idle: Interval,
}

/// How far measuring periods have got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodsSoFar {
    pub length: SimTime,
    pub complete: u64,
}

/// How many measuring periods have to be complete before the Results tab gives intervals. Fewer
/// give intervals too wide to say much, and too few samples to trust their spread.
pub const MIN_PERIODS: u64 = 5;

/// A measuring period should be at least this many times the model's longest mean time between
/// failures or mean repair time, or periods follow each other too closely to be independent
/// samples, and intervals come out too narrow.
pub const PERIOD_FAILURE_FACTOR: u64 = 5;

/// The measuring period runs use until the user sets another: an hour of simulated time.
pub const DEFAULT_MEASURING_PERIOD: SimTime = SimTime::from_secs(3_600);

/// What the Results tab shows, from the run's full state at `as_of`.
#[derive(Debug, Clone, PartialEq)]
pub struct Results {
    pub as_of: SimTime,
    /// When the statistics began: at Init, or when they were last cleared.
    pub since: SimTime,
    pub drains: Vec<DrainResult>,
    /// In model order.
    pub stations: Vec<StationResult>,
    /// By the catalogue's rule, the same `find_bottleneck` gives agents.
    pub bottleneck: Option<LikelyBottleneck>,
    /// How far measuring periods have got, when the run measures in them.
    pub periods: Option<PeriodsSoFar>,
    /// The runner-up the likely bottleneck can't be told from: their busy-or-stopped intervals
    /// overlap. Only once intervals are given.
    pub too_close: Option<String>,
    /// The model's longest mean time between failures or repair time, when the measuring period
    /// is shorter than `PERIOD_FAILURE_FACTOR` times it.
    pub short_against: Option<SimTime>,
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
            selection: Vec::new(),
            selected_connection: None,
            details: None,
            details_stale: false,
            last_request: None,
            marks: Marks::default(),
            locating: false,
            end_time: None,
            model_changed: false,
            trip: None,
            trip_ended: false,
            longest_failure_time: model.longest_failure_time(),
            animate: true,
            results_open: false,
        }
    }

    /// A view of `model` that keeps the window's settings from `old`: its speed, time mode, end
    /// time and animation, which the window gives the new session too.
    pub fn with_settings_of(model: &ModelDescription, old: &ViewState) -> ViewState {
        let mut view = ViewState::new(model);
        view.speed = old.speed;
        view.time_mode = old.time_mode;
        view.end_time = old.end_time;
        // A raced trip is over for a new session, so animation is as it was before it.
        view.animate = old.animate || old.trip.is_some();
        view
    }

    /// Whether advancing is animated.
    pub fn animate(&self) -> bool {
        self.animate
    }

    /// Notes that animation went on or off, as the window tells the session too.
    pub fn set_animate(&mut self, on: bool) {
        self.animate = on;
        // The user chose, so a raced trip leaves animation as they set it.
        self.trip = None;
        self.pace.clear();
    }

    /// What going on to `to` should do: ask first when animated it would take more than
    /// `LONG_TRIP` of wall time at the speed asked for, up to the end time if that comes first.
    /// Skipping gaps can make a trip shorter, which this doesn't foresee. Going back never asks
    /// this.
    pub fn going_on(&self, to: SimTime) -> GoingOn {
        let shown = self.picture.shown();
        let to = self.end_time.map_or(to, |end| to.min(end));
        if !self.animate || to <= shown {
            return GoingOn::Now;
        }
        let wall = Duration::from_millis((to.saturating_sub(shown).as_millis() as f64 / self.speed).round() as u64);
        if wall > LONG_TRIP { GoingOn::Ask { wall } } else { GoingOn::Now }
    }

    /// Notes that the user chose to race to a later time: the window turns animation off and
    /// goes there. Animation goes on again when the trip ends, by arriving, Halt or anything else
    /// that stops the run, and `trip_ended` then tells the window to say so to the session.
    pub fn race_to(&mut self) {
        self.animate = false;
        self.pace.clear();
        self.trip = Some(if self.status == SessionStatus::Advancing { Trip::Racing } else { Trip::Sent });
        self.trip_ended = false;
    }

    /// Whether a raced trip has just ended, so the window turns animation on in the session.
    /// True once per trip.
    pub fn trip_ended(&mut self) -> bool {
        std::mem::take(&mut self.trip_ended)
    }

    fn end_trip(&mut self) {
        self.trip = None;
        self.animate = true;
        self.trip_ended = true;
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
                match self.trip {
                    Some(Trip::Sent) if *status == SessionStatus::Advancing => self.trip = Some(Trip::Racing),
                    Some(Trip::Racing) if *status != SessionStatus::Advancing => self.end_trip(),
                    _ => {}
                }
                if *status == SessionStatus::Advancing {
                    self.locating = false;
                }
            }
            SessionOutput::Error(error) => {
                self.error = Some(error.to_string());
                // Going there was refused, such as at the end time, so the trip never began.
                if self.trip == Some(Trip::Sent) {
                    self.end_trip();
                }
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
        if !self.animate || self.status != SessionStatus::Advancing || wall < ACHIEVED_MINIMUM {
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

    /// The time to go to for `typed`, as the Go to field takes it, in the form shown time is
    /// written: earlier goes back, later goes on. Before Init there is nothing to go back in.
    pub fn jump_target(&self, typed: &str) -> Result<SimTime, JumpProblem> {
        let to = parse_time(typed).ok_or(JumpProblem::NotATime)?;
        if !self.initialised && to <= self.picture.shown() {
            return Err(JumpProblem::NotInitialised);
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
                ProcessState::Processing => Shade::Busy,
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

    /// Whether there is a run that opening another model would end: from Init, or a run error,
    /// until Reset.
    pub fn run_in_progress(&self) -> bool {
        self.has_model() && (self.initialised || self.status == SessionStatus::Failed)
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
            && (self.status != SessionStatus::Advancing || (self.animate && self.speed <= MU_MAX_SPEED))
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

    /// Draws the object with its centre at `centre`, and its connections with it, such as while
    /// the user drags it. The model file changes only when the window saves the move.
    pub fn move_object(&mut self, id: u32, centre: Point) {
        let Some(object) = self.objects.get_mut(id as usize) else { return };
        object.centre = centre;
        let objects = &self.objects;
        for connection in self.connections.iter_mut().filter(|connection| connection.from == id || connection.to == id) {
            let (from, to) = (&objects[connection.from as usize], &objects[connection.to as usize]);
            connection.start = from.edge_towards(to.centre);
            connection.end = to.edge_towards(from.centre);
        }
    }

    /// Selects the object, or nothing, which unselects any connection too.
    pub fn select(&mut self, id: Option<u32>) {
        self.selection = id.filter(|&id| (id as usize) < self.objects.len()).into_iter().collect();
        self.selected_connection = None;
    }

    /// Adds the object to the selection, or takes it away if it's there, as Ctrl+click does.
    pub fn toggle(&mut self, id: u32) {
        if (id as usize) >= self.objects.len() {
            return;
        }
        match self.selection.iter().position(|&selected| selected == id) {
            Some(place) => {
                self.selection.remove(place);
            }
            None => self.selection.push(id),
        }
        self.selected_connection = None;
    }

    /// Selects every object whose centre lies in the rectangle between two corners, as dragging
    /// one on the canvas does, in model order.
    pub fn select_within(&mut self, corner: Point, opposite: Point) {
        let (low_x, high_x) = (corner.x.min(opposite.x), corner.x.max(opposite.x));
        let (low_y, high_y) = (corner.y.min(opposite.y), corner.y.max(opposite.y));
        self.selection = self
            .objects
            .iter()
            .filter(|object| (low_x..=high_x).contains(&object.centre.x) && (low_y..=high_y).contains(&object.centre.y))
            .map(|object| object.id)
            .collect();
        self.selected_connection = None;
    }

    /// Every object selected.
    pub fn selection(&self) -> &[u32] {
        &self.selection
    }

    /// The object after `from` in model order, or before it `backwards`, going round, as Tab
    /// and Shift+Tab step through them. From nothing, the first, or the last.
    pub fn next_object(&self, from: Option<u32>, backwards: bool) -> Option<u32> {
        let count = self.objects.len() as u32;
        if count == 0 {
            return None;
        }
        Some(match (from.filter(|&id| id < count), backwards) {
            (None, false) => 0,
            (None, true) => count - 1,
            (Some(id), false) => (id + 1) % count,
            (Some(id), true) => (id + count - 1) % count,
        })
    }

    /// Selects the connection from and to these objects, or nothing, which unselects any object
    /// too. One the model hasn't selects nothing.
    pub fn select_connection(&mut self, connection: Option<(u32, u32)>) {
        self.selected_connection = connection.filter(|&(from, to)| self.connections.iter().any(|c| c.from == from && c.to == to));
        self.selection.clear();
    }

    pub fn selected_connection(&self) -> Option<(u32, u32)> {
        self.selected_connection
    }

    /// The connection whose line passes within `within` metres of `floor`, the nearest if more
    /// do, such as for a click on its arrow. The window gives `within` from the zoom, so a line is
    /// as easy to hit however close the view is.
    pub fn connection_at(&self, floor: Point, within: f64) -> Option<(u32, u32)> {
        self.connections
            .iter()
            .map(|connection| (distance_to_segment(floor, connection.start, connection.end), connection))
            .filter(|(distance, _)| *distance <= within)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, connection)| (connection.from, connection.to))
    }

    /// Where the object's connect handle is: just past the middle of its outgoing side, the side
    /// its own x axis points to. Dragging from it to another object connects the two. `None` for
    /// a Drain, which passes no MU on, and while a run is in progress.
    pub fn handle_of(&self, id: u32) -> Option<Point> {
        let object = self.objects.get(id as usize).filter(|object| object.kind != ObjectKind::Drain)?;
        if !self.allows_building() {
            return None;
        }
        Some(object.from_local(Point { x: object.length / 2.0 + HANDLE_GAP, y: 0.0 }))
    }

    /// The object whose connect handle is within `within` metres of `floor`, the nearest if more
    /// are. The window gives `within` from the zoom, as for `connection_at`.
    pub fn handle_at(&self, floor: Point, within: f64) -> Option<u32> {
        self.objects
            .iter()
            .filter_map(|object| {
                let handle = self.handle_of(object.id)?;
                let distance = ((handle.x - floor.x).powi(2) + (handle.y - floor.y).powi(2)).sqrt();
                (distance <= within).then_some((distance, object.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// Whether the model can be built on now: only with no run in progress, before Init or after
    /// Reset, as with changing attributes. During a run the window says "Reset first to change the
    /// model".
    pub fn allows_building(&self) -> bool {
        self.reloads_now()
    }

    /// What Delete does with what is selected: removes a connection at once, or an instance,
    /// which first names the connections going with it. `None` with nothing selected or a run in
    /// progress.
    pub fn deletion(&self) -> Option<Deletion> {
        if !self.allows_building() {
            return None;
        }
        let name = |id: u32| self.objects[id as usize].name.clone();
        if let Some((from, to)) = self.selected_connection {
            return Some(Deletion::Connection { from: name(from), to: name(to) });
        }
        let touching = |c: &&Connection| self.selection.contains(&c.from) || self.selection.contains(&c.to);
        let connections: Vec<(String, String)> = self.connections.iter().filter(touching).map(|c| (name(c.from), name(c.to))).collect();
        match self.selection.as_slice() {
            [] => None,
            [id] => Some(Deletion::Instance { name: name(*id), connections }),
            several => {
                let mut ids = several.to_vec();
                ids.sort_unstable();
                Some(Deletion::Instances { names: ids.into_iter().map(name).collect(), connections })
            }
        }
    }

    /// The object selected, when exactly one is.
    pub fn selected(&self) -> Option<u32> {
        match self.selection.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    /// What hovering over an object on the canvas says: its name and the state it shows, such as
    /// "Station 5a: Failed".
    pub fn hover_text(&self, id: u32) -> String {
        let name = self.objects.get(id as usize).map_or("An unknown object", |object| object.name.as_str());
        format!("{name}: {}", self.shade(id).label())
    }

    /// What the inspector says about the selected object.
    pub fn inspection(&self) -> Option<Inspection> {
        let id = self.selected()?;
        let object = &self.objects[id as usize];
        let pictured = self.picture.object(id);
        let state = match pictured {
            None => "No state yet",
            Some(pictured) if pictured.is_stopped() => "Stopped",
            Some(pictured) => match pictured.process() {
                ProcessState::Processing => "Busy",
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
        (self.selected().is_some() || self.results_open)
            && self.initialised
            && out_of_date
            && self.last_request.is_none_or(|last| wall >= last + DETAILS_INTERVAL)
    }

    /// A figure of the run at its last full state, by object id, as the results store names it,
    /// such as `working_share` or `mus_in`. `None` until a full state arrives after Init, and
    /// `None` for an object without it, such as a share before any time was measured.
    pub fn live_figure(&self, figure: &str) -> Option<Vec<Option<f64>>> {
        let state = self.details.as_ref().filter(|state| state.initialised)?;
        let mut values = vec![None; self.objects.len()];
        for object in &state.objects {
            let statistics = &object.statistics;
            let measured = statistics.measured_time().as_millis() as f64;
            let share = |time: SimTime| (measured > 0.0).then(|| time.as_millis() as f64 / measured);
            let value = match figure {
                "working_share" => share(statistics.time_busy),
                "blocked_share" => share(statistics.time_blocked),
                "idle_share" => share(statistics.time_idle),
                "failed_share" => share(statistics.time_failed),
                "mus_in" => Some(statistics.mus_in as f64),
                "mus_out" => Some(statistics.mus_out as f64),
                _ => None,
            };
            if let Some(slot) = values.get_mut(object.id as usize) {
                *slot = value;
            }
        }
        Some(values)
    }

    /// Notes whether the Results tab is open, so full states are asked for while it is.
    pub fn set_results_open(&mut self, open: bool) {
        self.results_open = open;
    }

    /// What the Results tab shows: each Drain's throughput and lead time, every Station's shares
    /// of time, and the likely bottleneck, from the last full state of the run. `None` until one
    /// has arrived after Init.
    pub fn results(&self) -> Option<Results> {
        let state = self.details.as_ref().filter(|state| state.initialised)?;
        let periods = state.periods.as_ref();
        // Each object's period figures, once enough periods are complete to give intervals.
        let figures = |id: u32| periods.filter(|periods| periods.complete >= MIN_PERIODS)?.objects.get(id as usize).copied();
        let mut shares = Vec::new();
        let mut drains = Vec::new();
        let mut stations = Vec::new();
        for object in &state.objects {
            let Some(view) = self.objects.get(object.id as usize) else { continue };
            let statistics = &object.statistics;
            let measured = statistics.measured_time().as_millis() as f64;
            let share = |time: SimTime| if measured > 0.0 { time.as_millis() as f64 / measured } else { 0.0 };
            let built_in = crate::help::built_in(view.kind);
            shares.push(Shares {
                name: view.name.clone(),
                built_in: built_in.to_string(),
                working: share(statistics.time_busy),
                blocked: share(statistics.time_blocked),
                idle: share(statistics.time_idle),
                failed: share(statistics.time_failed),
                paused: share(statistics.time_paused),
            });
            match view.kind {
                ObjectKind::Drain => drains.push(DrainResult {
                    id: object.id,
                    name: view.name.clone(),
                    mus_in: statistics.mus_in,
                    throughput_per_hour: statistics.mus_per_hour(state.time),
                    mean_lead_time: statistics.lead_time_mean(),
                    throughput_interval: figures(object.id).and_then(|figures| Interval::of(figures.mus_in_per_hour)),
                }),
                ObjectKind::Station => stations.push(StationResult {
                    id: object.id,
                    name: view.name.clone(),
                    busy: share(statistics.time_busy),
                    blocked: share(statistics.time_blocked),
                    idle: share(statistics.time_idle),
                    failed: share(statistics.time_failed),
                    paused: share(statistics.time_paused),
                    intervals: figures(object.id).and_then(|figures| {
                        Some(StationIntervals {
                            busy: Interval::of(figures.busy)?,
                            blocked: Interval::of(figures.blocked)?,
                            idle: Interval::of(figures.idle)?,
                            stopped: Interval::of(figures.stopped)?,
                            not_idle: Interval::of(figures.not_idle)?,
                        })
                    }),
                }),
                _ => {}
            }
        }
        let connections: Vec<(String, String)> = self
            .connections
            .iter()
            .map(|connection| (self.objects[connection.from as usize].name.clone(), self.objects[connection.to as usize].name.clone()))
            .collect();
        let bottleneck = likely_bottleneck(&shares, &connections);
        let not_idle = |name: &str| stations.iter().find(|station| station.name == name)?.intervals.map(|intervals| intervals.not_idle);
        let too_close = bottleneck.as_ref().and_then(|likely| {
            let (runner_up, _) = likely.ranking.get(1)?;
            not_idle(&likely.instance)?.overlaps(&not_idle(runner_up)?).then(|| runner_up.clone())
        });
        let short_against = periods
            .and_then(|periods| Some((periods.length, self.longest_failure_time?)))
            .filter(|&(length, longest)| length < longest.times(PERIOD_FAILURE_FACTOR))
            .map(|(_, longest)| longest);
        Some(Results {
            as_of: state.time,
            since: state.statistics_since,
            drains,
            stations,
            bottleneck,
            periods: periods.map(|periods| PeriodsSoFar { length: periods.length, complete: periods.complete }),
            too_close,
            short_against,
        })
    }

    /// When the results are from, as the Results tab says it: whole seconds while the run is
    /// advancing, and to the millisecond once it stands still.
    pub fn results_time(&self, results: &Results) -> String {
        match self.status {
            SessionStatus::Advancing => format_time(results.as_of),
            _ => format_time_exact(results.as_of),
        }
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

    /// What Reset would keep or drop: each object's last process time set during the run, where it
    /// differs from the one configured, in id order. Pauses and forced failures are momentary, so
    /// they are never among them. The marks mirror the session's log, so this holds at shown time.
    pub fn changes(&self) -> Vec<Change> {
        let mut last: std::collections::BTreeMap<u32, SimTime> = std::collections::BTreeMap::new();
        for mark in self.marks.all() {
            if let Intervention::SetProcessTime(time) = mark.intervention {
                last.insert(mark.object, time);
            }
        }
        let configured = |id: u32| {
            self.details.as_ref()?.objects.iter().find(|object| object.id == id)?.configured_process_time
        };
        last.into_iter()
            .filter(|&(id, time)| configured(id) != Some(time))
            .map(|(id, time)| Change {
                object: id,
                name: self.objects[id as usize].name.clone(),
                attribute: "process_time",
                value: time,
            })
            .collect()
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
impl Shade {
    /// The colour a state is drawn in.
    pub fn of(state: State) -> Shade {
        match state {
            State::Busy => Shade::Busy,
            State::Blocked => Shade::Blocked,
            State::Idle => Shade::Idle,
            State::Failed => Shade::Failed,
            State::ForcedFailure => Shade::ForcedFailure,
            State::Paused => Shade::Paused,
        }
    }

    /// The state it shows, such as "Forced failure", or that there is none before Init.
    pub fn label(self) -> &'static str {
        match self {
            Shade::Unknown => "no state yet, before Init",
            Shade::Idle => State::Idle.label(),
            Shade::Busy => State::Busy.label(),
            Shade::Blocked => State::Blocked.label(),
            Shade::Paused => State::Paused.label(),
            Shade::Failed => State::Failed.label(),
            Shade::ForcedFailure => State::ForcedFailure.label(),
        }
    }
}

/// The one colour of each state, wherever crabsim shows it: on the canvas, in the help and in any
/// chart later, as red, green and blue from 0 to 1. Fixed for the whole app, so a colour always
/// means the same state.
pub fn rgb(shade: Shade) -> [f32; 3] {
    match shade {
        Shade::Unknown => [0.28, 0.29, 0.32],
        Shade::Idle => [0.50, 0.52, 0.56],
        Shade::Busy => [0.20, 0.70, 0.30],
        Shade::Blocked => [0.95, 0.60, 0.10],
        Shade::Paused => [0.25, 0.45, 0.95],
        Shade::Failed => [0.90, 0.15, 0.15],
        Shade::ForcedFailure => [0.55, 0.08, 0.08],
    }
}

fn doing(shade: Shade) -> &'static str {
    match shade {
        Shade::Unknown => "has no state yet",
        Shade::Idle => "is idle",
        Shade::Busy => "is busy",
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
