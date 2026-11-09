//! crabsim's UI: a Bevy application with egui panels (ADR 0006). It runs a model in a live session
//! and draws it in 2D, as flat meshes under an orthographic 3D camera, the way a 3D view will draw
//! it too. What to draw comes from `crabsim_ui::view`, and what the window works on from
//! `crabsim_ui::workspace`, both tested without a window. UI stage 1 is described in
//! docs/agents/handoff.md, "UI design", and stage 2 in spec issue 63.
//!
//! cargo run -p crabsim-ui --profile ui -- --models <folder>
//!
//! The window lists the folder's model files and runs the one clicked. The repository's `models`
//! folder holds the demo model, `demo.json`.
//!
//! Drag with any mouse button to pan, and use the wheel to zoom. Before Init or after Reset, a
//! left drag on an object moves it, saved to the model file when let go.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    log::LogPlugin,
    prelude::*,
    window::PrimaryWindow,
    winit::{EventLoopProxyWrapper, UpdateMode, WinitSettings, WinitUserEvent},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui, input::egui_wants_any_pointer_input};
use clap::Parser;
use crabsim::{LiveSession, SessionStatus, SimTime, TimeMode, WallClock};
use crabsim_ui::camera::Camera as ViewCamera;
use crabsim_ui::view::{
    Control, DEFAULT_SPEED, Details, EndTimeProblem, Inspection, Intervention, JumpProblem, MAX_SPEED, MIN_SPEED, Mark,
    Point, MU_METRES, Shade, StepBack, ViewState, format_time, format_time_exact, label_size, parse_time,
};
use crabsim_ui::attributes::{Form, RepairKind};
use crabsim_ui::editing::{ActRefusal, Claim, EditAction, Editors};
use crabsim_ui::workspace::{AttributeView, ModelChange, Refusal, ValueFrom, Workspace};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "crabsim-ui",
    about = "crabsim's window: runs model files live and draws them in 2D",
    after_help = "Example: crabsim-ui --models models"
)]
struct Opts {
    /// The models folder whose model files to list and run, such as the repository's `models`.
    /// It must not hold source code.
    #[arg(long)]
    models: PathBuf,
}

const BACKGROUND: Color = Color::srgb(0.10, 0.11, 0.13);
const CONNECTION: Color = Color::srgba(0.65, 0.67, 0.75, 0.6);
const INSPECTOR_WIDTH: f32 = 280.0;
const MODELS_WIDTH: f32 = 230.0;
/// The longest wait between frames while anything can change: about 60 frames a second.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

fn shade_colour(shade: Shade) -> Color {
    match shade {
        Shade::Unknown => Color::srgb(0.28, 0.29, 0.32),
        Shade::Idle => Color::srgb(0.50, 0.52, 0.56),
        Shade::Working => Color::srgb(0.20, 0.70, 0.30),
        Shade::Blocked => Color::srgb(0.95, 0.60, 0.10),
        Shade::Paused => Color::srgb(0.25, 0.45, 0.95),
        Shade::Failed => Color::srgb(0.90, 0.15, 0.15),
        Shade::ForcedFailure => Color::srgb(0.75, 0.20, 0.65),
    }
}

const SHADES: [Shade; 7] =
    [Shade::Unknown, Shade::Idle, Shade::Working, Shade::Blocked, Shade::Paused, Shade::Failed, Shade::ForcedFailure];

fn main() {
    let opts = Opts::parse();
    let workspace = Workspace::open(&opts.models);
    let view = ViewState::new(workspace.description());
    let session = workspace.session(WallClock::new());
    session.set_speed(DEFAULT_SPEED).expect("a positive speed");
    let seed_text = workspace.seed().to_string();

    App::new()
        .insert_resource(ClearColor(BACKGROUND))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window { title: "crabsim".into(), ..default() }),
                    ..default()
                })
                // The engine logs every event at info level, which Bevy's logger would print, and
                // the graphics driver's Vulkan loader warns about duplicate layers on every start.
                .set(LogPlugin { filter: "warn,crabsim=warn,wgpu_hal=error".into(), ..default() }),
        )
        .add_plugins(EguiPlugin::default())
        .insert_resource(WinitSettings::desktop_app())
        .insert_resource(Ui {
            view,
            camera: None,
            camera_moved: false,
            canvas: None,
            jump_text: String::new(),
            press: None,
            process_time_text: (None, String::new()),
            pending: None,
            workspace,
            seed_text,
            end_text: String::new(),
            replacing: None,
            rebuild_scene: false,
            attributes: None,
            forms: HashMap::new(),
            refused: None,
            tab: Tab::Inspector,
            editors: None,
            editors_stamp: None,
            editors_read: Duration::ZERO,
            editors_note: None,
            confirm_take: None,
            moving: None,
            confirm_move: None,
        })
        .insert_resource(Session(Mutex::new(session)))
        .insert_resource(WatchedFile(Arc::new(Mutex::new(Vec::new()))))
        .add_systems(Startup, (setup, watch_the_open_file))
        .add_systems(
            Update,
            (
                pick_up_changes,
                rebuild_scene,
                drain_session,
                request_details,
                apply_shades,
                canvas_input.run_if(not(egui_wants_any_pointer_input)),
                place_camera,
                draw_connections,
                place_mus,
            )
                .chain(),
        )
        .add_systems(EguiPrimaryContextPass, panels)
        .add_systems(Last, pace_frames)
        .run();
}

/// What the window knows beyond the session: the view state, and where the canvas is.
#[derive(Resource)]
struct Ui {
    view: ViewState,
    /// Set once the canvas's size is known, fitting the whole model.
    camera: Option<ViewCamera>,
    /// Whether the user has panned or zoomed. Until then the whole model stays fitted into the
    /// canvas, whose size can change over the first frames as the window settles.
    camera_moved: bool,
    /// The canvas between the panels, in logical pixels from the window's top-left corner.
    canvas: Option<egui::Rect>,
    /// What has been typed into the field for going back to a time.
    jump_text: String,
    /// Where the left button went down on the canvas, and how far the pointer has moved since,
    /// in logical pixels. A short enough press is a click, which selects.
    press: Option<(Vec2, f32)>,
    /// What has been typed into the inspector's process time field, and for which object.
    process_time_text: (Option<u32>, String),
    /// Going back that drops interventions, waiting for the user to confirm it.
    pending: Option<GoingBack>,
    /// The models folder and the model open.
    workspace: Workspace,
    /// What has been typed into the seed field.
    seed_text: String,
    /// What has been typed into the end time field.
    end_text: String,
    /// A model chosen while a run is in progress, waiting for the user to confirm replacing it.
    replacing: Option<String>,
    /// The view changed model or seed, so the objects are drawn afresh.
    rebuild_scene: bool,
    /// The selected instance's attributes, read from the model file, and for which object.
    attributes: Option<(u32, Result<Vec<AttributeView>, String>)>,
    /// The fields for each attribute of the selected instance, by attribute.
    forms: HashMap<String, Form>,
    /// The last change refused, for its attribute: why, and the change to make anyway if an agent
    /// holds the file.
    refused: Option<(String, Refusal, Option<AttributeChange>)>,
    /// Which tab the right-hand column shows.
    tab: Tab,
    /// Who holds each model file, as last read, while the Editing tab shows it.
    editors: Option<Result<Editors, String>>,
    /// The edit state file's size and modified time when it was read.
    editors_stamp: Option<(u64, Option<std::time::SystemTime>)>,
    /// When the edit state was last read, in wall time since the window opened.
    editors_read: Duration,
    /// Why the last control did nothing, such as who holds the model having changed.
    editors_note: Option<String>,
    /// Taking the right from this agent, waiting for the user to confirm it.
    confirm_take: Option<(Claim, Editors)>,
    /// The object being dragged to a new place.
    moving: Option<Moving>,
    /// A move refused because an agent holds the file or it changed outside crabsim, waiting for
    /// the user to make it anyway or put the object back, with why it was refused.
    confirm_move: Option<(Moving, Refusal)>,
}

/// An object dragged on the canvas, before Init or after Reset.
#[derive(Clone, Copy)]
struct Moving {
    id: u32,
    /// Where the object's centre was when the drag began, to put it back if the move is refused.
    from: Point,
    /// The object's centre less the pointer, in metres, so the object keeps where it was gripped.
    grip: Point,
    /// Whether the pointer has gone far enough for a drag rather than a click.
    dragged: bool,
}

/// The tabs of the right-hand column.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Inspector,
    Editing,
}

/// A change to one attribute of the selected instance, asked for in the inspector.
#[derive(Clone)]
enum AttributeChange {
    Set(String, serde_json::Value),
    Restore(String),
}

impl AttributeChange {
    fn attribute(&self) -> &str {
        match self {
            AttributeChange::Set(attribute, _) | AttributeChange::Restore(attribute) => attribute,
        }
    }
}

/// A way of going back that waits for confirmation, and the time it goes to.
#[derive(Clone, Copy)]
enum GoingBack {
    To(SimTime),
    /// Stepping back, to the instant before the one on screen.
    Step(SimTime),
}

/// A press that moves less than this, in logical pixels, is a click rather than a drag.
const CLICK_SLOP: f32 = 4.0;
const SELECTION: Color = Color::srgb(1.0, 1.0, 1.0);
const MU_COLOUR: Color = Color::srgb(0.96, 0.96, 0.93);

/// The model file open and the edit state, which a thread of its own watches while the window is
/// idle.
#[derive(Resource)]
struct WatchedFile(Arc<Mutex<Vec<PathBuf>>>);

/// How often the watcher looks at the open file while the window is idle.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Wakes the window when the open model file's size or modified time changes, such as when an
/// agent edits it, or the edit state's does, such as when an agent claims a model, so a halted
/// window stays idle until then. While the window draws frames, it checks them itself, each frame.
fn watch_the_open_file(watched: Res<WatchedFile>, proxy: Res<EventLoopProxyWrapper>) {
    let file = watched.0.clone();
    let proxy = (**proxy).clone();
    std::thread::spawn(move || {
        let mut seen = None;
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            let paths = file.lock().expect("the watched paths' lock is never poisoned").clone();
            let stamp: Vec<_> = paths.iter().map(|path| stamp_of(path)).collect();
            let stamp = Some(stamp);
            if stamp != seen {
                seen = stamp;
                if proxy.send_event(WinitUserEvent::WakeUp).is_err() {
                    return;
                }
            }
        }
    });
}

/// How long a shown claim may go unread, since it can end without the edit state file changing.
const CLAIMS_READ_AGAIN: Duration = Duration::from_secs(3);

/// A file's size and modified time, which change whenever it is written.
fn stamp_of(path: &std::path::Path) -> Option<(u64, Option<std::time::SystemTime>)> {
    std::fs::metadata(path).ok().map(|metadata| (metadata.len(), metadata.modified().ok()))
}

/// Notices that the open model file changed. With no run in progress the window uses the new
/// model at once, and otherwise it says that Reset will.
fn pick_up_changes(mut ui: ResMut<Ui>, session: Res<Session>, watched: Res<WatchedFile>, time: Res<Time<Real>>) {
    let wall = time.elapsed();
    let ui = &mut *ui;
    {
        let files: Vec<PathBuf> = ui.workspace.watched_file().into_iter().chain(ui.workspace.editing_file()).collect();
        let mut watching = watched.0.lock().expect("the watched paths' lock is never poisoned");
        if *watching != files {
            *watching = files;
        }
    }
    match ui.workspace.check() {
        Some(ModelChange::Changed) if ui.view.reloads_now() => {
            ui.workspace.apply_pending();
            replace_session(ui, &session, false, wall);
        }
        Some(ModelChange::Changed) => {
            ui.view.set_model_changed(true);
            ui.view.commanded(wall);
        }
        Some(ModelChange::NowHasProblems) => ui.view.commanded(wall),
        None => {}
    }
}

/// A `LiveSession` holds a `Receiver`, which can be sent between threads but not shared.
#[derive(Resource)]
struct Session(Mutex<LiveSession>);

/// The entity drawing each object, by id, and the shade it shows, so only changes touch the ECS.
#[derive(Resource)]
struct Scene {
    entities: Vec<Entity>,
    drawn: Vec<Shade>,
    materials: HashMap<Shade, Handle<StandardMaterial>>,
}

#[derive(Component)]
struct WorldCamera;

/// Entities drawing MUs, reused from frame to frame: the first `shown` are visible.
#[derive(Resource)]
struct MuPool {
    entities: Vec<Entity>,
    shown: usize,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

fn setup(
    mut commands: Commands,
    ui: Res<Ui>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut material_assets: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection::default_3d()),
        Transform::from_xyz(0.0, 0.0, 100.0).looking_at(Vec3::ZERO, Vec3::Y),
        WorldCamera,
    ));
    let materials: HashMap<Shade, Handle<StandardMaterial>> = SHADES
        .iter()
        .map(|&shade| {
            (shade, material_assets.add(StandardMaterial { base_color: shade_colour(shade), unlit: true, ..default() }))
        })
        .collect();
    let entities = spawn_objects(&mut commands, &ui.view, &mut meshes, &materials);
    let count = ui.view.objects().len();
    commands.insert_resource(Scene { entities, drawn: vec![Shade::Unknown; count], materials });
    commands.insert_resource(MuPool {
        entities: Vec::new(),
        shown: 0,
        mesh: meshes.add(Circle::new(MU_METRES as f32 / 2.0)),
        material: material_assets.add(StandardMaterial { base_color: MU_COLOUR, unlit: true, ..default() }),
    });
}

/// An entity for each object of the view, unshaded. Objects of the same size share a mesh.
fn spawn_objects(
    commands: &mut Commands,
    view: &ViewState,
    meshes: &mut Assets<Mesh>,
    materials: &HashMap<Shade, Handle<StandardMaterial>>,
) -> Vec<Entity> {
    let mut sizes: HashMap<(u64, u64), Handle<Mesh>> = HashMap::new();
    view.objects()
        .iter()
        .map(|object| {
            let mesh = sizes
                .entry((object.length.to_bits(), object.width.to_bits()))
                .or_insert_with(|| meshes.add(Rectangle::new(object.length as f32, object.width as f32)))
                .clone();
            let transform = Transform::from_xyz(object.centre.x as f32, object.centre.y as f32, 0.0)
                .with_rotation(Quat::from_rotation_z((object.rotation_degrees as f32).to_radians()));
            commands.spawn((Mesh3d(mesh), MeshMaterial3d(materials[&Shade::Unknown].clone()), transform)).id()
        })
        .collect()
}

/// Draws the objects afresh after the view changed model or seed: the old entities and MUs go.
fn rebuild_scene(
    mut commands: Commands,
    mut ui: ResMut<Ui>,
    mut scene: ResMut<Scene>,
    mut pool: ResMut<MuPool>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if !ui.rebuild_scene {
        return;
    }
    ui.rebuild_scene = false;
    for entity in scene.entities.drain(..).chain(pool.entities.drain(..)) {
        commands.entity(entity).despawn();
    }
    pool.shown = 0;
    let entities = spawn_objects(&mut commands, &ui.view, &mut meshes, &scene.materials);
    scene.drawn = vec![Shade::Unknown; entities.len()];
    scene.entities = entities;
}

/// Reads every output waiting, every frame: past a backlog the session stops releasing.
fn drain_session(session: Res<Session>, mut ui: ResMut<Ui>, time: Res<Time<Real>>) {
    let wall = time.elapsed();
    let session = session.0.lock().expect("the session lock is never poisoned");
    for output in session.outputs().try_iter() {
        ui.view.observe(&output, wall);
    }
}

fn apply_shades(ui: Res<Ui>, mut scene: ResMut<Scene>, mut drawn: Query<&mut MeshMaterial3d<StandardMaterial>>) {
    let scene = &mut *scene;
    for object in ui.view.objects() {
        let id = object.id as usize;
        let shade = ui.view.shade(object.id);
        if scene.drawn[id] != shade {
            scene.drawn[id] = shade;
            if let Ok(mut material) = drawn.get_mut(scene.entities[id]) {
                material.0 = scene.materials[&shade].clone();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn canvas_input(
    mut ui: ResMut<Ui>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    scene: Res<Scene>,
    mut transforms: Query<&mut Transform>,
    time: Res<Time<Real>>,
) {
    let Some(canvas) = ui.canvas else { return };
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    let ui = &mut *ui;
    let Some(camera) = ui.camera else { return };
    let floor = |at: Vec2| camera.to_world(Point { x: (at.x - canvas.min.x) as f64, y: (at.y - canvas.min.y) as f64 }, size);
    if buttons.just_pressed(MouseButton::Left) && ui.confirm_move.is_none() {
        ui.press = window.cursor_position().map(|at| (at, 0.0));
        // Before Init or after Reset, pressing on an object grips it, to move it.
        let movable = ui.view.reloads_now() && ui.workspace.opened().is_some();
        ui.moving = ui.press.filter(|_| movable).and_then(|(at, _)| {
            let pointer = floor(at);
            let id = ui.view.object_at(pointer)?;
            let from = ui.view.objects()[id as usize].centre;
            Some(Moving { id, from, grip: Point { x: from.x - pointer.x, y: from.y - pointer.y }, dragged: false })
        });
    }
    if let Some((_, travelled)) = ui.press.as_mut() {
        *travelled += motion.delta.length();
    }
    if let (Some(moving), Some((_, travelled)), Some(cursor)) = (ui.moving.as_mut(), ui.press, window.cursor_position())
        && travelled >= CLICK_SLOP
        && motion.delta != Vec2::ZERO
    {
        moving.dragged = true;
        let pointer = floor(cursor);
        let centre = Point { x: pointer.x + moving.grip.x, y: pointer.y + moving.grip.y };
        ui.view.move_object(moving.id, centre);
        ui.view.select(Some(moving.id));
        place_object(&scene, &mut transforms, moving.id, centre);
    }
    // A release over a panel never reaches the canvas, so a drag ends whenever the button is up.
    if !buttons.pressed(MouseButton::Left)
        && let Some(moving) = ui.moving.take().filter(|moving| moving.dragged)
    {
        ui.press = None;
        ui.view.commanded(time.elapsed());
        save_move(ui, &scene, &mut transforms, moving, false);
    }
    if buttons.just_released(MouseButton::Left) {
        if let Some((at, travelled)) = ui.press.take() {
            if travelled < CLICK_SLOP {
                ui.view.select(ui.view.object_at(floor(at)));
            }
        }
    }
    let Some(camera) = ui.camera.as_mut() else { return };
    let panning = ui.moving.is_none() && buttons.pressed(MouseButton::Left);
    let dragging = panning || [MouseButton::Right, MouseButton::Middle].iter().any(|&b| buttons.pressed(b));
    if dragging && motion.delta != Vec2::ZERO {
        camera.pan(Point { x: motion.delta.x as f64, y: motion.delta.y as f64 });
        ui.camera_moved = true;
    }
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if notches != 0.0 {
        if let Some(cursor) = window.cursor_position() {
            let pointer = Point { x: (cursor.x - canvas.min.x) as f64, y: (cursor.y - canvas.min.y) as f64 };
            camera.zoom_at(1.15f64.powf(notches as f64), pointer, size);
            ui.camera_moved = true;
        }
    }
}

/// Draws the object at `centre`, keeping its rotation.
fn place_object(scene: &Scene, transforms: &mut Query<&mut Transform>, id: u32, centre: Point) {
    if let Some(&entity) = scene.entities.get(id as usize)
        && let Ok(mut transform) = transforms.get_mut(entity)
    {
        transform.translation = Vec3::new(centre.x as f32, centre.y as f32, transform.translation.z);
    }
}

/// Saves where the user dragged the object to the model file, which the window then picks up
/// and rebuilds the model from. While an agent holds the file, or when it changed outside
/// crabsim, it asks whether to move it anyway, and any other refusal puts the object back and
/// says why.
fn save_move(ui: &mut Ui, scene: &Scene, transforms: &mut Query<&mut Transform>, moving: Moving, take_over: bool) {
    match moved(ui, moving, take_over) {
        Ok(()) => {}
        Err(refusal @ (Refusal::NotEditor(_) | Refusal::ChangedOutside(_))) => ui.confirm_move = Some((moving, refusal)),
        Err(refusal) => {
            ui.view.move_object(moving.id, moving.from);
            place_object(scene, transforms, moving.id, moving.from);
            let name = &ui.view.objects()[moving.id as usize].name;
            ui.view.show_error(format!("{name} wasn't moved: {refusal}"));
        }
    }
}

/// Saves the dragged object where the view now draws it.
fn moved(ui: &mut Ui, moving: Moving, take_over: bool) -> Result<(), Refusal> {
    let object = &ui.view.objects()[moving.id as usize];
    let (name, centre) = (object.name.clone(), object.centre);
    let z = ui.workspace.description().position(moving.id).map_or(0.0, |position| position.z);
    ui.workspace.move_instance(&name, crabsim::Position { x: centre.x, y: centre.y, z }, take_over)
}

/// Points the Bevy camera so that the view camera's centre sits in the middle of the canvas. The
/// camera covers the whole window, and the panels cover the rest.
fn place_camera(
    mut ui: ResMut<Ui>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    let Some(canvas) = ui.canvas else { return };
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    if !ui.camera_moved {
        let fitted = ViewCamera::fitting(ui.view.bounds(), size);
        if ui.camera != Some(fitted) {
            ui.camera = Some(fitted);
        }
    }
    let Some(view_camera) = ui.camera else { return };
    let (mut transform, mut projection) = camera.into_inner();
    let ppm = view_camera.pixels_per_metre;
    let offset = Vec2::new(
        (canvas.center().x - window.width() / 2.0) / ppm as f32,
        -(canvas.center().y - window.height() / 2.0) / ppm as f32,
    );
    let at = Vec2::new(view_camera.centre.x as f32, view_camera.centre.y as f32) - offset;
    if transform.translation.truncate() != at {
        transform.translation = at.extend(100.0);
    }
    if let Projection::Orthographic(ortho) = &mut *projection {
        let scale = (1.0 / ppm) as f32;
        if ortho.scale != scale {
            ortho.scale = scale;
        }
    }
}

/// Asks the session for a full state when the view wants one for the inspector.
fn request_details(session: Res<Session>, mut ui: ResMut<Ui>, time: Res<Time<Real>>) {
    let wall = time.elapsed();
    if ui.view.wants_state(wall) {
        let sent = session.0.lock().expect("the session lock is never poisoned").request_state();
        ui.view.state_requested(wall);
        if let Err(error) = sent {
            ui.view.observe(&crabsim::SessionOutput::Error(error), wall);
        }
    }
}


/// Draws each MU in the object that holds it, when the view state says MUs are shown, reusing
/// the entities from the frame before. Only what changed touches the ECS.
fn place_mus(
    mut commands: Commands,
    ui: Res<Ui>,
    mut pool: ResMut<MuPool>,
    mut drawn: Query<(&mut Transform, &mut Visibility)>,
) {
    let markers = ui.camera.map(|camera| ui.view.mu_markers(&camera)).unwrap_or_default();
    let pool = &mut *pool;
    for (index, marker) in markers.iter().enumerate() {
        let at = Vec3::new(marker.at.x as f32, marker.at.y as f32, 0.2);
        match pool.entities.get(index) {
            Some(&entity) => {
                if let Ok((mut transform, mut visibility)) = drawn.get_mut(entity) {
                    if transform.translation != at {
                        transform.translation = at;
                    }
                    if *visibility != Visibility::Inherited {
                        *visibility = Visibility::Inherited;
                    }
                }
            }
            None => {
                let entity = commands
                    .spawn((Mesh3d(pool.mesh.clone()), MeshMaterial3d(pool.material.clone()), Transform::from_translation(at)))
                    .id();
                pool.entities.push(entity);
            }
        }
    }
    for &entity in pool.entities.iter().take(pool.shown).skip(markers.len()) {
        if let Ok((_, mut visibility)) = drawn.get_mut(entity) {
            *visibility = Visibility::Hidden;
        }
    }
    pool.shown = markers.len();
}
fn draw_connections(mut gizmos: Gizmos, ui: Res<Ui>) {
    for connection in ui.view.connections() {
        let start = Vec3::new(connection.start.x as f32, connection.start.y as f32, 0.0);
        let end = Vec3::new(connection.end.x as f32, connection.end.y as f32, 0.0);
        gizmos.arrow(start, end, CONNECTION).with_tip_length(0.4);
    }
    if let Some(id) = ui.view.selected() {
        let outline = ui.view.objects()[id as usize].outline(0.15);
        let corner = |point: Point| Vec3::new(point.x as f32, point.y as f32, 0.1);
        gizmos.linestrip(outline.iter().chain(outline.first()).map(|&point| corner(point)), SELECTION);
    }
}

fn panels(mut contexts: EguiContexts, mut ui: ResMut<Ui>, session: Res<Session>, time: Res<Time<Real>>) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    let mut root = egui::Ui::new(
        ctx.clone(),
        "root".into(),
        egui::UiBuilder::new().layer_id(egui::LayerId::background()).max_rect(ctx.viewport_rect()),
    );
    let ui = &mut *ui;
    let wall = time.elapsed();
    // Who holds each model, for the marker and the Editing tab, read whenever the edit state
    // changes and when a tab opens. A claim also ends without the file changing, when its agent's
    // process ends or it times out, which only reading finds out. So while a claim is shown it's
    // read again every few seconds, on frames drawn anyway: an idle window isn't woken for it.
    if ui.workspace.has_folder() {
        let stamp = ui.workspace.editing_file().and_then(|file| stamp_of(&file));
        let claims_shown = matches!(&ui.editors, Some(Ok(editors)) if editors.files.iter().any(|file| file.claim.is_some()));
        let stale = claims_shown && wall.saturating_sub(ui.editors_read) >= CLAIMS_READ_AGAIN;
        if ui.editors.is_none() || stamp != ui.editors_stamp || stale {
            ui.editors = Some(ui.workspace.editors());
            ui.editors_stamp = ui.workspace.editing_file().and_then(|file| stamp_of(&file));
            ui.editors_read = wall;
        }
    }
    let mut request = None;
    let top = egui::Panel::top("top bar").show(&mut root, |panel| {
        request = top_bar(panel, &ui.view, &ui.workspace, &mut ui.jump_text, &mut ui.seed_text, &mut ui.end_text)
    });
    let mut bottom_of_bars = top.response.rect.bottom();
    // An agent's claim on the open model, whichever tab is open.
    let marker = ui.workspace.opened().and_then(|path| ui.editors.as_ref()?.as_ref().ok()?.marker(path));
    if let Some(marker) = marker {
        let shown = egui::Panel::top("claim marker")
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(70, 50, 110)).inner_margin(6.0))
            .show(&mut root, |panel| {
                panel.colored_label(egui::Color32::WHITE, marker);
            });
        bottom_of_bars = shown.response.rect.bottom();
    }
    if let Some(banner) = ui.workspace.banner() {
        let shown = egui::Panel::top("folder banner")
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(110, 80, 20)).inner_margin(6.0))
            .show(&mut root, |panel| {
                panel.colored_label(egui::Color32::WHITE, banner);
            });
        bottom_of_bars = shown.response.rect.bottom();
    }
    if ui.view.model_changed() {
        let shown = egui::Panel::top("model changed banner")
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(30, 70, 110)).inner_margin(6.0))
            .show(&mut root, |panel| {
                panel.colored_label(egui::Color32::WHITE, "The model file changed. The run carries on with the model it started with: Reset to use the new one.");
            });
        bottom_of_bars = shown.response.rect.bottom();
    }
    if let Some(error) = ui.view.error() {
        let banner = egui::Panel::top("error banner")
            .frame(egui::Frame::new().fill(egui::Color32::from_rgb(120, 25, 25)).inner_margin(6.0))
            .show(&mut root, |panel| error_banner(panel, error, ui.view.run_failed()));
        bottom_of_bars = banner.response.rect.bottom();
    }
    // What has been typed for a process time belongs to the object it was typed for.
    if ui.process_time_text.0 != ui.view.selected() {
        ui.process_time_text = (ui.view.selected(), String::new());
    }
    let inspection = ui.view.inspection();
    // The selected instance's attributes, read again when the selection or the model changes.
    let selected = ui.view.selected();
    if selected.is_none() {
        ui.attributes = None;
    }
    if let Some(id) = selected.filter(|&id| ui.attributes.as_ref().is_none_or(|(read, _)| *read != id)) {
        let name = ui.view.objects()[id as usize].name.clone();
        let read = if ui.workspace.opened().is_some() { ui.workspace.attributes(&name) } else { Err("No model file is open".to_string()) };
        if let Ok(attributes) = &read {
            ui.forms = attributes.iter().map(|view| (view.attribute.clone(), Form::new(&view.attribute, view.value.as_ref()))).collect();
        }
        ui.attributes = Some((id, read));
        ui.refused = None;
    }
    let editable = ui.view.reloads_now();
    let mut intervention = None;
    let mut attribute_change = None;
    let mut editors_action = None;
    let right = egui::Panel::right("inspector").exact_size(INSPECTOR_WIDTH).show(&mut root, |panel| {
        panel.horizontal(|row| {
            for (tab, label) in [(Tab::Inspector, "Inspector"), (Tab::Editing, "Editing")] {
                if row.selectable_label(ui.tab == tab, label).clicked() && ui.tab != tab {
                    ui.tab = tab;
                    ui.editors = None;
                    ui.editors_note = None;
                }
            }
        });
        panel.separator();
        egui::ScrollArea::vertical().show(panel, |panel| match ui.tab {
            Tab::Inspector => {
                intervention = inspector(panel, inspection.as_ref(), &mut ui.process_time_text.1);
                if let Some((_, attributes)) = &ui.attributes {
                    attribute_change = attributes_section(panel, attributes, &mut ui.forms, editable, ui.refused.as_ref());
                }
            }
            Tab::Editing => editors_action = editing_tab(panel, ui.workspace.opened(), ui.editors.as_ref(), ui.editors_note.as_deref()),
        });
    });
    // Taking a model from an agent asks first, naming it, its goal and what it's doing.
    if let Some(action) = editors_action {
        let shown = ui.editors.as_ref().and_then(|editors| editors.as_ref().ok()).cloned().unwrap_or_default();
        let holder = ui.workspace.opened().and_then(|path| shown.agent_holding(path)).cloned();
        match (action, holder) {
            (EditAction::Take, Some(agent)) => ui.confirm_take = Some((agent, shown)),
            (action, _) => act_on_editors(ui, &shown, action),
        }
        ui.view.commanded(wall);
    }
    if let Some((agent, shown)) = ui.confirm_take.clone()
        && let Some(confirmed) = confirm_taking(&ctx, &agent)
    {
        ui.confirm_take = None;
        if confirmed {
            act_on_editors(ui, &shown, EditAction::Take);
        }
        ui.view.commanded(wall);
    }
    if let Some((moving, refusal)) = ui.confirm_move.clone()
        && let Some(anyway) = confirm_moving(&ctx, &ui.view.objects()[moving.id as usize].name, &refusal)
    {
        ui.confirm_move = None;
        ui.view.commanded(wall);
        let done = match (anyway, &refusal) {
            (false, _) => Ok(()),
            (true, Refusal::ChangedOutside(_)) => ui.workspace.adopt().map_err(Refusal::Invalid).and_then(|()| moved(ui, moving, false)),
            (true, _) => moved(ui, moving, true),
        };
        if !anyway || done.is_err() {
            // Back where it was: the window draws the objects afresh from the view.
            ui.view.move_object(moving.id, moving.from);
            ui.rebuild_scene = true;
        }
        if let Err(refusal) = done {
            let name = &ui.view.objects()[moving.id as usize].name;
            ui.view.show_error(format!("{name} wasn't moved: {refusal}"));
        }
    }
    if let (Some((change, take_over)), Some(id)) = (attribute_change, selected) {
        let name = ui.view.objects()[id as usize].name.clone();
        let done = match &change {
            AttributeChange::Set(attribute, value) => ui.workspace.set_attribute(&name, attribute, value.clone(), take_over),
            AttributeChange::Restore(attribute) => ui.workspace.restore(&name, attribute, take_over),
        };
        ui.refused = match done {
            // The window picks up the saved file next frame and rebuilds the model.
            Ok(()) => None,
            Err(refusal) => {
                let again = matches!(refusal, Refusal::NotEditor(_)).then(|| change.clone());
                Some((change.attribute().to_string(), refusal, again))
            }
        };
        ui.view.commanded(wall);
    }
    let mut chosen = None;
    let mut panel_error = None;
    let mut left_edge = ctx.viewport_rect().left();
    if ui.workspace.has_folder() {
        let left = egui::Panel::left("models").exact_size(MODELS_WIDTH).show(&mut root, |panel| {
            egui::ScrollArea::vertical().show(panel, |panel| (chosen, panel_error) = models_panel(panel, &mut ui.workspace));
        });
        left_edge = left.response.rect.right();
    }
    if let Some(error) = panel_error {
        ui.view.commanded(wall);
        ui.view.show_error(error);
    }
    let canvas = egui::Rect::from_min_max(
        egui::pos2(left_edge, bottom_of_bars),
        egui::pos2(right.response.rect.left(), ctx.viewport_rect().bottom()),
    );
    ui.canvas = Some(canvas);

    if let (Some(intervention), Some(id)) = (intervention, ui.view.selected()) {
        let session = session.0.lock().expect("the session lock is never poisoned");
        let sent = match intervention {
            Intervention::Pause => session.pause(id),
            Intervention::Resume => session.resume(id),
            Intervention::ForceFailure => session.force_failure(id),
            Intervention::EndForcedFailure => session.end_forced_failure(id),
            Intervention::SetProcessTime(time) => session.set_process_time(id, time),
        };
        match sent {
            Ok(()) => ui.view.intervened(id, intervention, wall),
            Err(error) => ui.view.observe(&crabsim::SessionOutput::Error(error), wall),
        }
    }

    // Going back past interventions drops them, so it asks first. Stepping back may first have to
    // ask the session where it goes, and decides once the answer is in.
    let request = match request {
        Some(Request::GoBack(to)) if ui.view.lost_going_back(to) > 0 => {
            ui.pending = Some(GoingBack::To(to));
            None
        }
        Some(Request::Control(Control::StepBack)) => stepping_back(ui, ui.view.step_back()),
        other => other,
    };
    let request = request.or_else(|| {
        let located = ui.view.located_step_back()?;
        stepping_back(ui, located)
    });
    let request = request.or_else(|| confirm_going_back(&ctx, ui));

    // A model clicked replaces the run, so a run in progress is replaced only once confirmed.
    if let Some(path) = chosen {
        if ui.view.allows(Control::Init) {
            open_model(ui, &session, &path, wall);
        } else {
            ui.replacing = Some(path);
        }
    }
    if let Some(path) = confirm_replacing(&ctx, ui) {
        open_model(ui, &session, &path, wall);
    }
    let request = match request {
        Some(Request::Seed(seed)) => {
            ui.workspace.set_seed(seed);
            replace_session(ui, &session, false, wall);
            None
        }
        // Reset uses a model that changed during the run: a new session is at its default state.
        Some(Request::Control(Control::Reset)) if ui.workspace.has_pending() => {
            ui.workspace.apply_pending();
            replace_session(ui, &session, false, wall);
            None
        }
        other => other,
    };

    if let Some(request) = request {
        let locating = matches!(request, Request::LocateStepBack);
        let session = session.0.lock().expect("the session lock is never poisoned");
        let sent = match request {
            // A step forward from before Init does Init, and nothing else.
            Request::Control(Control::Init) => session.step_forward(),
            Request::Control(Control::Start) => session.start(),
            Request::Control(Control::Halt) => session.halt(),
            Request::Control(Control::Reset) => session.reset(),
            Request::Control(Control::StepForward) => session.step_forward(),
            Request::Control(Control::StepBack) => session.step_back(),
            Request::Speed(speed) => session.set_speed(ui.view.set_speed(speed)),
            Request::TimeMode(mode) => {
                ui.view.set_time_mode(mode);
                session.set_time_mode(mode)
            }
            Request::GoBack(to) => session.go_back(to),
            // Shown time has to stand still for the answer to hold.
            Request::LocateStepBack => session.halt().and_then(|()| session.locate_step_back()),
            Request::EndTime(end) => {
                ui.view.set_end_time(end);
                session.set_end_time(end)
            }
            Request::Seed(_) => unreachable!("handled above"),
        };
        ui.view.commanded(wall);
        match sent {
            Ok(()) if locating => ui.view.locating_step_back(),
            Ok(()) => {}
            Err(error) => ui.view.observe(&crabsim::SessionOutput::Error(error), wall),
        }
    }

    if !ui.view.has_model() {
        let hint = if ui.workspace.has_folder() { "Choose a model from the list on the left" } else { "No models folder is open" };
        ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("no model"))).text(
            canvas.center(),
            egui::Align2::CENTER_CENTER,
            hint,
            egui::FontId::proportional(18.0),
            egui::Color32::from_gray(150),
        );
    }
    if let Some((camera, size)) = ui.camera.and_then(|camera| Some((camera, label_size(&camera)?))) {
        labels(&ctx, &ui.view, &camera, canvas, size as f32);
    }
    Ok(())
}

/// What the top bar asks the session for.
enum Request {
    Control(Control),
    Speed(f64),
    TimeMode(TimeMode),
    GoBack(SimTime),
    /// Ask the session where stepping back goes, before deciding whether to ask the user.
    LocateStepBack,
    /// Set or clear the end time.
    EndTime(Option<SimTime>),
    /// Run with this seed from now on, which means a new session.
    Seed(u64),
}

/// Opens the model at `path` in the folder: a new view and session for it. One with problems
/// opens nothing, and the workspace keeps its problems for the models panel.
fn open_model(ui: &mut Ui, session: &Session, path: &str, wall: Duration) {
    if ui.workspace.open_model(path) {
        replace_session(ui, session, true, wall);
    }
}

/// A new session on the workspace's model and seed, and a view of it keeping the window's speed,
/// time mode and end time. A new model is fitted into the canvas, and the same one keeps the view
/// where it was.
fn replace_session(ui: &mut Ui, session: &Session, new_model: bool, wall: Duration) {
    let mut view = ViewState::with_settings_of(ui.workspace.description(), &ui.view);
    // The same model, reloaded or with another seed, keeps the object selected, found by its name.
    if !new_model {
        let name = ui.view.selected().map(|id| ui.view.objects()[id as usize].name.clone());
        view.select(name.and_then(|name| view.objects().iter().find(|object| object.name == name).map(|object| object.id)));
    }
    let fresh = ui.workspace.session(WallClock::new());
    let settled = fresh
        .set_speed(view.speed())
        .and_then(|()| fresh.set_time_mode(view.time_mode()))
        .and_then(|()| fresh.set_end_time(view.end_time()));
    *session.0.lock().expect("the session lock is never poisoned") = fresh;
    ui.view = view;
    ui.pending = None;
    ui.attributes = None;
    ui.forms.clear();
    ui.refused = None;
    ui.seed_text = ui.workspace.seed().to_string();
    if new_model {
        ui.camera_moved = false;
        ui.camera = None;
    }
    ui.rebuild_scene = true;
    ui.view.commanded(wall);
    if let Err(error) = settled {
        ui.view.observe(&crabsim::SessionOutput::Error(error), wall);
    }
}

/// Asks before a model chosen replaces a run in progress. Returns the model once confirmed.
fn confirm_replacing(ctx: &egui::Context, ui: &mut Ui) -> Option<String> {
    let path = ui.replacing.clone()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm replacing")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Open {path}?"));
        modal.label("The run in progress ends, with everything it has done.");
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if row.button("Open").clicked() {
                answer = Some(true);
            }
            if row.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let answer = answer?;
    ui.replacing = None;
    answer.then_some(path)
}

/// The folder's model files, each marked with how many problems it has, whether the one open was
/// changed outside crabsim, and why the one last chosen can't run. Returns the model clicked, if
/// any, and an error to show.
fn models_panel(ui: &mut egui::Ui, workspace: &mut Workspace) -> (Option<String>, Option<String>) {
    let mut chosen = None;
    let mut error = None;
    ui.horizontal(|row| {
        row.heading("Models");
        if row.small_button("Refresh").on_hover_text("List the folder's model files again").clicked() {
            workspace.refresh();
        }
    });
    if workspace.models().is_empty() {
        ui.label("No model files in this folder yet.");
    }
    for entry in workspace.models() {
        let open = workspace.opened() == Some(entry.path.as_str());
        let label = if entry.problems > 0 { format!("⚠ {}  ({})", entry.path, entry.problems) } else { entry.path.clone() };
        let clicked = ui.selectable_label(open, label).on_hover_text(if entry.problems > 0 {
            format!("{} problem{}: it can't run until they are fixed", entry.problems, plural(entry.problems))
        } else {
            "Click to run it".to_string()
        });
        if clicked.clicked() && !open {
            chosen = Some(entry.path.clone());
        }
    }
    if workspace.changed_outside() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Changed outside crabsim").strong().color(egui::Color32::from_rgb(240, 200, 80)));
        ui.label(egui::RichText::new("It was written past crabsim, or crabsim never saved it, so neither you nor agents can change it through crabsim until it's adopted. It runs all the same.").weak());
        if ui.button("Adopt").on_hover_text("Accept the file as it is").clicked() {
            error = workspace.adopt().err();
        }
    }
    if let Some(report) = workspace.problems().cloned() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new(format!("{} can't run", report.path)).strong().color(egui::Color32::from_rgb(240, 120, 120)));
        for line in &report.summary {
            ui.label(egui::RichText::new(line).weak());
        }
        ui.add_space(4.0);
        ui.label(format!("{} problem{}:", report.problems.len(), plural(report.problems.len())));
        for problem in &report.problems {
            ui.label(format!("•  {problem}"));
        }
        if ui.small_button("Close").clicked() {
            workspace.dismiss_problems();
        }
    }
    (chosen, error)
}

/// What to send for a step back the view has decided on: the step itself, a question to the
/// session first, or nothing yet while the user is asked to confirm.
fn stepping_back(ui: &mut Ui, decided: StepBack) -> Option<Request> {
    match decided {
        StepBack::Now => Some(Request::Control(Control::StepBack)),
        StepBack::Locate => Some(Request::LocateStepBack),
        StepBack::Confirm(to) => {
            ui.pending = Some(GoingBack::Step(to));
            None
        }
    }
}

/// The run controls, shown time, going back to a time, speed and time mode. `jump_text` is what
/// has been typed into the field for going back. Returns what was asked for, if anything.
fn top_bar(
    ui: &mut egui::Ui,
    view: &ViewState,
    workspace: &Workspace,
    jump_text: &mut String,
    seed_text: &mut String,
    end_text: &mut String,
) -> Option<Request> {
    let mut request = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(workspace.opened().unwrap_or("No model open")).strong());
        ui.label("seed");
        let seed_field = ui
            .add_enabled(view.allows_seed(), egui::TextEdit::singleline(seed_text).desired_width(70.0))
            .on_disabled_hover_text("The seed can change only before Init or after Reset");
        let typed_seed = seed_text.trim().parse::<u64>().ok();
        if seed_field.lost_focus() {
            match typed_seed {
                Some(seed) if seed != workspace.seed() => request = Some(Request::Seed(seed)),
                _ => *seed_text = workspace.seed().to_string(),
            }
        }
        ui.separator();
        let mut button = |ui: &mut egui::Ui, control: Control, label: &str| {
            if ui.add_enabled(view.allows(control), egui::Button::new(label)).clicked() {
                request = Some(Request::Control(control));
            }
        };
        for (control, label) in
            [(Control::Init, "Init"), (Control::Start, "Start"), (Control::Halt, "Halt"), (Control::Reset, "Reset")]
        {
            button(ui, control, label);
        }
        ui.separator();
        button(ui, Control::StepBack, "Step back");
        button(ui, Control::StepForward, "Step forward");
        ui.separator();
        ui.label(egui::RichText::new(view.shown_time()).monospace().size(16.0));
        ui.label(match view.status() {
            SessionStatus::Advancing => "Running",
            SessionStatus::Halted => "Halted",
            SessionStatus::ReachedEnd => "Reached the end time",
            SessionStatus::Failed => "Run error",
        });
        ui.separator();

        ui.label("End at");
        let hint = view.end_time().map_or_else(|| "no end".to_string(), format_time);
        let field = ui.add(egui::TextEdit::singleline(end_text).desired_width(90.0).hint_text(hint));
        let end = view.end_time_target(end_text);
        let set = ui.add_enabled(end.is_ok(), egui::Button::new("Set")).on_disabled_hover_text(match end {
            Err(EndTimeProblem::NotATime) => "Type a time such as 8:00:00 or 30d",
            Err(EndTimeProblem::NotLater) => "Only a time later than the one shown",
            Ok(_) => "",
        });
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if let (true, Ok(end)) = (set.clicked() || entered, end) {
            request = Some(Request::EndTime(Some(end)));
            end_text.clear();
        }
        if view.end_time().is_some() && ui.button("Clear").on_hover_text("Run with no end time").clicked() {
            request = Some(Request::EndTime(None));
        }
        ui.separator();

        ui.label("Go back to");
        let field = ui.add(egui::TextEdit::singleline(jump_text).desired_width(90.0).hint_text(view.shown_time()));
        let target = view.jump_target(jump_text);
        let go = ui.add_enabled(target.is_ok(), egui::Button::new("Go")).on_disabled_hover_text(match target {
            Err(JumpProblem::NotInitialised) => "Init first",
            Err(JumpProblem::NotATime) => "Type a time such as 1:30, 02:00:00 or 1d 06:00:00",
            Err(JumpProblem::NotEarlier) => "Only an earlier time than the one shown",
            Ok(_) => "",
        });
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if let (true, Ok(to)) = (go.clicked() || entered, target) {
            request = Some(Request::GoBack(to));
            jump_text.clear();
        }
        ui.separator();

        let mut speed = view.speed();
        let slider = egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED)
            .logarithmic(true)
            .suffix("×")
            .max_decimals(1)
            .text("speed");
        if ui.add(slider).changed() {
            request = Some(Request::Speed(speed));
        }
        if let Some(achieved) = view.achieved_speed() {
            ui.colored_label(egui::Color32::from_rgb(240, 170, 60), format!("reaching {achieved:.0}×"))
                .on_hover_text("Playback can't keep up with the speed asked for");
        }
        ui.separator();

        for (mode, label, hint) in [
            (TimeMode::Continuous, "Continuous", "Shown time passes at the speed throughout"),
            (TimeMode::SKIPPING, "Skip gaps", "No gap between instants takes longer than a quarter of a second"),
        ] {
            if ui.selectable_label(view.time_mode() == mode, label).on_hover_text(hint).clicked() && view.time_mode() != mode {
                request = Some(Request::TimeMode(mode));
            }
        }
    });
    if !view.marks().is_empty() {
        timeline(ui, view);
    }
    request
}

/// Each intervention as a mark on a line from t=0 to shown time. Hovering over a mark says what
/// it was and when.
fn timeline(ui: &mut egui::Ui, view: &ViewState) {
    let shown = view.picture().shown();
    let end = view.marks().iter().map(|mark| mark.at).max().unwrap_or(shown).max(shown).as_millis().max(1);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let line = egui::Stroke::new(1.0, egui::Color32::from_gray(90));
    painter.line_segment([rect.left_center(), rect.right_center()], line);
    for (index, mark) in view.marks().iter().enumerate() {
        let x = rect.left() + rect.width() * (mark.at.as_millis() as f64 / end as f64) as f32;
        let tick = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(4.0, rect.height()));
        painter.rect_filled(tick, 1.0, egui::Color32::from_rgb(240, 200, 80));
        let hover = ui.interact(tick.expand2(egui::vec2(3.0, 0.0)), ui.id().with(("mark", index)), egui::Sense::hover());
        hover.on_hover_text(format!("{}  {}", format_time_exact(mark.at), view.describe(mark)));
    }
}

/// The last error the session reported. A run error says how to get going again.
fn error_banner(ui: &mut egui::Ui, error: &str, run_failed: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(egui::Color32::WHITE, error);
        if run_failed {
            ui.colored_label(
                egui::Color32::from_rgb(255, 210, 210),
                "Reset, step back or go back to an earlier time to continue.",
            );
        }
    });
}

/// Asks before going back past interventions, naming those it would drop. Returns the request
/// once confirmed.
fn confirm_going_back(ctx: &egui::Context, ui: &mut Ui) -> Option<Request> {
    let pending = ui.pending?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm going back")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        let (heading, button, to) = match pending {
            GoingBack::To(to) => (format!("Go back to {}?", format_time_exact(to)), "Go back", to),
            GoingBack::Step(to) => (format!("Step back to {}?", format_time_exact(to)), "Step back", to),
        };
        let lost: Vec<&Mark> = ui.view.marks().iter().filter(|mark| mark.at > to).collect();
        modal.heading(heading);
        modal.label(format!("It drops {} later intervention{}:", lost.len(), plural(lost.len())));
        for mark in lost {
            modal.label(format!("•  {}  {}", format_time_exact(mark.at), ui.view.describe(mark)));
        }
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if row.button(button).clicked() {
                answer = Some(true);
            }
            if row.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let answer = answer?;
    ui.pending = None;
    // Stepping back goes to the instant before the one on screen, which is `to`, so going back
    // there is the same step, to exactly the time the user was shown.
    answer.then_some(match pending {
        GoingBack::To(to) | GoingBack::Step(to) => Request::GoBack(to),
    })
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// What the view state says about the selected object, and the interventions it offers. Returns
/// the intervention asked for, if any. `process_time_text` is what has been typed for a new
/// process time.
fn inspector(ui: &mut egui::Ui, inspection: Option<&Inspection>, process_time_text: &mut String) -> Option<Intervention> {
    ui.heading("Inspector");
    let Some(inspection) = inspection else {
        ui.label("Click an object to inspect it.");
        return None;
    };
    let mut asked = None;
    ui.label(egui::RichText::new(&inspection.name).strong().size(16.0));
    ui.label(egui::RichText::new(format!("{:?}, id {}", inspection.kind, inspection.id)).weak());
    ui.add_space(4.0);
    egui::Grid::new("inspection").num_columns(2).spacing([12.0, 4.0]).show(ui, |grid| {
        grid.label("State");
        grid.label(inspection.state);
        grid.end_row();
        grid.label("Holds");
        grid.label(match inspection.contents.as_slice() {
            [] => "nothing".to_string(),
            mus => mus.iter().map(|mu| format!("MU {mu}")).collect::<Vec<_>>().join(", "),
        });
        grid.end_row();
        if let Some(remaining) = inspection.remaining {
            grid.label("Work left");
            grid.label(format_time_exact(remaining));
            grid.end_row();
        }
    });
    if let Some(blocked) = &inspection.blocked {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Why it's blocked").strong());
        ui.label(blocked);
    }
    if !inspection.stopped.is_empty() {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Why it's stopped").strong());
        for interruption in &inspection.stopped {
            ui.label(interruption);
        }
    }

    if let Some(offered) = inspection.interventions {
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Interventions").strong());
        ui.horizontal_wrapped(|row| {
            let (pause, pause_label) =
                if offered.paused { (Intervention::Resume, "Resume") } else { (Intervention::Pause, "Pause") };
            if row.button(pause_label).clicked() {
                asked = Some(pause);
            }
            let (force, force_label) = if offered.forced {
                (Intervention::EndForcedFailure, "End forced failure")
            } else {
                (Intervention::ForceFailure, "Force failure")
            };
            if row.button(force_label).clicked() {
                asked = Some(force);
            }
        });
        ui.horizontal(|row| {
            row.label("Process time");
            let hint = inspection.details.as_ref().and_then(|details| details.process_time).map(format_time_exact);
            let field = row.add(
                egui::TextEdit::singleline(process_time_text).desired_width(80.0).hint_text(hint.unwrap_or_default()),
            );
            let typed = parse_time(process_time_text).filter(|&time| time > SimTime::ZERO);
            let set = row
                .add_enabled(typed.is_some(), egui::Button::new("Set"))
                .on_hover_text("From the next MU on. The MU in work keeps its time.")
                .on_disabled_hover_text("Type a time longer than zero, such as 25 or 1:30");
            let entered = field.lost_focus() && row.input(|input| input.key_pressed(egui::Key::Enter));
            if let (true, Some(time)) = (set.clicked() || entered, typed) {
                asked = Some(Intervention::SetProcessTime(time));
                process_time_text.clear();
            }
        });
    }

    if let Some(details) = &inspection.details {
        ui.add_space(8.0);
        ui.separator();
        egui::Grid::new("details").num_columns(2).spacing([12.0, 4.0]).show(ui, |grid| {
            if let (Some(now), Some(configured)) = (details.process_time, details.configured_process_time) {
                grid.label("Process time");
                grid.label(format_time_exact(now));
                grid.end_row();
                grid.label("Configured");
                grid.label(format_time_exact(configured));
                grid.end_row();
            }
            grid.label("Waiting for it");
            grid.label(if details.waiting.is_empty() { "nothing".to_string() } else { details.waiting.join(", ") });
            grid.end_row();
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(format!("Statistics since {}", format_time(details.statistics_since))).strong());
        egui::Grid::new("statistics").num_columns(2).spacing([12.0, 2.0]).striped(true).show(ui, |grid| {
            for (label, value) in statistics_rows(details) {
                grid.label(label);
                grid.label(value);
                grid.end_row();
            }
        });
        ui.label(egui::RichText::new(format!("as of {}", format_time_exact(details.as_of))).weak().small());
    }
    asked
}

/// Does one of the user's controls, if who holds the open model is still as shown. Either way the
/// tab shows the state as it is now, and why nothing was done, if it wasn't.
fn act_on_editors(ui: &mut Ui, shown: &Editors, action: EditAction) {
    match ui.workspace.act(shown, action) {
        Ok(now) => {
            ui.editors = Some(Ok(now));
            ui.editors_note = None;
        }
        Err(ActRefusal::Changed { now, what }) => {
            ui.editors = Some(Ok(now));
            ui.editors_note = Some(format!("{what} Nothing was done: look again and decide."));
        }
        Err(ActRefusal::Refused(why)) => ui.editors_note = Some(why),
    }
    ui.editors_stamp = ui.workspace.editing_file().and_then(|file| stamp_of(&file));
}

/// Asks whether to move an object while an agent holds the model file, or the file changed
/// outside crabsim, saying why it was refused. `Some(true)` to move it anyway, adopting the file
/// first if it changed outside, `Some(false)` to put it back.
fn confirm_moving(ctx: &egui::Context, name: &str, refused: &Refusal) -> Option<bool> {
    let outside = matches!(refused, Refusal::ChangedOutside(_));
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm moving")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Move {name} anyway?"));
        modal.label(refused.to_string());
        modal.label(if outside {
            "Adopting it accepts the file as it is now, then saves the move."
        } else {
            "Moving it anyway makes this one change, and the agent keeps its right."
        });
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if row.button(if outside { "Adopt and move" } else { "Move anyway" }).clicked() {
                answer = Some(true);
            }
            if row.button("Put it back").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// Asks before taking the right to change the model from the agent that holds it. `Some(true)`
/// once confirmed, `Some(false)` once cancelled.
fn confirm_taking(ctx: &egui::Context, claim: &Claim) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm taking")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Take this model from {}?", claim.agent));
        modal.label(format!("{} ({}) is working on it, with the goal \"{}\": {}.", claim.agent, claim.client, claim.goal, claim.doing));
        modal.label("Its claim is paused while you hold the model, and comes back to it when you release it. If you change the model, it reads it again before its next change.");
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if row.button("Take it").clicked() {
                answer = Some(true);
            }
            if row.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// Who holds the open model, with an agent's goal and what it's doing, the other files someone
/// holds, and the blocked agents, with the user's controls. Returns the control used, if any.
fn editing_tab(ui: &mut egui::Ui, opened: Option<&str>, editors: Option<&Result<Editors, String>>, note: Option<&str>) -> Option<EditAction> {
    ui.heading("Editing");
    let mut asked = None;
    let editors = match editors {
        Some(Ok(editors)) => editors,
        Some(Err(error)) => {
            ui.label(egui::RichText::new(error).weak());
            return None;
        }
        None => {
            ui.label(egui::RichText::new("No models folder is open.").weak());
            return None;
        }
    };
    if let Some(note) = note {
        ui.colored_label(egui::Color32::from_rgb(240, 200, 80), note);
        ui.add_space(4.0);
    }
    let claim_lines = |ui: &mut egui::Ui, path: &str, claim: &Claim| {
        let paused = if claim.paused { ", paused while you hold it" } else { "" };
        ui.label(format!("{} ({}), since {}{paused}", claim.agent, claim.client, claim.since));
        ui.label(format!("Goal: {}", claim.goal));
        ui.label(format!("Doing: {}", claim.doing));
        ui.label(egui::RichText::new(format!("Last active {}", claim.last_active)).weak().small());
        // What it has done so far, newest first, closed until asked for.
        if claim.trail.len() > 1 {
            egui::CollapsingHeader::new(format!("Activities ({})", claim.trail.len())).id_salt(("trail", path)).show(ui, |trail| {
                for (doing, since) in &claim.trail {
                    trail.label(format!("{since}  {doing}"));
                }
            });
        }
    };
    match opened {
        None => {
            ui.label(egui::RichText::new("Open a model to see who may change it.").weak());
        }
        Some(path) => {
            let file = editors.file(path);
            ui.label(egui::RichText::new(path).strong());
            if let Some(since) = &file.user_since {
                ui.label(format!("You hold it, since {since}."));
                if ui.button("Release").on_hover_text("Let go of it, giving a paused claim back to its agent").clicked() {
                    asked = Some(EditAction::Release);
                }
            }
            match &file.claim {
                None if file.user_since.is_none() => {
                    ui.label("Nobody holds it.");
                    if ui.button("Take").on_hover_text("Hold this model until you release it or close the window").clicked() {
                        asked = Some(EditAction::Take);
                    }
                }
                None => {}
                Some(claim) => {
                    ui.add_space(4.0);
                    claim_lines(ui, path, claim);
                    ui.horizontal(|row| {
                        if !claim.paused && row.button("Take").on_hover_text("Take it now. The agent's claim is paused until you release it").clicked() {
                            asked = Some(EditAction::Take);
                        }
                        if row.button("Revoke").on_hover_text("End the agent's claim").clicked() {
                            asked = Some(EditAction::Revoke(path.to_string()));
                        }
                        if row.button("Block").on_hover_text("Block this agent session from changing models in the folder").clicked() {
                            asked = Some(EditAction::Block(claim.agent.clone()));
                        }
                    });
                }
            }
        }
    }
    let others = editors.others(opened.unwrap_or_default());
    if !others.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Other files").strong());
        for file in others {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(&file.path).strong());
            if file.user_since.is_some() {
                ui.label("You hold it.");
            }
            if let Some(claim) = &file.claim {
                claim_lines(ui, &file.path, claim);
                ui.horizontal(|row| {
                    if row.button("Revoke").on_hover_text("End the agent's claim").clicked() {
                        asked = Some(EditAction::Revoke(file.path.clone()));
                    }
                    if row.button("Block").on_hover_text("Block this agent session from changing models in the folder").clicked() {
                        asked = Some(EditAction::Block(claim.agent.clone()));
                    }
                });
            }
        }
    }
    if !editors.blocked.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Blocked").strong());
        for (agent, client) in &editors.blocked {
            ui.horizontal(|row| {
                row.label(format!("{agent} ({client})"));
                if row.small_button("Unblock").clicked() {
                    asked = Some(EditAction::Unblock(agent.clone()));
                }
            });
        }
    }
    asked
}

/// An attribute's name as people read it.
fn attribute_label(attribute: &str) -> &str {
    match attribute {
        "interval" => "Interval",
        "process_time" => "Process time",
        "failures" => "Failures",
        "starting_mus" => "Starting MUs",
        "capacity" => "Capacity",
        "dimensions" => "Dimensions",
        "description" => "Description",
        other => other,
    }
}

/// Every attribute of the selected instance, its value and where it comes from. Before Init or
/// after Reset each can be changed, saved to the model file, and each set on the instance can be
/// restored on its own. Returns the change asked for, and whether it takes over from an agent.
fn attributes_section(
    ui: &mut egui::Ui,
    attributes: &Result<Vec<AttributeView>, String>,
    forms: &mut HashMap<String, Form>,
    editable: bool,
    refused: Option<&(String, Refusal, Option<AttributeChange>)>,
) -> Option<(AttributeChange, bool)> {
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Attributes").strong());
    let attributes = match attributes {
        Ok(attributes) => attributes,
        Err(error) => {
            ui.label(egui::RichText::new(error).weak());
            return None;
        }
    };
    if !editable {
        ui.label(egui::RichText::new("Change them before Init or after Reset. During a run, use the interventions above.").weak().small());
    }
    let mut asked = None;
    for view in attributes {
        ui.add_space(4.0);
        let from = match &view.from {
            ValueFrom::Instance => "set here".to_string(),
            ValueFrom::Class(class) => format!("from class {class}"),
            ValueFrom::BuiltIn => "built in".to_string(),
            ValueFrom::NotSet => "not set".to_string(),
        };
        ui.horizontal(|row| {
            row.label(egui::RichText::new(attribute_label(&view.attribute)).strong());
            row.label(egui::RichText::new(&from).weak().small());
            if editable && view.from == ValueFrom::Instance {
                let restore = row.small_button("Restore").on_hover_text("Take this attribute alone back to its class's value");
                if restore.clicked() {
                    asked = Some((AttributeChange::Restore(view.attribute.clone()), false));
                }
            }
        });
        let Some(form) = forms.get_mut(&view.attribute) else { continue };
        if view.attribute == "failures" && view.value.is_none() && !editable {
            ui.label("Never fails");
            continue;
        }
        let mut commit = false;
        ui.add_enabled_ui(editable, |fields| {
            if let Some(kind) = form.repair {
                egui::ComboBox::from_id_salt(("repair", &view.attribute)).selected_text(kind.name()).show_ui(fields, |menu| {
                    for choice in RepairKind::ALL {
                        if menu.selectable_label(kind == choice, choice.name()).clicked() && kind != choice {
                            form.set_repair(choice);
                        }
                    }
                });
            }
            egui::Grid::new(("fields", &view.attribute)).num_columns(2).spacing([8.0, 2.0]).show(fields, |grid| {
                for part in form.parts.iter_mut().filter(|part| !part.label.is_empty() || form.repair.is_none()) {
                    grid.label(part.label);
                    let field = grid.add(egui::TextEdit::singleline(&mut part.text).desired_width(120.0));
                    commit |= field.lost_focus();
                    grid.end_row();
                }
            });
        });
        let waiting = !form.matches(view.value.as_ref());
        if editable && waiting {
            match form.value() {
                Ok(value) => {
                    let apply = ui.small_button("Apply").on_hover_text("Saves it to the model file, cutting its inheritance");
                    if commit || apply.clicked() {
                        asked = Some((AttributeChange::Set(view.attribute.clone(), value), false));
                    }
                }
                Err(problem) => {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 120), problem);
                }
            }
        }
        if let Some((_, refusal, again)) = refused.filter(|(attribute, _, _)| *attribute == view.attribute) {
            ui.colored_label(egui::Color32::from_rgb(240, 120, 120), refusal.to_string());
            match (refusal, again) {
                (Refusal::NotEditor(_), Some(change)) => {
                    if ui.small_button("Change anyway").on_hover_text("Make this one change while the agent keeps its right").clicked() {
                        asked = Some((change.clone(), true));
                    }
                }
                (Refusal::ChangedOutside(_), _) => {
                    ui.label(egui::RichText::new("Adopt the file in the Models panel first.").weak());
                }
                _ => {}
            }
        }
    }
    asked
}

/// An object's statistics as rows of the inspector: counts, and each state as a share of the
/// time measured.
fn statistics_rows(details: &Details) -> Vec<(&'static str, String)> {
    let statistics = &details.statistics;
    let measured = statistics.measured_time();
    let share = |time: SimTime| match measured.as_millis() {
        0 => "–".to_string(),
        total => format!("{:.1} %", time.as_millis() as f64 * 100.0 / total as f64),
    };
    let mut rows = vec![
        ("MUs in", statistics.mus_in.to_string()),
        ("MUs out", statistics.mus_out.to_string()),
        ("Working", share(statistics.time_busy)),
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
    if let Some(per_hour) = statistics.mus_per_hour(details.as_of) {
        rows.push(("MUs an hour", format!("{per_hour:.1}")));
    }
    if let Some(lead_time) = statistics.lead_time_mean() {
        rows.push(("Mean lead time", format_time_exact(lead_time)));
    }
    rows
}

/// Each object's name under it, and counts of MUs where they aren't drawn, `text_size` pixels
/// tall, clipped to the canvas.
fn labels(ctx: &egui::Context, view: &ViewState, camera: &ViewCamera, canvas: egui::Rect, text_size: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("labels"))).with_clip_rect(canvas);
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    for object in view.objects() {
        let (sin, cos) = object.rotation_degrees.to_radians().sin_cos();
        let below = (sin.abs() * object.length + cos.abs() * object.width) / 2.0;
        let floor = Point { x: object.centre.x, y: object.centre.y - below };
        let at = camera.to_screen(floor, size);
        let at = egui::pos2(canvas.min.x + at.x as f32, canvas.min.y + at.y as f32 + text_size / 4.0);
        let font = egui::FontId::proportional(text_size);
        painter.text(at, egui::Align2::CENTER_TOP, &object.name, font, egui::Color32::LIGHT_GRAY);
    }
    // Where MUs can't be followed, each holder shows how many it holds, in its middle.
    for (id, count) in view.counts(camera) {
        let at = camera.to_screen(view.objects()[id as usize].centre, size);
        let at = egui::pos2(canvas.min.x + at.x as f32, canvas.min.y + at.y as f32);
        let font = egui::FontId::proportional(text_size);
        painter.text(at, egui::Align2::CENTER_CENTER, count.to_string(), font, egui::Color32::from_gray(15));
    }
}

/// Draws a steady stream of frames while anything can change, and otherwise only on input, so a
/// halted window costs nothing. Asking for vsync didn't cap the frame rate on the owner's PC, so
/// the stream is capped here, and input still draws a frame at once.
fn pace_frames(ui: Res<Ui>, time: Res<Time<Real>>, mut winit: ResMut<WinitSettings>, mut streaming: Local<bool>) {
    let wanted = ui.view.wants_frames(time.elapsed());
    if wanted != *streaming {
        *streaming = wanted;
        *winit = if wanted {
            let frame = UpdateMode::reactive(FRAME_INTERVAL);
            WinitSettings { focused_mode: frame, unfocused_mode: frame }
        } else {
            WinitSettings::desktop_app()
        };
    }
}
