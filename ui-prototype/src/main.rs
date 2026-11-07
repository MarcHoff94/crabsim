//! THROWAWAY PROTOTYPE, never merged. It answers whether Bevy 0.20 with bevy_egui holds up for
//! crabsim's UI, against the pass marks in docs/agents/handoff.md ("UI design", "The prototype"):
//!
//! 1. Scale: 6,000 flat objects with state colours, 5,000 gliding markers and labels on zoom,
//!    drawn as sprites (`--render sprites`) or as flat meshes under an orthographic 3D camera
//!    (`--render meshes`).
//! 2. A real live session at 6,000 objects behind a window, on a real clock (`--mode session`).
//! 3. egui panels and the canvas together: drag from the palette into the canvas, click to
//!    select, and little work while Halted.
//!
//! cargo run --release -- --mode scale --render sprites
//! cargo run --release -- --mode session --render meshes --speed 3600
//!
//! Right or middle drag pans, the wheel zooms, a left click selects. It prints a frame-time
//! summary when it exits. No tests and no error handling: it is meant to be thrown away.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    log::LogPlugin,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PresentMode, PrimaryWindow, WindowResolution},
    winit::{UpdateMode, WinitSettings},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui, input::egui_wants_any_pointer_input};
use crabsim::{
    FailureProfile, LiveSession, ModelDescription, ObjectState, Observation, OccurrenceKind, ProcessState,
    SessionStatus, SimTime, StopCause, WallClock, random_numbers::Distribution,
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

// ---------- Configuration ----------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Render {
    Sprites,
    Meshes,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Scale,
    Session,
    Windowless,
}

#[derive(Resource, Clone, Debug)]
struct Config {
    mode: Mode,
    render: Render,
    lines: usize,
    markers: usize,
    speed: f64,
    seconds: Option<f64>,
    halt_after: Option<f64>,
    vsync: bool,
    close: bool,
    screenshot: Option<String>,
    poll_ms: u64,
    buffer: usize,
    checkpoint_spacing: u64,
}

fn config() -> Config {
    let mut config = Config {
        mode: Mode::Scale,
        render: Render::Sprites,
        lines: 429,
        markers: 5_000,
        speed: 3_600.0,
        seconds: None,
        halt_after: None,
        vsync: false,
        close: false,
        screenshot: None,
        poll_ms: 2,
        buffer: 100_000,
        checkpoint_spacing: 100_000,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().expect("a value");
        match arg.as_str() {
            "--mode" => {
                config.mode = match value().as_str() {
                    "session" => Mode::Session,
                    "windowless" => Mode::Windowless,
                    _ => Mode::Scale,
                }
            }
            "--poll-ms" => config.poll_ms = value().parse().unwrap(),
            "--buffer" => config.buffer = value().parse().unwrap(),
            "--checkpoint-spacing" => config.checkpoint_spacing = value().parse().unwrap(),
            "--render" => config.render = if value() == "meshes" { Render::Meshes } else { Render::Sprites },
            "--lines" => config.lines = value().parse().unwrap(),
            "--markers" => config.markers = value().parse().unwrap(),
            "--speed" => config.speed = value().parse().unwrap(),
            "--seconds" => config.seconds = Some(value().parse().unwrap()),
            "--halt-after" => config.halt_after = Some(value().parse().unwrap()),
            "--vsync" => config.vsync = true,
            "--close" => config.close = true,
            "--screenshot" => config.screenshot = Some(value()),
            other => panic!("unknown argument {other}"),
        }
    }
    config
}

// ---------- Layout: the demo line, repeated in a grid, in metres ----------

const PER_LINE: usize = 14;
/// Where each object of a line sits, in grid units, in the demo's id order.
const OFFSETS: [(f32, f32); PER_LINE] = [
    (0., 0.),   // source
    (1., 0.),   // st1
    (2., 0.),   // st2
    (3., 1.),   // st3
    (4., 1.),   // st4
    (5., 2.),   // st4a
    (5., 1.),   // st4b
    (5., 0.),   // st4c
    (6., 1.),   // st7
    (3., -1.5), // st5
    (4., -1.),  // st5a
    (4., -2.),  // st5b
    (4., -3.),  // st5c
    (7., -0.5), // drain
];
const EDGES: [(usize, usize); 18] = [
    (0, 1), (1, 2), (2, 3), (2, 9), (3, 4), (4, 5), (4, 6), (4, 7), (5, 8),
    (6, 8), (7, 8), (8, 13), (9, 10), (9, 11), (9, 12), (10, 13), (11, 13), (12, 13),
];
const NAMES: [&str; PER_LINE] =
    ["Source", "St1", "St2", "St3", "St4", "St4a", "St4b", "St4c", "St7", "St5", "St5a", "St5b", "St5c", "Drain"];
const GRID: f32 = 2.0;
const LINE_WIDTH: f32 = 9.0;
const LINE_HEIGHT: f32 = 6.5;
const COLUMNS: usize = 17;
const OBJECT_SIZE: Vec2 = Vec2::new(1.2, 1.0);
const MARKER_RADIUS: f32 = 0.25;
/// Pixels per metre above which MU markers and labels are drawn.
const MARKER_MIN_PPM: f32 = 6.0;
const LABEL_MIN_PPM: f32 = 25.0;
/// Speeds above which MUs are not drawn in a session: too fast to follow.
const MARKER_MAX_SPEED: f64 = 120.0;
const GLIDE_SECONDS: f64 = 0.2;

fn position(id: usize) -> Vec2 {
    let line = id / PER_LINE;
    let (dx, dy) = OFFSETS[id % PER_LINE];
    let (col, row) = (line % COLUMNS, line / COLUMNS);
    Vec2::new((col as f32 * LINE_WIDTH + dx) * GRID, (dy - row as f32 * LINE_HEIGHT) * GRID)
}

fn build_model(lines: usize) -> crabsim::SimulationRun {
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    for _ in 0..lines {
        let first = model.add_source(SimTime::from_secs(5));
        for secs in [10, 15, 20, 25, 12, 15, 18, 30, 18, 10, 14, 16] {
            model.add_station(SimTime::from_secs(secs), failures.clone());
        }
        model.add_drain();
        for (from, to) in EDGES {
            model.connect(first + from as u32, first + to as u32);
        }
    }
    model.build(0)
}

// ---------- The picture, copied from the session tests' rebuild helper ----------

#[derive(Clone)]
struct ObjectPicture {
    contents: Vec<u32>,
    process: ProcessState,
    failed: bool,
    paused: bool,
    completion: Option<SimTime>,
    frozen: Option<SimTime>,
}

impl ObjectPicture {
    fn from_state(state: &ObjectState, now: SimTime) -> ObjectPicture {
        let stopped = state.stop.is_some();
        ObjectPicture {
            contents: state.contents.clone(),
            process: state.process.clone(),
            failed: state.stop == Some(StopCause::Failure),
            paused: state.stop == Some(StopCause::Pause),
            completion: state.remaining.filter(|_| !stopped).map(|left| now + left),
            frozen: state.remaining.filter(|_| stopped),
        }
    }
    fn stopped(&self) -> bool {
        self.failed || self.paused
    }
    fn colour(&self) -> u8 {
        if self.failed {
            FAILED
        } else if self.paused {
            PAUSED
        } else {
            match self.process {
                ProcessState::Processing => BUSY,
                ProcessState::Blocked => BLOCKED,
                ProcessState::Idle => IDLE,
            }
        }
    }
}

/// Applies one occurrence. Returns a move, for drawing a glide.
fn apply(objects: &mut [ObjectPicture], kind: &OccurrenceKind, now: SimTime) -> Option<(u32, u32)> {
    match *kind {
        OccurrenceKind::RunInitialised => {}
        OccurrenceKind::MuCreated { mu_id, at, duration } => {
            let object = &mut objects[at as usize];
            object.contents = vec![mu_id];
            object.process = ProcessState::Processing;
            object.completion = Some(now + duration);
        }
        OccurrenceKind::MuMoved { mu_id, from, to } => {
            let object = &mut objects[from as usize];
            object.contents.retain(|&id| id != mu_id);
            object.process = ProcessState::Idle;
            object.completion = None;
            objects[to as usize].contents.push(mu_id);
            return Some((from, to));
        }
        OccurrenceKind::MuLeft { mu_id, at, .. } => objects[at as usize].contents.retain(|&id| id != mu_id),
        OccurrenceKind::WorkStarted { at, duration, .. } => {
            let object = &mut objects[at as usize];
            object.process = ProcessState::Processing;
            object.completion = Some(now + duration);
        }
        OccurrenceKind::WorkFinished { at, .. } => objects[at as usize].completion = None,
        OccurrenceKind::Blocked { object_id } => {
            objects[object_id as usize].process = ProcessState::Blocked;
            objects[object_id as usize].completion = None;
        }
        OccurrenceKind::Unblocked { object_id } => {
            let object = &mut objects[object_id as usize];
            if object.process == ProcessState::Blocked {
                object.process = ProcessState::Idle;
            }
        }
        OccurrenceKind::InterruptionBegan { object_id, cause } => {
            let object = &mut objects[object_id as usize];
            if !object.stopped() {
                object.frozen = object.completion.take().map(|completion| completion.saturating_sub(now));
            }
            match cause {
                StopCause::Failure => object.failed = true,
                StopCause::Pause => object.paused = true,
            }
        }
        OccurrenceKind::InterruptionEnded { object_id, cause } => {
            let object = &mut objects[object_id as usize];
            match cause {
                StopCause::Failure => object.failed = false,
                StopCause::Pause => object.paused = false,
            }
            if !object.stopped() {
                object.completion = object.frozen.take().map(|left| now + left);
            }
        }
    }
    None
}

// ---------- Resources ----------

const IDLE: u8 = 0;
const BUSY: u8 = 1;
const BLOCKED: u8 = 2;
const FAILED: u8 = 3;
const PAUSED: u8 = 4;
const ADDED: u8 = 5;
const COLOURS: [Color; 6] = [
    Color::srgb(0.45, 0.47, 0.50),
    Color::srgb(0.20, 0.70, 0.30),
    Color::srgb(0.95, 0.60, 0.10),
    Color::srgb(0.90, 0.15, 0.15),
    Color::srgb(0.25, 0.45, 0.95),
    Color::srgb(0.95, 0.95, 0.95),
];
const MARKER_COLOUR: Color = Color::srgb(1.0, 0.9, 0.2);

#[derive(Resource)]
struct Scene {
    positions: Vec<Vec2>,
    entities: Vec<Entity>,
    /// The colour each object's entity has now, so only changes touch the ECS.
    drawn: Vec<u8>,
    /// What each object should show, in the scale test.
    synthetic: Vec<u8>,
    edges: Vec<(u32, u32)>,
    materials: Vec<Handle<StandardMaterial>>,
    markers: Vec<Entity>,
    glides: Vec<Glide>,
    free: Vec<usize>,
    dropped_glides: u64,
    rng: u64,
    added: usize,
}

#[derive(Clone, Copy)]
struct Glide {
    marker: usize,
    from: Vec2,
    to: Vec2,
    start: f64,
    duration: f64,
}

impl Scene {
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
}

#[derive(Resource)]
struct Session {
    session: Mutex<LiveSession>,
    picture: Vec<ObjectPicture>,
    shown: SimTime,
    status: SessionStatus,
    started: Option<Instant>,
    shown_at_start: SimTime,
    halted_at: Option<Instant>,
    occurrences: u64,
    errors: Vec<String>,
}

#[derive(Resource, Default)]
struct Selection(Option<usize>);

/// The object being dragged on the canvas, and where in it the pointer took hold.
#[derive(Resource, Default)]
struct CanvasDrag(Option<(usize, Vec2)>);

#[derive(Resource, Default)]
struct PaletteDrag(Option<&'static str>);

#[derive(Resource)]
struct Stats {
    started: Instant,
    last_frame: Option<Instant>,
    /// Wall time between frames, in seconds, after the first second.
    frames: Vec<f64>,
    /// Time draining the session's channel and applying occurrences, per frame, in seconds.
    drains: Vec<f64>,
    occurrences_per_frame: Vec<u64>,
    halted_frames: u64,
    screenshot_taken: bool,
    exiting: bool,
}

#[derive(Component)]
struct WorldCamera;

// ---------- Setup ----------

/// The same session with no window: a loop that reads every observation, to tell the UI's cost
/// from the session's.
fn windowless_session(config: &Config) {
    let lines = config.lines;
    let settings = crabsim::SessionSettings { buffer_instants: config.buffer, checkpoint_spacing: config.checkpoint_spacing, ..Default::default() };
    let session = LiveSession::with_settings(move || build_model(lines), WallClock::new(), settings);
    session.set_speed(config.speed).unwrap();
    session.start().unwrap();
    let begun = Instant::now();
    let mut shown = SimTime::ZERO;
    let mut advancing_since: Option<(Instant, SimTime)> = None;
    let mut occurrences = 0u64;
    let mut samples = Vec::new();
    while begun.elapsed().as_secs_f64() < config.seconds.unwrap_or(20.0) {
        for observation in session.observations().try_iter() {
            match observation {
                Observation::Occurrences(list) => {
                    occurrences += list.len() as u64;
                    if let Some(last) = list.last() {
                        shown = shown.max(last.time);
                    }
                }
                Observation::ShownTime(time) => shown = time,
                Observation::State(state) => shown = state.time,
                Observation::Status(SessionStatus::Advancing) => advancing_since = Some((Instant::now(), shown)),
                _ => {}
            }
        }
        if let Some((since, from)) = advancing_since {
            samples.push((since.elapsed().as_secs_f64(), (shown.as_secs_f64() - from.as_secs_f64())));
        }
        std::thread::sleep(Duration::from_millis(config.poll_ms));
    }
    let (wall, simulated) = samples.last().copied().unwrap_or((0.0, 0.0));
    println!(
        "windowless session, polled every {} ms: shown {simulated:.0} s in {wall:.1} s, achieved {:.0}x of {:.0}x, {:.0} occurrences per second",
        config.poll_ms,
        simulated / wall,
        config.speed,
        occurrences as f64 / wall
    );
    // How shown time moved, second by second.
    let mut line = String::new();
    let mut next = 1.0;
    let mut previous = 0.0;
    for &(w, s) in &samples {
        if w >= next {
            line.push_str(&format!("{:.0} ", (s - previous) / config.speed * 100.0));
            previous = s;
            next += 1.0;
        }
    }
    println!("percent of the asked speed in each wall second: {line}");
    println!("probe: {}", crabsim::live_session::probe::report());
}

fn main() {
    let config = config();
    println!("{config:?}");
    if config.mode == Mode::Windowless {
        windowless_session(&config);
        return;
    }
    let present_mode = if config.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.10, 0.11, 0.13)))
        .insert_resource(WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous })
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "crabsim UI prototype (throwaway)".into(),
                        resolution: WindowResolution::new(1600, 900),
                        present_mode,
                        ..default()
                    }),
                    ..default()
                })
                // The engine logs every event at info, which Bevy's logger would otherwise print.
                .set(LogPlugin { filter: "warn,crabsim=warn".into(), ..default() }),
        )
        .add_plugins(EguiPlugin::default())
        .insert_resource(config.clone())
        .insert_resource(Selection::default())
        .insert_resource(CanvasDrag::default())
        .insert_resource(PaletteDrag::default())
        .insert_resource(Stats {
            started: Instant::now(),
            last_frame: None,
            frames: Vec::new(),
            drains: Vec::new(),
            occurrences_per_frame: Vec::new(),
            halted_frames: 0,
            screenshot_taken: false,
            exiting: false,
        })
        .add_systems(Startup, setup)
        .add_systems(First, record_frame)
        .add_systems(
            Update,
            (
                drain_session,
                change_synthetic_states,
                apply_colours,
                move_markers,
                place_mus,
                draw_connections,
                camera_input.run_if(not(egui_wants_any_pointer_input)),
                script,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, panels);
    if config.mode == Mode::Session {
        let lines = config.lines;
        let session = LiveSession::new(move || build_model(lines), WallClock::new());
        session.set_speed(config.speed).unwrap();
        app.insert_resource(Session {
            session: Mutex::new(session),
            picture: Vec::new(),
            shown: SimTime::ZERO,
            status: SessionStatus::Halted,
            started: None,
            shown_at_start: SimTime::ZERO,
            halted_at: None,
            occurrences: 0,
            errors: Vec::new(),
        });
    }
    app.run();
}

fn setup(
    mut commands: Commands,
    config: Res<Config>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    session: Option<Res<Session>>,
) {
    let count = config.lines * PER_LINE;
    let positions: Vec<Vec2> = (0..count).map(position).collect();
    let edges: Vec<(u32, u32)> = (0..config.lines)
        .flat_map(|line| EDGES.iter().map(move |&(a, b)| ((line * PER_LINE + a) as u32, (line * PER_LINE + b) as u32)))
        .collect();

    let max = positions.iter().fold(Vec2::NEG_INFINITY, |m, p| m.max(*p));
    let min = positions.iter().fold(Vec2::INFINITY, |m, p| m.min(*p));
    let centre = if config.close { positions[0] + Vec2::new(8.0, 0.0) } else { (min + max) / 2.0 };
    // WindowSize scaling: scale is metres per pixel.
    let scale = if config.close { 1.0 / 40.0 } else { ((max.x - min.x + 10.0) / 1300.0).max((max.y - min.y + 10.0) / 820.0) };

    let object_mesh = meshes.add(Rectangle::new(OBJECT_SIZE.x, OBJECT_SIZE.y));
    let marker_mesh = meshes.add(Circle::new(MARKER_RADIUS));
    let state_materials: Vec<Handle<StandardMaterial>> = COLOURS
        .iter()
        .map(|&colour| materials.add(StandardMaterial { base_color: colour, unlit: true, ..default() }))
        .collect();
    let marker_material = materials.add(StandardMaterial { base_color: MARKER_COLOUR, unlit: true, ..default() });

    match config.render {
        Render::Sprites => {
            commands.spawn((
                Camera2d,
                Projection::Orthographic(OrthographicProjection { scale, ..OrthographicProjection::default_2d() }),
                Transform::from_xyz(centre.x, centre.y, 0.0),
                WorldCamera,
            ));
        }
        Render::Meshes => {
            commands.spawn((
                Camera3d::default(),
                Projection::Orthographic(OrthographicProjection { scale, ..OrthographicProjection::default_3d() }),
                Transform::from_xyz(centre.x, centre.y, 100.0).looking_at(centre.extend(0.0), Vec3::Y),
                WorldCamera,
            ));
        }
    }

    let spawn_object = |commands: &mut Commands, at: Vec2, colour: u8| -> Entity {
        let transform = Transform::from_translation(at.extend(0.0));
        match config.render {
            Render::Sprites => commands.spawn((Sprite::from_color(COLOURS[colour as usize], OBJECT_SIZE), transform)).id(),
            Render::Meshes => commands
                .spawn((Mesh3d(object_mesh.clone()), MeshMaterial3d(state_materials[colour as usize].clone()), transform))
                .id(),
        }
    };
    let entities: Vec<Entity> = positions.iter().map(|&at| spawn_object(&mut commands, at, IDLE)).collect();

    let marker_count = config.markers;
    let markers: Vec<Entity> = (0..marker_count)
        .map(|_| {
            let transform = Transform::from_xyz(0.0, 0.0, 1.0);
            match config.render {
                Render::Sprites => commands
                    .spawn((Sprite::from_color(MARKER_COLOUR, Vec2::splat(MARKER_RADIUS * 2.0)), transform, Visibility::Hidden))
                    .id(),
                Render::Meshes => commands
                    .spawn((Mesh3d(marker_mesh.clone()), MeshMaterial3d(marker_material.clone()), transform, Visibility::Hidden))
                    .id(),
            }
        })
        .collect();

    let mut scene = Scene {
        positions,
        drawn: vec![IDLE; count],
        synthetic: vec![IDLE; count],
        entities,
        edges,
        materials: state_materials,
        free: (0..marker_count).rev().collect(),
        markers,
        glides: Vec::new(),
        dropped_glides: 0,
        rng: 0x9E37_79B9_7F4A_7C15,
        added: 0,
    };
    if config.mode == Mode::Scale {
        // Every marker glides along a random connection, starting at a random point of its glide.
        for _ in 0..marker_count {
            start_random_glide(&mut scene, 0.0, true);
        }
    }
    commands.insert_resource(scene);

    if let Some(session) = session {
        session.session.lock().unwrap().start().unwrap();
    }
}

fn start_random_glide(scene: &mut Scene, now: f64, part_way: bool) {
    let Some(marker) = scene.free.pop() else { return };
    let pick = (scene.random() % scene.edges.len() as u64) as usize;
    let edge = scene.edges[pick];
    let duration = 0.3 + (scene.random() % 700) as f64 / 1000.0;
    let start = if part_way { now - duration * (scene.random() % 1000) as f64 / 1000.0 } else { now };
    scene.glides.push(Glide {
        marker,
        from: scene.positions[edge.0 as usize],
        to: scene.positions[edge.1 as usize],
        start,
        duration,
    });
}

// ---------- Per frame ----------

fn record_frame(mut stats: ResMut<Stats>, session: Option<Res<Session>>) {
    let now = Instant::now();
    if let Some(last) = stats.last_frame {
        if now.duration_since(stats.started) > Duration::from_secs(1) {
            let frame = now.duration_since(last).as_secs_f64();
            stats.frames.push(frame);
        }
    }
    if session.is_some_and(|session| session.status == SessionStatus::Halted && session.halted_at.is_some()) {
        stats.halted_frames += 1;
    }
    stats.last_frame = Some(now);
}

fn drain_session(session: Option<ResMut<Session>>, mut scene: ResMut<Scene>, mut stats: ResMut<Stats>) {
    let Some(mut session) = session else { return };
    let started = Instant::now();
    let mut occurrences = 0u64;
    let session = &mut *session;
    let observations: Vec<Observation> = session.session.lock().unwrap().observations().try_iter().collect();
    for observation in observations {
        match observation {
            Observation::State(state) => {
                session.picture = state.objects.iter().map(|o| ObjectPicture::from_state(o, state.time)).collect();
                session.shown = state.time;
            }
            // Init's occurrences come before its full state, which already includes them.
            Observation::Occurrences(_) if session.picture.is_empty() => {}
            Observation::Occurrences(list) => {
                for occurrence in &list {
                    occurrences += 1;
                    apply(&mut session.picture, &occurrence.kind, occurrence.time);
                }
                if let Some(last) = list.last() {
                    session.shown = session.shown.max(last.time);
                }
            }
            Observation::ShownTime(time) => session.shown = time,
            Observation::Status(status) => {
                session.status = status;
                if status == SessionStatus::Advancing && session.started.is_none() {
                    session.started = Some(Instant::now());
                    session.shown_at_start = session.shown;
                }
            }
            Observation::Error(error) => session.errors.push(error.to_string()),
        }
    }
    session.occurrences += occurrences;
    if session.started.is_some() && session.halted_at.is_none() {
        stats.drains.push(started.elapsed().as_secs_f64());
        stats.occurrences_per_frame.push(occurrences);
    }
    let picture = &session.picture;
    for (id, object) in picture.iter().enumerate() {
        scene.synthetic[id] = object.colour();
    }
}

fn change_synthetic_states(config: Res<Config>, mut scene: ResMut<Scene>) {
    if config.mode != Mode::Scale {
        return;
    }
    // About 5% of objects change state each frame, as at a high speed.
    let count = scene.synthetic.len();
    for _ in 0..count / 20 {
        let id = (scene.random() % count as u64) as usize;
        let colour = (scene.random() % 5) as u8;
        scene.synthetic[id] = colour;
    }
}

fn apply_colours(
    config: Res<Config>,
    mut scene: ResMut<Scene>,
    mut sprites: Query<&mut Sprite>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    let scene = &mut *scene;
    for id in 0..scene.synthetic.len() {
        let wanted = scene.synthetic[id];
        if scene.drawn[id] == wanted {
            continue;
        }
        scene.drawn[id] = wanted;
        let entity = scene.entities[id];
        match config.render {
            Render::Sprites => sprites.get_mut(entity).unwrap().color = COLOURS[wanted as usize],
            Render::Meshes => materials.get_mut(entity).unwrap().0 = scene.materials[wanted as usize].clone(),
        }
    }
}

fn move_markers(
    config: Res<Config>,
    mut scene: ResMut<Scene>,
    mut markers: Query<(&mut Transform, &mut Visibility)>,
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    let scene = &mut *scene;
    let mut finished = 0;
    let mut index = 0;
    while index < scene.glides.len() {
        let glide = scene.glides[index];
        let progress = ((now - glide.start) / glide.duration) as f32;
        let (mut transform, mut visibility) = markers.get_mut(scene.markers[glide.marker]).unwrap();
        if progress >= 1.0 {
            *visibility = Visibility::Hidden;
            scene.free.push(glide.marker);
            scene.glides.swap_remove(index);
            finished += 1;
            continue;
        }
        transform.translation = glide.from.lerp(glide.to, progress.max(0.0)).extend(1.0);
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
        index += 1;
    }
    if config.mode == Mode::Scale {
        for _ in 0..finished {
            start_random_glide(scene, now, false);
        }
    }
}

/// A move takes no simulation time, so an MU is drawn in the object that holds it and jumps
/// straight to the next one. Only MUs in view are drawn, and only when zoomed in and slow enough.
fn place_mus(
    config: Res<Config>,
    session: Option<Res<Session>>,
    scene: Res<Scene>,
    camera: Single<(&Transform, &Projection), With<WorldCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut markers: Query<(&mut Transform, &mut Visibility), Without<WorldCamera>>,
    mut shown_before: Local<usize>,
) {
    let Some(session) = session else { return };
    let (camera_transform, projection) = *camera;
    let ppm = pixels_per_metre(projection);
    let mut used = 0;
    if ppm >= MARKER_MIN_PPM && config.speed <= MARKER_MAX_SPEED {
        let half = Vec2::new(window.width(), window.height()) / ppm / 2.0 + Vec2::splat(1.0);
        let centre = camera_transform.translation.truncate();
        'objects: for (id, object) in session.picture.iter().enumerate() {
            let at = scene.positions[id];
            if (at - centre).abs().cmpgt(half).any() {
                continue;
            }
            for (k, _) in object.contents.iter().enumerate() {
                if used == scene.markers.len() {
                    break 'objects;
                }
                let target = (at + Vec2::new(k as f32 * 0.3, 0.0)).extend(1.0);
                let (mut transform, mut visibility) = markers.get_mut(scene.markers[used]).unwrap();
                if transform.translation != target {
                    transform.translation = target;
                }
                if *visibility != Visibility::Inherited {
                    *visibility = Visibility::Inherited;
                }
                used += 1;
            }
        }
    }
    for marker in used..*shown_before {
        let (_, mut visibility) = markers.get_mut(scene.markers[marker]).unwrap();
        *visibility = Visibility::Hidden;
    }
    *shown_before = used;
}

fn draw_connections(mut gizmos: Gizmos, scene: Res<Scene>, selection: Res<Selection>) {
    let colour = Color::srgba(0.6, 0.6, 0.7, 0.35);
    for &(a, b) in &scene.edges {
        let (a, b) = (scene.positions[a as usize], scene.positions[b as usize]);
        gizmos.line(a.extend(0.5), b.extend(0.5), colour);
    }
    if let Some(id) = selection.0 {
        let at = scene.positions[id];
        gizmos.rect(Isometry3d::from_translation(at.extend(2.0)), OBJECT_SIZE + Vec2::splat(0.4), Color::WHITE);
    }
}

fn pixels_per_metre(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(ortho) => 1.0 / ortho.scale,
        _ => 1.0,
    }
}

fn camera_input(
    mut camera: Single<(&mut Transform, &mut Projection, &Camera, &GlobalTransform), With<WorldCamera>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut scene: ResMut<Scene>,
    mut selection: ResMut<Selection>,
    mut drag: ResMut<CanvasDrag>,
    mut objects: Query<&mut Transform, Without<WorldCamera>>,
) {
    let (transform, projection, camera, global) = &mut *camera;
    let pointer = window
        .cursor_position()
        .and_then(|cursor| camera.viewport_to_world(global, cursor).ok())
        .map(|ray| ray.origin.truncate());
    if let (Some((id, grip)), Some(at)) = (drag.0, pointer) {
        if buttons.pressed(MouseButton::Left) {
            let to = at + grip;
            if scene.positions[id] != to {
                scene.positions[id] = to;
                objects.get_mut(scene.entities[id]).unwrap().translation = to.extend(0.0);
            }
        }
    }
    if buttons.just_released(MouseButton::Left) {
        drag.0 = None;
    }
    let Projection::Orthographic(ortho) = &mut **projection else { return };
    if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
        transform.translation.x -= motion.delta.x * ortho.scale;
        transform.translation.y += motion.delta.y * ortho.scale;
    }
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if lines != 0.0 {
        ortho.scale = (ortho.scale * 0.85f32.powf(lines)).clamp(0.002, 2.0);
    }
    if buttons.just_pressed(MouseButton::Left) {
        if let Some(cursor) = window.cursor_position() {
            if let Ok(ray) = camera.viewport_to_world(global, cursor) {
                let at = ray.origin.truncate();
                // A plain scan of every object: about 6,000 distance checks per click.
                let nearest = scene
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(id, p)| (id, p.distance_squared(at)))
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                selection.0 = nearest.filter(|(_, d)| *d < 1.0).map(|(id, _)| id);
                drag.0 = selection.0.map(|id| (id, scene.positions[id] - at));
            }
        }
    }
}

/// Runs the scripted parts of a measurement: halting, a screenshot and exiting.
fn script(
    mut commands: Commands,
    config: Res<Config>,
    mut stats: ResMut<Stats>,
    session: Option<ResMut<Session>>,
    mut winit: ResMut<WinitSettings>,
    mut exit: MessageWriter<AppExit>,
) {
    let elapsed = stats.started.elapsed().as_secs_f64();
    let mut session = session;
    if let (Some(session), Some(halt_after)) = (session.as_mut(), config.halt_after) {
        if elapsed >= halt_after && session.halted_at.is_none() {
            session.session.lock().unwrap().halt().unwrap();
            session.halted_at = Some(Instant::now());
            // Halted: draw only when something asks for it.
            *winit = WinitSettings::desktop_app();
        }
    }
    let session_summary = session.as_deref();
    let Some(seconds) = config.seconds else { return };
    if let Some(path) = &config.screenshot {
        if !stats.screenshot_taken && elapsed >= seconds * 0.6 {
            stats.screenshot_taken = true;
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
        }
    }
    if elapsed >= seconds && !stats.exiting {
        stats.exiting = true;
        if let Some(session) = session_summary {
            let wall = session.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
            let until = session.halted_at.map_or(wall, |h| h.duration_since(session.started.unwrap()).as_secs_f64());
            let shown = session.shown.as_secs_f64() - session.shown_at_start.as_secs_f64();
            println!(
                "session: shown {:.0} s of simulated time in {:.1} s advancing, achieved {:.0}x of {:.0}x; {} occurrences, {:.0} per second; errors {:?}",
                shown,
                until,
                shown / until.max(1e-6),
                config.speed,
                session.occurrences,
                session.occurrences as f64 / until.max(1e-6),
                session.errors
            );
        }
        exit.write(AppExit::Success);
    }
}

// ---------- Panels ----------

#[allow(clippy::too_many_arguments)]
fn panels(
    mut contexts: EguiContexts,
    config: Res<Config>,
    mut scene: ResMut<Scene>,
    mut selection: ResMut<Selection>,
    mut drag: ResMut<PaletteDrag>,
    session: Option<ResMut<Session>>,
    stats: Res<Stats>,
    camera: Single<(&Camera, &GlobalTransform, &Projection), With<WorldCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut winit: ResMut<WinitSettings>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let (camera, global, projection) = *camera;
    let ppm = pixels_per_metre(projection);
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let mut session = session;

    let recent = |values: &[f64]| -> (f64, f64) {
        let tail = &values[values.len().saturating_sub(120)..];
        if tail.is_empty() {
            return (0.0, 0.0);
        }
        let mean = tail.iter().sum::<f64>() / tail.len() as f64;
        (mean, tail.iter().cloned().fold(0.0, f64::max))
    };
    // While halted, live numbers would change the panel every frame and keep egui repainting.
    let halted = session.as_ref().is_some_and(|session| session.halted_at.is_some());
    let (mean_frame, worst_frame) = if halted { (0.0, 0.0) } else { recent(&stats.frames) };

    let top = egui::Panel::top("top").show(&mut root, |ui| {
        ui.horizontal(|ui| {
            ui.label(format!("{:?} / {:?}", config.mode, config.render));
            ui.separator();
            ui.label(format!(
                "{:.0} fps, frame {:.1} ms, worst of last 120 {:.1} ms",
                1.0 / mean_frame.max(1e-6),
                mean_frame * 1000.0,
                worst_frame * 1000.0
            ));
            ui.separator();
            ui.label(format!("{} objects, {} glides, {:.1} px/m", scene.positions.len(), scene.glides.len(), ppm));
            if let Some(session) = session.as_mut() {
                ui.separator();
                let wall = session.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
                let achieved = (session.shown.as_secs_f64() - session.shown_at_start.as_secs_f64()) / wall.max(1e-6);
                ui.label(format!(
                    "shown {:.0} s, {:?}, achieved {:.0}x of {:.0}x",
                    session.shown.as_secs_f64(),
                    session.status,
                    achieved,
                    config.speed
                ));
                if ui.button("Start").clicked() {
                    session.session.lock().unwrap().start().unwrap();
                    session.halted_at = None;
                    *winit = WinitSettings { focused_mode: UpdateMode::Continuous, unfocused_mode: UpdateMode::Continuous };
                }
                if ui.button("Halt").clicked() {
                    session.session.lock().unwrap().halt().unwrap();
                    session.halted_at = Some(Instant::now());
                    *winit = WinitSettings::desktop_app();
                }
            }
        });
    });

    let left = egui::Panel::left("palette").exact_size(140.0).show(&mut root, |ui| {
        ui.heading("Palette");
        ui.label("Drag into the model");
        for kind in ["Source", "Station", "Buffer", "Drain"] {
            let response = ui.add(egui::Button::new(kind).sense(egui::Sense::drag()));
            if response.drag_started() {
                drag.0 = Some(kind);
            }
        }
        ui.separator();
        ui.label(format!("Added: {}", scene.added));
    });

    let right = egui::Panel::right("inspector").exact_size(260.0).show(&mut root, |ui| {
        ui.heading("Inspector");
        match selection.0 {
            None => {
                ui.label("Click an object");
            }
            Some(id) if id < config.lines * PER_LINE => {
                ui.label(format!("{} of line {}", NAMES[id % PER_LINE], id / PER_LINE));
                ui.label(format!("Object id {id}"));
                if let Some(object) = session.as_ref().and_then(|s| s.picture.get(id)) {
                    ui.label(format!("State: {:?}", object.process));
                    ui.label(format!("Failed: {}, paused: {}", object.failed, object.paused));
                    ui.label(format!("Contents: {:?}", object.contents));
                    let now = session.as_ref().unwrap().shown;
                    let remaining = object.frozen.or_else(|| object.completion.map(|c| c.saturating_sub(now)));
                    ui.label(format!("Remaining: {:?}", remaining.map(|r| r.as_secs_f64())));
                } else {
                    ui.label(format!("Synthetic colour: {}", scene.synthetic[id]));
                }
            }
            Some(id) => {
                ui.label(format!("Added object {id}"));
            }
        }
        if let Some(session) = session.as_ref() {
            ui.separator();
            for error in session.errors.iter().rev().take(5) {
                ui.colored_label(egui::Color32::RED, error);
            }
        }
    });

    // Labels, under the panels' rectangles.
    let canvas = egui::Rect::from_min_max(
        egui::pos2(left.response.rect.right(), top.response.rect.bottom()),
        egui::pos2(right.response.rect.left(), ctx.viewport_rect().bottom()),
    );
    if ppm >= LABEL_MIN_PPM {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("labels"))).with_clip_rect(canvas);
        let mut drawn = 0;
        for (id, &at) in scene.positions.iter().enumerate().take(config.lines * PER_LINE) {
            let Ok(screen) = camera.world_to_viewport(global, at.extend(0.0) + Vec3::new(0.0, -0.8, 0.0)) else { continue };
            let point = egui::pos2(screen.x, screen.y);
            if !canvas.contains(point) {
                continue;
            }
            painter.text(point, egui::Align2::CENTER_TOP, NAMES[id % PER_LINE], egui::FontId::proportional(11.0), egui::Color32::LIGHT_GRAY);
            drawn += 1;
            if drawn >= 3_000 {
                break;
            }
        }
    }

    // Dragging from the palette: a ghost follows the pointer, and letting go over the canvas adds an object.
    if let Some(kind) = drag.0 {
        if let Some(pointer) = ctx.pointer_latest_pos() {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ghost")));
            let size = egui::vec2(OBJECT_SIZE.x, OBJECT_SIZE.y) * ppm;
            painter.rect_filled(egui::Rect::from_center_size(pointer, size), 2.0, egui::Color32::from_white_alpha(120));
            painter.text(pointer + egui::vec2(0.0, size.y), egui::Align2::CENTER_TOP, kind, egui::FontId::proportional(12.0), egui::Color32::WHITE);
        }
        if ctx.input(|input| input.pointer.any_released()) {
            drag.0 = None;
            if let Some(cursor) = window.cursor_position() {
                if canvas.contains(egui::pos2(cursor.x, cursor.y)) {
                    if let Ok(ray) = camera.viewport_to_world(global, cursor) {
                        let at = ray.origin.truncate();
                        let transform = Transform::from_translation(at.extend(0.0));
                        let entity = match config.render {
                            Render::Sprites => commands.spawn((Sprite::from_color(COLOURS[ADDED as usize], OBJECT_SIZE), transform)).id(),
                            Render::Meshes => commands
                                .spawn((
                                    Mesh3d(meshes.add(Rectangle::new(OBJECT_SIZE.x, OBJECT_SIZE.y))),
                                    MeshMaterial3d(scene.materials[ADDED as usize].clone()),
                                    transform,
                                ))
                                .id(),
                        };
                        scene.positions.push(at);
                        scene.entities.push(entity);
                        scene.drawn.push(ADDED);
                        scene.synthetic.push(ADDED);
                        scene.added += 1;
                        selection.0 = Some(scene.positions.len() - 1);
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------- Summary on exit ----------

impl Drop for Stats {
    fn drop(&mut self) {
        let mut frames = self.frames.clone();
        if frames.is_empty() {
            return;
        }
        frames.sort_by(|a, b| a.total_cmp(b));
        let pick = |values: &[f64], q: f64| values[((values.len() - 1) as f64 * q) as usize] * 1000.0;
        let total: f64 = frames.iter().sum();
        println!(
            "frames {} over {:.1} s: mean {:.1} fps; frame ms p50 {:.2} p95 {:.2} p99 {:.2} max {:.2}; over 33 ms: {}, over 50 ms: {}",
            frames.len(),
            total,
            frames.len() as f64 / total,
            pick(&frames, 0.5),
            pick(&frames, 0.95),
            pick(&frames, 0.99),
            frames.last().unwrap() * 1000.0,
            frames.iter().filter(|&&f| f > 0.033).count(),
            frames.iter().filter(|&&f| f > 0.050).count(),
        );
        if !self.drains.is_empty() {
            let mut drains = self.drains.clone();
            drains.sort_by(|a, b| a.total_cmp(b));
            let occurrences: u64 = self.occurrences_per_frame.iter().sum();
            println!(
                "draining and applying, per frame: ms p50 {:.2} p99 {:.2} max {:.2}; occurrences per frame mean {:.0} max {}",
                pick(&drains, 0.5),
                pick(&drains, 0.99),
                drains.last().unwrap() * 1000.0,
                occurrences as f64 / self.occurrences_per_frame.len() as f64,
                self.occurrences_per_frame.iter().max().unwrap(),
            );
        }
        println!("frames drawn while halted: {}", self.halted_frames);
    }
}
