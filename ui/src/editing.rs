//! The edit queue as the Editing tab shows it: who may change each model file, who waits, and the
//! blocked agents, read with `get_edit_queue`, and the user's controls over it (ADR 0011). Every
//! control is checked against the latest queue before it acts: see `Workspace::act`. Plain Rust
//! with no egui types, so it is tested without a window. See spec issue 63.

use serde_json::Value;

/// An agent, as the queue shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    /// Such as `agent-3`.
    pub agent: String,
    /// The name its harness gave, such as `claude-code`.
    pub client: String,
    pub purpose: String,
    /// Since when it edits or waits, UTC.
    pub since: String,
}

/// Who may change a model file now.
#[derive(Debug, Clone, PartialEq)]
pub enum Holder {
    User { since: String },
    Agent(Agent),
}

/// One model file's editor and the agents waiting for it, in the order they'll get it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileQueue {
    pub path: String,
    pub editor: Option<Holder>,
    pub queue: Vec<Agent>,
}

/// The whole folder's edit queue: every file someone edits or waits for, and the blocked agents.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Queue {
    pub files: Vec<FileQueue>,
    /// Each blocked agent and its harness.
    pub blocked: Vec<(String, String)>,
}

/// One of the user's controls, on the open model file or an agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAction {
    /// Takes the right to change the open model, from any agent, which keeps its place first.
    Take,
    /// Gives the right back, to the first waiting.
    Release,
    /// Gives the right to a waiting agent now.
    Give(String),
    /// Moves a waiting agent one place nearer the front.
    Up(String),
    /// Moves a waiting agent one place further back.
    Down(String),
    /// Takes an agent out of the queue, or its right away if it edits.
    Remove(String),
    /// Blocks an agent session from changing models in the folder.
    Block(String),
    Unblock(String),
}

/// Why a control did nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum ActRefusal {
    /// The queue changed since it was shown, so nothing was done. Here is the queue now, and
    /// what changed, to decide again.
    Changed { now: Queue, what: String },
    /// The operation refused it, or it can't apply, with why.
    Refused(String),
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn agent(value: &Value) -> Agent {
    Agent { agent: text(&value["agent"]), client: text(&value["client"]), purpose: text(&value["purpose"]), since: text(&value["since"]) }
}

impl Queue {
    /// The queue as `get_edit_queue` gives it without a path.
    pub fn from_json(value: &Value) -> Queue {
        let files = value["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|file| FileQueue {
                path: text(&file["path"]),
                editor: match &file["editor"] {
                    Value::Null => None,
                    editor if editor["user"] == true => Some(Holder::User { since: text(&editor["since"]) }),
                    editor => Some(Holder::Agent(agent(editor))),
                },
                queue: file["queue"].as_array().into_iter().flatten().map(agent).collect(),
            })
            .collect();
        let blocked = value["blocked"].as_array().into_iter().flatten().map(|blocked| (text(&blocked["agent"]), text(&blocked["client"]))).collect();
        Queue { files, blocked }
    }

    /// The editor and queue of `path`, empty if nobody edits or waits for it.
    pub fn file(&self, path: &str) -> FileQueue {
        self.files.iter().find(|file| file.path == path).cloned().unwrap_or_else(|| FileQueue { path: path.to_string(), ..FileQueue::default() })
    }

    /// The other files someone edits or waits for.
    pub fn others(&self, path: &str) -> Vec<&FileQueue> {
        self.files.iter().filter(|file| file.path != path).collect()
    }

    /// The agent holding the right on `path`, if an agent does, for asking before taking it.
    pub fn agent_holding(&self, path: &str) -> Option<&Agent> {
        match self.files.iter().find(|file| file.path == path)?.editor.as_ref()? {
            Holder::Agent(agent) => Some(agent),
            Holder::User { .. } => None,
        }
    }

    /// What changed on `path`, and in the blocked agents, between `self` and `now`, as one
    /// sentence, or none if nothing did.
    pub fn changes_on(&self, path: &str, now: &Queue) -> Option<String> {
        let (before, after) = (self.file(path), now.file(path));
        if before.editor != after.editor {
            let who = match &after.editor {
                None => "Nobody holds the right to change it now".to_string(),
                Some(Holder::User { .. }) => "You hold the right to change it now".to_string(),
                Some(Holder::Agent(agent)) => format!("{} ({}) holds the right to change it now, {}", agent.agent, agent.client, agent.purpose),
            };
            return Some(format!("The queue changed a moment ago: {who}."));
        }
        if before.queue != after.queue {
            let waiting: Vec<String> = after.queue.iter().map(|agent| agent.agent.clone()).collect();
            let waiting = if waiting.is_empty() { "nobody".to_string() } else { waiting.join(", ") };
            return Some(format!("The queue changed a moment ago: waiting now are {waiting}."));
        }
        if self.blocked != now.blocked {
            return Some("The blocked agents changed a moment ago.".to_string());
        }
        None
    }
}
