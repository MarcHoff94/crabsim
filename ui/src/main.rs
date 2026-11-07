//! crabsim's UI: a Bevy application with egui panels (ADR 0006). It runs the demo model in a live
//! session and draws it in 2D, as flat meshes under an orthographic 3D camera, the way a 3D view
//! will draw it too. What to draw comes from `crabsim_ui::view`, which is tested without a window.
//! UI stage 1 is described in docs/agents/handoff.md, "UI design".
//!
//! cargo run -p crabsim-ui --profile ui
//!
//! Drag with any mouse button to pan, and use the wheel to zoom.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    log::LogPlugin,
    prelude::*,
    window::PrimaryWindow,
    winit::{UpdateMode, WinitSettings},
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui, input::egui_wants_any_pointer_input};
use crabsim::{LiveSession, SessionStatus, SimTime, TimeMode, WallClock, demo};
use crabsim_ui::camera::Camera as ViewCamera;
use crabsim_ui::view::{
    Control, DEFAULT_SPEED, Details, Inspection, Intervention, JumpProblem, MAX_SPEED, MIN_SPEED, Mark, Point,
    Shade, StepBackLoss, ViewState, format_time, format_time_exact, label_size, parse_time,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

const SEED: u64 = 0;
const BACKGROUND: Color = Color::srgb(0.10, 0.11, 0.13);
const CONNECTION: Color = Color::srgba(0.65, 0.67, 0.75, 0.6);
const INSPECTOR_WIDTH: f32 = 280.0;
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
    let model = demo::model();
    let view = ViewState::new(&model);
    let session = LiveSession::new(move || model.build(SEED), WallClock::new());
    session.set_speed(DEFAULT_SPEED).expect("a positive speed");

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
        })
        .insert_resource(Session(Mutex::new(session)))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                drain_session,
                request_details,
                apply_shades,
                canvas_input.run_if(not(egui_wants_any_pointer_input)),
                place_camera,
                draw_connections,
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
}

/// A way of going back that waits for confirmation.
#[derive(Clone, Copy)]
enum GoingBack {
    To(SimTime),
    Step,
}

/// A press that moves less than this, in logical pixels, is a click rather than a drag.
const CLICK_SLOP: f32 = 4.0;
const SELECTION: Color = Color::srgb(1.0, 1.0, 1.0);

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

fn setup(
    mut commands: Commands,
    ui: Res<Ui>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
            (shade, materials.add(StandardMaterial { base_color: shade_colour(shade), unlit: true, ..default() }))
        })
        .collect();
    // Objects of the same size share a mesh.
    let mut sizes: HashMap<(u64, u64), Handle<Mesh>> = HashMap::new();
    let entities = ui
        .view
        .objects()
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
        .collect();
    let count = ui.view.objects().len();
    commands.insert_resource(Scene { entities, drawn: vec![Shade::Unknown; count], materials });
}

/// Reads every observation waiting, every frame: past a backlog the session stops releasing.
fn drain_session(session: Res<Session>, mut ui: ResMut<Ui>, time: Res<Time<Real>>) {
    let wall = time.elapsed();
    let session = session.0.lock().expect("the session lock is never poisoned");
    for observation in session.observations().try_iter() {
        ui.view.observe(&observation, wall);
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

fn canvas_input(
    mut ui: ResMut<Ui>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let Some(canvas) = ui.canvas else { return };
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    let ui = &mut *ui;
    let Some(camera) = ui.camera.as_mut() else { return };
    if buttons.just_pressed(MouseButton::Left) {
        ui.press = window.cursor_position().map(|at| (at, 0.0));
    }
    if let Some((_, travelled)) = ui.press.as_mut() {
        *travelled += motion.delta.length();
    }
    if buttons.just_released(MouseButton::Left) {
        if let Some((at, travelled)) = ui.press.take() {
            if travelled < CLICK_SLOP {
                let pointer = Point { x: (at.x - canvas.min.x) as f64, y: (at.y - canvas.min.y) as f64 };
                ui.view.select(ui.view.object_at(camera.to_world(pointer, size)));
            }
        }
    }
    let dragging = [MouseButton::Left, MouseButton::Right, MouseButton::Middle].iter().any(|&b| buttons.pressed(b));
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
            ui.view.observe(&crabsim::Observation::Error(error), wall);
        }
    }
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
    let mut request = None;
    let top = egui::Panel::top("top bar").show(&mut root, |panel| request = top_bar(panel, &ui.view, &mut ui.jump_text));
    let mut bottom_of_bars = top.response.rect.bottom();
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
    let mut intervention = None;
    let right = egui::Panel::right("inspector").exact_size(INSPECTOR_WIDTH).show(&mut root, |panel| {
        egui::ScrollArea::vertical().show(panel, |panel| {
            intervention = inspector(panel, inspection.as_ref(), &mut ui.process_time_text.1);
        });
    });
    let canvas = egui::Rect::from_min_max(
        egui::pos2(ctx.viewport_rect().left(), bottom_of_bars),
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
            Err(error) => ui.view.observe(&crabsim::Observation::Error(error), wall),
        }
    }

    // Going back past interventions drops them, so it asks first.
    let request = match request {
        Some(Request::GoBack(to)) if ui.view.lost_going_back(to) > 0 => {
            ui.pending = Some(GoingBack::To(to));
            None
        }
        Some(Request::Control(Control::StepBack)) if ui.view.lost_stepping_back().count() > 0 => {
            ui.pending = Some(GoingBack::Step);
            None
        }
        other => other,
    };
    let request = request.or_else(|| confirm_going_back(&ctx, ui));

    if let Some(request) = request {
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
        };
        ui.view.commanded(wall);
        if let Err(error) = sent {
            ui.view.observe(&crabsim::Observation::Error(error), wall);
        }
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
}

/// The run controls, shown time, going back to a time, speed and time mode. `jump_text` is what
/// has been typed into the field for going back. Returns what was asked for, if anything.
fn top_bar(ui: &mut egui::Ui, view: &ViewState, jump_text: &mut String) -> Option<Request> {
    let mut request = None;
    ui.horizontal_wrapped(|ui| {
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
            SessionStatus::Failed => "Run error",
        });
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
        let (heading, lost): (String, Vec<&Mark>) = match pending {
            GoingBack::To(to) => (
                format!("Go back to {}?", format_time_exact(to)),
                ui.view.marks().iter().filter(|mark| mark.at > to).collect(),
            ),
            GoingBack::Step => ("Step back?".to_string(), Vec::new()),
        };
        modal.heading(heading);
        match (pending, ui.view.lost_stepping_back()) {
            (GoingBack::To(_), _) => {
                modal.label(format!("It drops {} later intervention{}:", lost.len(), plural(lost.len())));
                for mark in lost {
                    modal.label(format!("•  {}  {}", format_time_exact(mark.at), ui.view.describe(mark)));
                }
            }
            (GoingBack::Step, StepBackLoss::Exactly(count)) => {
                modal.label(format!("It drops the {count} intervention{} after the instant it goes to.", plural(count)));
            }
            (GoingBack::Step, StepBackLoss::UpTo(count)) => {
                modal.label(format!(
                    "It drops the interventions after the instant it goes to: up to {count}, because the time of \
                     that instant isn't known here."
                ));
            }
        }
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if row.button("Go back").clicked() {
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
    answer.then_some(match pending {
        GoingBack::To(to) => Request::GoBack(to),
        GoingBack::Step => Request::Control(Control::StepBack),
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

/// Each object's name under it, `text_size` pixels tall, clipped to the canvas.
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
