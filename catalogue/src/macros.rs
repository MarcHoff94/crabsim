//! Macros (spec issue 102): named lists of commands in the command language, kept in
//! `macros.json` at the root of the models folder, beside the models, so they're readable and
//! versioned with them. Each records its author. An agent's macros last for its work: when its
//! process has ended they leave the list for `.crabsim/agent-macros.jsonl`, where they're kept to
//! be looked at later. An agent can't change or remove the user's macros, and keeping one of an
//! agent's as the user's own is the user's alone.

use crate::error::OperationError;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<ListMacros>(), entry::<SaveMacro>(), entry::<RemoveMacro>(), entry::<KeepMacro>()]
}

/// The macros file, at the root of the models folder.
pub const MACROS_FILE: &str = "macros.json";
/// Agents' macros once their work ended, inside `.crabsim`.
pub const ARCHIVE_FILE: &str = "agent-macros.jsonl";
/// Held while the macros file is read and changed.
const LOCK_FILE: &str = "macros.lock";

/// A named list of commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Macro {
    /// Letters, digits, `-` and `_`, run as `@name`.
    pub name: String,
    /// Commands in the command language, run in order. Model changes next to each other run as
    /// one batch.
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub author: Author,
    /// When it was saved, in UTC.
    pub made: String,
    /// How often it ran.
    #[serde(default)]
    pub runs: u64,
}

/// Who made a macro.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    User,
    Agent {
        /// Such as `agent-3`.
        agent: String,
        /// The agent's harness, such as `claude-code`.
        client: String,
        /// The process lock id of the agent's catalogue: its macros go when it ends.
        process: String,
        /// The goals of the claims it held when it saved the macro.
        goals: Vec<String>,
    },
}

/// An agent's macro once its work ended, as the archive keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Archived {
    /// Unique in the archive, for keeping it: the process and the name.
    pub id: String,
    #[serde(flatten)]
    pub r#macro: Macro,
    /// When it left the list, in UTC.
    pub ended: String,
}

#[derive(Default, Serialize, Deserialize)]
struct MacrosFile {
    /// Not `format_version`, so it's never taken for a model file.
    macros_version: u32,
    macros: Vec<Macro>,
}

/// The macros file, read and swept of agents' macros whose process ended, under its lock until
/// dropped.
pub(crate) struct Held {
    _lock: File,
    path: PathBuf,
    archive: PathBuf,
    pub macros: Vec<Macro>,
    changed: bool,
}

fn fail(what: &str, error: std::io::Error) -> OperationError {
    OperationError::run_error(format!("the macros can't be {what}: {error}"))
}

/// Now, as a UTC timestamp.
fn now() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3_600, rest % 3_600 / 60, rest % 60)
}

impl Held {
    pub(crate) fn open(catalogue: &Catalogue) -> Result<Held, OperationError> {
        let crabsim_folder = catalogue.folder().crabsim_folder();
        std::fs::create_dir_all(&crabsim_folder).map_err(|error| fail("locked", error))?;
        let lock = File::options().create(true).truncate(false).read(true).write(true).open(crabsim_folder.join(LOCK_FILE)).map_err(|error| fail("locked", error))?;
        lock.lock().map_err(|error| fail("locked", error))?;
        let path = catalogue.folder().root().join(MACROS_FILE);
        let file: MacrosFile = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| OperationError::run_error(format!("{MACROS_FILE} can't be read: {error}")))?,
            Err(_) => MacrosFile::default(),
        };
        let mut held = Held { _lock: lock, path, archive: crabsim_folder.join(ARCHIVE_FILE), macros: file.macros, changed: false };
        held.sweep(&crabsim_folder, catalogue.process_lock().ok())?;
        Ok(held)
    }

    /// Moves agents' macros whose process ended to the archive.
    fn sweep(&mut self, crabsim_folder: &Path, own: Option<&str>) -> Result<(), OperationError> {
        let ended = |process: &str| Some(process) != own && !crate::process::is_alive(crabsim_folder, process);
        let (gone, kept): (Vec<Macro>, Vec<Macro>) = std::mem::take(&mut self.macros)
            .into_iter()
            .partition(|r#macro| matches!(&r#macro.author, Author::Agent { process, .. } if ended(process)));
        self.macros = kept;
        if gone.is_empty() {
            return Ok(());
        }
        let mut archive = File::options().create(true).append(true).open(&self.archive).map_err(|error| fail("archived", error))?;
        let ended = now();
        for r#macro in gone {
            let Author::Agent { process, .. } = &r#macro.author else { continue };
            let id = format!("{process}/{}", r#macro.name);
            let line = serde_json::to_string(&Archived { id, r#macro, ended: ended.clone() }).expect("an archived macro serialises");
            writeln!(archive, "{line}").map_err(|error| fail("archived", error))?;
        }
        self.changed = true;
        Ok(())
    }

    /// The archive, oldest first.
    pub(crate) fn archived(&self) -> Vec<Archived> {
        std::fs::read_to_string(&self.archive).unwrap_or_default().lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    pub(crate) fn find(&self, name: &str) -> Option<&Macro> {
        self.macros.iter().find(|r#macro| r#macro.name == name)
    }

    /// Counts a run of each macro named.
    pub(crate) fn ran(&mut self, names: &[String]) {
        for name in names {
            if let Some(r#macro) = self.macros.iter_mut().find(|r#macro| &r#macro.name == name) {
                r#macro.runs += 1;
                self.changed = true;
            }
        }
    }

    fn write(&mut self) -> Result<(), OperationError> {
        if !self.changed {
            return Ok(());
        }
        let file = MacrosFile { macros_version: 1, macros: std::mem::take(&mut self.macros) };
        let text = serde_json::to_string_pretty(&file).expect("macros serialise") + "\n";
        self.macros = file.macros;
        let temporary = self.path.with_extension("json.new");
        std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, &self.path)).map_err(|error| fail("written", error))?;
        self.changed = false;
        Ok(())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.write();
    }
}

/// Who is asking, as a macro records it.
fn author(catalogue: &Catalogue) -> Result<Author, OperationError> {
    Ok(match catalogue.asker()? {
        crate::editing::AskerView::User => Author::User,
        crate::editing::AskerView::Agent { id, client, process, goals } => Author::Agent { agent: id, client, process, goals },
    })
}

/// Whether `asker` may change or remove `existing`: the user may change any, an agent only its
/// own.
fn may_change(asker: &Author, existing: &Macro) -> Result<(), OperationError> {
    match (asker, &existing.author) {
        (Author::User, _) => Ok(()),
        (Author::Agent { process, .. }, Author::Agent { process: own, .. }) if process == own => Ok(()),
        (Author::Agent { .. }, Author::User) => {
            Err(OperationError::invalid_input(format!("`{}` is the user's macro, which an agent can't change or remove: save yours under another name", existing.name)))
        }
        (Author::Agent { .. }, Author::Agent { agent, .. }) => {
            Err(OperationError::invalid_input(format!("`{}` is {agent}'s macro, which only it or the user can change or remove", existing.name)))
        }
    }
}

/// A name a macro can have: letters, digits, `-` and `_`, as `@name` runs it.
fn check_name(name: &str) -> Result<(), OperationError> {
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(OperationError::invalid_input(format!("`{name}` can't be a macro's name: use letters, digits, - and _, as @name runs it")));
    }
    Ok(())
}

/// Saves a macro as the caller: a new one, or one the caller may change.
pub(crate) fn save(catalogue: &Catalogue, name: &str, commands: Vec<String>, description: Option<String>) -> Result<Macro, OperationError> {
    check_name(name)?;
    let commands: Vec<String> = commands.into_iter().map(|command| command.trim().to_string()).filter(|command| !command.is_empty()).collect();
    if commands.is_empty() {
        return Err(OperationError::invalid_input("a macro needs at least one command"));
    }
    let asker = author(catalogue)?;
    let mut held = Held::open(catalogue)?;
    let runs = match held.find(name) {
        Some(existing) => {
            may_change(&asker, existing)?;
            existing.runs
        }
        None => 0,
    };
    let saved = Macro { name: name.to_string(), commands, description, author: asker, made: now(), runs };
    held.macros.retain(|r#macro| r#macro.name != name);
    held.macros.push(saved.clone());
    held.changed = true;
    held.write()?;
    Ok(saved)
}

/// Removes a macro the caller may change.
pub(crate) fn remove(catalogue: &Catalogue, name: &str) -> Result<(), OperationError> {
    let asker = author(catalogue)?;
    let mut held = Held::open(catalogue)?;
    let existing = held.find(name).ok_or_else(|| OperationError::not_found(format!("there is no macro `{name}`")))?;
    may_change(&asker, existing)?;
    held.macros.retain(|r#macro| r#macro.name != name);
    held.changed = true;
    held.write()
}

/// Makes an agent's macro, in the list or archived, one of the user's.
pub(crate) fn keep(catalogue: &Catalogue, name: Option<&str>, archived: Option<&str>, keep_as: Option<&str>) -> Result<Macro, OperationError> {
    let mut held = Held::open(catalogue)?;
    let found = match (name, archived) {
        (Some(name), None) => held.find(name).cloned().ok_or_else(|| OperationError::not_found(format!("there is no macro `{name}`")))?,
        (None, Some(id)) => held
            .archived()
            .into_iter()
            .rev()
            .find(|entry| entry.id == id)
            .map(|entry| entry.r#macro)
            .ok_or_else(|| OperationError::not_found(format!("the archive has no macro `{id}`")))?,
        _ => return Err(OperationError::invalid_input("give either `name`, a macro in the list, or `archived`, an id from list_macros' archive")),
    };
    let new_name = keep_as.unwrap_or(&found.name).to_string();
    check_name(&new_name)?;
    if archived.is_some() && held.find(&new_name).is_some() {
        return Err(OperationError::invalid_input(format!("a macro is already called `{new_name}`: keep it under another name with `as`")));
    }
    let kept = Macro { name: new_name.clone(), author: Author::User, ..found.clone() };
    held.macros.retain(|r#macro| r#macro.name != found.name || archived.is_some());
    held.macros.retain(|r#macro| r#macro.name != new_name);
    held.macros.push(kept.clone());
    held.changed = true;
    held.write()?;
    Ok(kept)
}

// --- list_macros ---

pub struct ListMacros;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMacrosInput {
    /// Also the agents' macros archived when their work ended, for keep_macro.
    #[serde(default)]
    pub archived: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct ListMacrosOutput {
    /// Every macro in use, with its author.
    pub macros: Vec<Macro>,
    /// With `archived`: agents' macros once their work ended, oldest first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<Vec<Archived>>,
}

impl Operation for ListMacros {
    const NAME: &'static str = "list_macros";
    const DESCRIPTION: &'static str = "Lists the macros in the models folder: named lists of commands in the command language (run_commands), each with its author, the user or an agent, and how often it ran. Run one with run_commands as @name, with arguments for its $1, $2 and so on. An agent's macros last for its work and are archived when its process ends. With `archived`, also the archive.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = ListMacrosInput;
    type Output = ListMacrosOutput;

    fn run(catalogue: &Catalogue, input: ListMacrosInput) -> Result<ListMacrosOutput, OperationError> {
        let held = Held::open(catalogue)?;
        Ok(ListMacrosOutput { macros: held.macros.clone(), archived: input.archived.then(|| held.archived()) })
    }
}

// --- save_macro ---

pub struct SaveMacro;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMacroInput {
    /// Letters, digits, `-` and `_`.
    pub name: String,
    /// Commands in the command language, such as `add Station`, `connect $selected > $last`.
    /// `$1`, `$2` and so on are its arguments, `$selected` the instance selected, and `$last` the
    /// instance the last `add` placed.
    pub commands: Vec<String>,
    pub description: Option<String>,
}

impl Operation for SaveMacro {
    const NAME: &'static str = "save_macro";
    const DESCRIPTION: &'static str = "Saves a macro, a named list of commands in the command language (run_commands), with $1, $2 and so on for its arguments, $selected for the instance selected and $last for the instance the last add placed. It replaces one of the same name the caller made. An agent can't change the user's macros or another agent's, and an agent's macros last only while its process runs: then they're archived. Ask the user to keep one that should stay.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = SaveMacroInput;
    type Output = Macro;

    fn run(catalogue: &Catalogue, input: SaveMacroInput) -> Result<Macro, OperationError> {
        save(catalogue, &input.name, input.commands, input.description)
    }
}

// --- remove_macro ---

pub struct RemoveMacro;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveMacroInput {
    pub name: String,
}

#[derive(Serialize, JsonSchema)]
pub struct RemoveMacroOutput {
    pub removed: String,
}

impl Operation for RemoveMacro {
    const NAME: &'static str = "remove_macro";
    const DESCRIPTION: &'static str = "Removes a macro the caller made. An agent can't remove the user's macros or another agent's.";
    const EFFECT: Effect = Effect::Destructive;
    type Input = RemoveMacroInput;
    type Output = RemoveMacroOutput;

    fn run(catalogue: &Catalogue, input: RemoveMacroInput) -> Result<RemoveMacroOutput, OperationError> {
        remove(catalogue, &input.name)?;
        Ok(RemoveMacroOutput { removed: input.name })
    }
}

// --- keep_macro ---

pub struct KeepMacro;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeepMacroInput {
    /// A macro in the list, such as one an agent is using now.
    pub name: Option<String>,
    /// Or an archived one, by its id from list_macros.
    pub archived: Option<String>,
    /// Another name to keep it under.
    #[serde(rename = "as")]
    pub keep_as: Option<String>,
}

impl Operation for KeepMacro {
    const NAME: &'static str = "keep_macro";
    const DESCRIPTION: &'static str = "For the user: makes an agent's macro, one in the list or an archived one, one of the user's own, which then stays and agents can't change.";
    const EFFECT: Effect = Effect::Additive;
    const USER_ONLY: bool = true;
    type Input = KeepMacroInput;
    type Output = Macro;

    fn run(catalogue: &Catalogue, input: KeepMacroInput) -> Result<Macro, OperationError> {
        keep(catalogue, input.name.as_deref(), input.archived.as_deref(), input.keep_as.as_deref())
    }
}
