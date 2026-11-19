//! What an object window keeps about its instance, in plain Rust with no Bevy or egui types so it
//! can be tested without a window (spec issue 116). The docked panel's summary of the selection
//! uses the same module, as the window with the fewest sections.
//!
//! An [`Inspector`] belongs to one instance, named by its stream, which a model file keeps
//! through every edit (ADR 0012): so it follows renames, moves and the ids shifting when another
//! instance is removed, and goes when its own instance does. It owns the instance's forms, typed
//! texts, last refusal and which sections the user opened or closed. [`Inspectors`] is the set of
//! open windows.

use crate::attributes::Form;
use crate::view::ViewState;
use crate::workspace::{AttributeView, Refusal, Workspace};
use std::collections::HashMap;

/// A change to one attribute of an instance, asked for in its inspector.
#[derive(Clone, Debug, PartialEq)]
pub enum AttributeChange {
    Set(String, serde_json::Value),
    Restore(String),
}

impl AttributeChange {
    pub fn attribute(&self) -> &str {
        match self {
            AttributeChange::Set(attribute, _) | AttributeChange::Restore(attribute) => attribute,
        }
    }
}

/// The last change refused for an attribute: which, why, and the change to make anyway when an
/// agent holds the file.
pub type Refused = (String, Refusal, Option<AttributeChange>);

/// The parts of an object window, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    /// What each state its type can be in means, with the model's own notes.
    States,
    /// The MUs held, work left and waiting predecessors.
    Now,
    /// Pause, forced failure and process time, during a run.
    Interventions,
    Statistics,
    Attributes,
    /// Rename, Remove and Make a class.
    Actions,
}

impl Section {
    pub const ALL: [Section; 6] =
        [Section::States, Section::Now, Section::Interventions, Section::Statistics, Section::Attributes, Section::Actions];

    pub fn title(self) -> &'static str {
        match self {
            Section::States => "States",
            Section::Now => "Now",
            Section::Interventions => "Interventions",
            Section::Statistics => "Statistics",
            Section::Attributes => "Attributes",
            Section::Actions => "Actions",
        }
    }

    /// The section a field is in, by the name the keyboard reaches it by: `name`, the
    /// interventions, or an attribute.
    pub fn of_field(field: &str) -> Section {
        match field {
            "name" => Section::Actions,
            "pause" | "forced_failure" | "process_time" => Section::Interventions,
            _ => Section::Attributes,
        }
    }

    /// The name a window's field is marked by when the keyboard is sent there, so a field in one
    /// window isn't taken for the same field in another.
    pub fn focus_key(stream: u64, field: &str) -> String {
        format!("{stream}:{field}")
    }

    /// Whether the section is open when the user hasn't chosen: Attributes before Init and after
    /// Reset, Statistics and Interventions during a run, what is happening now always, and the
    /// help on states only when asked for.
    pub fn open_by_default(self, running: bool) -> bool {
        match self {
            Section::States => false,
            Section::Now => true,
            Section::Interventions | Section::Statistics => running,
            Section::Attributes | Section::Actions => !running,
        }
    }
}

/// What a window shows of one instance.
#[derive(Debug)]
pub struct Inspector {
    stream: u64,
    /// What has been typed into the process time field.
    pub process_time_text: String,
    /// What has been typed into the name field, and the name it was made for, so a rename from
    /// anywhere else replaces it.
    pub name_text: (String, String),
    /// The instance's attributes as last read from the model file. `None` reads them again.
    pub attributes: Option<Result<Vec<AttributeView>, String>>,
    /// The fields for each attribute, by attribute.
    pub forms: HashMap<String, Form>,
    /// The last change this window asked for that was refused.
    pub refused: Option<Refused>,
    /// Sections the user opened or closed, which stay so.
    chosen: HashMap<Section, bool>,
}

impl Inspector {
    pub fn new(stream: u64) -> Inspector {
        Inspector {
            stream,
            process_time_text: String::new(),
            name_text: (String::new(), String::new()),
            attributes: None,
            forms: HashMap::new(),
            refused: None,
            chosen: HashMap::new(),
        }
    }

    /// The instance's stream: what names it.
    pub fn stream(&self) -> u64 {
        self.stream
    }

    /// Whether `section` is shown open: as the user left it, else by the situation.
    pub fn is_open(&self, section: Section, running: bool) -> bool {
        self.chosen.get(&section).copied().unwrap_or_else(|| section.open_by_default(running))
    }

    /// Opens or closes a section, and keeps it so.
    pub fn set_open(&mut self, section: Section, open: bool) {
        self.chosen.insert(section, open);
    }

    /// Makes a section open for now, such as when a field in it is asked for, without it
    /// counting as chosen by the user.
    pub fn show(&mut self, section: Section) {
        self.chosen.insert(section, true);
    }

    /// Reads the instance's attributes and makes a form for each, when not read yet. `None` for
    /// `attributes` after anything changed the model makes this read them again.
    pub fn read(&mut self, workspace: &Workspace, name: &str, built_in: &str) {
        if self.attributes.is_some() {
            return;
        }
        let read = if workspace.opened().is_some() { workspace.attributes(name) } else { Err("No model file is open".to_string()) };
        if let Ok(attributes) = &read {
            let states = crabsim_model::State::of(built_in);
            let drains = workspace.drains();
            self.forms = attributes
                .iter()
                .map(|view| (view.attribute.clone(), Form::for_attribute(&view.attribute, view.value.as_ref(), states, &drains)))
                .collect();
        }
        self.attributes = Some(read);
        self.refused = None;
    }

    /// Makes the window read its values again, such as after the model changed.
    pub fn forget_values(&mut self) {
        self.attributes = None;
        self.forms.clear();
        self.refused = None;
    }

    /// Keeps the name field in step with the instance's name: what was typed stays while the
    /// name is the one it was typed for, and is replaced when the name changed.
    pub fn sync_name(&mut self, name: &str) {
        if self.name_text.0 != name {
            self.name_text = (name.to_string(), name.to_string());
        }
    }

    /// Notes a change refused, to show at its field.
    pub fn refuse(&mut self, change: &AttributeChange, refusal: Refusal) {
        let again = matches!(refusal, Refusal::NotEditor(_)).then(|| change.clone());
        self.refused = Some((change.attribute().to_string(), refusal, again));
    }
}

/// The object windows that are open, the last opened or brought forward last.
#[derive(Debug, Default)]
pub struct Inspectors {
    windows: Vec<Inspector>,
    /// The stream to bring to the front, when it was asked for.
    raise: Option<u64>,
}

impl Inspectors {
    /// Opens a window on the instance with this stream, or brings forward the one already open.
    /// Returns whether it was newly opened.
    pub fn open(&mut self, stream: u64) -> bool {
        self.raise = Some(stream);
        if self.windows.iter().any(|window| window.stream == stream) {
            return false;
        }
        self.windows.push(Inspector::new(stream));
        true
    }

    pub fn close(&mut self, stream: u64) {
        self.windows.retain(|window| window.stream != stream);
    }

    pub fn close_all(&mut self) {
        self.windows.clear();
        self.raise = None;
    }

    pub fn is_open(&self, stream: u64) -> bool {
        self.windows.iter().any(|window| window.stream == stream)
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn get(&self, stream: u64) -> Option<&Inspector> {
        self.windows.iter().find(|window| window.stream == stream)
    }

    pub fn get_mut(&mut self, stream: u64) -> Option<&mut Inspector> {
        self.windows.iter_mut().find(|window| window.stream == stream)
    }

    /// The streams of the open windows, in the order they were opened.
    pub fn streams(&self) -> Vec<u64> {
        self.windows.iter().map(|window| window.stream).collect()
    }

    /// Takes the stream asked to come forward, once.
    pub fn take_raise(&mut self) -> Option<u64> {
        self.raise.take()
    }

    /// Closes the windows whose instance the view's model doesn't have any more, and returns the
    /// ids of the others, for the view to watch.
    pub fn follow(&mut self, view: &ViewState) -> Vec<u32> {
        self.windows.retain(|window| view.id_of_stream(window.stream).is_some());
        self.windows.iter().filter_map(|window| view.id_of_stream(window.stream)).collect()
    }

    /// Every window reads its values again, as after the model changed.
    pub fn forget_values(&mut self) {
        for window in &mut self.windows {
            window.forget_values();
        }
    }

    /// Where the next window opens, stepped down and to the left of the one before: its offset
    /// from the canvas's top right, in logical pixels.
    pub fn stagger(&self, index: usize) -> (f32, f32) {
        let step = (index % 8) as f32;
        (-24.0 * step, 28.0 * step)
    }
}
