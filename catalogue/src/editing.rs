//! Who may change a model file: one editor at a time (ADR 0013, replacing ADR 0011's edit queue).
//! An agent claims a file, saying its goal and what it is doing, and gets it at once or is refused:
//! there is no queue. Every change an agent makes needs a claim. The user may take the right at any
//! time, which pauses the agent's claim until the user releases it, and after anyone else changes a
//! file an agent holds, the agent's next change is refused once, so it reads the model again. The
//! state is shared by every crabsim process on the models folder, in `.crabsim/editing.json`, read
//! and written only while holding an OS lock on `.crabsim/editing.lock`. See
//! `docs/design/operation-catalogue.md`.
//!
//! One agent is one crabsim process opened as an MCP server, and the user's hold belongs to the
//! window or command that took it, each known by its process lock (see `process.rs`), so a holder
//! whose process ended is noticed the next time anyone looks.

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
        entry::<GetEditors>(),
        entry::<TakeModel>(),
        entry::<RevokeClaim>(),
        entry::<BlockAgent>(),
        entry::<UnblockAgent>(),
    ]
}

/// The edit state's file in `.crabsim`.
pub const EDITING_FILE: &str = "editing.json";
const LOCK_FILE: &str = "editing.lock";

/// How long an agent's claim lasts without it changing the file or claiming again, unless set
/// when the server or the CLI starts. A paused claim doesn't run out.
pub const EDIT_TIMEOUT: Duration = Duration::from_secs(600);

/// How many of an agent's activities a claim keeps, newest first.
pub const TRAIL_LENGTH: usize = 10;

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
    /// Every model file someone holds, by its path in the models folder.
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
    /// The user's hold, while the user has taken the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<UserHold>,
    /// An agent's claim, paused while the user holds the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim: Option<Claim>,
    /// The editor of a state written before claims (ADR 0011), turned into a claim when read.
    #[serde(default, skip_serializing)]
    editor: Option<OldEditor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserHold {
    /// The process lock id of the window or command that took the file.
    process: String,
    since: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claim {
    /// The agent's process lock id.
    process: String,
    goal: String,
    doing: String,
    /// What it has been doing, newest first, the first being `doing`.
    trail: Vec<Activity>,
    since: u64,
    last_active: u64,
    /// The file's hash after the agent's last change to it, or when it claimed it if it has made
    /// none. A file whose hash differs was changed by someone else since.
    hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Activity {
    doing: String,
    since: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct OldEditor {
    process: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    since: u64,
    last_active: u64,
}

impl State {
    /// Turns a state written before claims into one with claims: an agent that held a file keeps
    /// it, its purpose as its goal, and queues and the user's holds are dropped. Gives whether
    /// anything changed.
    fn upgrade(&mut self) -> bool {
        let mut changed = false;
        for (path, file) in &mut self.files {
            if let Some(old) = file.editor.take() {
                changed = true;
                if let Some(process) = old.process {
                    let goal = old.purpose.unwrap_or_default();
                    let hash = self.records.get(path).cloned();
                    file.claim = Some(Claim { process, goal, doing: String::new(), trail: Vec::new(), since: old.since, last_active: old.last_active, hash });
                }
            }
        }
        self.files.retain(|_, file| file.user.is_some() || file.claim.is_some());
        changed
    }
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
        let mut state: State = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => State::default(),
        };
        let changed = state.upgrade();
        Ok(Held { _lock: lock, path, state, changed })
    }

    fn write(&mut self) -> Result<(), OperationError> {
        if !self.changed {
            return Ok(());
        }
        self.state.version = 2;
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

/// Who is asking, by the process lock id of its catalogue: an agent, or the user.
#[derive(Debug, Clone, PartialEq)]
enum Asker {
    Agent(String),
    User(String),
}

impl Asker {
    fn process(&self) -> &str {
        match self {
            Asker::Agent(process) | Asker::User(process) => process,
        }
    }
}

/// The edit state for one call: swept of ended agents, lost claims and the holds of windows and
/// commands that ended, and the caller known.
pub(crate) struct Editing {
    held: Held,
    asker: Asker,
    now: u64,
    /// The model file a change is about, once checked.
    key: Option<String>,
    /// The user changes past an agent's claim, for one command.
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
            Source::Cli | Source::Ui => Asker::User(process),
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

    /// Drops agents whose process ended, the user's holds whose window or command ended, giving
    /// the paused claim back, and claims of ended or blocked agents and claims unused for longer
    /// than `timeout`, unless paused.
    fn sweep(&mut self, crabsim_folder: &Path, timeout: Duration) {
        let own = self.asker.process().to_string();
        let alive = |process: &str| process == own || process::is_alive(crabsim_folder, process);
        let ended: Vec<String> = self.held.state.agents.keys().filter(|process| !alive(process)).cloned().collect();
        let now = self.now;
        let timeout = timeout.as_millis() as u64;
        let before = serde_json::to_string(&self.held.state.files).unwrap_or_default();
        let state = &mut self.held.state;
        for process in &ended {
            state.agents.remove(process);
        }
        let agents = &state.agents;
        let gone = |process: &str| agents.get(process).is_none_or(|agent| agent.blocked);
        for file in state.files.values_mut() {
            if file.user.as_ref().is_some_and(|hold| !alive(&hold.process)) {
                file.user = None;
                if let Some(claim) = &mut file.claim {
                    claim.last_active = now;
                }
            }
            if let Some(claim) = &file.claim
                && (gone(&claim.process) || (file.user.is_none() && now.saturating_sub(claim.last_active) > timeout))
            {
                file.claim = None;
            }
        }
        state.files.retain(|_, file| file.user.is_some() || file.claim.is_some());
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

    /// Who holds `file`, as messages name it.
    fn describe(&self, file: &FileState) -> String {
        if let Some(hold) = &file.user {
            return format!("the user, since {}", iso(hold.since));
        }
        match &file.claim {
            None => "nobody".to_string(),
            Some(claim) => {
                let agent = self.agent(&claim.process).map_or_else(|| "an agent that has ended".to_string(), |agent| format!("{} ({})", agent.id, agent.client));
                format!("{agent}, since {}, with the goal \"{}\", now \"{}\"", iso(claim.since), claim.goal, claim.doing)
            }
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

    /// Whether the asker may change `path` now. An agent's change renews its claim.
    pub(crate) fn may_change(&mut self, path: &str) -> Result<(), OperationError> {
        self.refuse_if_blocked()?;
        let now = self.now;
        let empty = FileState::default();
        let file = self.held.state.files.get(path).unwrap_or(&empty);
        match self.asker.clone() {
            Asker::User(_) => {
                if self.take_over || file.claim.is_none() || file.user.is_some() {
                    return Ok(());
                }
                let who = self.describe(file);
                Err(OperationError::new(ErrorKind::NotEditor, format!("`{path}` is being changed by {who}: take it first with take_model, which pauses the agent's claim")))
            }
            Asker::Agent(me) => {
                let mine = file.claim.as_ref().is_some_and(|claim| claim.process == me);
                if mine && file.user.is_some() {
                    return Err(OperationError::new(
                        ErrorKind::NotEditor,
                        format!("the user has taken `{path}`, so your claim is paused: wait and try again later. Your claim comes back to you when the user releases it"),
                    ));
                }
                if mine {
                    if let Some(claim) = self.state().files.get_mut(path).and_then(|file| file.claim.as_mut()) {
                        claim.last_active = now;
                    }
                    return Ok(());
                }
                let message = if file.user.is_some() || file.claim.is_some() {
                    format!("`{path}` is being changed by {}: you may not change it now. Claims aren't queued, so tell your user, or try again later", self.describe(file))
                } else {
                    format!("claim `{path}` with claim_model before changing it: every change an agent makes needs a claim")
                };
                Err(OperationError::new(ErrorKind::NotEditor, message))
            }
        }
    }

    /// For an agent holding `path`: refused once if someone else changed the file since the
    /// agent's last change, whose hash is now `current`, so it reads it again.
    fn read_again(&mut self, path: &str, current: Option<String>) -> Result<(), OperationError> {
        let Asker::Agent(me) = self.asker.clone() else { return Ok(()) };
        let stale = self.held.state.files.get(path).and_then(|file| file.claim.as_ref()).is_some_and(|claim| claim.process == me && claim.hash != current);
        if !stale {
            return Ok(());
        }
        if let Some(claim) = self.state().files.get_mut(path).and_then(|file| file.claim.as_mut()) {
            claim.hash = current;
        }
        Err(OperationError::new(
            ErrorKind::ModelChanged,
            format!("`{path}` changed since your last change, by the user: read it again with read_model, then make your change from what it is now"),
        ))
    }

    /// The asker's agent process, if it holds the claim on `path`.
    fn claim_mut(&mut self, path: &str) -> Option<&mut Claim> {
        let Asker::Agent(me) = self.asker.clone() else { return None };
        self.state().files.get_mut(path).and_then(|file| file.claim.as_mut()).filter(|claim| claim.process == me)
    }

    fn view(&self, path: &str) -> Option<FileView> {
        let file = self.held.state.files.get(path)?;
        let claim = file.claim.as_ref().map(|claim| {
            let agent = self.agent(&claim.process);
            ClaimView {
                agent: agent.map_or_else(|| "ended".to_string(), |agent| agent.id.clone()),
                client: agent.map_or_else(String::new, |agent| agent.client.clone()),
                started: agent.map_or_else(String::new, |agent| iso(agent.started)),
                goal: claim.goal.clone(),
                doing: claim.doing.clone(),
                trail: claim.trail.iter().map(|activity| ActivityView { doing: activity.doing.clone(), since: iso(activity.since) }).collect(),
                since: iso(claim.since),
                last_active: iso(claim.last_active),
                paused: file.user.is_some(),
            }
        });
        let editor = match (&file.user, &claim) {
            (Some(_), _) => "user".to_string(),
            (None, Some(claim)) => claim.agent.clone(),
            (None, None) => "nobody".to_string(),
        };
        Some(FileView { path: path.to_string(), editor, user_since: file.user.as_ref().map(|hold| iso(hold.since)), claim })
    }
}

impl Catalogue {
    /// Who is using this catalogue, as runs record it: such as `agent-3 (claude-code)`, or the user.
    pub(crate) fn who(&self) -> Result<String, OperationError> {
        let editing = self.editing()?;
        Ok(match &editing.asker {
            Asker::User(_) => "the user".to_string(),
            Asker::Agent(process) => editing.agent(process).map_or_else(|| "an agent".to_string(), |agent| format!("{} ({})", agent.id, agent.client)),
        })
    }
}

/// The key of a model file in the edit state: its path in the models folder, as listed.
fn key(catalogue: &Catalogue, path: &str) -> Result<String, OperationError> {
    let file = catalogue.folder().model_file(path)?;
    Ok(catalogue.folder().name_of(&file))
}

/// The hash of the model file at `path` now, if it exists.
fn current_hash(catalogue: &Catalogue, path: &str) -> Result<Option<String>, OperationError> {
    Ok(std::fs::read_to_string(catalogue.folder().model_file(path)?).ok().as_deref().map(crate::edit::hash))
}

/// Before an operation changes the model file at `path`: refused unless the caller may change it
/// now. The returned state is held until the change is written, so two changes can't both go
/// through.
pub(crate) fn before_change(catalogue: &Catalogue, path: &str, expected_hash: Option<&str>) -> Result<Editing, OperationError> {
    let key = key(catalogue, path)?;
    let mut editing = catalogue.editing()?;
    editing.may_change(&key)?;
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
    let current = current.as_deref().map(crate::edit::hash);
    if let Some(expected) = expected_hash
        && current.as_deref() != Some(expected)
    {
        let what = current.as_ref().map_or_else(|| "doesn't exist".to_string(), |hash| format!("has the hash {hash} now"));
        // This tells an agent all that read_again would, so it counts as that one refusal.
        let by = if editing.read_again(&key, current).is_err() { ", changed by the user since your last change" } else { "" };
        return Err(OperationError::new(
            ErrorKind::Conflict,
            format!("`{path}` isn't the version with the hash {expected} any more: it {what}{by}. Read it again and decide on your change from that"),
        ));
    }
    editing.read_again(&key, current)?;
    editing.key = Some(key);
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

    /// Records the file the change was about as saved with `text`, and, for an agent holding it,
    /// as its own last change.
    pub(crate) fn record(&mut self, text: &str) {
        let key = self.key.clone().expect("a change names its file");
        let hash = crate::edit::hash(text);
        if let Some(claim) = self.claim_mut(&key) {
            claim.hash = Some(hash.clone());
        }
        self.state().records.insert(key, hash);
    }

    /// Records `text` as crabsim's own at `key`, such as a copy of a file crabsim saved.
    pub(crate) fn record_at(&mut self, key: String, text: &str) {
        self.state().records.insert(key, crate::edit::hash(text));
    }

    /// Forgets the file the change was about, which was deleted.
    pub(crate) fn forget(&mut self) {
        let key = self.key.clone().expect("a change names its file");
        if let Some(claim) = self.claim_mut(&key) {
            claim.hash = None;
        }
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

// ---------- What a claim looks like ----------

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActivityView {
    pub doing: String,
    /// When the agent began it, UTC.
    pub since: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimView {
    /// Such as `agent-3`.
    pub agent: String,
    /// The name its harness gave, such as `claude-code`.
    pub client: String,
    /// When the agent's session with crabsim began, UTC, which may be before this claim: see
    /// `since` for the claim's own start.
    pub started: String,
    /// What the agent means to achieve overall.
    pub goal: String,
    /// What it is working on now.
    pub doing: String,
    /// What it has been doing, newest first, at most 10.
    pub trail: Vec<ActivityView>,
    /// Since when it holds the file, UTC.
    pub since: String,
    /// When it last changed the file or claimed it, UTC.
    pub last_active: String,
    /// The user has taken the file: the agent may not change it until the user releases it.
    pub paused: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileView {
    pub path: String,
    /// Who may change it now: `user`, or an agent's id, such as `agent-3`.
    pub editor: String,
    /// Since when the user holds it, UTC, while the user does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_since: Option<String>,
    /// The agent's claim on it, if any, paused while the user holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimView>,
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
    const DESCRIPTION: &'static str = "Accepts a model file as it is now, after it was changed outside crabsim, such as by a text editor, a harness's own file tools or git, or when crabsim never saved it. Until then crabsim reads and runs it but won't change it, refusing with changed_outside, so that an outside change is never overwritten or taken in by accident. Look at the file with read_model first, or, for a copy, compare its file_hash with one you trust, such as its source's. It changes nothing in the file, needs a claim like a change (claim_model), and gives the file's problems, if any: an invalid file is adopted too, so write_model can fix it.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
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
    /// What you mean to achieve overall, in a sentence, such as "find out whether a second mill
    /// removes the bottleneck". The user sees it.
    pub goal: String,
    /// What you are working on now, such as "adding Mill 2 after Station 3". The user sees it.
    pub doing: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ClaimModelOutput {
    pub file: FileView,
    /// Only while the user has taken the file: your claim is paused, so your changes are refused
    /// until the user releases it, and then it is yours again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<String>,
}

impl Operation for ClaimModel {
    const NAME: &'static str = "claim_model";
    const DESCRIPTION: &'static str = "Claims a model file, which you need before every change to it: edit_model, write_model (also for a new file), delete_model and adopt_model. Say your overall `goal` and what you are `doing` now: the user sees both. Granted at once if nobody holds the file, and refused with model_in_use otherwise, naming who holds it: claims aren't queued, so tell your user or try again later. Claim again as you go to update `goal` and `doing`, which also renews the claim. It lasts while you use it and ends when you call release_model, after 10 minutes without a change or a claim, or when your session ends. The user can take the file at any time, which pauses your claim until they release it (claiming then says so in `paused`), can revoke it, and can block you. After anyone else changes the file, your next change is refused once with model_changed: read the model again, then carry on. Reading, copying and running need no claim.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
    type Input = ClaimModelInput;
    type Output = ClaimModelOutput;

    fn run(catalogue: &Catalogue, input: ClaimModelInput) -> Result<ClaimModelOutput, OperationError> {
        if catalogue.source() != Source::Mcp {
            return Err(OperationError::invalid_input("the user doesn't claim: take the file with take_model"));
        }
        let (goal, doing) = (input.goal.trim().to_string(), input.doing.trim().to_string());
        if goal.is_empty() || doing.is_empty() {
            return Err(OperationError::invalid_input(
                "say your overall `goal`, such as \"find out whether a second mill removes the bottleneck\", and what you are `doing` now, such as \"adding Mill 2 after Station 3\"",
            ));
        }
        let path = key(catalogue, &input.path)?;
        let hash = current_hash(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        editing.refuse_if_blocked()?;
        let Asker::Agent(me) = editing.asker.clone() else { unreachable!("an MCP catalogue is an agent") };
        let now = editing.now;
        if let Some(file) = editing.held.state.files.get(&path)
            && (file.claim.as_ref().is_some_and(|claim| claim.process != me) || (file.user.is_some() && file.claim.is_none()))
        {
            let who = editing.describe(file);
            return Err(OperationError::new(
                ErrorKind::ModelInUse,
                format!("`{path}` is being changed by {who}. Claims aren't queued: tell your user, or try again later"),
            ));
        }
        let file = editing.state().files.entry(path.clone()).or_default();
        match &mut file.claim {
            Some(claim) => {
                claim.goal = goal;
                if claim.doing != doing {
                    claim.trail.insert(0, Activity { doing: doing.clone(), since: now });
                    claim.trail.truncate(TRAIL_LENGTH);
                    claim.doing = doing;
                }
                claim.last_active = now;
            }
            None => {
                let trail = vec![Activity { doing: doing.clone(), since: now }];
                file.claim = Some(Claim { process: me, goal, doing, trail, since: now, last_active: now, hash });
            }
        }
        let file = editing.view(&path).expect("just claimed");
        let paused = file.claim.as_ref().filter(|claim| claim.paused).map(|_| {
            let since = file.user_since.as_deref().map_or_else(String::new, |since| format!(" since {since}"));
            format!("The user has taken `{path}`{since}, so your claim is paused: your changes are refused until the user releases it. Then it is yours again, and if the user changed it, your next change is refused once so you read it again")
        });
        Ok(ClaimModelOutput { file, paused })
    }
}

// --- release_model ---

pub struct ReleaseModel;

#[derive(Serialize, JsonSchema)]
pub struct ReleaseModelOutput {
    /// Whether you held the file, which you now don't.
    pub released: bool,
    /// The file as it stands now, if anyone still holds it, such as an agent whose claim the
    /// user's release gave back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileView>,
}

impl Operation for ReleaseModel {
    const NAME: &'static str = "release_model";
    const DESCRIPTION: &'static str = "Ends your claim on a model file, so others may claim it. Call it when you're done changing a file you claimed. For the user, it gives the file back to the agent whose claim it paused, if any.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
    type Input = PathInput;
    type Output = ReleaseModelOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<ReleaseModelOutput, OperationError> {
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let asker = editing.asker.clone();
        let now = editing.now;
        let mut released = false;
        if let Some(file) = editing.state().files.get_mut(&path) {
            match &asker {
                Asker::User(_) => {
                    if file.user.take().is_some() {
                        released = true;
                        if let Some(claim) = &mut file.claim {
                            claim.last_active = now;
                        }
                    }
                }
                Asker::Agent(me) => {
                    if file.claim.as_ref().is_some_and(|claim| &claim.process == me) {
                        file.claim = None;
                        released = true;
                    }
                }
            }
            if file.user.is_none() && file.claim.is_none() {
                editing.state().files.remove(&path);
            }
        }
        Ok(ReleaseModelOutput { released, file: editing.view(&path) })
    }
}

// --- get_editors ---

pub struct GetEditors;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEditorsInput {
    /// A model file, such as `line.json`. Left out, every file someone holds.
    pub path: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct BlockedView {
    pub agent: String,
    pub client: String,
}

#[derive(Serialize, JsonSchema)]
pub struct GetEditorsOutput {
    /// Every file someone holds, or with a path, that file if someone holds it.
    pub files: Vec<FileView>,
    /// The agents the user has blocked.
    pub blocked: Vec<BlockedView>,
    /// Your own id, if you're an agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub you: Option<String>,
}

impl Operation for GetEditors {
    const NAME: &'static str = "get_editors";
    const DESCRIPTION: &'static str = "Shows who holds a model file now, or every file someone holds: the user, or an agent's claim with its id, harness, goal, what it is doing, its last activities, since when, and whether it is paused because the user has taken the file. Also the agents the user has blocked, and `you`, your own id.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = GetEditorsInput;
    type Output = GetEditorsOutput;

    fn run(catalogue: &Catalogue, input: GetEditorsInput) -> Result<GetEditorsOutput, OperationError> {
        let path = input.path.as_deref().map(|path| key(catalogue, path)).transpose()?;
        let editing = catalogue.editing()?;
        let blocked = editing.held.state.agents.values().filter(|agent| agent.blocked).map(|agent| BlockedView { agent: agent.id.clone(), client: agent.client.clone() }).collect();
        let you = match &editing.asker {
            Asker::Agent(process) => editing.agent(process).map(|agent| agent.id.clone()),
            Asker::User(_) => None,
        };
        let files = match path {
            Some(path) => editing.view(&path).into_iter().collect(),
            None => editing.held.state.files.keys().filter_map(|path| editing.view(path)).collect(),
        };
        Ok(GetEditorsOutput { files, blocked, you })
    }
}

// ---------- The user's controls ----------

#[derive(Serialize, JsonSchema)]
pub struct HeldOutput {
    pub file: FileView,
}

// --- take_model ---

pub struct TakeModel;

impl Operation for TakeModel {
    const NAME: &'static str = "take_model";
    const DESCRIPTION: &'static str = "For the user: takes a model file now. An agent's claim on it is paused: its changes are refused until the user releases the file with release_model, which gives the claim back. The user's hold ends when the window or command that took it ends.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
    const USER_ONLY: bool = true;
    type Input = PathInput;
    type Output = HeldOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<HeldOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let (now, process) = (editing.now, editing.asker.process().to_string());
        let file = editing.state().files.entry(path.clone()).or_default();
        if file.user.is_none() {
            file.user = Some(UserHold { process, since: now });
        }
        Ok(HeldOutput { file: editing.view(&path).expect("just taken") })
    }
}

// --- revoke_claim ---

pub struct RevokeClaim;

#[derive(Serialize, JsonSchema)]
pub struct RevokeClaimOutput {
    /// The agent whose claim ended, such as `agent-3`.
    pub revoked: String,
    /// The file as it stands now, if the user still holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<FileView>,
}

impl Operation for RevokeClaim {
    const NAME: &'static str = "revoke_claim";
    const DESCRIPTION: &'static str = "For the user: ends an agent's claim on a model file at once, paused or not. The agent may claim it again.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
    const USER_ONLY: bool = true;
    type Input = PathInput;
    type Output = RevokeClaimOutput;

    fn run(catalogue: &Catalogue, input: PathInput) -> Result<RevokeClaimOutput, OperationError> {
        user_only(catalogue, Self::NAME)?;
        let path = key(catalogue, &input.path)?;
        let mut editing = catalogue.editing()?;
        let claim = editing.state().files.get_mut(&path).and_then(|file| file.claim.take()).ok_or_else(|| OperationError::not_found(format!("no agent has claimed `{path}`")))?;
        let revoked = editing.agent(&claim.process).map_or_else(|| "ended".to_string(), |agent| agent.id.clone());
        if editing.held.state.files.get(&path).is_some_and(|file| file.user.is_none()) {
            editing.state().files.remove(&path);
        }
        Ok(RevokeClaimOutput { revoked, file: editing.view(&path) })
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
    const DESCRIPTION: &'static str = "For the user: blocks an agent session from changing models in this folder. Its claims end, and every claim or change it makes is refused as blocked, until unblock_agent or the end of its session.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
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
    const DESCRIPTION: &'static str = "For the user: lets a blocked agent claim and change models again. It gets back no claim it lost.";
    const EFFECT: Effect = Effect::Additive;
    const CHANGES_MODELS: bool = true;
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
