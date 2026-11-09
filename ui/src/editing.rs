//! Who may change each model file as the Editing tab shows it: the user's hold, the agents' claims
//! with their goals and what they're doing, and the blocked agents, read with `get_editors`, and
//! the user's controls (ADR 0013). Every control is checked against the latest state before it
//! acts: see `Workspace::act`. Plain Rust with no egui types, so it is tested without a window.

use serde_json::Value;

/// An agent's claim on a model file.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// Such as `agent-3`.
    pub agent: String,
    /// The name its harness gave, such as `claude-code`.
    pub client: String,
    pub goal: String,
    pub doing: String,
    /// What it has been doing and since when, UTC, newest first.
    pub trail: Vec<(String, String)>,
    /// Since when it holds the file, UTC.
    pub since: String,
    /// When it last changed the file or claimed it, UTC.
    pub last_active: String,
    /// The user has taken the file, so the agent waits until the user releases it.
    pub paused: bool,
}

/// Who holds one model file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileEditors {
    pub path: String,
    /// Since when the user holds it, UTC, while the user does.
    pub user_since: Option<String>,
    pub claim: Option<Claim>,
}

/// Every model file someone holds, and the blocked agents.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Editors {
    pub files: Vec<FileEditors>,
    /// Each blocked agent and its harness.
    pub blocked: Vec<(String, String)>,
}

/// One of the user's controls, on the open model file or an agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditAction {
    /// Takes the open model, pausing an agent's claim on it.
    Take,
    /// Releases it, giving a paused claim back.
    Release,
    /// Ends the agent's claim on this model file, the open one or another.
    Revoke(String),
    /// Blocks an agent session from changing models in the folder.
    Block(String),
    Unblock(String),
}

/// Why a control did nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum ActRefusal {
    /// Who holds the model acted on changed since it was shown, so nothing was done. Here is the
    /// state now, and what changed, to decide again.
    Changed { now: Editors, what: String },
    /// The operation refused it, or it can't apply, with why.
    Refused(String),
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn claim(value: &Value) -> Claim {
    Claim {
        agent: text(&value["agent"]),
        client: text(&value["client"]),
        goal: text(&value["goal"]),
        doing: text(&value["doing"]),
        trail: value["trail"].as_array().into_iter().flatten().map(|entry| (text(&entry["doing"]), text(&entry["since"]))).collect(),
        since: text(&value["since"]),
        last_active: text(&value["last_active"]),
        paused: value["paused"] == true,
    }
}

impl Editors {
    /// The state as `get_editors` gives it.
    pub fn from_json(value: &Value) -> Editors {
        let files = value["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|file| FileEditors {
                path: text(&file["path"]),
                user_since: file["user_since"].as_str().map(str::to_string),
                claim: file["claim"].is_object().then(|| claim(&file["claim"])),
            })
            .collect();
        let blocked = value["blocked"].as_array().into_iter().flatten().map(|blocked| (text(&blocked["agent"]), text(&blocked["client"]))).collect();
        Editors { files, blocked }
    }

    /// Who holds `path`, nobody if nobody does.
    pub fn file(&self, path: &str) -> FileEditors {
        self.files.iter().find(|file| file.path == path).cloned().unwrap_or_else(|| FileEditors { path: path.to_string(), ..FileEditors::default() })
    }

    /// The other files someone holds.
    pub fn others(&self, path: &str) -> Vec<&FileEditors> {
        self.files.iter().filter(|file| file.path != path).collect()
    }

    /// The claim of the agent changing `path` now, unpaused, for asking before taking it.
    pub fn agent_holding(&self, path: &str) -> Option<&Claim> {
        self.files.iter().find(|file| file.path == path)?.claim.as_ref().filter(|claim| !claim.paused)
    }

    /// The line under the top bar while an agent has a claim on the open model `path`: who it is
    /// and what it's doing, or that it's paused while the user holds the model. None otherwise.
    pub fn marker(&self, path: &str) -> Option<String> {
        let claim = self.files.iter().find(|file| file.path == path)?.claim.as_ref()?;
        let who = format!("{} ({})", claim.agent, claim.client);
        Some(match (claim.paused, claim.doing.is_empty()) {
            (false, false) => format!("{who} is editing this model: {}", claim.doing),
            (false, true) => format!("{who} is editing this model"),
            (true, false) => format!("{who} is paused while you hold this model. It was: {}", claim.doing),
            (true, true) => format!("{who} is paused while you hold this model"),
        })
    }

    /// What changed in who holds `path`, and in the blocked agents, between `self` and `now`, as
    /// one sentence, or none if nothing did. An agent saying what it does now isn't a change.
    /// `opened` is the open model, which the sentence calls "this model".
    pub fn changes_on(&self, path: &str, opened: Option<&str>, now: &Editors) -> Option<String> {
        let (before, after) = (self.file(path), now.file(path));
        let holder = |file: &FileEditors| (file.user_since.is_some(), file.claim.as_ref().map(|claim| (claim.agent.clone(), claim.paused)));
        if holder(&before) != holder(&after) {
            let who = match (&after.user_since, &after.claim) {
                (Some(_), _) => "You hold it now".to_string(),
                (None, Some(claim)) => format!("{} ({}) holds it now, with the goal \"{}\"", claim.agent, claim.client, claim.goal),
                (None, None) => "Nobody holds it now".to_string(),
            };
            let model = if opened == Some(path) { "this model" } else { path };
            return Some(format!("Who holds {model} changed a moment ago: {who}."));
        }
        if self.blocked != now.blocked {
            return Some("The blocked agents changed a moment ago.".to_string());
        }
        None
    }
}
