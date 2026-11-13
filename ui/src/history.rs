//! The open model's change history as the window shows it, and what undo and redo would do (spec
//! issue 107): which changes a number of steps takes back or brings back, which of them the user
//! is asked about first, how far a click on an entry goes, and what a step touched, for selecting
//! it. Plain Rust with no egui types, so it is tested without a window.

use crabsim_model::Model;
use serde_json::Value;

/// What kind of change an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A batch of changes.
    Change,
    /// The whole file written over.
    Written,
    /// A change made outside crabsim, adopted.
    Adopted,
}

/// One change in the history.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub number: i64,
    /// `the user`, or an agent such as `agent-3 (claude-code)`.
    pub author: String,
    /// When, UTC.
    pub at: String,
    /// What it did, in the command language.
    pub what: String,
    pub kind: Kind,
    /// Entries of one group were one command, and are one step.
    pub group: i64,
    /// It was undone and can be redone.
    pub undone: bool,
    /// The user made it.
    pub yours: bool,
}

impl Entry {
    /// Whether undoing it asks first: someone else's change, or one from outside crabsim, which
    /// crabsim can't describe.
    pub fn asks(&self) -> bool {
        !self.yours || self.kind == Kind::Adopted
    }

    /// The entry on one line, as a dialog names it: what it did, who and when.
    pub fn line(&self) -> String {
        format!("{} ({}, {})", self.what, self.author, self.at)
    }
}

/// Where a click on an entry goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    /// This many steps of undo, so the entry is the latest in effect.
    Undo(usize),
    /// This many steps of redo, so the entry is in effect again.
    Redo(usize),
    /// The model is already as it was after the entry.
    Here,
}

/// A model file's changes, newest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct History {
    pub entries: Vec<Entry>,
    /// There are older entries than these.
    pub more: bool,
}

/// The entries of the first `steps` groups of `entries`, in their order.
fn steps_of<'e>(entries: impl Iterator<Item = &'e Entry>, steps: usize) -> Vec<&'e Entry> {
    let mut taken = Vec::new();
    let mut groups = Vec::new();
    for entry in entries {
        if !groups.contains(&entry.group) {
            if groups.len() == steps {
                break;
            }
            groups.push(entry.group);
        }
        taken.push(entry);
    }
    taken
}

impl History {
    /// The history as `get_history` gives it.
    pub fn from_json(value: &Value) -> History {
        let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
        let entries = value["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| Entry {
                number: entry["number"].as_i64().unwrap_or_default(),
                author: text(&entry["author"]),
                at: text(&entry["at"]),
                what: text(&entry["what"]),
                kind: match entry["kind"].as_str() {
                    Some("written") => Kind::Written,
                    Some("adopted") => Kind::Adopted,
                    _ => Kind::Change,
                },
                group: entry["group"].as_i64().unwrap_or_default(),
                undone: entry["undone"] == true,
                yours: entry["yours"] == true,
            })
            .collect();
        History { entries, more: value["more"] == true }
    }

    /// The changes `steps` steps of undo would take back, newest first. A step is every entry of
    /// one group.
    pub fn undo_steps(&self, steps: usize) -> Vec<&Entry> {
        steps_of(self.entries.iter().filter(|entry| !entry.undone), steps)
    }

    /// The changes `steps` steps of redo would bring back, oldest first.
    pub fn redo_steps(&self, steps: usize) -> Vec<&Entry> {
        steps_of(self.entries.iter().rev().filter(|entry| entry.undone), steps)
    }

    /// How to make the model as it was after the entry `number`: undo every step after it, or
    /// redo up to it and the rest of its step. None if the entry isn't among those shown.
    pub fn jump_to(&self, number: i64) -> Option<Jump> {
        let entry = self.entries.iter().find(|entry| entry.number == number)?;
        let groups = |entries: &mut dyn Iterator<Item = &Entry>| {
            let mut groups: Vec<i64> = entries.map(|entry| entry.group).collect();
            groups.dedup();
            groups.len()
        };
        Some(if entry.undone {
            Jump::Redo(groups(&mut self.entries.iter().rev().filter(|other| other.undone && (other.number <= number || other.group == entry.group))))
        } else {
            match groups(&mut self.entries.iter().filter(|other| !other.undone && other.number > number && other.group != entry.group)) {
                0 => Jump::Here,
                steps => Jump::Undo(steps),
            }
        })
    }
}

/// What a step did, for the line at the foot of the canvas: `Undone: connect A > B`, or with
/// several changes how many, and the last of them.
pub fn summary(done: &str, entries: &[Entry]) -> String {
    match entries {
        [] => format!("Nothing was {}", done.to_lowercase()),
        [only] => format!("{done}: {}", only.what),
        [.., last] => format!("{done} {} changes, the last: {}", entries.len(), last.what),
    }
}

/// The instances a step touched, by name, as they are in `after`: those new or changed against
/// `before`, and those at an end of a connection that is new, gone or changed. Empty when the
/// step only removed instances.
pub fn touched(before: &Model, after: &Model) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if after.instances.iter().any(|instance| instance.name == name) && !names.iter().any(|known| known == name) {
            names.push(name.to_string());
        }
    };
    for instance in &after.instances {
        if before.instances.iter().find(|other| other.name == instance.name) != Some(instance) {
            add(&instance.name);
        }
    }
    let differing = |from: &Model, against: &Model| -> Vec<(String, String)> {
        from.connections.iter().filter(|connection| !against.connections.contains(connection)).map(|connection| (connection.from.clone(), connection.to.clone())).collect()
    };
    for (from, to) in differing(after, before).into_iter().chain(differing(before, after)) {
        add(&from);
        add(&to);
    }
    // In model order, as the selection is everywhere else.
    names.sort_by_key(|name| after.instances.iter().position(|instance| &instance.name == name));
    names
}
