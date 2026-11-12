//! crabsim's UI: a Bevy application with egui panels (ADR 0006). It runs a model in a live session
//! and draws it in 2D, as flat meshes under an orthographic 3D camera, the way a 3D view will draw
//! it too. What to draw comes from `crabsim_ui::view`, and what the window works on from
//! `crabsim_ui::workspace`, both tested without a window. UI stage 1 is described in
//! docs/agents/handoff.md, "UI design", and stage 2 in spec issue 63.
//!
//! cargo run -p crabsim-ui --profile ui -- --models <folder>
//!
//! The window lists the folder's model files and runs the one clicked. The repository's `models`
//! folder holds the demo model, `demo.json`. Without `--models` it opens `crabsim-models` in the
//! user's Documents, made on the first start with the demo in it.
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
    Change, Control, DEFAULT_SPEED, Deletion, Details, EndTimeProblem, GoingOn, Inspection, Intervention, Interval, JumpProblem, MAX_SPEED,
    MIN_PERIODS, MIN_SPEED, Mark, PERIOD_FAILURE_FACTOR, Point, MU_METRES, Shade, StepBack, ViewState, format_time, format_time_exact, label_size, parse_time, rgb,
    wall_time_roughly,
};
use crabsim_ui::keys::{Command, Direction, Key, Keys, PromptKind};
use crabsim_ui::paths::{Paths, Strategy, Weigh};
use crabsim_ui::attributes::{Form, RepairKind};
use crabsim_ui::editing::{ActRefusal, Claim, EditAction, Editors};
use crabsim_ui::help;
use crabsim_ui::workspace::{AttributeView, ClassEntry, ConnectionView, End, ModelChange, Refusal, ValueFrom, Workspace};
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
    /// It must not hold source code. Left out, `crabsim-models` in your Documents, made with the
    /// demo model in it the first time.
    #[arg(long)]
    models: Option<PathBuf>,
}

/// The models folder used when none is given.
const DEFAULT_MODELS_FOLDER: &str = "crabsim-models";

const BACKGROUND: Color = Color::srgb(0.10, 0.11, 0.13);
const CONNECTION: Color = Color::srgba(0.65, 0.67, 0.75, 0.6);
const INSPECTOR_WIDTH: f32 = 280.0;
const MODELS_WIDTH: f32 = 230.0;
/// The model's name in the top bar, so the run controls after it never move.
const NAME_WIDTH: f32 = 160.0;
/// The longest wait between frames while anything can change: about 60 frames a second.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

fn shade_colour(shade: Shade) -> Color {
    let [r, g, b] = rgb(shade);
    Color::srgb(r, g, b)
}

fn shade_colour32(shade: Shade) -> egui::Color32 {
    let [r, g, b] = rgb(shade).map(|channel| (channel * 255.0).round() as u8);
    egui::Color32::from_rgb(r, g, b)
}

const SHADES: [Shade; 7] =
    [Shade::Unknown, Shade::Idle, Shade::Busy, Shade::Blocked, Shade::Paused, Shade::Failed, Shade::ForcedFailure];

fn main() {
    let opts = Opts::parse();
    let workspace = match opts.models {
        Some(models) => Workspace::open(&models),
        None => match dirs::document_dir() {
            Some(documents) => Workspace::open_or_create(&documents.join(DEFAULT_MODELS_FOLDER)),
            None => {
                eprintln!("Your Documents folder can't be found, so give a models folder with --models.");
                std::process::exit(2);
            }
        },
    };
    let view = ViewState::new(workspace.description());
    let session = workspace.session(WallClock::new());
    session.set_speed(DEFAULT_SPEED).expect("a positive speed");
    let seed_text = workspace.seed().to_string();
    let period_text = format_time(workspace.measuring_period());

    App::new()
        .insert_resource(ClearColor(BACKGROUND))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "crabsim".into(),
                        // Narrower than this, the top bar's rows no longer fit.
                        resize_constraints: bevy::window::WindowResizeConstraints {
                            min_width: 1000.0,
                            min_height: 500.0,
                            ..default()
                        },
                        ..default()
                    }),
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
            long_trip: None,
            workspace,
            seed_text,
            period_text,
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
            keep_changes: false,
            confirm_reset: None,
            help_open: false,
            results_sort: SortBy::Model,
            confirm_building: None,
            new_model: None,
            duplicating: None,
            class_selected: None,
            placing: None,
            name_text: (None, String::new()),
            select_name: None,
            connecting: None,
            pointer_floor: None,
            boxing: None,
            description_text: (None, String::new()),
            asking: None,
            confirm_removal: None,
            class_forms: (None, HashMap::new()),
            several: (Vec::new(), Vec::new()),
            build_error: None,
            several_forms: HashMap::new(),
            validation: None,
            paths: Paths::default(),
            model_generation: 0,
            overlay: None,
            right_press: None,
            canvas_menu: None,
            over_egui: false,
            keys: Keys::default(),
            key_note: None,
            search: (Vec::new(), 0),
            steps_pending: (0, 0),
            keys_help: false,
            cursor: None,
            confirm_line: None,
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
    /// Going on to a later time that would take long animated, waiting for the user to choose
    /// racing there or animating anyway, with how long animating would take.
    long_trip: Option<(SimTime, Duration)>,
    /// The models folder and the model open.
    workspace: Workspace,
    /// What has been typed into the seed field.
    seed_text: String,
    /// What has been typed into the measuring period field.
    period_text: String,
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
    /// Who holds each model file, as last read, while the Agents tab shows it.
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
    /// Whether Reset saves the values changed during the run to the model file. Off by default,
    /// and not remembered between windows.
    keep_changes: bool,
    /// A Reset waiting for the user: to drop the changes it names, or to keep them anyway while an
    /// agent holds the file.
    confirm_reset: Option<ResetAsk>,
    /// Whether the help on the selected object's states is open.
    help_open: bool,
    /// How the Results tab sorts its Stations.
    results_sort: SortBy,
    /// A building action refused because an agent holds the file or it changed outside crabsim,
    /// waiting for the user to make it anyway or leave it, with why it was refused.
    confirm_building: Option<(Building, Refusal)>,
    /// The name typed for a new model file, while the New model dialog is open.
    new_model: Option<String>,
    /// The model file being duplicated and the name typed for the copy, while asking for it.
    duplicating: Option<(String, String)>,
    /// The class selected in the Classes panel, whose values the inspector shows instead of an
    /// object's, and those values as read.
    class_selected: Option<(String, Result<Vec<AttributeView>, String>)>,
    /// The class being dragged from the Classes panel to place an instance of it.
    placing: Option<String>,
    /// What has been typed into the inspector's name field, and for which object.
    name_text: (Option<u32>, String),
    /// An instance to select once the model it was just saved into is drawn, by name.
    select_name: Option<String>,
    /// The object a connection is being dragged from, by its connect handle.
    connecting: Option<u32>,
    /// Where the pointer is on the floor while it is over the canvas, for the handle and the
    /// connection being dragged.
    pointer_floor: Option<Point>,
    /// Where a rectangle to select with began on the floor, while it is being dragged.
    boxing: Option<Point>,
    /// What has been typed into the selected connection's description, and for which connection.
    description_text: (Option<(u32, u32)>, String),
    /// A name being asked for, such as for a new class, while its dialog is open.
    asking: Option<Asking>,
    /// A removal that takes more than it names, waiting for the user to confirm it.
    confirm_removal: Option<Removal>,
    /// The fields for the selected class's values, and for which class they were made. `None`
    /// makes them again from the values read, as after the class changed.
    class_forms: (Option<String>, HashMap<String, Form>),
    /// Why a building change was refused, to show once the frame's other commands are done.
    build_error: Option<String>,
    /// The objects selected together, and the values they share, each with a value only where
    /// all of them have the same one.
    several: (Vec<u32>, Vec<AttributeView>),
    /// The fields for the values several selected objects share.
    several_forms: HashMap<String, Form>,
    /// What Validate found in the open model, shown until closed.
    validation: Option<Result<Vec<crabsim_ui::workspace::Issue>, String>>,
    /// The Paths tab's views and routes.
    paths: Paths,
    /// Counts the models the window has used, so what was worked out for an earlier one is
    /// worked out again.
    model_generation: u64,
    /// The Paths tab's weights and strategies on the canvas, and what they were worked out for:
    /// the model, the weight, and a live figure's values.
    overlay: Option<Overlay>,
    /// Where the right button went down on the canvas and how far it has moved, so letting go
    /// where it went down opens the object's menu rather than ending a pan.
    right_press: Option<(Vec2, f32)>,
    /// The menu of the object right-clicked, and where.
    canvas_menu: Option<(u32, egui::Pos2)>,
    /// The pointer was over a panel or window last frame, so a scroll there isn't the canvas's.
    over_egui: bool,
    /// What has been typed that isn't a command yet: a count, a sequence, a prompt.
    keys: Keys,
    /// A word on what the last key did, such as why it did nothing, shown on the key line.
    key_note: Option<String>,
    /// The last search's matches, by id, and which of them is selected.
    search: (Vec<u32>, usize),
    /// Steps forward and back still to go, asked for with a count.
    steps_pending: (u32, u32),
    /// Whether the panel listing every key is open.
    keys_help: bool,
    /// The object the last key move landed on: in visual mode moves go from it.
    cursor: Option<u32>,
    /// A command line that removes something, waiting for the user to confirm it.
    confirm_line: Option<(Building, Vec<String>)>,
}

/// What the Paths tab's weights and strategies were worked out for: the model, by generation, the
/// weight, and a live figure's values.
type OverlayKey = (u64, Weigh, Option<Vec<Option<f64>>>);

/// The Paths tab's weights and strategies, by object id, and what they were worked out for.
struct Overlay {
    made_for: OverlayKey,
    weights: Vec<Option<f64>>,
    strategies: Vec<Option<Strategy>>,
}

/// A dialog asking for a name, and what the name is for.
#[derive(Clone, Debug)]
struct Asking {
    for_what: AskFor,
    typed: String,
}

/// What a name asked for is used for.
#[derive(Clone, Debug)]
enum AskFor {
    DeriveClass { parent: String },
    DuplicateClass { name: String },
    RenameClass { name: String },
    MakeClass { instance: String },
    MakeClassFromAll { names: Vec<String> },
}

impl AskFor {
    fn heading(&self) -> String {
        match self {
            AskFor::DeriveClass { parent } => format!("A class derived from {parent}"),
            AskFor::DuplicateClass { name } => format!("Duplicate {name}"),
            AskFor::RenameClass { name } => format!("Rename {name}"),
            AskFor::MakeClass { instance } => format!("Make a class from {instance}"),
            AskFor::MakeClassFromAll { names } => format!("Make a class from these {}", names.len()),
        }
    }

    fn explanation(&self) -> &'static str {
        match self {
            AskFor::DeriveClass { .. } => "The new class's name. It gives what its parent gives until you set its own values.",
            AskFor::DuplicateClass { .. } => "The copy's name. It has the same parent and values, and isn't linked to the original.",
            AskFor::RenameClass { .. } => "Its new name. Classes and instances of it follow it.",
            AskFor::MakeClassFromAll { .. } => {
                "The new class's name. It derives from the class they share and takes the values they all set alike, which they then follow."
            }
            AskFor::MakeClass { .. } => {
                "The new class's name. It derives from the instance's class and takes the values set on the instance, which then follows it and behaves as before."
            }
        }
    }

    fn building(&self, name: String) -> Building {
        match self {
            AskFor::DeriveClass { parent } => Building::DeriveClass { parent: parent.clone(), name },
            AskFor::DuplicateClass { name: from } => Building::DuplicateClass { name: from.clone(), to: name },
            AskFor::RenameClass { name: from } => Building::RenameClass { name: from.clone(), to: name },
            AskFor::MakeClass { instance } => Building::MakeClass { instance: instance.clone(), name },
            AskFor::MakeClassFromAll { names } => Building::MakeClassFromAll { names: names.clone(), name },
        }
    }
}

/// A removal that takes more with it, named before it goes ahead.
#[derive(Clone, Debug)]
enum Removal {
    /// An instance, with its connections as (from, to).
    Instance { name: String, connections: Vec<(String, String)> },
    /// Several instances, with their connections.
    Instances { names: Vec<String>, connections: Vec<(String, String)> },
    /// A class, with the classes deriving from it and the instances of any of them.
    Class { name: String, classes: Vec<String>, instances: Vec<String> },
}

/// A change to the model's structure, made with no run in progress, each one `edit_model` batch.
#[derive(Clone, Debug)]
enum Building {
    /// An instance of the class at this point on the floor.
    AddInstance { class: String, at: crabsim::Position },
    RenameInstance { name: String, to: String },
    RemoveInstance { name: String },
    RemoveInstances { names: Vec<String> },
    SetAttributeOnAll { names: Vec<String>, attribute: String, value: serde_json::Value },
    MakeClassFromAll { names: Vec<String>, name: String },
    Connect { from: String, to: String },
    Disconnect { from: String, to: String },
    /// A connection moved to another place among its object's exits or its successor's entrances.
    MoveConnection { from: String, to: String, end: End, place: u64 },
    DescribeConnection { from: String, to: String, description: String },
    DeriveClass { parent: String, name: String },
    DuplicateClass { name: String, to: String },
    RenameClass { name: String, to: String },
    /// A class, with everything depending on it.
    RemoveClass { name: String },
    MakeClass { instance: String, name: String },
    SetClassAttribute { class: String, attribute: String, value: serde_json::Value },
    RestoreClassAttribute { class: String, attribute: String },
    /// A line of commands from the `:` line, read with the selection and where to place with
    /// nothing selected.
    Line { text: String, selected: Option<String>, at: (f64, f64) },
}

impl Building {
    /// What it does, for a question or an error, such as "Placing a Station".
    fn what(&self) -> String {
        match self {
            Building::AddInstance { class, .. } => format!("Placing a {class}"),
            Building::RenameInstance { name, to } => format!("Renaming {name} to {to}"),
            Building::RemoveInstance { name } => format!("Removing {name}"),
            Building::RemoveInstances { names } => format!("Removing {} objects", names.len()),
            Building::SetAttributeOnAll { names, attribute, .. } => format!("Setting the {} of {} objects", attribute_label(attribute).to_lowercase(), names.len()),
            Building::MakeClassFromAll { name, .. } => format!("Making the class {name}"),
            Building::Connect { from, to } => format!("Connecting {from} to {to}"),
            Building::Disconnect { from, to } => format!("Removing the connection from {from} to {to}"),
            Building::MoveConnection { from, to, .. } => format!("Moving the connection from {from} to {to}"),
            Building::DescribeConnection { from, to, .. } => format!("Describing the connection from {from} to {to}"),
            Building::DeriveClass { name, .. } => format!("Adding the class {name}"),
            Building::DuplicateClass { name, .. } => format!("Duplicating the class {name}"),
            Building::RenameClass { name, to } => format!("Renaming the class {name} to {to}"),
            Building::RemoveClass { name } => format!("Removing the class {name}"),
            Building::MakeClass { name, .. } => format!("Making the class {name}"),
            Building::SetClassAttribute { class, attribute, .. } => format!("Setting {class}'s {}", attribute_label(attribute).to_lowercase()),
            Building::RestoreClassAttribute { class, attribute } => format!("Restoring {class}'s {}", attribute_label(attribute).to_lowercase()),
            Building::Line { .. } => "The command line".to_string(),
        }
    }

    /// Makes the change, giving the instance to select once the model is drawn again, if any.
    fn apply(&self, workspace: &mut Workspace, take_over: bool) -> Result<Option<String>, Refusal> {
        match self {
            Building::AddInstance { class, at } => workspace.add_instance(class, *at, take_over).map(Some),
            Building::RenameInstance { name, to } => workspace.rename_instance(name, to, take_over).map(|()| Some(to.clone())),
            Building::RemoveInstance { name } => workspace.remove_instance(name, take_over).map(|()| None),
            Building::RemoveInstances { names } => workspace.remove_instances(names, take_over).map(|()| None),
            Building::SetAttributeOnAll { names, attribute, value } => workspace.set_attribute_on_all(names, attribute, value.clone(), take_over).map(|()| None),
            Building::MakeClassFromAll { names, name } => workspace.make_class_from_all(names, name, take_over).map(|()| None),
            Building::Connect { from, to } => workspace.connect(from, to, take_over).map(|()| None),
            Building::Disconnect { from, to } => workspace.disconnect(from, to, take_over).map(|()| None),
            Building::MoveConnection { from, to, end, place } => workspace.move_connection(from, to, *end, *place, take_over).map(|()| None),
            Building::DescribeConnection { from, to, description } => {
                workspace.describe_connection(from, to, description, take_over).map(|()| None)
            }
            Building::DeriveClass { parent, name } => workspace.derive_class(parent, name, take_over).map(|()| None),
            Building::DuplicateClass { name, to } => workspace.duplicate_class(name, to, take_over).map(|()| None),
            Building::RenameClass { name, to } => workspace.rename_class(name, to, take_over).map(|()| None),
            Building::RemoveClass { name } => workspace.remove_class(name, take_over).map(|()| None),
            Building::MakeClass { instance, name } => workspace.make_class(instance, name, take_over).map(|()| Some(instance.clone())),
            Building::SetClassAttribute { class, attribute, value } => workspace.set_class_attribute(class, attribute, value.clone(), take_over).map(|()| None),
            Building::RestoreClassAttribute { class, attribute } => workspace.restore_class_attribute(class, attribute, take_over).map(|()| None),
            // The last instance placed is selected, so `p` again places beside it.
            Building::Line { text, selected, at } => workspace.run_commands(text, selected.as_deref(), *at, take_over).map(|placed| placed.last().cloned()),
        }
    }
}

/// What a Reset with changes asks before it goes ahead.
#[derive(Clone)]
enum ResetAsk {
    /// Keeping changes is off, so Reset drops these.
    Drop(Vec<Change>),
    /// Keeping them was refused, such as while an agent holds the file.
    Keep(Vec<Change>, Refusal),
}

/// An object dragged on the canvas, before Init or after Reset, and the others selected with it,
/// which move by as much.
#[derive(Clone)]
struct Moving {
    id: u32,
    /// Where the object's centre was when the drag began, to put it back if the move is refused.
    from: Point,
    /// The object's centre less the pointer, in metres, so the object keeps where it was gripped.
    grip: Point,
    /// Whether the pointer has gone far enough for a drag rather than a click.
    dragged: bool,
    /// The other objects selected, each with where its centre was when the drag began.
    others: Vec<(u32, Point)>,
}

impl Moving {
    /// Every object moving, with where its centre was when the drag began.
    fn all(&self) -> Vec<(u32, Point)> {
        std::iter::once((self.id, self.from)).chain(self.others.iter().copied()).collect()
    }
}

/// The tabs of the right-hand column.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Inspector,
    Results,
    Paths,
    Agents,
}

/// What the Results tab's Stations are sorted by, most first, or model order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Model,
    Busy,
    Blocked,
    Idle,
    Failed,
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

/// How close to a connect handle a press must be to grip it, in logical pixels.
const HANDLE_PIXELS: f64 = 9.0;
/// How close to a connection's line a click must be to select it, in logical pixels.
const CLICK_PIXELS: f64 = 6.0;
const HANDLE_COLOUR: Color = Color::srgb(0.45, 0.75, 1.0);
/// Objects matching a search.
const SEARCHED: Color = Color::srgb(0.95, 0.85, 0.3);
/// The highlighted route in the Paths tab.
const ROUTE: Color = Color::srgb(1.0, 0.62, 0.2);
/// A weighted object's preferred successors in the Paths tab's Strategies view.
const PREFERRED: Color = Color::srgb(0.55, 0.9, 0.55);

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
    keys: Res<ButtonInput<KeyCode>>,
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
    // A handle or a line is as easy to hit however close the view is.
    let within = |pixels: f64| pixels / camera.pixels_per_metre;
    ui.pointer_floor = window.cursor_position().filter(|at| canvas.contains(egui::pos2(at.x, at.y))).map(floor);
    // Ctrl held, or pressed or let go this frame, as a quick Ctrl+click can be.
    let control = [KeyCode::ControlLeft, KeyCode::ControlRight];
    let ctrl = keys.any_pressed(control) || keys.any_just_pressed(control) || keys.any_just_released(control);
    // Right-clicking an object, without moving, opens its menu. Moving pans instead.
    if buttons.just_pressed(MouseButton::Right) {
        ui.right_press = window.cursor_position().filter(|at| canvas.contains(egui::pos2(at.x, at.y))).map(|at| (at, 0.0));
    }
    if let Some((_, travelled)) = ui.right_press.as_mut() {
        *travelled += motion.delta.length();
    }
    if buttons.just_released(MouseButton::Right)
        && let Some((at, travelled)) = ui.right_press.take()
    {
        ui.canvas_menu = (travelled < CLICK_SLOP).then(|| ui.view.object_at(floor(at))).flatten().map(|id| (id, egui::pos2(at.x, at.y)));
    }
    if buttons.just_pressed(MouseButton::Left) && ui.confirm_move.is_none() {
        // Only a press on the canvas is the canvas's: one on a panel, which egui may not have
        // claimed yet when the pointer arrived and pressed within one frame, leaves it alone.
        ui.press = window.cursor_position().filter(|at| canvas.contains(egui::pos2(at.x, at.y))).map(|at| (at, 0.0));
        // Pressing on an object's connect handle starts a connection from it.
        ui.connecting = ui.press.filter(|_| ui.workspace.opened().is_some()).and_then(|(at, _)| ui.view.handle_at(floor(at), within(HANDLE_PIXELS)));
        // Pressing on empty canvas starts a rectangle to select with.
        ui.boxing = ui
            .press
            .filter(|_| ui.connecting.is_none())
            .map(|(at, _)| floor(at))
            .filter(|&at| ui.view.object_at(at).is_none() && ui.view.connection_at(at, within(CLICK_PIXELS)).is_none());
        // Before Init or after Reset, pressing on an object grips it, to move it.
        let movable = ui.view.reloads_now() && ui.workspace.opened().is_some() && ui.connecting.is_none() && !ctrl;
        ui.moving = ui.press.filter(|_| movable).and_then(|(at, _)| {
            let pointer = floor(at);
            let id = ui.view.object_at(pointer)?;
            let from = ui.view.objects()[id as usize].centre;
            // Gripping one of several selected moves them all.
            let selection = ui.view.selection();
            let others = if selection.len() > 1 && selection.contains(&id) {
                selection.iter().filter(|&&other| other != id).map(|&other| (other, ui.view.objects()[other as usize].centre)).collect()
            } else {
                Vec::new()
            };
            Some(Moving { id, from, grip: Point { x: from.x - pointer.x, y: from.y - pointer.y }, dragged: false, others })
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
        let (dx, dy) = (centre.x - moving.from.x, centre.y - moving.from.y);
        for (id, start) in moving.all() {
            let at = Point { x: start.x + dx, y: start.y + dy };
            ui.view.move_object(id, at);
            place_object(&scene, &mut transforms, id, at);
        }
        if moving.others.is_empty() {
            ui.view.select(Some(moving.id));
        }
    }
    // A connection dragged from a handle ends on the object let go over, if any.
    if !buttons.pressed(MouseButton::Left)
        && let Some(from) = ui.connecting.take()
    {
        ui.press = None;
        ui.boxing = None;
        let to = ui.pointer_floor.and_then(|at| ui.view.object_at(at)).filter(|&to| to != from);
        if let Some(to) = to {
            let names = |id: u32| ui.view.objects()[id as usize].name.clone();
            let building = Building::Connect { from: names(from), to: names(to) };
            ui.view.commanded(time.elapsed());
            build(ui, building, false);
        }
    }
    // A release over a panel never reaches the canvas, so a drag ends whenever the button is up.
    if !buttons.pressed(MouseButton::Left)
        && let Some(moving) = ui.moving.take().filter(|moving| moving.dragged)
    {
        ui.press = None;
        ui.view.commanded(time.elapsed());
        save_move(ui, &scene, &mut transforms, moving, false);
    }
    if buttons.just_released(MouseButton::Left)
        && let Some((at, travelled)) = ui.press.take()
    {
        let started = ui.boxing.take();
        if let (Some(start), Some(end)) = (started, ui.pointer_floor)
            && travelled >= CLICK_SLOP
        {
            // A rectangle dragged on empty canvas selects every object inside it.
            ui.view.select_within(start, end);
        } else if travelled < CLICK_SLOP && ctrl {
            // Ctrl+click adds an object to the selection, or takes it away.
            if let Some(id) = ui.view.object_at(floor(at)) {
                ui.view.toggle(id);
            }
        } else if travelled < CLICK_SLOP {
            let pointer = floor(at);
            match ui.view.object_at(pointer) {
                Some(id) => ui.view.select(Some(id)),
                // Off every object, a click near a connection's line selects the connection.
                None => match ui.view.connection_at(pointer, within(CLICK_PIXELS)) {
                    Some(connection) => ui.view.select_connection(Some(connection)),
                    None => ui.view.select(None),
                },
            }
        }
    }
    let Some(camera) = ui.camera.as_mut() else { return };
    // The left button selects with a rectangle on empty canvas, so panning is on the others.
    let dragging = [MouseButton::Right, MouseButton::Middle].iter().any(|&b| buttons.pressed(b));
    if dragging && motion.delta != Vec2::ZERO {
        camera.pan(Point { x: motion.delta.x as f64, y: motion.delta.y as f64 });
        ui.camera_moved = true;
    }
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    // Only a scroll over the canvas zooms it: one over a panel or a window on top scrolls that.
    if notches != 0.0
        && !ui.over_egui
        && let Some(cursor) = window.cursor_position().filter(|at| canvas.contains(egui::pos2(at.x, at.y)))
    {
        let pointer = Point { x: (cursor.x - canvas.min.x) as f64, y: (cursor.y - canvas.min.y) as f64 };
        camera.zoom_at(1.15f64.powf(notches as f64), pointer, size);
        ui.camera_moved = true;
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
    match moved(ui, moving.clone(), take_over) {
        Ok(()) => {}
        Err(refusal @ (Refusal::NotEditor(_) | Refusal::ChangedOutside(_))) => ui.confirm_move = Some((moving, refusal)),
        Err(refusal) => {
            for (id, from) in moving.all() {
                ui.view.move_object(id, from);
                place_object(scene, transforms, id, from);
            }
            let name = &ui.view.objects()[moving.id as usize].name;
            ui.view.show_error(format!("{name} wasn't moved: {refusal}"));
        }
    }
}

/// Saves the dragged object where the view now draws it.
fn moved(ui: &mut Ui, moving: Moving, take_over: bool) -> Result<(), Refusal> {
    let moves: Vec<(String, crabsim::Position)> = moving
        .all()
        .into_iter()
        .map(|(id, _)| {
            let object = &ui.view.objects()[id as usize];
            let z = ui.workspace.description().position(id).map_or(0.0, |position| position.z);
            (object.name.clone(), crabsim::Position { x: object.centre.x, y: object.centre.y, z })
        })
        .collect();
    ui.workspace.move_instances(&moves, take_over)
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
    for &id in ui.view.selection() {
        let outline = ui.view.objects()[id as usize].outline(0.15);
        let corner = |point: Point| Vec3::new(point.x as f32, point.y as f32, 0.1);
        gizmos.linestrip(outline.iter().chain(outline.first()).map(|&point| corner(point)), SELECTION);
    }
    if let (Some(start), Some(end)) = (ui.boxing, ui.pointer_floor) {
        let corners = [start, Point { x: end.x, y: start.y }, end, Point { x: start.x, y: end.y }, start];
        gizmos.linestrip(corners.map(|point| Vec3::new(point.x as f32, point.y as f32, 0.3)), HANDLE_COLOUR);
    }
    let at = |point: Point, z: f32| Vec3::new(point.x as f32, point.y as f32, z);
    if let Some((from, to)) = ui.view.selected_connection()
        && let Some(connection) = ui.view.connections().iter().find(|connection| connection.from == from && connection.to == to)
    {
        gizmos.arrow(at(connection.start, 0.1), at(connection.end, 0.1), SELECTION).with_tip_length(0.5);
    }
    // The visual selection's cursor, where the next move starts.
    if ui.keys.visual()
        && let Some(object) = ui.cursor.and_then(|id| ui.view.objects().get(id as usize))
    {
        let outline = object.outline(0.3);
        gizmos.linestrip(outline.iter().chain(outline.first()).map(|&point| Vec3::new(point.x as f32, point.y as f32, 0.25)), HANDLE_COLOUR);
    }
    // What a search found.
    for &id in &ui.search.0 {
        if let Some(object) = ui.view.objects().get(id as usize) {
            let outline = object.outline(0.35);
            gizmos.linestrip(outline.iter().chain(outline.first()).map(|&point| Vec3::new(point.x as f32, point.y as f32, 0.2)), SEARCHED);
        }
    }
    // The Paths tab's highlighted route, and each weighted object's preferred successors.
    let between = |from: u32, to: u32| ui.view.connections().iter().find(|connection| connection.from == from && connection.to == to).copied();
    if let Some(route) = ui.paths.highlighted() {
        for pair in route.windows(2) {
            if let Some(connection) = between(pair[0], pair[1]) {
                gizmos.arrow(at(connection.start, 0.15), at(connection.end, 0.15), ROUTE).with_tip_length(0.5);
            }
        }
        for &id in route {
            let outline = ui.view.objects()[id as usize].outline(0.25);
            gizmos.linestrip(outline.iter().chain(outline.first()).map(|&point| at(point, 0.15)), ROUTE);
        }
    }
    if ui.paths.strategies_on
        && let Some(Overlay { strategies, .. }) = &ui.overlay
    {
        for (from, strategy) in strategies.iter().enumerate() {
            let Some(strategy) = strategy.as_ref().filter(|strategy| strategy.weighted) else { continue };
            for &(to, place) in &strategy.places {
                if let (1, Some(connection)) = (place, between(from as u32, to)) {
                    // Three lines side by side read as one thick arrow.
                    for offset in [-0.04, 0.0, 0.04] {
                        let shift = |point: Point| Point { x: point.x + offset, y: point.y + offset };
                        gizmos.arrow(at(shift(connection.start), 0.12), at(shift(connection.end), 0.12), PREFERRED).with_tip_length(0.45);
                    }
                }
            }
        }
    }
    let Some(camera) = ui.camera else { return };
    let radius = (HANDLE_PIXELS / camera.pixels_per_metre) as f32 * 0.6;
    // The handle of the object under the pointer, or of the one a connection is dragged from.
    let hovered = ui.connecting.or_else(|| {
        let pointer = ui.pointer_floor?;
        ui.view.object_at(pointer).or_else(|| ui.view.handle_at(pointer, HANDLE_PIXELS / camera.pixels_per_metre))
    });
    if let Some(handle) = hovered.and_then(|id| ui.view.handle_of(id)) {
        gizmos.circle(Isometry3d::from_translation(at(handle, 0.2)), radius, HANDLE_COLOUR);
        if ui.connecting.is_some()
            && let Some(pointer) = ui.pointer_floor
        {
            gizmos.arrow(at(handle, 0.2), at(pointer, 0.2), HANDLE_COLOUR).with_tip_length(0.4);
        }
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
    // The keyboard (spec issue 99): every key is a command unless a widget has the keyboard or a
    // dialog is open. Keys are taken before anything is drawn, so egui's own handling of Tab,
    // Space or Enter never sees them, and its Ctrl zoom of the whole window is off: Ctrl + and -
    // zoom the canvas.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    let dialog_open = ui.asking.is_some()
        || ui.confirm_removal.is_some()
        || ui.confirm_building.is_some()
        || ui.new_model.is_some()
        || ui.duplicating.is_some()
        || ui.confirm_line.is_some();
    let keyboard_free = !dialog_open && ctx.memory(|memory| memory.focused().is_none());
    let key_request = if keyboard_free {
        let typed = read_keys(&ctx);
        // egui has already read Tab and the arrows as moves of the keyboard to another widget,
        // which happen as the widgets are drawn: they're the canvas's keys, so that's called off.
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        let commands: Vec<Command> = typed.into_iter().flat_map(|key| ui.keys.press(key)).collect();
        if !commands.is_empty() {
            ui.key_note = None;
        }
        let mut asked = None;
        for command in commands {
            asked = run_key(&ctx, ui, &session, command, wall).or(asked);
        }
        asked
    } else {
        None
    };
    // Steps asked for with a count go one a frame, each once the one before allows it.
    let key_request = key_request.or_else(|| pending_step(ui));
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
        let Ui { view, workspace, jump_text, seed_text, period_text, end_text, keep_changes, .. } = &mut *ui;
        request = top_bar(panel, view, workspace, jump_text, seed_text, period_text, end_text, keep_changes)
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
            let states = crabsim_model::State::of(help::built_in(ui.view.objects()[id as usize].kind));
            let drains = ui.workspace.drains();
            ui.forms = attributes
                .iter()
                .map(|view| (view.attribute.clone(), Form::for_attribute(&view.attribute, view.value.as_ref(), states, &drains)))
                .collect();
        }
        ui.attributes = Some((id, read));
        ui.refused = None;
    }
    let editable = ui.view.reloads_now();
    let mut intervention = None;
    let mut attribute_change = None;
    let mut editors_action = None;
    let mut results_chosen = None;
    let mut renamed = None;
    let mut several_asked = None;
    // The values shared by several selected objects, read again when the selection changes.
    let several: Vec<u32> = ui.view.selection().to_vec();
    if several.len() > 1 && ui.several.0 != several {
        let names: Vec<String> = several.iter().map(|&id| ui.view.objects()[id as usize].name.clone()).collect();
        let shared = shared_values(&ui.workspace, &names);
        let drains = ui.workspace.drains();
        ui.several_forms = shared.iter().map(|view| (view.attribute.clone(), Form::for_attribute(&view.attribute, view.value.as_ref(), &[], &drains))).collect();
        ui.several = (several.clone(), shared);
    }
    let mut class_asked_change = None;
    // The selected class's fields, made from its values whenever they were read again.
    if let Some((class, Ok(values))) = &ui.class_selected
        && ui.class_forms.0.as_deref() != Some(class.as_str())
    {
        let states = crabsim_model::State::of(ui.workspace.built_in_of(class).unwrap_or("Station"));
        let drains = ui.workspace.drains();
        ui.class_forms.1 = values
            .iter()
            .map(|view| (view.attribute.clone(), Form::for_attribute(&view.attribute, view.value.as_ref(), states, &drains)))
            .collect();
        ui.class_forms.0 = Some(class.clone());
    }
    let mut connection_asked = None;
    let mut instance_asked = None;
    // What has been typed for a description belongs to the connection it was typed for.
    let shown_connection = ui.view.selected_connection();
    if ui.description_text.0 != shown_connection {
        let text = shown_connection
            .and_then(|(from, to)| {
                let names = (ui.view.objects()[from as usize].name.clone(), ui.view.objects()[to as usize].name.clone());
                ui.workspace.connection(&names.0, &names.1)?.description
            })
            .unwrap_or_default();
        ui.description_text = (shown_connection, text);
    }
    if shown_connection.is_some() {
        ui.class_selected = None;
    }
    // What has been typed for a name belongs to the object it was typed for.
    if ui.name_text.0 != selected {
        let name = selected.map_or_else(String::new, |id| ui.view.objects()[id as usize].name.clone());
        ui.name_text = (selected, name);
    }
    if selected.is_some() {
        ui.class_selected = None;
    }
    let right = egui::Panel::right("inspector").exact_size(INSPECTOR_WIDTH).show(&mut root, |panel| {
        panel.horizontal(|row| {
            for (tab, label, hint) in [
                (Tab::Inspector, "Inspector", "The object clicked on the canvas: its state, statistics and attributes (t1)"),
                (Tab::Results, "Results", "The run so far: each Drain's throughput, every Station's shares of time, and the likely bottleneck (t2)"),
                (Tab::Paths, "Paths", "See the model's routes, weights and distribution strategies on the canvas. Values change in the inspector (t3)"),
                (Tab::Agents, "Agents", "The AI agents changing this folder's models, what they're doing, and your control of them (t4)"),
            ] {
                if row.selectable_label(ui.tab == tab, label).on_hover_text(hint).clicked() && ui.tab != tab {
                    ui.tab = tab;
                    ui.editors = None;
                    ui.editors_note = None;
                }
            }
        });
        panel.separator();
        egui::ScrollArea::vertical().show(panel, |panel| match ui.tab {
            Tab::Inspector => match (&ui.class_selected, selected, ui.view.selected_connection()) {
                (_, None, Some((from, to))) => {
                    let names = (ui.view.objects()[from as usize].name.clone(), ui.view.objects()[to as usize].name.clone());
                    match ui.workspace.connection(&names.0, &names.1) {
                        Some(view) => connection_asked = connection_view(panel, &view, &mut ui.description_text.1, editable),
                        None => {
                            panel.label("This connection isn't in the model file any more.");
                        }
                    }
                }
                (Some((class, values)), None, None) => {
                    let entry = ui.workspace.classes().into_iter().find(|entry| &entry.name == class);
                    class_asked_change = class_view(panel, class, values, entry.as_ref(), &mut ui.class_forms.1, editable).map(|change| (class.clone(), change));
                }
                _ if ui.view.selection().len() > 1 => {
                    let names: Vec<String> = ui.view.selection().iter().map(|&id| ui.view.objects()[id as usize].name.clone()).collect();
                    several_asked = several_view(panel, &names, &ui.several, &mut ui.several_forms, editable);
                }
                _ => {
                    intervention = inspector(panel, inspection.as_ref(), &mut ui.process_time_text.1, &mut ui.help_open);
                    if let Some(id) = selected {
                        renamed = name_field(panel, &ui.view.objects()[id as usize].name, &mut ui.name_text.1, editable);
                        instance_asked = instance_actions(panel, editable);
                    }
                    if let Some((_, attributes)) = &ui.attributes {
                        attribute_change = attributes_section(panel, attributes, &mut ui.forms, editable, ui.refused.as_ref());
                    }
                }
            },
            Tab::Results => {
                let results = ui.view.results().map(|results| (ui.view.results_time(&results), results));
                results_chosen = results_tab(panel, results.as_ref().map(|(at, results)| (at.as_str(), results)), &mut ui.results_sort);
            }
            Tab::Paths => paths_tab(panel, &mut ui.paths, ui.view.objects(), ui.view.live_figure("mus_in").is_some()),
            Tab::Agents => editors_action = agents_tab(panel, ui.workspace.opened(), ui.editors.as_ref(), ui.editors_note.as_deref()),
        });
    });
    // Full states are asked for while results are shown, or the Paths tab weighs the live run.
    let live_paths = ui.tab == Tab::Paths && matches!(ui.paths.weigh, Weigh::Figure(_));
    ui.view.set_results_open(ui.tab == Tab::Results || live_paths);
    find_paths(ui);
    if let Some(id) = results_chosen {
        ui.view.select(Some(id));
        ui.view.commanded(wall);
    }
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
            (true, Refusal::ChangedOutside(_)) => ui.workspace.adopt().map_err(Refusal::Invalid).and_then(|()| moved(ui, moving.clone(), false)),
            (true, _) => moved(ui, moving.clone(), true),
        };
        if !anyway || done.is_err() {
            // Back where they were: the window draws the objects afresh from the view.
            for (id, from) in moving.all() {
                ui.view.move_object(id, from);
            }
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
    if let (Some(to), Some(id)) = (renamed, selected) {
        let name = ui.view.objects()[id as usize].name.clone();
        ui.view.commanded(wall);
        build(ui, Building::RenameInstance { name, to }, false);
    }
    if let Some(asked) = several_asked {
        let names: Vec<String> = ui.view.selection().iter().map(|&id| ui.view.objects()[id as usize].name.clone()).collect();
        match asked {
            SeveralAsk::Remove => delete_selected(ui),
            SeveralAsk::MakeClass => ui.asking = Some(Asking { for_what: AskFor::MakeClassFromAll { names }, typed: String::new() }),
            SeveralAsk::Set(attribute, value) => {
                ui.several.0.clear();
                build(ui, Building::SetAttributeOnAll { names, attribute, value }, false);
            }
        }
        ui.view.commanded(wall);
    }
    if let Some((class, (change, take_over))) = class_asked_change {
        let building = match change {
            AttributeChange::Set(attribute, value) => Building::SetClassAttribute { class, attribute, value },
            AttributeChange::Restore(attribute) => Building::RestoreClassAttribute { class, attribute },
        };
        ui.view.commanded(wall);
        build(ui, building, take_over);
    }
    if let (Some(asked), Some((from, to))) = (connection_asked, shown_connection) {
        let (from, to) = (ui.view.objects()[from as usize].name.clone(), ui.view.objects()[to as usize].name.clone());
        let building = match asked {
            ConnectionAsk::Move(end, place) => Building::MoveConnection { from, to, end, place },
            ConnectionAsk::Describe(description) => Building::DescribeConnection { from, to, description },
            ConnectionAsk::Remove => Building::Disconnect { from, to },
        };
        ui.view.commanded(wall);
        build(ui, building, false);
    }
    match (instance_asked, selected) {
        (Some(InstanceAsk::Remove), Some(_)) => delete_selected(ui),
        (Some(InstanceAsk::MakeClass), Some(id)) => {
            let instance = ui.view.objects()[id as usize].name.clone();
            ui.asking = Some(Asking { for_what: AskFor::MakeClass { instance }, typed: String::new() });
        }
        _ => {}
    }
    // Delete removes what is selected, unless a field is being typed in or a dialog is open.
    let dialog_open = ui.asking.is_some() || ui.confirm_removal.is_some() || ui.confirm_building.is_some() || ui.new_model.is_some() || ui.duplicating.is_some();
    if !dialog_open && !ctx.text_edit_focused() && ctx.input(|input| input.key_pressed(egui::Key::Delete)) {
        delete_what_is_selected(ui);
        ui.view.commanded(wall);
    }
    let mut chosen = None;
    let mut panel_error = None;
    let mut models_asked = None;
    let mut class_asked = None;
    let mut left_edge = ctx.viewport_rect().left();
    let building = ui.view.allows_building();
    if ui.workspace.has_folder() {
        let left = egui::Panel::left("models").exact_size(MODELS_WIDTH).show(&mut root, |panel| {
            egui::ScrollArea::vertical().show(panel, |panel| {
                (chosen, panel_error) = models_panel(panel, &mut ui.workspace, building, &mut models_asked);
                if ui.workspace.opened().is_some() {
                    let selected_class = ui.class_selected.as_ref().map(|(class, _)| class.as_str());
                    class_asked = classes_panel(panel, &ui.workspace.classes(), selected_class, building);
                }
            });
        });
        left_edge = left.response.rect.right();
    }
    match models_asked {
        Some(ModelsAsk::New) => ui.new_model = Some(String::new()),
        Some(ModelsAsk::Duplicate(path)) => {
            let stem = path.strip_suffix(".json").unwrap_or(&path);
            ui.duplicating = Some((path.clone(), format!("{stem} copy")));
        }
        None => {}
    }
    match class_asked {
        Some(ClassAsk::Select(class)) => {
            let values = ui.workspace.class_attributes(&class);
            ui.class_selected = Some((class, values));
            ui.class_forms.0 = None;
            ui.view.select(None);
            ui.tab = Tab::Inspector;
            ui.view.commanded(wall);
        }
        Some(ClassAsk::Place(class)) => ui.placing = Some(class),
        Some(ClassAsk::Derive(parent)) => ui.asking = Some(Asking { for_what: AskFor::DeriveClass { parent }, typed: String::new() }),
        Some(ClassAsk::Duplicate(name)) => {
            let typed = format!("{name} copy");
            ui.asking = Some(Asking { for_what: AskFor::DuplicateClass { name }, typed });
        }
        Some(ClassAsk::Rename(name)) => {
            let typed = name.clone();
            ui.asking = Some(Asking { for_what: AskFor::RenameClass { name }, typed });
        }
        Some(ClassAsk::Remove(name)) => {
            remove_class_asking(ui, name);
            ui.view.commanded(wall);
        }
        None => {}
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
    place_dragged_class(&ctx, ui, canvas);
    if let Some(path) = ask_new_model(&ctx, ui) {
        match ui.workspace.new_model(&path) {
            Ok(path) => open_model(ui, &session, &path, wall),
            Err(refusal) => ui.view.show_error(format!("The new model wasn't made: {refusal}")),
        }
        ui.view.commanded(wall);
    }
    if let Some((from, to)) = ask_duplicate(&ctx, ui) {
        if let Err(refusal) = ui.workspace.duplicate_model(&from, &to) {
            ui.view.show_error(format!("{from} wasn't duplicated: {refusal}"));
        }
        ui.view.commanded(wall);
    }
    if let Some((for_what, name)) = ask_name(&ctx, ui) {
        ui.view.commanded(wall);
        build(ui, for_what.building(name), false);
    }
    if let Some(removal) = confirm_removal(&ctx, ui) {
        let building = match removal {
            Removal::Instance { name, .. } => Building::RemoveInstance { name },
            Removal::Instances { names, .. } => Building::RemoveInstances { names },
            Removal::Class { name, .. } => Building::RemoveClass { name },
        };
        ui.view.commanded(wall);
        build(ui, building, false);
    }
    if let Some((building, refusal)) = ui.confirm_building.clone()
        && let Some(anyway) = confirm_building(&ctx, &building, &refusal)
    {
        ui.confirm_building = None;
        if anyway {
            let outside = matches!(refusal, Refusal::ChangedOutside(_));
            match if outside { ui.workspace.adopt().map_err(Refusal::Invalid) } else { Ok(()) } {
                Ok(()) => build(ui, building, !outside),
                Err(refusal) => ui.view.show_error(format!("{} didn't happen: {refusal}", building.what())),
            }
        }
        ui.view.commanded(wall);
    }
    hover_tooltip(&ctx, ui, canvas);
    help_window(&ctx, ui, canvas);
    canvas_menu(&ctx, ui);
    if let Some((building, removing)) = ui.confirm_line.clone()
        && let Some(run) = confirm_line(&ctx, &removing)
    {
        ui.confirm_line = None;
        if run {
            build(ui, building, false);
        }
        ui.view.commanded(wall);
    }
    key_line(&ctx, ui, canvas);
    keys_help(&ctx, ui);
    ui.over_egui = ctx.is_pointer_over_egui();
    if let Some(issue) = validation_window(&ctx, ui) {
        // Clicking a problem or warning selects what it's about.
        if let Some(id) = issue.instance.filter(|&id| (id as usize) < ui.view.objects().len()) {
            ui.view.select(Some(id));
            ui.class_selected = None;
            ui.tab = Tab::Inspector;
        } else if let Some(class) = issue.class {
            let values = ui.workspace.class_attributes(&class);
            ui.class_selected = Some((class, values));
            ui.class_forms.0 = None;
            ui.view.select(None);
            ui.tab = Tab::Inspector;
        }
        ui.view.commanded(wall);
    }

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

    let request = request.or(key_request);
    // Going back past interventions drops them, so it asks first. Stepping back may first have to
    // ask the session where it goes, and decides once the answer is in.
    let request = match request {
        Some(Request::GoTo(to)) if ui.view.lost_going_back(to) > 0 => {
            ui.pending = Some(GoingBack::To(to));
            None
        }
        Some(Request::GoTo(to)) => match ui.view.going_on(to) {
            GoingOn::Now => Some(Request::GoTo(to)),
            GoingOn::Ask { wall } => {
                ui.long_trip = Some((to, wall));
                None
            }
        },
        Some(Request::Control(Control::StepBack)) => stepping_back(ui, ui.view.step_back()),
        other => other,
    };
    let request = request.or_else(|| {
        let located = ui.view.located_step_back()?;
        stepping_back(ui, located)
    });
    let request = request.or_else(|| confirm_going_back(&ctx, ui));
    let request = request.or_else(|| confirm_long_trip(&ctx, ui));
    // A raced trip ended, so the session animates again, as the view does already.
    let request = request.or_else(|| ui.view.trip_ended().then_some(Request::Animate(true)));

    // A model clicked replaces the run, so a run in progress is replaced only once confirmed.
    if let Some(path) = chosen {
        if ui.view.run_in_progress() {
            ui.replacing = Some(path);
        } else {
            open_model(ui, &session, &path, wall);
        }
    }
    if let Some(path) = confirm_replacing(&ctx, ui) {
        open_model(ui, &session, &path, wall);
    }
    // Reset with values changed during the run keeps them, saved to the model file, or names those
    // it drops first.
    let request = match request {
        Some(Request::Control(Control::Reset)) if !ui.view.changes().is_empty() => {
            let changes = ui.view.changes();
            if ui.keep_changes {
                keep_and_reset(ui, changes, false)
            } else {
                ui.confirm_reset = Some(ResetAsk::Drop(changes));
                None
            }
        }
        other => other,
    };
    let request = request.or_else(|| confirm_reset(&ctx, ui));
    let request = match request {
        Some(Request::Seed(seed)) => {
            ui.workspace.set_seed(seed);
            replace_session(ui, &session, false, wall);
            None
        }
        Some(Request::MeasuringPeriod(length)) => {
            ui.workspace.set_measuring_period(length);
            replace_session(ui, &session, false, wall);
            None
        }
        Some(Request::Validate) => {
            ui.validation = Some(ui.workspace.validate());
            None
        }
        // Reset uses a model that changed during the run, such as by keeping its changes: a new
        // session is at its default state.
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
            Request::GoTo(to) => session.go_to(to),
            Request::RaceTo(to) => {
                ui.view.race_to();
                session.set_animation(false).and_then(|()| session.go_to(to))
            }
            Request::Animate(on) => {
                ui.view.set_animate(on);
                session.set_animation(on)
            }
            // Shown time has to stand still for the answer to hold.
            Request::LocateStepBack => session.halt().and_then(|()| session.locate_step_back()),
            Request::EndTime(end) => {
                ui.view.set_end_time(end);
                session.set_end_time(end)
            }
            Request::Seed(_) | Request::MeasuringPeriod(_) | Request::Validate => unreachable!("handled above"),
        };
        ui.view.commanded(wall);
        match sent {
            Ok(()) if locating => ui.view.locating_step_back(),
            Ok(()) => {}
            Err(error) => ui.view.observe(&crabsim::SessionOutput::Error(error), wall),
        }
    }

    if !ui.view.has_model() {
        let hint = match (ui.workspace.has_folder(), ui.workspace.opened().is_some()) {
            (false, _) => "No models folder is open",
            (true, false) => "Choose a model from the list on the left",
            (true, true) => "Drag a class from the Classes panel onto the canvas to place one",
        };
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
        paths_labels(&ctx, ui, &camera, canvas, size as f32);
    }
    // A building change refused this frame, or from the canvas before it, says why last of all.
    if let Some(error) = ui.build_error.take() {
        ui.view.show_error(error);
    }
    Ok(())
}

/// What the top bar asks the session for.
enum Request {
    Control(Control),
    Speed(f64),
    TimeMode(TimeMode),
    /// Go to a time: back to an earlier one, or on to a later one.
    GoTo(SimTime),
    /// Go on to a later time as fast as possible, with animation off until there.
    RaceTo(SimTime),
    /// Animation on or off.
    Animate(bool),
    /// Ask the session where stepping back goes, before deciding whether to ask the user.
    LocateStepBack,
    /// Set or clear the end time.
    EndTime(Option<SimTime>),
    /// Run with this seed from now on, which means a new session.
    Seed(u64),
    /// Measure in periods this long from now on, which means a new session too.
    MeasuringPeriod(SimTime),
    /// Check the open model for problems and warnings.
    Validate,
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
    ui.model_generation += 1;
    if new_model {
        ui.paths = ui.paths.for_another_model();
    }
    let mut view = ViewState::with_settings_of(ui.workspace.description(), &ui.view);
    // The same model, reloaded or with another seed, keeps the object selected, found by its name.
    if !new_model {
        // Everything selected stays selected, found by name.
        let names: Vec<String> = ui.view.selection().iter().map(|&id| ui.view.objects()[id as usize].name.clone()).collect();
        for name in names {
            if let Some(id) = view.objects().iter().find(|object| object.name == name).map(|object| object.id) {
                view.toggle(id);
            }
        }
    }
    // A connection selected stays selected, found by the names of its ends.
    if !new_model && let Some((from, to)) = ui.view.selected_connection() {
        let names = (ui.view.objects()[from as usize].name.clone(), ui.view.objects()[to as usize].name.clone());
        let id = |name: &str| view.objects().iter().find(|object| object.name == name).map(|object| object.id);
        if let (Some(from), Some(to)) = (id(&names.0), id(&names.1)) {
            view.select_connection(Some((from, to)));
        }
    }
    // An instance just placed or renamed is selected, found by its name.
    if let Some(name) = ui.select_name.take() {
        view.select(view.objects().iter().find(|object| object.name == name).map(|object| object.id));
    }
    let fresh = ui.workspace.session(WallClock::new());
    let settled = fresh
        .set_speed(view.speed())
        .and_then(|()| fresh.set_time_mode(view.time_mode()))
        .and_then(|()| fresh.set_end_time(view.end_time()))
        .and_then(|()| fresh.set_animation(view.animate()));
    *session.0.lock().expect("the session lock is never poisoned") = fresh;
    ui.view = view;
    ui.pending = None;
    ui.attributes = None;
    ui.forms.clear();
    ui.refused = None;
    ui.seed_text = ui.workspace.seed().to_string();
    ui.period_text = format_time(ui.workspace.measuring_period());
    if new_model {
        ui.camera_moved = false;
        ui.camera = None;
        ui.class_selected = None;
    } else if let Some((class, _)) = ui.class_selected.take() {
        // The same model, changed: the class's values are read again, if it's still there.
        let values = ui.workspace.class_attributes(&class);
        ui.class_selected = values.is_ok().then_some((class, values));
        ui.class_forms.0 = None;
    }
    ui.rebuild_scene = true;
    ui.view.commanded(wall);
    if let Err(error) = settled {
        ui.view.observe(&crabsim::SessionOutput::Error(error), wall);
    }
}

/// Asks before a model chosen replaces a run in progress. Returns the model once confirmed.
fn keep_and_reset(ui: &mut Ui, changes: Vec<Change>, take_over: bool) -> Option<Request> {
    match ui.workspace.keep_changes(&changes, take_over) {
        Ok(()) => {
            // Noticing the saved file now lets Reset use it, rather than the model it began with.
            ui.workspace.check();
            Some(Request::Control(Control::Reset))
        }
        Err(refusal @ Refusal::NotEditor(_)) if !take_over => {
            ui.confirm_reset = Some(ResetAsk::Keep(changes, refusal));
            None
        }
        Err(refusal) => {
            ui.view.show_error(format!("The changes weren't kept, so nothing was reset: {refusal}"));
            None
        }
    }
}

/// Asks before a Reset that drops changes, naming each, or that keeps them while an agent holds
/// the file. Returns the Reset once confirmed.
fn confirm_reset(ctx: &egui::Context, ui: &mut Ui) -> Option<Request> {
    let asking = ui.confirm_reset.clone()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm reset")).show(ctx, |modal| {
        modal.set_max_width(460.0);
        let (heading, changes, go) = match &asking {
            ResetAsk::Drop(changes) => (format!("Reset drops {} change{}", changes.len(), plural(changes.len())), changes, "Reset"),
            ResetAsk::Keep(changes, _) => ("The changes can't be kept".to_string(), changes, "Keep anyway"),
        };
        modal.heading(heading);
        if let ResetAsk::Keep(_, refusal) = &asking {
            modal.label(refusal.to_string());
            modal.label("Keeping them anyway saves them while the agent keeps its right:");
        }
        for change in changes {
            modal.label(format!("• {}", change.describe()));
        }
        if matches!(asking, ResetAsk::Drop(_)) {
            modal.label(egui::RichText::new("Tick Keep changes beside Reset to save them to the model file instead.").weak());
        }
        modal.add_space(8.0);
        if let Some(chosen) = answers(modal, &[go, "Cancel"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let go = answer?;
    ui.confirm_reset = None;
    match (go, asking) {
        (false, _) => None,
        (true, ResetAsk::Drop(_)) => Some(Request::Control(Control::Reset)),
        (true, ResetAsk::Keep(changes, _)) => keep_and_reset(ui, changes, true),
    }
}

fn confirm_replacing(ctx: &egui::Context, ui: &mut Ui) -> Option<String> {
    let path = ui.replacing.clone()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm replacing")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Open {path}?"));
        modal.label("The run in progress ends, with everything it has done.");
        modal.add_space(8.0);
        if let Some(chosen) = answers(modal, &["Open", "Cancel"]) {
            answer = Some(chosen == 0);
        }
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
fn models_panel(ui: &mut egui::Ui, workspace: &mut Workspace, building: bool, asked: &mut Option<ModelsAsk>) -> (Option<String>, Option<String>) {
    let mut chosen = None;
    let mut error = None;
    ui.horizontal(|row| {
        row.heading("Models");
        if row.small_button("Refresh").on_hover_text("List the folder's model files again").clicked() {
            workspace.refresh();
        }
        let new = row
            .add_enabled(building, egui::Button::new("New model").small())
            .on_hover_text("Make an empty model file in this folder and open it, to build on")
            .on_disabled_hover_text(RESET_FIRST);
        if new.clicked() {
            *asked = Some(ModelsAsk::New);
        }
    });
    if workspace.models().is_empty() {
        ui.label("No model files in this folder yet.");
    }
    for entry in workspace.models() {
        let open = workspace.opened() == Some(entry.path.as_str());
        let label = if entry.problems > 0 { format!("⚠ {}  ({})", entry.path, entry.problems) } else { entry.path.clone() };
        let clicked = ui.selectable_label(open, label).on_hover_text(if entry.problems > 0 {
            format!("{} problem{}: it can't run until they are fixed. Right-click to duplicate it", entry.problems, plural(entry.problems))
        } else {
            "Click to run it, right-click to duplicate it".to_string()
        });
        if clicked.clicked() && !open {
            chosen = Some(entry.path.clone());
        }
        clicked.context_menu(|menu| {
            let duplicate = menu
                .add_enabled(building, egui::Button::new("Duplicate…"))
                .on_hover_text("Copy this model file under a new name, to try a variant")
                .on_disabled_hover_text(RESET_FIRST);
            if duplicate.clicked() {
                *asked = Some(ModelsAsk::Duplicate(entry.path.clone()));
                menu.close();
            }
        });
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

/// What the Models panel asks for besides opening a model.
enum ModelsAsk {
    New,
    /// Duplicating this model file.
    Duplicate(String),
}

/// What the Classes panel asks for.
enum ClassAsk {
    /// Showing the class's values in the inspector.
    Select(String),
    /// Placing an instance of the class where it is dropped on the canvas.
    Place(String),
    Derive(String),
    Duplicate(String),
    Rename(String),
    Remove(String),
}

/// What the selected connection's inspector asks for.
enum ConnectionAsk {
    /// Its place at one end, from 1.
    Move(End, u64),
    /// Its description, which an empty one removes.
    Describe(String),
    Remove,
}

/// What the inspector asks for with several objects selected.
enum SeveralAsk {
    Remove,
    MakeClass,
    /// An attribute's value, set on all of them.
    Set(String, serde_json::Value),
}

/// The attributes several instances share, which they have when they come from one built-in class:
/// each with a value only where all of them have the same one. None when their built-in classes
/// differ.
fn shared_values(workspace: &Workspace, names: &[String]) -> Vec<AttributeView> {
    let all: Vec<Vec<AttributeView>> = names.iter().filter_map(|name| workspace.attributes(name).ok()).collect();
    let Some(first) = all.first() else { return Vec::new() };
    if all.len() != names.len() || all.iter().any(|views| views.iter().map(|view| &view.attribute).ne(first.iter().map(|view| &view.attribute))) {
        return Vec::new();
    }
    first
        .iter()
        .enumerate()
        .map(|(place, view)| {
            let alike = all.iter().all(|views| views[place].value == view.value);
            AttributeView { attribute: view.attribute.clone(), value: if alike { view.value.clone() } else { None }, from: view.from.clone() }
        })
        .collect()
}

/// Several selected objects: their names, Remove, Make a class from them, and the values they
/// share, any of which can be set on all of them at once.
fn several_view(
    ui: &mut egui::Ui,
    names: &[String],
    several: &(Vec<u32>, Vec<AttributeView>),
    forms: &mut HashMap<String, Form>,
    editable: bool,
) -> Option<SeveralAsk> {
    let mut asked = None;
    ui.heading(format!("{} objects", names.len()));
    ui.add(egui::Label::new(egui::RichText::new(names.join(", ")).weak()).wrap());
    ui.label(egui::RichText::new("Ctrl+click adds or takes one away. Dragging one moves them all.").weak());
    if editable {
        ui.horizontal(|row| {
            if row.button("Make a class from them…").on_hover_text("A class derived from the class they share, with the values they all set alike, which they then follow").clicked() {
                asked = Some(SeveralAsk::MakeClass);
            }
            if row.button("Remove…").on_hover_text("Remove them all and their connections. Delete does the same").clicked() {
                asked = Some(SeveralAsk::Remove);
            }
        });
    }
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Values they share").strong());
    if several.1.is_empty() {
        ui.label(egui::RichText::new("They're of different kinds, so they share no values.").weak());
        return asked;
    }
    if !editable {
        ui.label("Values change before Init or after Reset.");
        return asked;
    }
    ui.label(egui::RichText::new("Empty where they differ. A value applied is set on every one of them.").weak());
    for view in &several.1 {
        let Some(form) = forms.get_mut(&view.attribute) else { continue };
        if view.attribute == "state_notes" {
            continue;
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new(attribute_label(&view.attribute)).strong());
        let mut commit = false;
        egui::Grid::new(("several", &view.attribute)).num_columns(2).spacing([8.0, 2.0]).show(ui, |grid| {
            for part in form.parts.iter_mut().filter(|part| !part.label.is_empty() || form.repair.is_none()) {
                grid.label(part.label);
                commit |= grid.add(egui::TextEdit::singleline(&mut part.text).desired_width(120.0)).lost_focus();
                grid.end_row();
            }
        });
        if !form.matches(view.value.as_ref()) {
            match form.value() {
                Ok(serde_json::Value::Null) => {
                    ui.label(egui::RichText::new("Never failing can't be set on several at once yet: restore each one's failures instead.").weak());
                }
                Ok(value) => {
                    if ui.small_button("Apply to all").clicked() || commit {
                        asked = Some(SeveralAsk::Set(view.attribute.clone(), value));
                    }
                }
                Err(problem) => {
                    ui.colored_label(egui::Color32::from_rgb(240, 120, 120), problem);
                }
            }
        }
    }
    asked
}

/// What the selected instance's inspector asks for besides its values.
enum InstanceAsk {
    Remove,
    MakeClass,
}

/// Why a building control is greyed out during a run.
const RESET_FIRST: &str = "Reset first to change the model";

/// The classes an instance can be placed from, as a tree by inheritance. Clicking one shows its
/// values in the inspector, and dragging one onto the canvas places an instance of it.
fn classes_panel(ui: &mut egui::Ui, classes: &[ClassEntry], selected: Option<&str>, building: bool) -> Option<ClassAsk> {
    let mut asked = None;
    ui.add_space(8.0);
    ui.separator();
    ui.heading("Classes");
    for entry in classes {
        ui.horizontal(|row| {
            row.add_space(14.0 * entry.depth as f32);
            let text = egui::RichText::new(&entry.name);
            let text = if entry.built_in { text.strong() } else { text };
            let text = if selected == Some(entry.name.as_str()) { text.background_color(row.visuals().selection.bg_fill) } else { text };
            let hint = if entry.built_in && building {
                "Drag it onto the canvas to place one, click to see and change this model's values for it, or right-click to derive a class of your own from it"
            } else if building {
                "Drag it onto the canvas to place one, click to see and change its values, or right-click for more"
            } else {
                "Click to see its values. Reset first to place one"
            };
            let label = row.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click_and_drag())).on_hover_text(hint);
            if building && label.drag_started() {
                asked = Some(ClassAsk::Place(entry.name.clone()));
            } else if label.clicked() {
                asked = Some(ClassAsk::Select(entry.name.clone()));
            }
            if label.hovered() && building {
                row.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            label.context_menu(|menu| {
                let mut item = |menu: &mut egui::Ui, text: &str, hint: &str, ask: ClassAsk| {
                    let clicked = menu.add_enabled(building, egui::Button::new(text)).on_hover_text(hint).on_disabled_hover_text(RESET_FIRST).clicked();
                    if clicked {
                        asked = Some(ask);
                        menu.close();
                    }
                };
                item(menu, "Derive…", "A new class derived from this one, giving what it gives", ClassAsk::Derive(entry.name.clone()));
                if !entry.built_in {
                    item(menu, "Duplicate…", "A copy of this class, with its parent and values, not linked to it", ClassAsk::Duplicate(entry.name.clone()));
                    item(menu, "Rename…", "Give this class another name, which its classes and instances follow", ClassAsk::Rename(entry.name.clone()));
                    item(menu, "Remove…", "Remove this class, naming first what depends on it", ClassAsk::Remove(entry.name.clone()));
                }
            });
        });
    }
    asked
}

/// A class's values in the inspector, each with where it comes from, read-only.
fn class_view(
    ui: &mut egui::Ui,
    class: &str,
    values: &Result<Vec<AttributeView>, String>,
    entry: Option<&ClassEntry>,
    forms: &mut HashMap<String, Form>,
    editable: bool,
) -> Option<(AttributeChange, bool)> {
    ui.heading("Class");
    ui.label(egui::RichText::new(class).strong().size(16.0));
    if let Some(parent) = entry.and_then(|entry| entry.parent.as_deref()) {
        ui.label(egui::RichText::new(format!("Derived from {parent}")).weak());
    }
    let reach = if entry.is_some_and(|entry| entry.built_in) {
        "This model's values for every class and instance built on it, unless they set their own. Restore takes one back to crabsim's default. Drag it onto the canvas to place one."
    } else {
        "Its values reach every instance of it that doesn't set its own. Drag it onto the canvas to place one."
    };
    ui.label(egui::RichText::new(reach).weak());
    attributes_section(ui, values, forms, editable, None)
}

/// The selected instance's name, which can be changed with no run in progress. Returns a new name
/// once typed and confirmed with Enter or by leaving the field.
fn name_field(ui: &mut egui::Ui, name: &str, text: &mut String, editable: bool) -> Option<String> {
    if !editable {
        return None;
    }
    let mut renamed = None;
    ui.add_space(4.0);
    ui.horizontal(|row| {
        row.label("Name").on_hover_text("The instance's name in the model file. Its connections keep it.");
        let field = row.add(egui::TextEdit::singleline(text).desired_width(170.0));
        focus_if_wanted(row, &field, "name");
        if field.lost_focus() {
            let typed = text.trim().to_string();
            if !typed.is_empty() && typed != name {
                renamed = Some(typed);
            } else {
                *text = name.to_string();
            }
        }
    });
    renamed
}

/// The selected connection: its ends, its place among its object's exits and among its
/// successor's entrances, each movable up or down, its description and Remove. Everything but
/// looking waits for no run in progress.
fn connection_view(ui: &mut egui::Ui, view: &ConnectionView, description: &mut String, editable: bool) -> Option<ConnectionAsk> {
    let mut asked = None;
    ui.heading("Connection");
    ui.label(egui::RichText::new(format!("{} to {}", view.from, view.to)).strong().size(16.0));
    ui.label(egui::RichText::new("MUs pass along it at once, taking no time.").weak());
    ui.add_space(6.0);
    let mut place = |ui: &mut egui::Ui, text: String, hint: &str, end: End, at: u64, of: u64| {
        ui.horizontal(|row| {
            row.label(text).on_hover_text(hint);
            let up = row.add_enabled(editable && at > 1, egui::Button::new("Up").small()).on_hover_text("One place earlier").on_disabled_hover_text(if editable { "Already first" } else { RESET_FIRST });
            if up.clicked() {
                asked = Some(ConnectionAsk::Move(end, at - 1));
            }
            let down = row.add_enabled(editable && at < of, egui::Button::new("Down").small()).on_hover_text("One place later").on_disabled_hover_text(if editable { "Already last" } else { RESET_FIRST });
            if down.clicked() {
                asked = Some(ConnectionAsk::Move(end, at + 1));
            }
        });
    };
    place(ui, view.exit_text(), "Its exit: the order its object offers MUs to its successors, round-robin", End::Exit, view.exit, view.exits);
    place(
        ui,
        view.entrance_text(),
        "Its entrance: its place among the connections into its successor. Nothing reads it yet, so it doesn't change results until an entrance rule does",
        End::Entrance,
        view.entrance,
        view.entrances,
    );
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Description").strong());
    let field = ui.add_enabled(editable, egui::TextEdit::singleline(description).hint_text("such as via the lift").desired_width(240.0)).on_disabled_hover_text(RESET_FIRST);
    focus_if_wanted(ui, &field, "description");
    if field.lost_focus() && description.trim() != view.description.as_deref().unwrap_or_default() {
        asked = Some(ConnectionAsk::Describe(description.trim().to_string()));
    }
    ui.add_space(8.0);
    let remove = ui.add_enabled(editable, egui::Button::new("Remove")).on_hover_text("Remove this connection. Delete does the same").on_disabled_hover_text(RESET_FIRST);
    if remove.clicked() {
        asked = Some(ConnectionAsk::Remove);
    }
    asked
}

/// The selected instance's building actions, with no run in progress.
fn instance_actions(ui: &mut egui::Ui, editable: bool) -> Option<InstanceAsk> {
    if !editable {
        return None;
    }
    let mut asked = None;
    ui.horizontal(|row| {
        if row.button("Make a class from this…").on_hover_text("A class derived from its class, with the values set on it, which it then follows").clicked() {
            asked = Some(InstanceAsk::MakeClass);
        }
        if row.button("Remove…").on_hover_text("Remove it and its connections. Delete does the same").clicked() {
            asked = Some(InstanceAsk::Remove);
        }
    });
    asked
}

/// Removes a class of the model's own, first naming what depends on it if anything does.
fn remove_class_asking(ui: &mut Ui, name: String) {
    let dependents = ui.workspace.class_dependents(&name);
    if dependents.is_empty() {
        build(ui, Building::RemoveClass { name }, false);
    } else {
        ui.confirm_removal = Some(Removal::Class { name, classes: dependents.classes, instances: dependents.instances });
    }
}

/// Removes what is selected, as Delete does: a connection at once, or an instance, first naming
/// the connections going with it.
fn delete_selected(ui: &mut Ui) {
    match ui.view.deletion() {
        Some(Deletion::Connection { from, to }) => build(ui, Building::Disconnect { from, to }, false),
        Some(deletion @ Deletion::Instance { .. }) if deletion.asks_first() => {
            if let Deletion::Instance { name, connections } = deletion {
                ui.confirm_removal = Some(Removal::Instance { name, connections });
            }
        }
        Some(Deletion::Instance { name, .. }) => build(ui, Building::RemoveInstance { name }, false),
        Some(Deletion::Instances { names, connections }) if !connections.is_empty() => {
            ui.confirm_removal = Some(Removal::Instances { names, connections });
        }
        Some(Deletion::Instances { names, .. }) => build(ui, Building::RemoveInstances { names }, false),
        None => {}
    }
}

/// Asks for a name while its dialog is open. Returns what it is for and the name once confirmed.
fn ask_name(ctx: &egui::Context, ui: &mut Ui) -> Option<(AskFor, String)> {
    let asking = ui.asking.as_mut()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("ask a name")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(asking.for_what.heading());
        modal.label(asking.for_what.explanation());
        let field = modal.add(egui::TextEdit::singleline(&mut asking.typed).desired_width(300.0));
        let entered = field.lost_focus() && modal.input(|input| input.key_pressed(egui::Key::Enter));
        if !entered && modal.memory(|memory| memory.focused().is_none()) {
            field.request_focus();
        }
        let typed = asking.typed.trim().to_string();
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if (row.add_enabled(!typed.is_empty(), egui::Button::new("OK")).clicked() || entered) && !typed.is_empty() {
                answer = Some(Some((asking.for_what.clone(), typed.clone())));
            }
            if row.button("Cancel").clicked() {
                answer = Some(None);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(None);
    }
    let answer = answer?;
    ui.asking = None;
    answer
}

/// Asks before a removal that takes more than it names, naming everything that goes. Returns the
/// removal once confirmed.
fn confirm_removal(ctx: &egui::Context, ui: &mut Ui) -> Option<Removal> {
    let removal = ui.confirm_removal.clone()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm removal")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        match &removal {
            Removal::Instance { name, connections } => {
                modal.heading(format!("Remove {name}?"));
                modal.label(if connections.len() == 1 { "Its connection goes with it:".to_string() } else { format!("Its {} connections go with it:", connections.len()) });
                for (from, to) in connections {
                    modal.label(format!("•  {from} to {to}"));
                }
            }
            Removal::Instances { names, connections } => {
                modal.heading(format!("Remove these {} objects?", names.len()));
                modal.label(names.join(", "));
                modal.label(if connections.len() == 1 { "Their connection goes with them:".to_string() } else { format!("Their {} connections go with them:", connections.len()) });
                for (from, to) in connections {
                    modal.label(format!("•  {from} to {to}"));
                }
            }
            Removal::Class { name, classes, instances } => {
                modal.heading(format!("Remove the class {name}?"));
                if !classes.is_empty() {
                    modal.label(format!("The class{} deriving from it go{} with it:", if classes.len() == 1 { "" } else { "es" }, if classes.len() == 1 { "es" } else { "" }));
                    for class in classes {
                        modal.label(format!("•  {class}"));
                    }
                }
                if !instances.is_empty() {
                    modal.label(format!("So do these instance{}, with their connections:", plural(instances.len())));
                    for instance in instances {
                        modal.label(format!("•  {instance}"));
                    }
                }
            }
        }
        modal.add_space(8.0);
        if let Some(chosen) = answers(modal, &["Remove", "Cancel"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let answer = answer?;
    ui.confirm_removal = None;
    answer.then_some(removal)
}

/// Makes a building change. While an agent holds the file, or when it changed outside crabsim, it
/// asks whether to make it anyway, and any other refusal says why in the banner.
fn build(ui: &mut Ui, building: Building, take_over: bool) {
    match building.apply(&mut ui.workspace, take_over) {
        // The window picks up the saved file next frame and draws the model again.
        Ok(select) => {
            ui.select_name = select;
            // A class renamed or added stays, or becomes, the one shown.
            let shown = match &building {
                Building::RenameClass { to, .. } | Building::DuplicateClass { to, .. } => Some(to.clone()),
                Building::DeriveClass { name, .. } => Some(name.clone()),
                _ => None,
            };
            if let Some(class) = shown {
                ui.class_selected = Some((class, Err(String::new())));
                ui.view.select(None);
            }
        }
        Err(refusal @ (Refusal::NotEditor(_) | Refusal::ChangedOutside(_))) => ui.confirm_building = Some((building, refusal)),
        // Shown at the end of the frame, after whatever else this frame clears the banner for.
        Err(refusal) => ui.build_error = Some(format!("{} didn't happen: {refusal}", building.what())),
    }
}

/// Asks whether to make a refused building change anyway. `Some(true)` to make it, `Some(false)`
/// to leave it.
fn confirm_building(ctx: &egui::Context, building: &Building, refused: &Refusal) -> Option<bool> {
    let outside = matches!(refused, Refusal::ChangedOutside(_));
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm building")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("{} anyway?", building.what()));
        modal.label(refused.to_string());
        modal.label(if outside {
            "Adopting it accepts the file as it is now, then makes the change."
        } else {
            "Changing it anyway makes this one change, and the agent keeps its right."
        });
        modal.add_space(8.0);
        if let Some(chosen) = answers(modal, &[if outside { "Adopt and change" } else { "Change anyway" }, "Leave it"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// While a class is dragged from the Classes panel, shows its name at the pointer, and places an
/// instance of it where it is let go over the canvas. Let go anywhere else, nothing happens.
fn place_dragged_class(ctx: &egui::Context, ui: &mut Ui, canvas: egui::Rect) {
    let Some(class) = ui.placing.clone() else { return };
    let pointer = ctx.input(|input| input.pointer.latest_pos());
    if let Some(at) = pointer {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("placing"))).text(
            at + egui::vec2(14.0, 4.0),
            egui::Align2::LEFT_CENTER,
            format!("+ {class}"),
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
    }
    if !ctx.input(|input| input.pointer.any_released()) {
        return;
    }
    ui.placing = None;
    let (Some(at), Some(camera)) = (pointer, ui.camera) else { return };
    if !canvas.contains(at) {
        return;
    }
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    let floor = camera.to_world(Point { x: (at.x - canvas.min.x) as f64, y: (at.y - canvas.min.y) as f64 }, size);
    // The view stays where it is, so the next drop lands where it's aimed.
    ui.camera_moved = true;
    build(ui, Building::AddInstance { class, at: crabsim::Position { x: floor.x, y: floor.y, z: 0.0 } }, false);
}

/// Asks for a new model file's name while the dialog is open. Returns the name once confirmed.
fn ask_new_model(ctx: &egui::Context, ui: &mut Ui) -> Option<String> {
    let typed = ui.new_model.as_mut()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("new model")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading("New model");
        modal.label("A file name in this models folder, such as press line or plant/press line.");
        let field = modal.add(egui::TextEdit::singleline(typed).hint_text("press line").desired_width(300.0));
        // Enter takes the focus away, so the field is given it only when nothing has it yet.
        let entered = field.lost_focus() && modal.input(|input| input.key_pressed(egui::Key::Enter));
        if !entered && modal.memory(|memory| memory.focused().is_none()) {
            field.request_focus();
        }
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if (row.add_enabled(!typed.trim().is_empty(), egui::Button::new("Make it")).clicked() || entered) && !typed.trim().is_empty() {
                answer = Some(Some(typed.trim().to_string()));
            }
            if row.button("Cancel").clicked() {
                answer = Some(None);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(None);
    }
    let answer = answer?;
    ui.new_model = None;
    answer
}

/// Asks for the copy's name while duplicating a model file. Returns the file and the copy's name
/// once confirmed.
fn ask_duplicate(ctx: &egui::Context, ui: &mut Ui) -> Option<(String, String)> {
    let (from, typed) = ui.duplicating.as_mut()?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("duplicate model")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Duplicate {from}"));
        modal.label("The copy's file name in this models folder.");
        let field = modal.add(egui::TextEdit::singleline(typed).desired_width(300.0));
        let entered = field.lost_focus() && modal.input(|input| input.key_pressed(egui::Key::Enter));
        if !entered && modal.memory(|memory| memory.focused().is_none()) {
            field.request_focus();
        }
        modal.add_space(8.0);
        modal.horizontal(|row| {
            if (row.add_enabled(!typed.trim().is_empty(), egui::Button::new("Duplicate")).clicked() || entered) && !typed.trim().is_empty() {
                answer = Some(Some((from.clone(), typed.trim().to_string())));
            }
            if row.button("Cancel").clicked() {
                answer = Some(None);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(None);
    }
    let answer = answer?;
    ui.duplicating = None;
    answer
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
#[allow(clippy::too_many_arguments)]
fn top_bar(
    ui: &mut egui::Ui,
    view: &ViewState,
    workspace: &Workspace,
    jump_text: &mut String,
    seed_text: &mut String,
    period_text: &mut String,
    end_text: &mut String,
    keep_changes: &mut bool,
) -> Option<Request> {
    let mut request = None;
    // Two rows, each narrower than the window's minimum width, so neither wraps or loses a
    // control off its end: running the model first, then where and how fast it goes.
    ui.horizontal(|ui| {
        // A fixed width, so the controls after it stay where they are when a model opens. A
        // longer name is cut short, with all of it on hover.
        let name = egui::Label::new(egui::RichText::new(workspace.opened().unwrap_or("No model open")).strong()).truncate();
        ui.allocate_ui_with_layout(egui::vec2(NAME_WIDTH, ui.spacing().interact_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_width(NAME_WIDTH);
            let validate = ui
                .add_enabled(workspace.opened().is_some(), egui::Button::new("Validate").small())
                .on_hover_text("Check the model for problems, which stop it running, and warnings: what can run but is probably a mistake");
            if validate.clicked() {
                request = Some(Request::Validate);
            }
            ui.add(name);
        });
        ui.label("seed").on_hover_text(SEED_HINT);
        let seed_field = ui
            .add_enabled(view.allows_seed(), egui::TextEdit::singleline(seed_text).desired_width(70.0))
            .on_hover_text(SEED_HINT)
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
            let hint = control_hint(control);
            let pressed = ui.add_enabled(view.allows(control), egui::Button::new(label)).on_hover_text(hint).on_disabled_hover_text(hint);
            if pressed.clicked() {
                request = Some(Request::Control(control));
            }
        };
        for (control, label) in
            [(Control::Init, "Init"), (Control::Start, "Start"), (Control::Halt, "Halt"), (Control::Reset, "Reset")]
        {
            button(ui, control, label);
        }
        ui.checkbox(keep_changes, "Keep changes").on_hover_text(
            "On Reset, save the values changed during the run, such as process times, to the model file. Off, Reset drops them, naming each first.",
        );
        ui.separator();
        button(ui, Control::StepBack, "Step back");
        button(ui, Control::StepForward, "Step forward");
        ui.separator();
        ui.label(egui::RichText::new(view.shown_time()).monospace().size(16.0));
        ui.label(match view.status() {
            SessionStatus::Advancing if !view.animate() => "Running as fast as it can",
            SessionStatus::Advancing => "Running",
            SessionStatus::Halted => "Halted",
            SessionStatus::ReachedEnd => "Reached the end time",
            SessionStatus::Failed => "Run error",
        });
    });
    ui.horizontal(|ui| {
        const END_HINT: &str = "A time at which the run halts by itself, exactly there, such as 8:00:00 or 30d. Start goes no further until it is moved or cleared.";
        ui.label("End at").on_hover_text(END_HINT);
        let hint = view.end_time().map_or_else(|| "no end".to_string(), format_time);
        let field = ui.add(egui::TextEdit::singleline(end_text).desired_width(90.0).hint_text(hint)).on_hover_text(END_HINT);
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
        ui.label("Period").on_hover_text(PERIOD_HINT);
        let period_field = ui
            .add_enabled(view.allows_seed(), egui::TextEdit::singleline(period_text).desired_width(60.0))
            .on_hover_text(PERIOD_HINT)
            .on_disabled_hover_text("The measuring period can change only before Init or after Reset");
        if period_field.lost_focus() {
            match parse_time(period_text).filter(|&length| length > SimTime::ZERO) {
                Some(length) if length != workspace.measuring_period() => request = Some(Request::MeasuringPeriod(length)),
                _ => *period_text = format_time(workspace.measuring_period()),
            }
        }
        ui.separator();

        const GO_HINT: &str = "Goes to a time, such as 30d (gt to type one). An earlier one goes back to it as it was then, dropping interventions made after it, so it asks first. A later one runs there and halts: at the speed while animated, as fast as it can otherwise.";
        ui.label("Go to").on_hover_text(GO_HINT);
        let field = ui.add(egui::TextEdit::singleline(jump_text).desired_width(90.0).hint_text(view.shown_time())).on_hover_text(GO_HINT);
        focus_if_wanted(ui, &field, "goto");
        let target = view.jump_target(jump_text);
        let go = ui.add_enabled(target.is_ok(), egui::Button::new("Go")).on_disabled_hover_text(match target {
            Err(JumpProblem::NotInitialised) => "Init first to go back, or type a later time",
            Err(JumpProblem::NotATime) => "Type a time such as 1:30, 02:00:00 or 1d 06:00:00",
            Ok(_) => "",
        });
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        if let (true, Ok(to)) = (go.clicked() || entered, target) {
            request = Some(Request::GoTo(to));
            jump_text.clear();
        }
        ui.separator();

        let mut animate = view.animate();
        let animate_hint = "Draw what happens at the chosen speed. Off, a run goes as fast as the machine allows, and the objects' states update a few times a second. Results are the same either way.";
        if ui.checkbox(&mut animate, "Animate").on_hover_text(animate_hint).changed() {
            request = Some(Request::Animate(animate));
        }
        let mut speed = view.speed();
        let slider = egui::Slider::new(&mut speed, MIN_SPEED..=MAX_SPEED)
            .logarithmic(true)
            .suffix("×")
            .max_decimals(1)
            .text("speed");
        let slider = ui
            .add_enabled(view.animate(), slider)
            .on_hover_text("How much simulated time passes in a second of real time. Changing it never makes the time jump.")
            .on_disabled_hover_text("Animation is off, so a run goes as fast as it can");
        if slider.changed() {
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
        if let Some(chosen) = answers(modal, &[button, "Cancel"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let answer = answer?;
    ui.pending = None;
    // Stepping back goes to the instant before the one on screen, which is `to`, so going back
    // there is the same step, to exactly the time the user was shown.
    answer.then_some(match pending {
        GoingBack::To(to) | GoingBack::Step(to) => Request::GoTo(to),
    })
}

/// Asks before going on animated to a time that would take long, offering to race there instead.
/// Returns the request once answered.
fn confirm_long_trip(ctx: &egui::Context, ui: &mut Ui) -> Option<Request> {
    let (to, wall) = ui.long_trip?;
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm long trip")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading(format!("Go to {}?", format_time_exact(to)));
        modal.label(format!(
            "Animated at {}× it takes {}. Racing there takes as long as the machine needs, then animation goes on again.",
            (ui.view.speed() * 10.0).round() / 10.0,
            wall_time_roughly(wall)
        ));
        modal.add_space(8.0);
        match answers(modal, &["Race there", "Animate anyway", "Cancel"]) {
            Some(0) => answer = Some(Some(Request::RaceTo(to))),
            Some(1) => answer = Some(Some(Request::GoTo(to))),
            Some(_) => answer = Some(None),
            None => {}
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(None);
    }
    let answer = answer?;
    ui.long_trip = None;
    answer
}

const PERIOD_HINT: &str = "The measuring period: a stretch of simulated time whose figures count as one sample, such as 1:00:00. The Results tab gives intervals from the spread between periods once 5 are complete. Make it long against the time between failures.";

const SEED_HINT: &str = "The number every random value in a run follows from, such as when a Station fails. The same seed gives the same result.";

/// What each run control does, worded from the glossary.
fn control_hint(control: Control) -> &'static str {
    match control {
        Control::Init => "Set time to 0 and give every object its starting state, ready to run. Start does this by itself if it hasn't happened",
        Control::Start => "Begin or carry on advancing time, doing Init first if it hasn't happened (Space)",
        Control::Halt => "Stop time advancing, with everything staying exactly as it is. Start carries on from here (Space)",
        Control::Reset => "Return every object to its default state, ready for a new run. The objects stay in the model (R)",
        Control::StepBack => "Go back to the previous instant, the last time something happened (y, or 10y for ten)",
        Control::StepForward => "Go on to the next instant, the next time something happens (x, or 10x for ten)",
    }
}

/// The name and state of the object under the pointer on the canvas, beside the pointer.
fn hover_tooltip(ctx: &egui::Context, ui: &Ui, canvas: egui::Rect) {
    let (Some(pointer), Some(camera)) = (ctx.pointer_hover_pos(), ui.camera) else { return };
    // An object's menu, once open, isn't covered by its name.
    if !canvas.contains(pointer) || ctx.is_pointer_over_egui() || ui.moving.as_ref().is_some_and(|moving| moving.dragged) || ui.canvas_menu.is_some() {
        return;
    }
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    let floor = camera.to_world(Point { x: (pointer.x - canvas.min.x) as f64, y: (pointer.y - canvas.min.y) as f64 }, size);
    let Some(id) = ui.view.object_at(floor) else { return };
    egui::Area::new(egui::Id::new("hovered object"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + egui::vec2(14.0, 14.0))
        .interactable(false)
        .show(ctx, |area| {
            egui::Frame::popup(area.style()).show(area, |frame| {
                frame.horizontal(|row| {
                    swatch(row, ui.view.shade(id));
                    row.label(ui.view.hover_text(id));
                });
            });
        });
}

/// A small square of a state's colour.
fn swatch(ui: &mut egui::Ui, shade: Shade) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, shade_colour32(shade));
}

/// What the selected object's states mean: each state its type can be in, with its colour,
/// crabsim's meaning and the model's own note, if it has one. It follows the selection.
fn help_window(ctx: &egui::Context, ui: &mut Ui, canvas: egui::Rect) {
    if !ui.help_open {
        return;
    }
    let mut open = true;
    let selected = ui.view.selected().map(|id| (ui.view.objects()[id as usize].name.clone(), ui.view.objects()[id as usize].kind));
    let notes = match &ui.attributes {
        Some((_, Ok(attributes))) => attributes.iter().find(|view| view.attribute == "state_notes").and_then(|view| view.value.clone()),
        _ => None,
    };
    // It opens at the canvas's right edge, beside the inspector, clear of the top bar and models.
    const WIDTH: f32 = 320.0;
    egui::Window::new("What the states mean")
        .open(&mut open)
        .resizable(false)
        .default_width(WIDTH)
        .default_pos(egui::pos2(canvas.right() - WIDTH - 16.0, canvas.top() + 8.0))
        .show(ctx, |window| {
        let Some((name, kind)) = selected else {
            window.label("Click an object to see what its states mean.");
            return;
        };
        window.label(egui::RichText::new(format!("{name}, a {}", help::built_in(kind))).strong());
        window.add_space(4.0);
        for state in help::states(kind, notes.as_ref()) {
            window.horizontal(|row| {
                swatch(row, Shade::of(state.state));
                row.label(egui::RichText::new(state.state.label()).strong());
            });
            if let Some(note) = &state.note {
                window.label(egui::RichText::new(format!("In this model: {note}")).italics());
            }
            window.label(state.meaning);
            window.add_space(4.0);
        }
        window.horizontal(|row| {
            swatch(row, Shade::Unknown);
            row.label(egui::RichText::new("Before Init, an object has no state yet.").weak());
        });
        window.add_space(4.0);
        window.label(
            egui::RichText::new("A model can add its own note to a state, under State notes in the attributes, set on a class or an instance.")
                .weak()
                .small(),
        );
    });
    ui.help_open = open;
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

/// What the view state says about the selected object, and the interventions it offers. Returns
/// the intervention asked for, if any. `process_time_text` is what has been typed for a new
/// process time.
fn inspector(ui: &mut egui::Ui, inspection: Option<&Inspection>, process_time_text: &mut String, help_open: &mut bool) -> Option<Intervention> {
    ui.heading("Inspector");
    let Some(inspection) = inspection else {
        ui.label("Click an object to inspect it.");
        return None;
    };
    let mut asked = None;
    ui.horizontal(|row| {
        row.label(egui::RichText::new(&inspection.name).strong().size(16.0));
        if row.selectable_label(*help_open, " ? ").on_hover_text("What this object's states and their colours mean").clicked() {
            *help_open = !*help_open;
        }
    });
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
            let pause_hint = if offered.paused { "End the Pause: the work carries on where it stopped" } else { "Stop this Station until you resume it. The work on its MU waits" };
            let pause_button = row.button(pause_label).on_hover_text(pause_hint);
            focus_if_wanted(row, &pause_button, "pause");
            if pause_button.clicked() {
                asked = Some(pause);
            }
            let (force, force_label) = if offered.forced {
                (Intervention::EndForcedFailure, "End forced failure")
            } else {
                (Intervention::ForceFailure, "Force failure")
            };
            let force_hint = if offered.forced {
                "End the forced failure: the Station carries on, unless something else still stops it"
            } else {
                "Stop this Station as a failure until you end it. It counts as a failure in the statistics"
            };
            let force_button = row.button(force_label).on_hover_text(force_hint);
            focus_if_wanted(row, &force_button, "forced_failure");
            if force_button.clicked() {
                asked = Some(force);
            }
        });
        // The process time in force shows once, here, with the configured one only if it differs.
        if let Some(text) = inspection.details.as_ref().and_then(Details::process_time_text) {
            ui.horizontal(|row| {
                row.label("Process time");
                row.label(egui::RichText::new(text).strong());
            });
        }
        ui.horizontal(|row| {
            row.label("Change to");
            let field = row.add(egui::TextEdit::singleline(process_time_text).desired_width(80.0).hint_text("such as 25"));
            focus_if_wanted(row, &field, "process_time");
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
            grid.label("Waiting for it");
            grid.label(if details.waiting.is_empty() { "nothing".to_string() } else { details.waiting.join(", ") });
            grid.end_row();
        });
        ui.add_space(6.0);
        ui.label(egui::RichText::new(format!("Statistics since {}", format_time(details.statistics_since))).strong());
        egui::Grid::new("statistics").num_columns(2).spacing([12.0, 2.0]).striped(true).show(ui, |grid| {
            for (label, value) in inspection.statistics_rows() {
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
        if let Some(chosen) = answers(modal, &[if outside { "Adopt and move" } else { "Move anyway" }, "Put it back"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// A dialog's answers as a row of buttons, the one marked drawn selected: the first to start
/// with, so Enter confirms. The arrow keys move the mark and Enter presses it, as does a click.
/// Escape closes the dialog as cancelled, as egui's modals do. Returns the answer pressed, by
/// place.
fn answers(modal: &mut egui::Ui, labels: &[&str]) -> Option<usize> {
    let id = modal.id().with("marked answer");
    let mut marked = modal.data(|data| data.get_temp::<usize>(id)).unwrap_or(0).min(labels.len() - 1);
    let (back, on, enter) = modal.input(|input| {
        (
            input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::ArrowUp),
            input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::ArrowDown),
            input.key_pressed(egui::Key::Enter),
        )
    });
    if back {
        marked = marked.saturating_sub(1);
    }
    if on {
        marked = (marked + 1).min(labels.len() - 1);
    }
    let mut chosen = None;
    modal.horizontal(|row| {
        for (place, label) in labels.iter().enumerate() {
            if row.add(egui::Button::new(*label).selected(place == marked)).clicked() {
                chosen = Some(place);
            }
        }
    });
    if enter && chosen.is_none() {
        chosen = Some(marked);
    }
    if chosen.is_some() {
        modal.data_mut(|data| data.remove::<usize>(id));
    } else {
        modal.data_mut(|data| data.insert_temp(id, marked));
    }
    chosen
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
        if let Some(chosen) = answers(modal, &["Take it", "Cancel"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// A share from 0 to 1 as the Results tab writes it.
fn percent(share: f64) -> String {
    format!("{:.1} %", share * 100.0)
}

/// A share as its mean over measuring periods, plus or minus half its 95% interval.
fn percent_interval(interval: Interval) -> String {
    format!("{:.1} ± {:.1} %", interval.mean * 100.0, interval.half_width * 100.0)
}

/// Notes that ask the user to do something, such as run longer.
const NOTE_COLOUR: egui::Color32 = egui::Color32::from_rgb(240, 200, 80);

/// The run so far: each Drain's throughput and lead time, the likely bottleneck with its
/// neighbours as evidence, and every Station's shares of time, sorted by a column clicked.
/// Returns an object whose name was clicked, to select it.
fn results_tab(ui: &mut egui::Ui, results: Option<(&str, &crabsim_ui::view::Results)>, sort: &mut SortBy) -> Option<u32> {
    ui.heading("Results");
    let Some((at, results)) = results else {
        ui.label(egui::RichText::new("Start the model, or go to a time such as 8:00:00, and its results show here.").weak());
        return None;
    };
    let mut chosen = None;
    let pick = |ui: &mut egui::Ui, id: u32, name: &str, chosen: &mut Option<u32>| {
        let link = ui.link(name).on_hover_text("Select it on the canvas");
        focus_if_wanted(ui, &link, "tab");
        if link.clicked() {
            *chosen = Some(id);
        }
    };
    ui.label(egui::RichText::new(format!("At {at}, measured since {}", format_time(results.since))).weak());
    let intervals = results.periods.is_some_and(|periods| periods.complete >= MIN_PERIODS);
    if let Some(periods) = results.periods {
        let length = format_time(periods.length);
        if intervals {
            ui.label(egui::RichText::new(format!("± is a 95% interval from {} measuring periods of {length}", periods.complete)).weak())
                .on_hover_text("Each period's figures are one sample, and the interval comes from how much they vary between periods. Narrower takes a longer run.");
        } else {
            ui.colored_label(
                NOTE_COLOUR,
                format!("Run longer: {} of {MIN_PERIODS} measuring periods of {length}, before anything says how sure it is.", periods.complete),
            );
        }
    }
    if let Some(longest) = results.short_against {
        ui.colored_label(
            NOTE_COLOUR,
            format!(
                "Periods are short against the model's failures, {} between failures or repairs at the longest, so the intervals may be too narrow. Make the period at least {} after Reset.",
                format_time(longest),
                format_time(longest.times(PERIOD_FAILURE_FACTOR))
            ),
        );
    }
    ui.add_space(6.0);

    ui.label(egui::RichText::new("Drains").strong());
    egui::Grid::new("drains").num_columns(3).spacing([10.0, 2.0]).striped(true).show(ui, |grid| {
        grid.label("");
        grid.label("Throughput").on_hover_text("MUs an hour, from the first one in up to now");
        grid.label("Lead time").on_hover_text("How long an MU spends in the model on average, from leaving its Source to entering the Drain");
        grid.end_row();
        for drain in &results.drains {
            pick(grid, drain.id, &drain.name, &mut chosen);
            grid.label(match (drain.throughput_interval, drain.throughput_per_hour) {
                (Some(interval), _) => format!("{:.1} ± {:.1} an hour", interval.mean, interval.half_width),
                (None, Some(per_hour)) => format!("{per_hour:.1} an hour"),
                (None, None) => "–".to_string(),
            });
            grid.label(drain.mean_lead_time.map_or_else(|| "–".to_string(), format_time_exact));
            grid.end_row();
        }
    });
    ui.add_space(8.0);

    ui.label(egui::RichText::new("Likely bottleneck").strong()).on_hover_text(crabsim_catalogue::bottleneck::RULE);
    match &results.bottleneck {
        None => {
            ui.label(egui::RichText::new("The model has no Station.").weak());
        }
        Some(likely) => {
            let id = results.stations.iter().find(|station| station.name == likely.instance).map(|station| station.id);
            ui.horizontal(|row| {
                match id {
                    Some(id) => pick(row, id, &likely.instance, &mut chosen),
                    None => {
                        row.label(&likely.instance);
                    }
                }
                row.label(format!("busy or stopped {}", percent(likely.not_idle_share)));
            });
            if let Some(runner_up) = &results.too_close {
                ui.colored_label(
                    NOTE_COLOUR,
                    format!("Too close to call: {} and {runner_up} can't be told apart yet, since their busy-or-stopped intervals overlap.", likely.instance),
                );
            }
            for (name, blocked) in &likely.upstream {
                ui.label(format!("Upstream {name}: blocked {}", percent(*blocked)));
            }
            for (name, idle) in &likely.downstream {
                ui.label(format!("Downstream {name}: idle {}", percent(*idle)));
            }
            ui.label(egui::RichText::new("A bottleneck's upstream is mostly blocked and its downstream mostly idle. Make sure by running it faster and comparing.").weak());
        }
    }
    ui.add_space(8.0);

    ui.label(egui::RichText::new("Stations").strong());
    let mut stations: Vec<&crabsim_ui::view::StationResult> = results.stations.iter().collect();
    let key = |station: &crabsim_ui::view::StationResult| match *sort {
        SortBy::Model => 0.0,
        SortBy::Busy => station.busy,
        SortBy::Blocked => station.blocked,
        SortBy::Idle => station.idle,
        SortBy::Failed => station.failed + station.paused,
    };
    stations.sort_by(|a, b| key(b).total_cmp(&key(a)));
    egui::Grid::new("stations").num_columns(5).spacing([4.0, 2.0]).striped(true).show(ui, |grid| {
        if grid.selectable_label(*sort == SortBy::Model, "Station").on_hover_text("In model order").clicked() {
            *sort = SortBy::Model;
        }
        for (by, label, shade, hint) in [
            (SortBy::Busy, "Busy", Shade::Busy, "Working on an MU. Click to sort"),
            (SortBy::Blocked, "Blocked", Shade::Blocked, "Holding a finished MU no successor can take. Click to sort"),
            (SortBy::Idle, "Idle", Shade::Idle, "Waiting for an MU. Click to sort"),
            (SortBy::Failed, "Stop", Shade::Failed, "Stopped: failed, under a forced failure or paused. Click to sort"),
        ] {
            let text = egui::RichText::new(label).color(shade_colour32(shade));
            if grid.selectable_label(*sort == by, text).on_hover_text(hint).clicked() {
                *sort = by;
            }
        }
        grid.end_row();
        for station in stations {
            pick(grid, station.id, &station.name, &mut chosen);
            match station.intervals {
                Some(intervals) => {
                    for interval in [intervals.busy, intervals.blocked, intervals.idle, intervals.stopped] {
                        grid.label(percent_interval(interval));
                    }
                }
                None => {
                    grid.label(percent(station.busy));
                    grid.label(percent(station.blocked));
                    grid.label(percent(station.idle));
                    grid.label(percent(station.failed + station.paused));
                }
            }
            grid.end_row();
        }
    });
    chosen
}

/// Who holds the open model, with an agent's goal and what it's doing, the other files someone
/// holds, and the blocked agents, with the user's controls. Returns the control used, if any.
fn agents_tab(ui: &mut egui::Ui, opened: Option<&str>, editors: Option<&Result<Editors, String>>, note: Option<&str>) -> Option<EditAction> {
    ui.heading("Agents");
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
    let nobody = editors.files.iter().all(|file| file.claim.is_none()) && editors.blocked.is_empty();
    if nobody {
        ui.label(egui::RichText::new("Agents working on this folder's models show here, with their goal and what they're doing. An agent changes a model only while it holds a claim on it.").weak());
        ui.add_space(4.0);
    }
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
                let release = ui.button("Release").on_hover_text("Let go of it, giving a paused claim back to its agent");
                focus_if_wanted(ui, &release, "tab");
                if release.clicked() {
                    asked = Some(EditAction::Release);
                }
            }
            match &file.claim {
                None if file.user_since.is_none() => {
                    ui.label("Nobody holds it.");
                    let take = ui.button("Take").on_hover_text("Hold this model until you release it or close the window");
                    focus_if_wanted(ui, &take, "tab");
                    if take.clicked() {
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
                        let revoke = row.button("Revoke").on_hover_text("End the agent's claim");
                        focus_if_wanted(row, &revoke, "tab");
                        if revoke.clicked() {
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
                if row.small_button("Unblock").on_hover_text("Let this agent session claim models again").clicked() {
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
        "state_notes" => "State notes",
        "distribution" => "Distribution",
        other => other,
    }
}

/// Every attribute of the selected instance, its value and where it comes from. Before Init or
/// after Reset each can be changed, saved to the model file, and each set on the instance can be
/// restored on its own. During a run, when they aren't `editable`, one line says when they can
/// change. Returns the change asked for, and whether it takes over from an agent.
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
    // During a run they can't change, and the interventions above show what does, so they fold
    // away to one line rather than repeating values greyed out.
    if !editable {
        ui.label("Attributes change before Init or after Reset.");
        return None;
    }
    let attributes = match attributes {
        Ok(attributes) => attributes,
        Err(error) => {
            ui.label(egui::RichText::new(error).weak());
            return None;
        }
    };
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
            if view.from == ValueFrom::Instance {
                let restore = row.small_button("Restore").on_hover_text("Take this attribute alone back to its class's value");
                if restore.clicked() {
                    asked = Some((AttributeChange::Restore(view.attribute.clone()), false));
                }
            }
        });
        let Some(form) = forms.get_mut(&view.attribute) else { continue };
        let mut commit = false;
        ui.scope(|fields| {
            if let Some(choice) = form.distribution.as_mut() {
                distribution_fields(fields, choice, &view.attribute);
                return;
            }
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
                for (place, part) in form.parts.iter_mut().filter(|part| !part.label.is_empty() || form.repair.is_none()).enumerate() {
                    grid.label(part.label);
                    let field = grid.add(egui::TextEdit::singleline(&mut part.text).desired_width(120.0));
                    if place == 0 {
                        focus_if_wanted(grid, &field, &view.attribute);
                    }
                    commit |= field.lost_focus();
                    grid.end_row();
                }
            });
        });
        let waiting = !form.matches(view.value.as_ref());
        if waiting {
            match form.value() {
                // Failures at 100 %: none, so whatever was set here comes off.
                Ok(serde_json::Value::Null) if view.from == ValueFrom::Instance => {
                    let apply = ui.small_button("Apply").on_hover_text("Takes its own failures off, so it never fails");
                    if commit || apply.clicked() {
                        asked = Some((AttributeChange::Restore(view.attribute.clone()), false));
                    }
                }
                Ok(serde_json::Value::Null) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 120, 120),
                        "Its class gives it failures, and a model file can't take them off one instance. Change the class, or give this one its own class.",
                    );
                }
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

/// The keys typed this frame, taken out of egui's input so no widget acts on them too: every
/// character, and the keys that type none that the keyboard uses. Delete stays, for its own
/// handling.
fn read_keys(ctx: &egui::Context) -> Vec<Key> {
    let mut keys = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let key = match event {
                egui::Event::Text(text) => {
                    keys.extend(text.chars().map(Key::Char));
                    return false;
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    egui::Key::Enter => Some(Key::Enter),
                    egui::Key::Escape => Some(Key::Escape),
                    egui::Key::Backspace => Some(Key::Backspace),
                    egui::Key::Tab if modifiers.shift => Some(Key::ShiftTab),
                    egui::Key::Tab => Some(Key::Tab),
                    egui::Key::ArrowUp => Some(Key::Up),
                    egui::Key::ArrowDown => Some(Key::Down),
                    egui::Key::ArrowLeft => Some(Key::Left),
                    egui::Key::ArrowRight => Some(Key::Right),
                    egui::Key::Plus | egui::Key::Equals if modifiers.command => Some(Key::ZoomIn),
                    egui::Key::Minus if modifiers.command => Some(Key::ZoomOut),
                    _ => None,
                },
                _ => None,
            };
            match key {
                Some(key) => {
                    keys.push(key);
                    false
                }
                None => true,
            }
        });
    });
    keys
}

/// Marks the field or button called `key` to take the keyboard when it is next drawn, within a
/// second, such as `name`, `process_time` or `goto`.
fn want_focus(ctx: &egui::Context, key: &str) {
    let now = ctx.input(|input| input.time);
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("focus wanted"), (key.to_string(), now)));
}

/// Gives `field` the keyboard, scrolled into sight, if it is the one marked by `want_focus`.
fn focus_if_wanted(ui: &egui::Ui, field: &egui::Response, key: &str) {
    let id = egui::Id::new("focus wanted");
    let wanted: Option<(String, f64)> = ui.data(|data| data.get_temp(id));
    if let Some((wanted, at)) = wanted
        && wanted == key
        && ui.input(|input| input.time) - at < 1.0
    {
        field.request_focus();
        field.scroll_to_me(Some(egui::Align::Center));
        ui.data_mut(|data| data.remove::<(String, f64)>(id));
    }
}

/// Removes what is selected, as Delete and `d` do: a class of the model's own, a connection, or
/// the objects selected, asking first where more goes with it.
fn delete_what_is_selected(ui: &mut Ui) {
    match (&ui.class_selected, ui.view.selected(), ui.view.selected_connection()) {
        // A built-in class stays: its values are restored one by one.
        (Some((class, _)), None, None) if crabsim_model::BUILT_IN_CLASSES.contains(&class.as_str()) => {}
        (Some((class, _)), None, None) if ui.view.allows_building() => {
            let class = class.clone();
            remove_class_asking(ui, class);
        }
        _ => delete_selected(ui),
    }
}

/// Selects the object, or adds it to the selection while selecting visually, and moves the view
/// to it when it is out of sight.
fn select_and_follow(ui: &mut Ui, id: u32) {
    ui.cursor = Some(id);
    if ui.keys.visual() {
        if !ui.view.selection().contains(&id) {
            ui.view.toggle(id);
        }
    } else {
        ui.view.select(Some(id));
    }
    ui.class_selected = None;
    if let (Some(camera), Some(canvas)) = (ui.camera.as_mut(), ui.canvas) {
        let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
        let centre = ui.view.objects()[id as usize].centre;
        let at = camera.to_screen(centre, size);
        if at.x < 0.0 || at.y < 0.0 || at.x > size.x || at.y > size.y {
            camera.centre = centre;
            ui.camera_moved = true;
        }
    }
}

/// What the `e` prompt can reach on the object at `id`: before Init or after Reset its name and
/// every attribute, during a run what an intervention changes. By name and label.
fn edit_targets(ui: &Ui, id: u32) -> Vec<(String, String)> {
    let object = &ui.view.objects()[id as usize];
    if ui.view.reloads_now() {
        let mut targets = vec![("name".to_string(), "Name".to_string())];
        if let Ok(values) = ui.workspace.attributes(&object.name) {
            targets.extend(values.iter().map(|view| (view.attribute.clone(), attribute_label(&view.attribute).to_string())));
        }
        return targets;
    }
    if object.kind == crabsim::ObjectKind::Station {
        return [("process_time", "Process time"), ("pause", "Pause"), ("forced_failure", "Forced failure")]
            .map(|(name, label)| (name.to_string(), label.to_string()))
            .to_vec();
    }
    Vec::new()
}

/// What the `e` prompt can reach: the selected object's values, or the selected class's.
fn selected_targets(ui: &Ui) -> Option<Vec<(String, String)>> {
    if let Some(id) = ui.view.selected() {
        return Some(edit_targets(ui, id));
    }
    let (class, _) = ui.class_selected.as_ref()?;
    let values = ui.workspace.class_attributes(class).ok()?;
    Some(values.iter().map(|view| (view.attribute.clone(), attribute_label(&view.attribute).to_string())).collect())
}

/// The object names, by id.
fn object_names(ui: &Ui) -> Vec<String> {
    ui.view.objects().iter().map(|object| object.name.clone()).collect()
}

/// What a prompt's text could become, for the key line and for Tab: matching names, values or
/// model files.
fn prompt_candidates(ui: &Ui, kind: PromptKind, text: &str) -> Vec<String> {
    let names = object_names(ui);
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match kind {
        PromptKind::Search => crabsim_ui::keys::matches(&names, text).into_iter().map(|index| names[index].to_string()).collect(),
        PromptKind::Command => crabsim_catalogue::commands::complete(text, ui.workspace.model()),
        PromptKind::Open => {
            let paths: Vec<&str> = ui.workspace.models().iter().map(|entry| entry.path.as_str()).collect();
            crabsim_ui::keys::matches(&paths, text).into_iter().map(|index| paths[index].to_string()).collect()
        }
        PromptKind::Edit => {
            let address = crabsim_ui::keys::address(text);
            let targets = match address.object {
                Some(object) => match crabsim_ui::keys::matches(&names, object).as_slice() {
                    [only] => edit_targets(ui, *only as u32),
                    several => return several.iter().map(|&index| format!("{}.", names[index])).collect(),
                },
                None => match selected_targets(ui) {
                    Some(targets) => targets,
                    // Nothing selected: the prompt names an object first.
                    None => return crabsim_ui::keys::matches(&names, text).into_iter().map(|index| format!("{}.", names[index])).collect(),
                },
            };
            let prefix = address.object.map_or(String::new(), |object| format!("{object}."));
            targets
                .iter()
                .filter(|(name, label)| address.attribute.is_empty() || crabsim_ui::keys::attribute_matches(address.attribute, name, label))
                .map(|(name, _)| format!("{prefix}{name}"))
                .collect()
        }
    }
}

/// What the `e` prompt answered asks for: select the object it names, if any, and put the
/// keyboard in the value it names.
fn answer_edit(ctx: &egui::Context, ui: &mut Ui, text: &str) {
    let address = crabsim_ui::keys::address(text);
    if let Some(object) = address.object {
        let names = object_names(ui);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        match crabsim_ui::keys::matches(&names, object).first() {
            Some(&index) => select_and_follow(ui, index as u32),
            None => {
                ui.key_note = Some(format!("No object is called anything like {object}"));
                return;
            }
        }
    }
    if address.attribute.is_empty() {
        return;
    }
    let Some(targets) = selected_targets(ui) else {
        ui.key_note = Some("Select an object first, or type its name and a dot, such as Station 3.proc".to_string());
        return;
    };
    match targets.iter().find(|(name, label)| crabsim_ui::keys::attribute_matches(address.attribute, name, label)) {
        Some((name, _)) => {
            ui.tab = Tab::Inspector;
            want_focus(ctx, name);
        }
        None => ui.key_note = Some(format!("Nothing here matches {}", address.attribute)),
    }
}

/// Carries out one key's command, giving a request for the run when it asks for one.
fn run_key(ctx: &egui::Context, ui: &mut Ui, session: &Session, command: Command, wall: Duration) -> Option<Request> {
    ui.view.commanded(wall);
    // In visual mode moves go from the cursor, the object last landed on, whatever the
    // selection holds (issue 101). Otherwise from the selection.
    let from = if ui.keys.visual() {
        ui.cursor.filter(|&id| (id as usize) < ui.view.objects().len()).or(ui.view.selection().last().copied())
    } else {
        ui.view.selected().or(ui.view.selection().last().copied())
    };
    let count = ui.view.objects().len() as u32;
    let needs_object = |ui: &mut Ui| {
        ui.key_note = Some("Select an object first".to_string());
        None
    };
    match command {
        Command::Move(direction, times) => {
            let Some(mut at) = from else {
                if count > 0 {
                    select_and_follow(ui, 0);
                }
                return None;
            };
            let centres: Vec<Point> = ui.view.objects().iter().map(|object| object.centre).collect();
            for _ in 0..times {
                match crabsim_ui::keys::nearest(&centres, at as usize, direction) {
                    Some(next) => {
                        at = next as u32;
                        select_and_follow(ui, at);
                    }
                    None => {
                        ui.key_note = Some("Nothing further that way".to_string());
                        break;
                    }
                }
            }
        }
        Command::Successor(exit) | Command::Predecessor(exit) => {
            let Some(at) = from else { return needs_object(ui) };
            let name = ui.view.objects()[at as usize].name.clone();
            let downstream = matches!(command, Command::Successor(_));
            let ends: Vec<String> = if downstream {
                ui.workspace.model().successors(&name).into_iter().map(str::to_string).collect()
            } else {
                ui.workspace.model().predecessors(&name).into_iter().map(str::to_string).collect()
            };
            let id = |wanted: &str| ui.view.objects().iter().find(|object| object.name == wanted).map(|object| object.id);
            match ends.get(exit - 1).and_then(|end| id(end)) {
                Some(next) => select_and_follow(ui, next),
                None => {
                    let (what, place) = if downstream { ("exit", "exits") } else { ("entrance", "entrances") };
                    ui.key_note = Some(match ends.len() {
                        0 => format!("{name} has no {place}"),
                        1 => format!("{name} has 1 {what}"),
                        many => format!("{name} has {many} {place}"),
                    });
                }
            }
        }
        Command::First | Command::Last => {
            if count > 0 {
                select_and_follow(ui, if command == Command::First { 0 } else { count - 1 });
            }
        }
        Command::Step { backwards } => {
            if let Some(next) = ui.view.next_object(from, backwards) {
                select_and_follow(ui, next);
            }
        }
        Command::Unselect => {
            ui.view.select(None);
            ui.view.select_connection(None);
            ui.class_selected = None;
            ui.search.0.clear();
        }
        Command::Visual(on) => {
            // The cursor starts on what was last selected.
            if on {
                ui.cursor = ui.view.selected().or(ui.view.selection().last().copied());
            }
        }
        Command::TakeOut => match ui.cursor.filter(|id| ui.view.selection().contains(id)) {
            Some(id) => ui.view.toggle(id),
            None => ui.key_note = Some("The cursor isn't on a selected object".to_string()),
        },
        Command::Delete => delete_what_is_selected(ui),
        Command::Pan(direction) => {
            if let Some(camera) = ui.camera.as_mut() {
                let step = 80.0;
                let (x, y) = match direction {
                    Direction::Left => (step, 0.0),
                    Direction::Right => (-step, 0.0),
                    Direction::Up => (0.0, step),
                    Direction::Down => (0.0, -step),
                };
                camera.pan(Point { x, y });
                ui.camera_moved = true;
            }
        }
        Command::Zoom { closer } => {
            if let (Some(camera), Some(canvas)) = (ui.camera.as_mut(), ui.canvas) {
                let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
                camera.zoom_at(if closer { 1.25 } else { 0.8 }, Point { x: size.x / 2.0, y: size.y / 2.0 }, size);
                ui.camera_moved = true;
            }
        }
        Command::Fit => ui.camera_moved = false,
        Command::Centre => {
            let Some(at) = from else { return needs_object(ui) };
            if let Some(camera) = ui.camera.as_mut() {
                camera.centre = ui.view.objects()[at as usize].centre;
                ui.camera_moved = true;
            }
        }
        Command::StartOrHalt => {
            if ui.view.allows(Control::Halt) {
                return Some(Request::Control(Control::Halt));
            }
            if ui.view.allows(Control::Start) {
                return Some(Request::Control(Control::Start));
            }
        }
        Command::StepForward(times) => ui.steps_pending.0 = ui.steps_pending.0.saturating_add(times).min(10_000),
        Command::StepBack(times) => ui.steps_pending.1 = ui.steps_pending.1.saturating_add(times).min(10_000),
        Command::Reset => {
            if ui.view.allows(Control::Reset) {
                return Some(Request::Control(Control::Reset));
            }
        }
        Command::Faster => return ui.view.has_model().then(|| Request::Speed(ui.view.speed() * 2.0)),
        Command::Slower => return ui.view.has_model().then(|| Request::Speed(ui.view.speed() / 2.0)),
        Command::Animate => return ui.view.has_model().then(|| Request::Animate(!ui.view.animate())),
        Command::GoTo => want_focus(ctx, "goto"),
        Command::Tab(place) => {
            ui.tab = [Tab::Inspector, Tab::Results, Tab::Paths, Tab::Agents][place.clamp(1, 4) - 1];
        }
        // On another tab, Enter goes to its first control, and Tab on from there.
        Command::Edit if ui.tab != Tab::Inspector => want_focus(ctx, "tab"),
        Command::Edit => {
            let field = if ui.view.selected_connection().is_some() {
                Some("description".to_string())
            } else if let Some(id) = ui.view.selected() {
                edit_targets(ui, id).first().map(|(name, _)| name.clone())
            } else {
                selected_targets(ui).and_then(|targets| targets.first().map(|(name, _)| name.clone()))
            };
            match field {
                Some(field) => {
                    ui.tab = Tab::Inspector;
                    want_focus(ctx, &field);
                }
                None => return needs_object(ui),
            }
        }
        Command::Prompted(PromptKind::Search, text) => {
            let names = object_names(ui);
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            ui.search = (crabsim_ui::keys::matches(&names, &text).into_iter().map(|index| index as u32).collect(), 0);
        }
        Command::Prompted(..) => {}
        Command::Complete(kind, text) => {
            if let Some(first) = prompt_candidates(ui, kind, &text).into_iter().next() {
                ui.keys.set_prompt_text(first);
            }
        }
        Command::Answered(PromptKind::Search, text) => {
            let names = object_names(ui);
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            ui.search = (crabsim_ui::keys::matches(&names, &text).into_iter().map(|index| index as u32).collect(), 0);
            match ui.search.0.first().copied() {
                Some(first) => select_and_follow(ui, first),
                None => ui.key_note = Some(format!("No object is called anything like {text}")),
            }
        }
        Command::Answered(PromptKind::Edit, text) => answer_edit(ctx, ui, &text),
        Command::Answered(PromptKind::Command, text) => return run_line(ui, session, &text, wall),
        Command::Prefill(prefill) => {
            let selected = ui.view.selected().map(|id| ui.view.objects()[id as usize].name.clone());
            let text = match (prefill, selected) {
                (crabsim_ui::keys::Prefill::Add, _) => "add ".to_string(),
                (crabsim_ui::keys::Prefill::Connect, Some(name)) => format!("connect {name} > "),
                (crabsim_ui::keys::Prefill::Rename, Some(name)) => format!("rename {name} to "),
                (crabsim_ui::keys::Prefill::Connect, None) => "connect ".to_string(),
                (crabsim_ui::keys::Prefill::Rename, None) => "rename ".to_string(),
            };
            ui.keys.open_prompt(PromptKind::Command, text);
        }
        Command::MoveObjects(direction, metres) => {
            let (dx, dy) = match direction {
                Direction::Left => (-(metres as f64), 0.0),
                Direction::Right => (metres as f64, 0.0),
                Direction::Up => (0.0, metres as f64),
                Direction::Down => (0.0, -(metres as f64)),
            };
            let names: Vec<String> = ui.view.selection().iter().map(|&id| ui.view.objects()[id as usize].name.clone()).collect();
            if names.is_empty() {
                return needs_object(ui);
            }
            let line = names.iter().map(|name| format!("move {name} by {dx} {dy}")).collect::<Vec<_>>().join("; ");
            return run_line(ui, session, &line, wall);
        }
        Command::Answered(PromptKind::Open, text) => match prompt_candidates(ui, PromptKind::Open, &text).into_iter().next() {
            Some(path) => open_model(ui, session, &path, wall),
            None => ui.key_note = Some(format!("No model file is called anything like {text}")),
        },
        Command::PromptClosed(PromptKind::Search) => ui.search.0.clear(),
        Command::PromptClosed(_) => {}
        Command::NextMatch { backwards } => {
            let found = ui.search.0.len();
            if found == 0 {
                ui.key_note = Some("Search first, with /".to_string());
                return None;
            }
            let place = if backwards { (ui.search.1 + found - 1) % found } else { (ui.search.1 + 1) % found };
            ui.search.1 = place;
            let next = ui.search.0[place];
            select_and_follow(ui, next);
        }
        Command::Help => ui.keys_help = !ui.keys_help,
    }
    None
}

/// Carries out a `:` line. A command for the window happens here. Model changes go through
/// `run_commands` as one batch with no run in progress, asking first when they remove something.
/// During a run only process times can change, as interventions.
fn run_line(ui: &mut Ui, session: &Session, line: &str, wall: Duration) -> Option<Request> {
    use crabsim_catalogue::commands::{parse, Context, Parsed, WindowCommand};
    let selected = ui.view.selected().map(|id| ui.view.objects()[id as usize].name.clone());
    let at = ui.camera.map_or((0.0, 0.0), |camera| (camera.centre.x, camera.centre.y));
    let context = Context { selected: selected.clone(), at: Some(at) };
    let planned = match parse(line, ui.workspace.model(), &context) {
        Err(why) => {
            ui.view.show_error(why);
            return None;
        }
        Ok(Parsed::Window(command)) => {
            return match command {
                WindowCommand::GoTo(to) => Some(Request::GoTo(to)),
                WindowCommand::Speed(speed) => Some(Request::Speed(speed)),
                WindowCommand::Seed(seed) if ui.view.allows_seed() => Some(Request::Seed(seed)),
                WindowCommand::Seed(_) => {
                    ui.key_note = Some("The seed changes only before Init or after Reset".to_string());
                    None
                }
                WindowCommand::Open(path) => {
                    let paths: Vec<&str> = ui.workspace.models().iter().map(|entry| entry.path.as_str()).collect();
                    match crabsim_catalogue::commands::find(&paths, &path, "model file") {
                        Ok(path) => open_model(ui, session, &path, wall),
                        Err(why) => ui.view.show_error(why),
                    }
                    None
                }
                WindowCommand::Validate => {
                    ui.validation = Some(ui.workspace.validate());
                    None
                }
            };
        }
        Ok(Parsed::Changes(planned)) => planned,
    };
    if !ui.view.allows_building() {
        // During a run a process time changes as an intervention, and nothing else changes.
        let mut changes = Vec::new();
        for change in planned.iter().flat_map(|command| &command.changes) {
            let name = change["target"]["instance"].as_str();
            let id = name.and_then(|name| ui.view.objects().iter().find(|object| object.name == name).filter(|object| object.kind == crabsim::ObjectKind::Station).map(|object| object.id));
            let time = change["value"].as_str().and_then(parse_time).filter(|&time| time > SimTime::ZERO);
            match (change["change"].as_str(), change["attribute"].as_str(), id, time) {
                (Some("set_attribute"), Some("process_time"), Some(id), Some(time)) => changes.push((id, time)),
                _ => {
                    ui.view.show_error(format!("{RESET_FIRST}: during a run a line can only set a Station's process_time"));
                    return None;
                }
            }
        }
        let session = session.0.lock().expect("the session lock is never poisoned");
        for (id, time) in changes {
            match session.set_process_time(id, time) {
                Ok(()) => ui.view.intervened(id, Intervention::SetProcessTime(time), wall),
                Err(error) => ui.view.observe(&crabsim::SessionOutput::Error(error), wall),
            }
        }
        return None;
    }
    let building = Building::Line { text: line.to_string(), selected, at };
    let removing: Vec<String> = planned.iter().filter(|command| command.removes).map(|command| command.command.clone()).collect();
    if removing.is_empty() {
        build(ui, building, false);
    } else {
        ui.confirm_line = Some((building, removing));
    }
    None
}

/// Asks before a command line that removes something. `Some(true)` to run it.
fn confirm_line(ctx: &egui::Context, removing: &[String]) -> Option<bool> {
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("confirm line")).show(ctx, |modal| {
        modal.set_max_width(420.0);
        modal.heading("Run this line?");
        modal.label("It removes, with their connections or what depends on them:");
        for command in removing {
            modal.label(format!("  {command}"));
        }
        if let Some(chosen) = answers(modal, &["Run it", "Cancel"]) {
            answer = Some(chosen == 0);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    answer
}

/// The next step asked for with a count, once the step before allows it. A step that isn't
/// allowed any more, such as at the end time, ends the count.
fn pending_step(ui: &mut Ui) -> Option<Request> {
    for (pending, control) in [(&mut ui.steps_pending.0, Control::StepForward), (&mut ui.steps_pending.1, Control::StepBack)] {
        if *pending == 0 {
            continue;
        }
        if !ui.view.allows(control) {
            *pending = 0;
            continue;
        }
        *pending -= 1;
        return Some(Request::Control(control));
    }
    None
}

/// The line at the foot of the canvas: an open prompt with what it could become, the visual
/// selection, a count being typed, or what the last key did.
fn key_line(ctx: &egui::Context, ui: &Ui, canvas: egui::Rect) {
    let line = if let Some((kind, text)) = ui.keys.prompt() {
        let candidates = prompt_candidates(ui, kind, text);
        let shown: Vec<&str> = candidates.iter().take(6).map(String::as_str).collect();
        let more = if candidates.len() > shown.len() { format!(" and {} more", candidates.len() - shown.len()) } else { String::new() };
        let what = match kind {
            PromptKind::Search if !text.is_empty() => format!("   {} found", candidates.len()),
            _ if shown.is_empty() => String::new(),
            _ => format!("   {}{more}   (Tab completes)", shown.join(", ")),
        };
        format!("{}{text}_{what}", kind.sign())
    } else if ui.keys.visual() {
        "-- VISUAL --   motions add what they land on, x takes one out, d deletes, Esc ends".to_string()
    } else if let Some(count) = ui.keys.count() {
        count.to_string()
    } else if let Some(note) = &ui.key_note {
        note.clone()
    } else {
        return;
    };
    egui::Area::new(egui::Id::new("key line"))
        .fixed_pos(egui::pos2(canvas.min.x + 8.0, canvas.max.y - 30.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |area| {
            egui::Frame::popup(area.style()).show(area, |frame| {
                frame.add(egui::Label::new(egui::RichText::new(line).monospace()).wrap_mode(egui::TextWrapMode::Extend));
            });
        });
}

/// Every key, by area, while `?` has it open.
fn keys_help(ctx: &egui::Context, ui: &mut Ui) {
    if !ui.keys_help {
        return;
    }
    const KEYS: [(&str, &[(&str, &str)]); 7] = [
        (
            "Select",
            &[
                ("h j k l", "the nearest object left, down, up, right"),
                ("> <", "a successor, a predecessor: 3> takes the third exit, and 3>2>1> chains"),
                ("gg G", "the first, the last object"),
                ("Tab", "the next object, Shift+Tab the one before"),
                ("v", "select visually: motions add what they land on, x takes the one under the cursor out"),
                ("d", "delete what is selected, asking first"),
                ("Esc", "unselect, or leave a field"),
            ],
        ),
        ("View", &[("arrows", "pan"), ("Ctrl + / -", "zoom"), ("0", "fit the model"), ("zz", "centre on the selection")]),
        (
            "Run",
            &[
                ("Space", "start or halt"),
                ("x y", "step forward, back: 10x steps ten"),
                ("R", "reset"),
                ("+ -", "faster, slower"),
                ("a", "animate on or off"),
                ("gt", "go to a time"),
            ],
        ),
        (
            "Build",
            &[
                (":", "a line of commands, such as add Mill; connect Split > Mill 3. Up and Down for earlier lines"),
                ("p c r", "add, connect from the selection, rename the selection"),
                ("H J K L", "move the selection 1 m left, down, up, right"),
            ],
        ),
        ("Panels", &[("t1 to t4", "Inspector, Results, Paths, Agents"), ("Enter", "into the selection's first field, or the open tab's first control, then Tab on"), ("o", "open a model file")]),
        ("Find", &[("/", "search by name, then n and N for the next and previous"), ("e", "go to a value, such as proc or Station 3.proc")]),
        ("Help", &[("?", "this list")]),
    ];
    let mut open = true;
    egui::Window::new("Keys").open(&mut open).collapsible(false).resizable(false).show(ctx, |window| {
        window.label(egui::RichText::new("When no field has the keyboard. A number first repeats a move or a step.").weak());
        for (area, keys) in KEYS {
            window.add_space(6.0);
            window.label(egui::RichText::new(area).strong());
            egui::Grid::new(("keys", area)).num_columns(2).spacing([16.0, 2.0]).show(window, |grid| {
                for (key, does) in keys {
                    grid.label(egui::RichText::new(*key).monospace());
                    grid.label(*does);
                    grid.end_row();
                }
            });
        }
    });
    if !open {
        ui.keys_help = false;
    }
}

/// The Paths tab: switches for its three views, and the route view's choices and routes. It
/// changes `paths`, and `find_paths` finds the routes once the panels are drawn.
fn paths_tab(ui: &mut egui::Ui, paths: &mut Paths, objects: &[crabsim_ui::view::ObjectView], live: bool) {
    ui.label(egui::RichText::new("See the model's routes, weights and strategies on the canvas. Values change in the inspector.").weak());
    ui.add_space(6.0);
    // What routes and weights are weighed by, shared by both views.
    ui.horizontal(|row| {
        row.label("Weigh by");
        let weigh = egui::ComboBox::from_id_salt("paths weigh").selected_text(paths.weigh.label()).show_ui(row, |menu| {
            menu.selectable_value(&mut paths.weigh, Weigh::Connections, "Connections").on_hover_text("Each connection counts one");
            menu.separator();
            for (name, label) in crabsim_ui::paths::ATTRIBUTES {
                menu.selectable_value(&mut paths.weigh, Weigh::Attribute(name), label).on_hover_text("Each object's value, resolved through inheritance");
            }
            menu.separator();
            for (name, label) in crabsim_ui::paths::FIGURES {
                menu.add_enabled_ui(live, |menu| {
                    menu.selectable_value(&mut paths.weigh, Weigh::Figure(name), label);
                })
                .response
                .on_hover_text("The run so far, at shown time")
                .on_disabled_hover_text("A figure of the run, after Init");
            }
        });
        focus_if_wanted(row, &weigh.response, "tab");
    });
    if matches!(paths.weigh, Weigh::Figure(_)) && !live {
        paths.weigh = Weigh::Connections;
    }
    ui.add_space(6.0);
    ui.checkbox(&mut paths.route_on, egui::RichText::new("Route").strong()).on_hover_text("Highlight a route from one object to another");
    if paths.route_on {
        let name = |id: Option<u32>| id.and_then(|id| objects.get(id as usize)).map_or("choose…".to_string(), |object| object.name.clone());
        egui::Grid::new("paths ends").num_columns(2).show(ui, |grid| {
            for (label, end) in [("From", &mut paths.from), ("To", &mut paths.to)] {
                grid.label(label);
                egui::ComboBox::from_id_salt(("paths end", label)).selected_text(name(*end)).width(160.0).show_ui(grid, |menu| {
                    for object in objects {
                        menu.selectable_value(end, Some(object.id), &object.name);
                    }
                });
                grid.end_row();
            }
        });
        if paths.weigh != Weigh::Connections {
            ui.horizontal(|row| {
                row.selectable_value(&mut paths.largest, false, "Smallest").on_hover_text("The lightest total first");
                row.selectable_value(&mut paths.largest, true, "Largest").on_hover_text("The heaviest total first, as for a critical path");
            });
        }
        if paths.from.is_some() && paths.from == paths.to {
            ui.label(egui::RichText::new("Choose two different objects.").weak());
        }
        match &paths.found {
            Some(Ok(found)) if found.routes.is_empty() => {
                ui.label("No route goes from one to the other.");
            }
            Some(Ok(found)) => {
                let unit = if paths.weigh == Weigh::Connections { "connections".to_string() } else { "total".to_string() };
                ui.label(egui::RichText::new(format!("Routes, best first, with their {unit}. Click one to highlight it.")).weak().small());
                let mut chosen = paths.chosen;
                for (place, route) in found.routes.iter().enumerate() {
                    let names: Vec<&str> = route.ids.iter().filter_map(|&id| objects.get(id as usize)).map(|object| object.name.as_str()).collect();
                    let total = if paths.weigh == Weigh::Connections { format!("{}", route.total) } else { paths.weigh.show(route.total) };
                    if ui.selectable_label(place == chosen, format!("{total}   {}", names.join(" to "))).clicked() {
                        chosen = place;
                    }
                }
                paths.chosen = chosen;
                if !found.weightless.is_empty() {
                    let names: Vec<&str> = found.weightless.iter().filter_map(|&id| objects.get(id as usize)).map(|object| object.name.as_str()).collect();
                    ui.label(egui::RichText::new(format!("Weighing 0, without the value: {}", names.join(", "))).weak().small());
                }
                if !found.exact {
                    ui.label(egui::RichText::new("Loops on the way stopped the search early, so a heavier route may exist.").weak().small());
                }
                let more = if found.more { "more than " } else { "" };
                ui.label(egui::RichText::new(format!("{more}{} route{} in all", found.count, plural(found.count as usize))).weak().small());
            }
            Some(Err(error)) => {
                ui.colored_label(egui::Color32::from_rgb(240, 120, 120), error);
            }
            None => {}
        }
    }
    ui.add_space(6.0);
    ui.checkbox(&mut paths.weights_on, egui::RichText::new("Weights").strong()).on_hover_text("Each object's weight, written above it");
    if paths.weights_on && paths.weigh == Weigh::Connections {
        ui.label(egui::RichText::new("Choose what to weigh by to see each object's weight.").weak());
    }
    ui.add_space(6.0);
    ui.checkbox(&mut paths.strategies_on, egui::RichText::new("Strategies").strong()).on_hover_text("How each object picks the successor for its MU");
    if paths.strategies_on {
        ui.label(
            egui::RichText::new(
                "RR takes its successors round robin, in turn. W ranks them: the green arrows go to its best-ranked, and each exit shows its place. Those sharing a place take turns.",
            )
            .weak(),
        );
    }
}

/// Finds the Paths tab's routes when what they depend on changed, and works out the weights and
/// strategies its views draw.
fn find_paths(ui: &mut Ui) {
    let values = match ui.paths.weigh {
        Weigh::Figure(figure) => ui.view.live_figure(figure),
        _ => None,
    };
    if ui.paths.needs_finding(ui.model_generation, values.as_deref()) {
        let answer = ui.paths.query(ui.view.objects(), values.as_deref()).map_or_else(|| Err("The route's ends aren't in the model.".to_string()), |query| ui.workspace.query(query));
        ui.paths.take_answer(answer, ui.view.objects(), ui.model_generation, values.as_deref());
    }
    let key = (ui.model_generation, ui.paths.weigh.clone(), values.clone());
    let wanted = ui.paths.weights_on || ui.paths.strategies_on;
    if wanted && ui.overlay.as_ref().is_none_or(|overlay| overlay.made_for != key) && ui.workspace.opened().is_some() {
        let weights = crabsim_ui::paths::weights(ui.workspace.model(), &ui.paths.weigh, values.as_deref());
        let strategies = crabsim_ui::paths::strategies(ui.workspace.model(), ui.workspace.description());
        ui.overlay = Some(Overlay { made_for: key, weights, strategies });
    }
}

/// The Paths tab's words on the canvas: each object's weight above it, its strategy beside it,
/// and each weighted object's ranking on its exits.
fn paths_labels(ctx: &egui::Context, ui: &Ui, camera: &ViewCamera, canvas: egui::Rect, text_size: f32) {
    let Some(Overlay { weights, strategies, .. }) = &ui.overlay else { return };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("paths labels"))).with_clip_rect(canvas);
    let size = Point { x: canvas.width() as f64, y: canvas.height() as f64 };
    let screen = |point: Point| {
        let at = camera.to_screen(point, size);
        egui::pos2(canvas.min.x + at.x as f32, canvas.min.y + at.y as f32)
    };
    let font = egui::FontId::proportional(text_size);
    let route = egui::Color32::from_rgb(255, 158, 51);
    let green = egui::Color32::from_rgb(140, 230, 140);
    for object in ui.view.objects() {
        let (sin, cos) = object.rotation_degrees.to_radians().sin_cos();
        let half_height = (sin.abs() * object.length + cos.abs() * object.width) / 2.0;
        let half_width = (cos.abs() * object.length + sin.abs() * object.width) / 2.0;
        if ui.paths.weights_on && ui.paths.weigh != Weigh::Connections {
            let top = screen(Point { x: object.centre.x, y: object.centre.y + half_height });
            let text = weights.get(object.id as usize).copied().flatten().map_or("–".to_string(), |weight| ui.paths.weigh.show(weight));
            painter.text(top - egui::vec2(0.0, 2.0), egui::Align2::CENTER_BOTTOM, text, font.clone(), route);
        }
        if ui.paths.strategies_on
            && let Some(strategy) = strategies.get(object.id as usize).cloned().flatten()
        {
            let corner = screen(Point { x: object.centre.x - half_width, y: object.centre.y + half_height });
            let (mark, colour) = if strategy.weighted { ("W", green) } else { ("RR", egui::Color32::LIGHT_GRAY) };
            painter.text(corner - egui::vec2(2.0, 2.0), egui::Align2::RIGHT_BOTTOM, mark, font.clone(), colour);
            for (to, place) in strategy.places {
                if let Some(connection) = ui.view.connections().iter().find(|connection| connection.from == object.id && connection.to == to) {
                    let near = Point {
                        x: connection.start.x + (connection.end.x - connection.start.x) * 0.3,
                        y: connection.start.y + (connection.end.y - connection.start.y) * 0.3,
                    };
                    painter.text(screen(near), egui::Align2::CENTER_BOTTOM, place.to_string(), font.clone(), green);
                }
            }
        }
    }
}

/// The menu of an object right-clicked on the canvas: where the Paths tab's route goes from or to.
fn canvas_menu(ctx: &egui::Context, ui: &mut Ui) {
    let Some((id, at)) = ui.canvas_menu else { return };
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("canvas menu")).fixed_pos(at).order(egui::Order::Foreground).show(ctx, |area| {
        egui::Frame::popup(area.style()).show(area, |menu| {
            menu.label(egui::RichText::new(&ui.view.objects()[id as usize].name).strong());
            for (label, from) in [("Paths from here", true), ("Paths to here", false)] {
                if menu.button(label).clicked() {
                    if from {
                        ui.paths.from = Some(id);
                    } else {
                        ui.paths.to = Some(id);
                    }
                    ui.paths.route_on = true;
                    ui.tab = Tab::Paths;
                    close = true;
                }
            }
        });
    });
    // A left click anywhere else closes it. A right click elsewhere opens another object's menu,
    // or none, from the canvas.
    if close || (ctx.input(|input| input.pointer.primary_pressed()) && !area.response.contains_pointer()) {
        ui.canvas_menu = None;
    }
}

/// What Validate found, until closed: each problem and warning on a line, which selects its
/// instance or class when clicked. Returns the one clicked.
fn validation_window(ctx: &egui::Context, ui: &mut Ui) -> Option<crabsim_ui::workspace::Issue> {
    let found = ui.validation.as_ref()?;
    let mut open = true;
    let mut clicked = None;
    let title = format!("Validate {}", ui.workspace.opened().unwrap_or("the model"));
    egui::Window::new(title).id(egui::Id::new("validation")).open(&mut open).collapsible(false).default_width(420.0).show(ctx, |window| match found {
        Err(error) => {
            window.colored_label(egui::Color32::from_rgb(240, 120, 120), error);
        }
        Ok(issues) if issues.is_empty() => {
            window.label("No problems and no warnings.");
        }
        Ok(issues) => {
            let problems = issues.iter().filter(|issue| !issue.warning).count();
            let warnings = issues.len() - problems;
            window.label(format!(
                "{problems} problem{}, which stop the model running, and {warnings} warning{}: what can run but is probably a mistake. Click one to select what it's about.",
                plural(problems),
                plural(warnings)
            ));
            window.separator();
            egui::ScrollArea::vertical().max_height(320.0).show(window, |list| {
                for issue in issues {
                    let (kind, colour) =
                        if issue.warning { ("Warning", egui::Color32::from_rgb(230, 190, 90)) } else { ("Problem", egui::Color32::from_rgb(240, 120, 120)) };
                    list.horizontal_wrapped(|row| {
                        row.label(egui::RichText::new(kind).color(colour).strong());
                        let what = issue.name.as_deref().map_or(String::new(), |name| format!("{name}: "));
                        let line = row.add(egui::Label::new(format!("{what}{}", issue.message)).sense(egui::Sense::click())).on_hover_text(&issue.pointer);
                        if line.clicked() {
                            clicked = Some(issue.clone());
                        }
                    });
                }
            });
        }
    });
    if !open {
        ui.validation = None;
    }
    clicked
}

/// A distribution strategy's choices: Round robin or Weighted, and for Weighted what it weighs,
/// which totals come first, what it ranks, and the Drains it counts.
fn distribution_fields(ui: &mut egui::Ui, choice: &mut crabsim_ui::attributes::DistributionChoice, salt: &str) {
    ui.horizontal(|row| {
        let first = row.selectable_value(&mut choice.weighted, false, "Round robin").on_hover_text("Each successor in turn");
        focus_if_wanted(row, &first, salt);
        row.selectable_value(&mut choice.weighted, true, "Weighted")
            .on_hover_text("The best-ranked successor that can take the MU, those ranking equal taking turns");
    });
    if !choice.weighted {
        return;
    }
    egui::Grid::new(("distribution", salt)).num_columns(2).spacing([8.0, 2.0]).show(ui, |grid| {
        grid.label("by");
        let label = crabsim_ui::attributes::WEIGHABLE.iter().find(|(name, _)| *name == choice.by).map_or("?", |(_, label)| label);
        egui::ComboBox::from_id_salt(("weigh by", salt)).selected_text(label).show_ui(grid, |menu| {
            for (name, label) in crabsim_ui::attributes::WEIGHABLE {
                if menu.selectable_label(choice.by == name, label).clicked() {
                    choice.by = name.to_string();
                }
            }
        });
        grid.end_row();
        grid.label("first");
        grid.horizontal(|row| {
            row.selectable_value(&mut choice.largest, false, "Smallest");
            row.selectable_value(&mut choice.largest, true, "Largest");
        });
        grid.end_row();
        grid.label("rank by");
        grid.horizontal(|row| {
            row.selectable_value(&mut choice.successor, false, "Route").on_hover_text("Each successor's best route onward to a Drain, itself included");
            row.selectable_value(&mut choice.successor, true, "Successor").on_hover_text("Each successor's own weight only");
        });
        grid.end_row();
    });
    if !choice.successor {
        ui.label(egui::RichText::new("Drains counted, none ticked for every Drain:").weak().small());
        for (drain, ticked) in &mut choice.drains {
            ui.checkbox(ticked, drain.as_str());
        }
    }
}

/// Each object's name under it, and counts of MUs where they aren't drawn, `text_size` pixels
/// tall, clipped to the canvas.
fn labels(ctx: &egui::Context, view: &ViewState, camera: &ViewCamera, canvas: egui::Rect, text_size: f32) {
    // Under every window, such as the help on states, as the canvas itself is.
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("labels"))).with_clip_rect(canvas);
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
