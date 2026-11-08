//! Who may change a model file: one editor at a time, and an edit queue of agents waiting for it,
//! which the user controls (ADR 0011). The state is shared by every crabsim process on the models
//! folder, in `.crabsim/editing.json`, read and written only while holding an OS lock on
//! `.crabsim/editing.lock`. See `docs/design/operation-catalogue.md`.
//!
//! One agent is one crabsim process opened as an MCP server, known by its process lock (see
//! `process.rs`), so an agent whose process ended is noticed the next time anyone looks.

use crate::error::{ErrorKind, OperationError};
use crate::{entry, process, Catalogue, Effect, Entry, Operation, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn entries() -> Vec<Entry> {
    vec![
        entry::<AdoptModel>(),
        entry::<ClaimModel>(),
        entry::<ReleaseModel>(),
        entry::<GetEditQueue>(),
        entry::<TakeModel>(),
        entry::<GiveModel>(),
        entry::<MoveInQueue>(),
        entry::<RemoveFromQueue>(),
        entry::<BlockAgent>(),
        entry::<UnblockAgent>(),
    ]
}

/// The edit state's file in `.crabsim`.
pub const EDITING_FILE: &str = "editing.json";
const LOCK_FILE: &str = "editing.lock";

/// How long an agent's right, or its place in the queue, lasts without it changing the file or
/// asking again, unless set when the server or the CLI starts.
pub const EDIT_TIMEOUT: Duration = Duration::from_secs(600);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as u64)
}

/// A time as UTC ISO 8601, such as `2026-11-08T09:12:03Z`.
fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

// ---------- The state ----------

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    version: u32,
    /// The number the next agent's id gets.
    next_agent: u64,
    /// Every agent seen and still alive, by its process lock id.
    agents: BTreeMap<String, Agent>,
    /// Every model file someone edits or waits for, by its path in the models folder.
    files: BTreeMap<String, FileState>,
    /// The hash of each model file as crabsim last saved or adopted it, by its path. A file whose
    /// hash differs, or that has none here, was changed outside crabsim.
    #[serde(default)]
    records: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Agent {
    /// Such as `agent-3`, what the user names it by.
    id: String,
    /// The name its harness gave, such as `claude-code`.
    client: String,
    started: u64,
    blocked: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileState {
    editor: Option<Editor>,
    queue: Vec<Waiting>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Editor {
    /// The agent's process lock id, or none for the user.
    process: Option<String>,
    purpose: Option<String>,
    since: u64,
    last_active: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Waiting {
    process: String,
    purpose: String,
    asked: u64,
    last_asked: u64,
}

/// The edit state, held under its lock until dropped, which writes it if it changed.
pub(crate) struct Held {
    _lock: File,
    path: PathBuf,
    state: State,
    changed: bool,
}

impl Held {
    fn open(crabsim_folder: &Path) -> Result<Held, OperationError> {
        let fail = |what: &str, error: std::io::Error| OperationError::run_error(format!("the edit state can't be {what}: {error}"));
        std::fs::create_dir_all(crabsim_folder).map_err(|error| fail("made", error))?;
        let lock = File::options().create(true).truncate(false).read(true).write(true).open(crabsim_folder.join(LOCK_FILE)).map_err(|error| fail("locked", error))?;
        lock.lock().map_err(|error| fail("locked", error))?;
        let path = crabsim_folder.join(EDITING_FILE);
        let state = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => State::default(),
        };
        Ok(Held { _lock: lock, path, state, changed: false })
    }

    fn write(&mut self) -> Result<(), OperationError> {
        if !self.changed {
            return Ok(());
        }
        self.state.version = 1;
        let text = serde_json::to_string_pretty(&self.state).expect("the edit state serialises");
        let temporary = self.path.with_extension("json.new");
        std::fs::write(&temporary, text)
            .and_then(|()| std::fs::rename(&temporary, &self.path))
            .map_err(|error| OperationError::run_error(format!("the edit state can't be written: {error}")))?;
        self.changed = false;
        Ok(())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.write();
    }
}

/// Who is asking: an agent, by its process lock id, or the user.
#[derive(Debug, Clone, PartialEq)]
enum Asker {
    Agent(String),
    User,
}

/// The edit state for one call: swept of ended agents, lost rights and forgotten places, with
/// rights passed on, and the caller known.
pub(crate) struct Editing {
    held: Held,
    asker: Asker,
    now: u64,
    /// The model file a change is about, once checked.
    key: Option<String>,
    /// The user changes past an agent's right, for one command.
    take_over: bool,
}

impl Catalogue {
    /// The edit state for one call, held until the call is done.
    pub(crate) fn editing(&self) -> Result<Editing, OperationError> {
        let process = self.process_lock()?.to_string();
        let crabsim_folder = self.folder().crabsim_folder();
        let mut held = Held::open(&crabsim_folder)?;
        let now = now_ms();
        let asker = match self.source() {
            Source::Mcp => {
                let client = self.client_name();
                let state = &mut held.state;
                if !state.agents.contains_key(&process) {
                    state.next_agent += 1;
                    let id = format!("agent-{}", state.next_agent);
                    state.agents.insert(process.clone(), Agent { id, client, started: now, blocked: false });
                    held.changed = true;
                } else if let Some(agent) = state.agents.get_mut(&process).filter(|agent| agent.client != client) {
                    agent.client = client;
                    held.changed = true;
                }
                Asker::Agent(process)
            }
            Source::Cli => Asker::User,
        };
        let mut editing = Editing { held, asker, now, key: None, take_over: self.take_over() };
        editing.sweep(&crabsim_folder, self.edit_timeout());
        Ok(editing)
    }
}

impl Editing {
    fn state(&mut self) -> &mut State {
        self.held.changed = true;
        &mut self.held.state
    }

    /// Drops agents whose process ended, rights unused for longer than `timeout` and places not
    /// asked for in as long, then passes each free right to the first waiting agent.
    fn sweep(&mut self, crabsim_folder: &Path, timeout: Duration) {
        let own = match &self.asker {
            Asker::Agent(process) => Some(process.clone()),
            Asker::User => None,
        };
        let ended: Vec<String> = self.held.state.agents.keys().filter(|process| Some(*process) != own.as_ref() && !process::is_alive(crabsim_folder, process)).cloned().collect();
        let now = self.now;
        let timeout = timeout.as_millis() as u64;
        let before = serde_json::to_string(&self.held.state.files).unwrap_or_default();
        let state = &mut self.held.state;
        for process in &ended {
            state.agents.remove(process);
        }
        let gone = |process: &str, state_agents: &BTreeMap<String, Agent>| state_agents.get(process).is_none_or(|agent| agent.blocked);
        let agents = &state.agents;
        for file in state.files.values_mut() {
            if let Some(editor) = &file.editor
                && let Some(process) = &editor.process
                && (gone(process, agents) || now.saturating_sub(editor.last_active) > timeout)
            {
                file.editor = None;
            }
            file.queue.retain(|waiting| !gone(&waiting.process, agents) && now.saturating_sub(waiting.last_asked) <= timeout);
            if file.editor.is_none() && !file.queue.is_empty() {
                let next = file.queue.remove(0);
                file.editor = Some(Editor { process: Some(next.process), purpose: Some(next.purpose), since: now, last_active: now });
            }
        }
        state.files.retain(|_, file| file.editor.is_some() || !file.queue.is_empty());
        if !ended.is_empty() || serde_json::to_string(&state.files).unwrap_or_default() != before {
            self.held.changed = true;
        }
    }

    fn agent(&self, process: &str) -> Option<&Agent> {
        self.held.state.agents.get(process)
    }

    /// The process of the agent with this id.
    fn process_of(&self, id: &str) -> Result<String, OperationError> {
        self.held
            .state
            .agents
            .iter()
            .find(|(_, agent)| agent.id == id)
            .map(|(process, _)| process.clone())
            .ok_or_else(|| OperationError::not_found(format!("there is no agent `{id}` on this models folder now")))
    }

    /// How an editor is named in messages.
    fn describe(&self, editor: &Editor) -> String {
        match &editor.process {
            None => "the user".to_string(),
            Some(process) => match self.agent(process) {
                Some(agent) => format!(
                    "{} ({}, since {}, \"{}\")",
                    agent.id,
                    agent.client,
                    iso(editor.since),
                    editor.purpose.as_deref().unwrap_or("")
                ),
                None => "an agent that has ended".to_string(),
            },
        }
    }

    fn refuse_if_blocked(&self) -> Result<(), OperationError> {
        if let Asker::Agent(process) = &self.asker
            && self.agent(process).is_some_and(|agent| agent.blocked)
        {
            return Err(OperationError::new(
                ErrorKind::Blocked,
                "the user has blocked this agent from changing models in this folder: waiting won't help, so stop and tell your user",
            ));
        }
        Ok(())
    }

    /// Whether the asker may change `path` now. An editor's change renews its right.
    pub(crate) fn may_change(&mut self, path: &str) -> Result<(), OperationError> {
        self.refuse_if_blocked()?;
        let asker = self.asker.clone();
        let now = self.now;
        let Some(file) = self.held.state.files.get(path) else { return Ok(()) };
        let Some(editor) = file.editor.clone() else { return Ok(()) };
        let waiting = file.queue.len();
        let mine = match (&asker, &editor.process) {
            (Asker::User, None) => true,
            (Asker::Agent(me), Some(process)) => me == process,
            _ => false,
        };
        if self.take_over && asker == Asker::User {
            // The agent keeps its right: the user borrows it for this change only.
            return Ok(());
        }
        if mine {
            if let Some(editor) = self.state().files.get_mut(path).and_then(|file| file.editor.as_mut()) {
                editor.last_active = now;
            }
            return Ok(());
        }
        let who = self.describe(&editor);
        let message = match asker {
            Asker::User => format!("`{path}` is being changed by {who}: take the editor right first with take_model"),
            Asker::Agent(_) => {
                let took = if editor.process.is_none() { ", who took the editor right" } else { "" };
                format!("`{path}` is being changed by {who}{took}, with {waiting} waiting: call claim_model to wait your turn")
            }
        };
        Err(OperationError::new(ErrorKind::NotEditor, message))
    }

    fn file(&mut self, path: &str) -> &mut FileState {
        self.state().files.entry(path.to_string()).or_default()
    }

    /// Hands a free right to the first waiting, and forgets a file nobody holds or waits for.
    fn settle(&mut self, path: &str) {
        let now = self.now;
        let state = self.state();
        if let Some(file) = state.files.get_mut(path) {
            if file.editor.is_none() && !file.queue.is_empty() {
                let next = file.queue.remove(0);
                file.editor = Some(Editor { process: Some(next.process), purpose: Some(next.purpose), since: now, last_active: now });
            }
            if file.editor.is_none() && file.queue.is_empty() {
                state.files.remove(path);
            }
        }
    }

    fn view(&self, path: &str) -> FileView {
        let empty = FileState::default();
        let file = self.held.state.files.get(path).unwrap_or(&empty);
        let agent_view = |process: &str, purpose: &str, since: u64| {
            let agent = self.agent(process);
            AgentView {
                agent: agent.map_or_else(|| "ended".to_string(), |agent| agent.id.clone()),
                client: agent.map_or_else(String::new, |agent| agent.client.clone()),
                started: agent.map_or_else(String::new, |agent| iso(agent.started)),
                purpose: purpose.to_string(),
                since: iso(since),
            }
        };
        FileView {
            path: path.to_string(),
            editor: file.editor.as_ref().map(|editor| match &editor.process {
                None => EditorView { user: true, agent: None, purpose: editor.purpose.clone(), since: iso(editor.since) },
                Some(process) => {
                    let view = agent_view(process, editor.purpose.as_deref().unwrap_or(""), editor.since);
                    EditorView { user: false, agent: Some(view), purpose: editor.purpose.clone(), since: iso(editor.since) }
                }
            }),
            queue: file.queue.iter().map(|waiting| agent_view(&waiting.process, &waiting.purpose, waiting.asked)).collect(),
        }
    }
}

impl Catalogue {
    /// Who is using this catalogue, as runs record it: such as `agent-3 (claude-code)`, or the user.
    pub(crate) fn who(&self) -> Result<String, OperationError> {
        let editing = self.editing()?;
        Ok(match &editing.asker {
            Asker::User => "the user".to_string(),
            Asker::Agent(process) => editing.agent(process).map_or_else(|| "an agent".to_string(), |agent| format!("{} ({})", agent.id, agent.client)),
        })
    }
}

/// The key of a model file in the edit state: its path in the models folder, as listed.
fn key(catalogue: &Catalogue, path: &str) -> Result<String, OperationError> {
    let file = catalogue.folder().model_file(path)?;
    Ok(catalogue.folder().name_of(&file))
}

/// Before an operation changes the model file at `path`: refused unless the caller may change it
/// now. The returned state is held until the change is written, so two changes nobody claimed
/// can't both go through.
pub(crate) fn before_change(catalogue: &Catalogue, path: &str, expected_hash: Option<&str>) -> Result<Editing, OperationError> {
    let editing = before_adopt(catalogue, path)?;
    let key = editing.key.clone().expect("set before adopting");
    let current = std::fs::read_to_string(catalogue.folder().model_file(path)?).ok();
    if let Some(text) = &current
        && !editing.recorded(&key, text)
    {
        return Err(OperationError::new(
            ErrorKind::ChangedOutside,
            format!(
                "`{path}` was changed outside crabsim since crabsim last saved it, or crabsim never saved it, so crabsim won't change it: look at it with read_model, then accept it as it is with adopt_model"
            ),
        ));
    }
    if let Some(expected) = expected_hash {
        let now = current.as_deref().map(crate::edit::hash);
        if now.as_deref() != Some(expected) {
            let what = now.map_or_else(|| "doesn't exist".to_string(), |hash| format!("has the hash {hash} now"));
            return Err(OperationError::new(
                ErrorKind::Conflict,
                format!("`{path}` isn't the version with the hash {expected} any more: it {what}. Read it again and decide on your change from that"),
            ));
        }
    }
    Ok(editing)
}

/// Before an operation that changes who owns a file's content but not the content, such as
/// adopting it: only the editor rules.
pub(crate) fn before_adopt(catalogue: &Catalogue, path: &str) -> Result<Editing, OperationError> {
    let key = key(catalogue, path)?;
    let mut editing = catalogue.editing()?;
    editing.may_change(&key)?;
    editing.key = Some(key);
    Ok(editing)
}

/// Whether the model file at `path`, whose text is `text`, was changed outside crabsim.
pub(crate) fn changed_outside(catalogue: &Catalogue, path: &str, text: &str) -> Result<bool, OperationError> {
    let key = key(catalogue, path)?;
    let editing = catalogue.editing()?;
    Ok(!editing.recorded(&key, text))
}

impl Editing {
    /// Whether `text` is the file at `key` as crabsim last saved or adopted it.
    pub(crate) fn recorded(&self, key: &str, text: &str) -> bool {
        self.held.state.records.get(key).is_some_and(|hash| *hash == crate::edit::hash(text))
    }

    /// Records the file the change was about as saved with `text`.
    pub(crate) fn record(&mut self, text: &str) {
        let key = self.key.clone().expect("a change names its file");
        self.state().records.insert(key, crate::edit::hash(text));
    }

    /// Records `text` as crabsim's own at `key`, such as a copy of a file crabsim saved.
    pub(crate) fn record_at(&mut self, key: String, text: &str) {
        self.state().records.insert(key, crate::edit::hash(text));
    }

    /// Forgets the file the change was about, which was deleted.
    pub(crate) fn forget(&mut self) {
        let key = self.key.clone().expect("a change names its file");
        self.state().records.remove(&key);
    }
}

/// The key of a model file, for recording a copy.
pub(crate) fn key_of(catalogue: &Catalogue, path: &str) -> Result<String, OperationError> {
    key(catalogue, path)
}

fn user_only(catalogue: &Catalogue, operation: &str) -> Result<(), OperationError> {
    if catalogue.source() == Source::Mcp {
        return Err(OperationError::invalid_input(format!("`{operation}` is only for the user, through the UI or the CLI")));
    }
    Ok(())
}

// ---------- What the queue looks like ----------

#[derive(Debug, Serialize, JsonSchema)]
pub struct AgentView {
    /// Such as `agent-3`.
    pub agent: String,
    /// The name its harness gave, such as `claude-code`.
    pub client: String,
    /// When the agent's server started, UTC.
    pub started: String,
    pub purpose: String,
    /// Since when it edits or waits, UTC.
    pub since: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EditorView {
    /// The user edits.
    pub user: bool,
    /// The agent that edits, if one does.
    #[serde(flatten)]
    pub agent: Option<AgentView>,
    pub purpose: Option<String>,
    pub since: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileView {
    pub path: String,
    /// Who may change the file now, if anyone.
    pub editor: Option<EditorView>,
    /// The agents waiting for it, in the order they'll get it.
    pub queue: Vec<AgentView>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathInput {
    /// A model file in the models folder, such as `line.json`. It needn't exist yet.
    pub path: String,
}

// --- adopt_model ---

pub struct AdoptModel;

#[derive(Serialize, JsonSchema)]
pub struct AdoptModelOutput {
    pub path: String,
    /// The SHA-256 of the file's bytes as adopted.
    pub file_hash: String,
    /// The SHA-256 of the model in canonical form, as runs record it. Only for a valid model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    /// Every problem in it, empty if it's valid. An invalid file is adopted too, so write_model
    /// can then fix it.
    pub problems: Vec<crate::ModelProblem>,
}

impl Operation for AdoptModel {
    const NAME: &'static str = "adopt_model";
    const DESCRIPTION: &'static str = "Accepts a model file as it is now, after it was changed outside crabsim, such as by a text editor, a harness's own file tools or git, or when crabsim never saved it. Until then crabsim reads and runs it but won't change it, refusing with changed_outside, so that an outside change is never overwritten or taken in by accident. Look at the file with read_model first, or, for a copy, compare its file_hash with one you trust, such as its source's. It changes nothing in the file, follows the same editor rules as a change, and gives the file's problems, if any: an invalid file is adopted too, so write_model can fix it.";
    const EFFECT: Effect = Effect::Additive;
    type Input = PathInput;
    type Output = AdoptModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<AdoptModelOutput, OperationError> {
        let mut editing = before_adopt(catalogue, &input.path)?;
        let (_, text) = crate::model_files::read_file(catalogue, &input.path)?;
        editing.record(&text);
        let problems = crabsim_model::load(&text).err().unwrap_or_default().iter().map(crate::ModelProblem::from).collect();
        Ok(AdoptModelOutput { path: input.path, file_hash: crate::edit::hash(&text), model_hash: crate::edit::model_hash(&text), problems })
    }
}

// --- claim_model ---

pub struct ClaimModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimModelInput {
    /// A model file in the models folder, such as `line.json`. It needn't exist yet.
    pub path: String,
    /// What you mean to do, in a few words, such as "adding a second mill". The user sees it.
    pub purpose: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    /// You may change the file now.
    Granted,
    /// Someone else edits it: ask again to keep your place.
    Queued,
}

#[derive(Serialize, JsonSchema)]
pub struct ClaimModelOutput {
    pub state: ClaimState,
    /// Your place in the queue, from 1, when queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<usize>,
    pub file: FileView,
}

impl Operation for ClaimModel {
    const NAME: &'static str = "claim_model";
    const DESCRIPTION: &'static str = "Asks for the right to change a model file, for several changes in a row: only its editor may change it, through edit_model, write_model or delete_model. Say in `purpose` what you mean to do: the user sees it. Gives `granted`, or `queued` with your place: then ask again from time to time, which keeps your place, until it's granted. The right lasts while you use it, renewed by every change and every claim, and passes on when you call release_model, after 10 minutes without use, or when your session ends. A queued place you don't ask for in as long is dropped too. The user can take the right from you, give it to someone else, reorder the queue or block you, at any time. A single change to a file nobody holds or waits for needs no claim.";
    const EFFECT: Effect = Effect::Additive;
    type Input = ClaimModelInput;
    type Output = ClaimModelOutput;

    fn run(catalogue: &Catalogue, input: ClaimModelInput) -> Result<ClaimModelOutput, OperationError> {
        if catalogue.source() != Source::Mcp {
            return Err(OperationError::invalid_input("the user doesn't claim: take the editor right with take_model"));
        }
        let purpose = input.purpose.trim().to_string();
        if purpose.is_empty() {
            return Err(OperationError::invalid_input("say what you mean to do in `purpose`, such as \"adding a second mill\""));
        }
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        editing.refuse_if_blocked()?;
        let Asker::Agent(me) = editing.asker.clone() else { unreachable!("an MCP catalogue is an agent") };
        let now = editing.now;
        let file = editing.file(&path);
        let (state, place) = match &mut file.editor {
            Some(editor) if editor.process.as_deref() == Some(me.as_str()) => {
                editor.last_active = now;
                editor.purpose = Some(purpose);
                (ClaimState::Granted, None)
            }
            Some(_) => match file.queue.iter_mut().position(|waiting| waiting.process == me) {
                Some(at) => {
                    file.queue[at].last_asked = now;
                    file.queue[at].purpose = purpose;
                    (ClaimState::Queued, Some(at + 1))
                }
                None => {
                    file.queue.push(Waiting { process: me.clone(), purpose, asked: now, last_asked: now });
                    (ClaimState::Queued, Some(file.queue.len()))
                }
            },
            None => {
                file.editor = Some(Editor { process: Some(me.clone()), purpose: Some(purpose), since: now, last_active: now });
                (ClaimState::Granted, None)
            }
        };
        Ok(ClaimModelOutput { state, place, file: editing.view(&path) })
    }
}

// --- release_model ---

pub struct ReleaseModel;

#[derive(Serialize, JsonSchema)]
pub struct ReleaseModelOutput {
    /// Whether you held the right or a place, which you now don't.
    pub released: bool,
    pub file: FileView,
}

impl Operation for ReleaseModel {
    const NAME: &'static str = "release_model";
    const DESCRIPTION: &'static str = "Gives back the right to change a model file, or your place in its queue, so the next waiting gets it. Call it when you're done changing a file you claimed.";
    const EFFECT: Effect = Effect::Additive;
    type Input = PathInput;
    type Output = ReleaseModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<ReleaseModelOutput, OperationError> {
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let asker = editing.asker.clone();
        let mut released = false;
        if let Some(file) = editing.state().files.get_mut(&path) {
            let holds = match (&asker, file.editor.as_ref().map(|editor| &editor.process)) {
                (Asker::User, Some(None)) => true,
                (Asker::Agent(me), Some(Some(process))) => me == process,
                _ => false,
            };
            if holds {
                file.editor = None;
                released = true;
            }
            if let Asker::Agent(me) = &asker {
                let before = file.queue.len();
                file.queue.retain(|waiting| &waiting.process != me);
                released |= file.queue.len() != before;
            }
        }
        editing.settle(&path);
        Ok(ReleaseModelOutput { released, file: editing.view(&path) })
    }
}

// --- get_edit_queue ---

pub struct GetEditQueue;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEditQueueInput {
    /// A model file, such as `line.json`. Left out, every file someone edits or waits for.
    pub path: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct BlockedView {
    pub agent: String,
    pub client: String,
}

#[derive(Serialize, JsonSchema)]
pub struct GetEditQueueOutput {
    /// With a path: who edits it, if anyone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<Option<EditorView>>,
    /// With a path: who waits for it, in order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<Vec<AgentView>>,
    /// Without a path: every file someone edits or waits for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<FileView>>,
    /// The agents the user has blocked.
    pub blocked: Vec<BlockedView>,
    /// Your own id, if you're an agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub you: Option<String>,
}

impl Operation for GetEditQueue {
    const NAME: &'static str = "get_edit_queue";
    const DESCRIPTION: &'static str = "Shows who may change a model file now, its editor, and the agents waiting for it in order, each with its id, harness, purpose and since when, or the same for every file someone edits or waits for. Also the agents the user has blocked, and `you`, your own id.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = GetEditQueueInput;
    type Output = GetEditQueueOutput;

    fn run(catalogue: &Catalogue, input: GetEditQueueInput) -> Result<GetEditQueueOutput, OperationError> {
        let path = input.path.as_deref().map(|path| key(catalogue, path)).transpose()?;
        let editing = catalogue.editing()?;
        let blocked = editing.held.state.agents.values().filter(|agent| agent.blocked).map(|agent| BlockedView { agent: agent.id.clone(), client: agent.client.clone() }).collect();
        let you = match &editing.asker {
            Asker::Agent(process) => editing.agent(process).map(|agent| agent.id.clone()),
            Asker::User => None,
        };
        Ok(match path {
            Some(path) => {
                let view = editing.view(&path);
                GetEditQueueOutput { editor: Some(view.editor), queue: Some(view.queue), files: None, blocked, you }
            }
            None => {
                let files = editing.held.state.files.keys().map(|path| editing.view(path)).collect();
                GetEditQueueOutput { editor: None, queue: None, files: Some(files), blocked, you }
            }
        })
    }
}

// ---------- The user's controls ----------

// --- take_model ---

pub struct TakeModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeModelInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: String,
}

#[derive(Serialize, JsonSchema)]
pub struct QueueOutput {
    pub file: FileView,
}

impl Operation for TakeModel {
    const NAME: &'static str = "take_model";
    const DESCRIPTION: &'static str = "For the user: takes the right to change a model file now, from any agent that holds it, which isn't queued again. The user's right never runs out: release_model gives it back to the first waiting agent.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = TakeModelInput;
    type Output = QueueOutput;

    fn run(catalogue: &Catalogue, input: TakeModelInput) -> Result<QueueOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let now = editing.now;
        editing.file(&path).editor = Some(Editor { process: None, purpose: None, since: now, last_active: now });
        Ok(QueueOutput { file: editing.view(&path) })
    }
}

// --- give_model ---

pub struct GiveModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInFileInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: String,
    /// The agent's id, such as `agent-3`.
    pub agent: String,
}

impl Operation for GiveModel {
    const NAME: &'static str = "give_model";
    const DESCRIPTION: &'static str = "For the user: gives the right to change a model file to an agent waiting for it, now, whoever held it. The one who held it isn't queued again.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = AgentInFileInput;
    type Output = QueueOutput;

    fn run(catalogue: &Catalogue, input: AgentInFileInput) -> Result<QueueOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let process = editing.process_of(&input.agent)?;
        let now = editing.now;
        let file = editing.file(&path);
        let at = file
            .queue
            .iter()
            .position(|waiting| waiting.process == process)
            .ok_or_else(|| OperationError::not_found(format!("`{}` isn't waiting for `{path}`", input.agent)))?;
        let waiting = file.queue.remove(at);
        file.editor = Some(Editor { process: Some(process), purpose: Some(waiting.purpose), since: now, last_active: now });
        Ok(QueueOutput { file: editing.view(&path) })
    }
}

// --- move_in_queue ---

pub struct MoveInQueue;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveInQueueInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: String,
    /// The waiting agent's id, such as `agent-3`.
    pub agent: String,
    /// Its new place, 0 first.
    pub place: usize,
}

impl Operation for MoveInQueue {
    const NAME: &'static str = "move_in_queue";
    const DESCRIPTION: &'static str = "For the user: moves an agent waiting for a model file to another place in its queue, 0 first, as dragging it in a list does. The others keep their order.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = MoveInQueueInput;
    type Output = QueueOutput;

    fn run(catalogue: &Catalogue, input: MoveInQueueInput) -> Result<QueueOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let process = editing.process_of(&input.agent)?;
        let file = editing.file(&path);
        let at = file
            .queue
            .iter()
            .position(|waiting| waiting.process == process)
            .ok_or_else(|| OperationError::not_found(format!("`{}` isn't waiting for `{path}`", input.agent)))?;
        if input.place >= file.queue.len() {
            let last = file.queue.len() - 1;
            return Err(OperationError::invalid_input(format!("{} wait for `{path}`, so the place can be 0 to {last}", file.queue.len())));
        }
        let waiting = file.queue.remove(at);
        file.queue.insert(input.place, waiting);
        Ok(QueueOutput { file: editing.view(&path) })
    }
}

// --- remove_from_queue ---

pub struct RemoveFromQueue;

impl Operation for RemoveFromQueue {
    const NAME: &'static str = "remove_from_queue";
    const DESCRIPTION: &'static str = "For the user: takes an agent out of a model file's queue, or, if it's the editor, takes its right away, which passes to the first waiting. It may claim again.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = AgentInFileInput;
    type Output = QueueOutput;

    fn run(catalogue: &Catalogue, input: AgentInFileInput) -> Result<QueueOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let process = editing.process_of(&input.agent)?;
        let file = editing.file(&path);
        let before = file.queue.len();
        file.queue.retain(|waiting| waiting.process != process);
        let mut found = file.queue.len() != before;
        if file.editor.as_ref().is_some_and(|editor| editor.process.as_deref() == Some(process.as_str())) {
            file.editor = None;
            found = true;
        }
        if !found {
            return Err(OperationError::not_found(format!("`{}` neither edits nor waits for `{path}`", input.agent)));
        }
        editing.settle(&path);
        Ok(QueueOutput { file: editing.view(&path) })
    }
}

// --- block_agent, unblock_agent ---

pub struct BlockAgent;
pub struct UnblockAgent;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInput {
    /// The agent's id, such as `agent-3`.
    pub agent: String,
}

#[derive(Serialize, JsonSchema)]
pub struct BlockOutput {
    pub agent: String,
    pub blocked: bool,
}

fn set_blocked(catalogue: &Catalogue, agent: &str, blocked: bool) -> Result<BlockOutput, OperationError> {
    let mut editing = catalogue.editing()?;
    let process = editing.process_of(agent)?;
    if let Some(found) = editing.state().agents.get_mut(&process) {
        found.blocked = blocked;
    }
    let crabsim_folder = catalogue.folder().crabsim_folder();
    editing.sweep(&crabsim_folder, catalogue.edit_timeout());
    Ok(BlockOutput { agent: agent.to_string(), blocked })
}

impl Operation for BlockAgent {
    const NAME: &'static str = "block_agent";
    const DESCRIPTION: &'static str = "For the user: blocks an agent session from changing models in this folder. It loses any right it holds and its places in queues, and every claim or change it makes is refused as blocked, until unblock_agent or the end of its session.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = AgentInput;
    type Output = BlockOutput;

    fn run(catalogue: &Catalogue, input: AgentInput) -> Result<BlockOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        set_blocked(catalogue, &input.agent, true)
    }
}

impl Operation for UnblockAgent {
    const NAME: &'static str = "unblock_agent";
    const DESCRIPTION: &'static str = "For the user: lets a blocked agent claim and change models again. It gets back no right or place it lost.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = AgentInput;
    type Output = BlockOutput;

    fn run(catalogue: &Catalogue, input: AgentInput) -> Result<BlockOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        set_blocked(catalogue, &input.agent, false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn times_are_written_as_utc_iso_8601() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso(1_794_130_323_000), "2026-11-08T09:32:03Z");
        assert_eq!(super::iso(951_782_400_000), "2000-02-29T00:00:00Z");
    }
}
