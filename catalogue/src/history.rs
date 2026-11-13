//! The change history, undo and redo (spec issue 107, ADR 0017): every change crabsim makes to a
//! model file, by anyone, in order, each with its author, time and wording, kept in one SQLite
//! database, `<models folder>/.crabsim/history.sqlite`, so it survives restarts and is shared by
//! the window, the CLI and agents.
//!
//! An entry points at the file's whole text before and after its change, each text stored once,
//! so an undo writes the earlier text back exactly: a removed instance returns with its stream,
//! its place in the file and its connections' order, and the file's hash is what it was. The
//! history is one line with no branches: the undone entries are always its newest, and a new
//! change drops them. It is only ever read and written while the edit state is held (see
//! `editing.rs`), so two processes never change it at once.
//!
//! A history that no longer ends at the file's text, such as one restored from a backup, is
//! dropped rather than used. Keeping it is never allowed to fail a change that was already saved.

use crate::editing::Editing;
use crate::error::{ErrorKind, OperationError};
use crate::model_files::read_file;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<GetHistory>(), entry::<UndoModel>(), entry::<RedoModel>()]
}

/// The history's file name inside `.crabsim`.
pub const HISTORY_FILE: &str = "history.sqlite";

/// The most changes a model file's history keeps, unless set when the catalogue is opened.
pub const HISTORY_CHANGES: usize = 200;

/// The most bytes of model text a model file's history keeps, unless set when the catalogue is
/// opened. The newest change always stays.
pub const HISTORY_BYTES: u64 = 50 * 1024 * 1024;

/// The most entries `get_history` gives.
pub const HISTORY_LIST_LIMIT: usize = 200;

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    text TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    at INTEGER NOT NULL,
    author TEXT NOT NULL,
    process TEXT,
    kind TEXT NOT NULL,
    wording TEXT NOT NULL,
    grp INTEGER NOT NULL,
    before INTEGER NOT NULL,
    after INTEGER NOT NULL,
    undone INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS entries_by_path ON entries (path, id);
CREATE INDEX IF NOT EXISTS versions_by_path ON versions (path);
";

/// What kind of change an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A batch of changes, through edit_model or run_commands.
    Change,
    /// The whole file written over, through write_model.
    Written,
    /// A change made outside crabsim, accepted with adopt_model.
    Adopted,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Change => "change",
            Kind::Written => "written",
            Kind::Adopted => "adopted",
        }
    }

    fn of(name: &str) -> Kind {
        match name {
            "written" => Kind::Written,
            "adopted" => Kind::Adopted,
            _ => Kind::Change,
        }
    }
}

/// The file's text before a change.
pub(crate) enum Before<'a> {
    /// As it was read.
    Text(&'a str),
    /// Whatever the history ends at, which must have this hash: for an adoption, where the text
    /// crabsim last saved is known only by its hash.
    Head(&'a str),
}

/// A change to keep.
pub(crate) struct Change<'a> {
    pub before: Before<'a>,
    pub after: &'a str,
    pub kind: Kind,
    pub wording: String,
}

/// An entry as stored.
struct Row {
    id: i64,
    at: u64,
    author: String,
    process: Option<String>,
    kind: String,
    wording: String,
    group: i64,
    before: i64,
    after: i64,
    undone: bool,
}

const COLUMNS: &str = "id, at, author, process, kind, wording, grp, before, after, undone";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: row.get(0)?,
        at: row.get::<_, i64>(1)? as u64,
        author: row.get(2)?,
        process: row.get(3)?,
        kind: row.get(4)?,
        wording: row.get(5)?,
        group: row.get(6)?,
        before: row.get(7)?,
        after: row.get(8)?,
        undone: row.get::<_, i64>(9)? != 0,
    })
}

impl Row {
    /// Whether the asker made it: an agent by its process, the user when no agent did.
    fn by(&self, agent: Option<&str>) -> bool {
        self.process.as_deref() == agent
    }

    fn view(&self, agent: Option<&str>) -> EntryView {
        EntryView {
            number: self.id,
            author: self.author.clone(),
            at: crate::editing::iso(self.at),
            what: self.wording.clone(),
            kind: Kind::of(&self.kind),
            group: self.group,
            undone: self.undone,
            yours: self.by(agent),
        }
    }
}

/// Which way a step goes.
#[derive(Clone, Copy, PartialEq)]
enum Way {
    Undo,
    Redo,
}

/// What a step did.
struct Stepped {
    /// The entries it undid or redid, in the order it did them.
    entries: Vec<Row>,
    /// The file's text now.
    text: String,
    /// For an agent given fewer steps than it asked for: whose change stopped it.
    stopped: Option<String>,
}

/// The history, open.
pub(crate) struct History {
    connection: Connection,
}

fn failed(error: rusqlite::Error) -> OperationError {
    OperationError::run_error(format!("the change history: {error}"))
}

impl History {
    pub(crate) fn open(crabsim_folder: &Path) -> Result<History, OperationError> {
        std::fs::create_dir_all(crabsim_folder).map_err(|error| OperationError::run_error(format!("the change history's folder can't be made: {error}")))?;
        let connection = Connection::open(crabsim_folder.join(HISTORY_FILE)).map_err(failed)?;
        connection.busy_timeout(Duration::from_secs(10)).map_err(failed)?;
        connection.pragma_update(None, "journal_mode", "WAL").map_err(failed)?;
        connection.execute_batch(TABLES).map_err(failed)?;
        Ok(History { connection })
    }

    /// The version the history ends at, with its hash: the text after its newest change still in
    /// effect, or before its oldest undone one when all are undone.
    fn head(transaction: &Transaction<'_>, path: &str) -> rusqlite::Result<Option<(i64, String)>> {
        let read = |row: &rusqlite::Row<'_>| Ok((row.get(0)?, row.get(1)?));
        let applied = transaction
            .query_row(
                "SELECT v.id, v.hash FROM entries e JOIN versions v ON v.id = e.after WHERE e.path = ?1 AND e.undone = 0 ORDER BY e.id DESC LIMIT 1",
                [path],
                read,
            )
            .optional()?;
        if applied.is_some() {
            return Ok(applied);
        }
        transaction
            .query_row(
                "SELECT v.id, v.hash FROM entries e JOIN versions v ON v.id = e.before WHERE e.path = ?1 AND e.undone = 1 ORDER BY e.id ASC LIMIT 1",
                [path],
                read,
            )
            .optional()
    }

    fn clear(transaction: &Transaction<'_>, path: &str) -> rusqlite::Result<()> {
        transaction.execute("DELETE FROM entries WHERE path = ?1", [path])?;
        transaction.execute("DELETE FROM versions WHERE path = ?1", [path])?;
        Ok(())
    }

    /// Removes the texts no entry points at any more.
    fn prune(transaction: &Transaction<'_>, path: &str) -> rusqlite::Result<()> {
        transaction.execute(
            "DELETE FROM versions WHERE path = ?1 AND id NOT IN (SELECT before FROM entries WHERE path = ?1 UNION SELECT after FROM entries WHERE path = ?1)",
            [path],
        )?;
        Ok(())
    }

    fn insert_version(transaction: &Transaction<'_>, path: &str, text: &str) -> rusqlite::Result<i64> {
        transaction.execute("INSERT INTO versions (path, hash, text) VALUES (?1, ?2, ?3)", (path, crate::edit::hash(text), text))?;
        Ok(transaction.last_insert_rowid())
    }

    /// Forgets a file's history, such as when the file is deleted.
    pub(crate) fn forget(&mut self, path: &str) -> rusqlite::Result<()> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        History::clear(&transaction, path)?;
        transaction.commit()
    }

    /// Keeps a change to `path` as its newest entry, dropping what could be redone, and the
    /// oldest entries past `changes` entries or `bytes` of text. A change that changed nothing
    /// isn't kept. Gives the entry's group: `group`, or its own number to start one.
    #[allow(clippy::too_many_arguments)]
    fn append(&mut self, path: &str, change: &Change<'_>, author: &str, process: Option<&str>, at: u64, group: Option<i64>, changes: usize, bytes: u64) -> rusqlite::Result<Option<i64>> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = History::head(&transaction, path)?;
        let after_hash = crate::edit::hash(change.after);
        let before = match (&change.before, head) {
            (Before::Text(text), Some((version, hash))) if hash == crate::edit::hash(text) => Some((version, hash)),
            (Before::Head(expected), Some((version, hash))) if hash == *expected => Some((version, hash)),
            (before, head) => {
                // The history doesn't end at this text, or there is none: it starts afresh.
                if head.is_some() {
                    History::clear(&transaction, path)?;
                }
                match before {
                    Before::Text(text) => Some((History::insert_version(&transaction, path, text)?, crate::edit::hash(text))),
                    Before::Head(_) => None,
                }
            }
        };
        let Some((before, _)) = before.filter(|(_, hash)| *hash != after_hash) else {
            History::prune(&transaction, path)?;
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute("DELETE FROM entries WHERE path = ?1 AND undone = 1", [path])?;
        let after = History::insert_version(&transaction, path, change.after)?;
        transaction.execute(
            "INSERT INTO entries (path, at, author, process, kind, wording, grp, before, after) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8)",
            (path, at as i64, author, process, change.kind.name(), &change.wording, before, after),
        )?;
        let id = transaction.last_insert_rowid();
        let group = group.unwrap_or(id);
        transaction.execute("UPDATE entries SET grp = ?1 WHERE id = ?2", (group, id))?;
        loop {
            History::prune(&transaction, path)?;
            let (count, size): (i64, i64) = transaction.query_row(
                "SELECT (SELECT COUNT(*) FROM entries WHERE path = ?1), (SELECT COALESCE(SUM(LENGTH(CAST(text AS BLOB))), 0) FROM versions WHERE path = ?1)",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count <= 1 || (count as usize <= changes && size as u64 <= bytes) {
                break;
            }
            transaction.execute("DELETE FROM entries WHERE id = (SELECT MIN(id) FROM entries WHERE path = ?1)", [path])?;
        }
        transaction.commit()?;
        Ok(Some(group))
    }

    /// A file's entries, newest first, at most `limit`, and whether there are more.
    fn list(&self, path: &str, limit: usize) -> rusqlite::Result<(Vec<Row>, bool)> {
        let mut statement = self.connection.prepare(&format!("SELECT {COLUMNS} FROM entries WHERE path = ?1 ORDER BY id DESC LIMIT ?2"))?;
        let mut rows = statement.query_map((path, limit as i64 + 1), row)?.collect::<rusqlite::Result<Vec<Row>>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        Ok((rows, more))
    }

    /// Undoes or redoes up to `steps` steps of `path`, whose text now has the hash `current`. A
    /// step is every entry of one group. An agent, given as its process, steps only through its
    /// own entries, and the user, given as none, through anyone's. `write` saves the file's new text, and nothing is kept unless it succeeds.
    fn step(
        &mut self,
        way: Way,
        path: &str,
        steps: usize,
        agent: Option<&str>,
        current: &str,
        write: impl FnOnce(&str) -> Result<(), OperationError>,
    ) -> Result<Stepped, OperationError> {
        let transaction = self.connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(failed)?;
        let nothing = |what: &str| OperationError::new(ErrorKind::NothingToUndo, what.to_string());
        match History::head(&transaction, path).map_err(failed)? {
            None => return Err(nothing(if way == Way::Undo { "there is nothing to undo: this model file's history is empty" } else { "there is nothing to redo" })),
            Some((_, hash)) if hash != current => {
                History::clear(&transaction, path).map_err(failed)?;
                transaction.commit().map_err(failed)?;
                return Err(nothing("this model file's history didn't match the file any more, so it was dropped: there is nothing to undo"));
            }
            Some(_) => {}
        }
        // The entries a step could take, nearest first: the newest in effect, or the oldest undone.
        let (undone, order) = if way == Way::Undo { (0, "DESC") } else { (1, "ASC") };
        let candidates = {
            let mut statement = transaction.prepare(&format!("SELECT {COLUMNS} FROM entries WHERE path = ?1 AND undone = ?2 ORDER BY id {order}")).map_err(failed)?;
            statement.query_map((path, undone), row).and_then(|rows| rows.collect::<rusqlite::Result<Vec<Row>>>()).map_err(failed)?
        };
        let mut taken: Vec<Row> = Vec::new();
        let mut stopped = None;
        let mut rest = candidates.into_iter().peekable();
        for _ in 0..steps {
            let Some(group) = rest.peek().map(|entry| entry.group) else { break };
            let mut step = Vec::new();
            while let Some(entry) = rest.next_if(|entry| entry.group == group) {
                step.push(entry);
            }
            if agent.is_some() && let Some(other) = step.iter().find(|entry| !entry.by(agent)) {
                stopped = Some(other.author.clone());
                break;
            }
            taken.extend(step);
        }
        let Some(last) = taken.last() else {
            return Err(match stopped {
                Some(author) if way == Way::Undo => OperationError::new(
                    ErrorKind::NotYours,
                    format!("the latest change to this model file was made by {author}, and you can undo only your own latest changes: get_history shows who changed what"),
                ),
                Some(author) => OperationError::new(ErrorKind::NotYours, format!("the change to redo was made by {author}, and you can redo only your own")),
                None => nothing(if way == Way::Undo { "there is nothing to undo" } else { "there is nothing to redo" }),
            });
        };
        let version = if way == Way::Undo { last.before } else { last.after };
        let text: String = transaction.query_row("SELECT text FROM versions WHERE id = ?1", [version], |row| row.get(0)).map_err(failed)?;
        for entry in &taken {
            transaction.execute("UPDATE entries SET undone = ?1 WHERE id = ?2", (i64::from(way == Way::Undo), entry.id)).map_err(failed)?;
        }
        write(&text)?;
        transaction.commit().map_err(failed)?;
        Ok(Stepped { entries: taken, text, stopped })
    }
}

impl Catalogue {
    /// Runs `with` on the change history, opened on first use.
    fn with_history<T>(&self, with: impl FnOnce(&mut History) -> Result<T, OperationError>) -> Result<T, OperationError> {
        let mut history = self.history.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if history.is_none() {
            *history = Some(History::open(&self.folder().crabsim_folder())?);
        }
        with(history.as_mut().expect("opened above"))
    }

    /// From now on, every change this catalogue makes is one step of undo, until
    /// [`Catalogue::end_group`]: for a front-end carrying out one command of the user's as
    /// several changes, such as a macro run with a count.
    pub fn begin_group(&self) {
        *self.group.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(None);
    }

    /// Ends the group begun with [`Catalogue::begin_group`]: each change is its own step again.
    pub fn end_group(&self) {
        *self.group.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

/// Keeps a change to the model file at `path`, already saved, in its history, made by whoever
/// holds `editing`. A history that can't be written is logged and never fails the change.
pub(crate) fn keep(catalogue: &Catalogue, editing: &Editing, path: &str, change: Change<'_>) {
    let kept = (|| -> Result<(), OperationError> {
        let key = crate::editing::key_of(catalogue, path)?;
        let (author, process) = editing.author();
        let mut group = catalogue.group.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let open = group.is_some();
        let joined = (*group).flatten();
        let (changes, bytes) = catalogue.history_limits;
        let started = catalogue.with_history(|history| history.append(&key, &change, &author, process.as_deref(), editing.now(), joined, changes, bytes).map_err(failed))?;
        if open && joined.is_none() {
            *group = Some(started);
        }
        Ok(())
    })();
    if let Err(error) = kept {
        log::warn!("a change to `{path}` wasn't kept in its history: {error}");
    }
}

/// Forgets the history of the model file at `path`, which was deleted or is new.
pub(crate) fn forget(catalogue: &Catalogue, path: &str) {
    let forgotten = crate::editing::key_of(catalogue, path).and_then(|key| catalogue.with_history(|history| history.forget(&key).map_err(failed)));
    if let Err(error) = forgotten {
        log::warn!("the history of `{path}` wasn't dropped: {error}");
    }
}

// ---------- What an entry looks like ----------

/// One change in a model file's history.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EntryView {
    /// Its number, rising with time across the models folder.
    pub number: i64,
    /// Who made it: `the user`, or an agent such as `agent-3 (claude-code)`.
    pub author: String,
    /// When, UTC.
    pub at: String,
    /// What it did, in the command language, such as `connect Split > Drill`, or `written whole`
    /// or `changed outside crabsim`.
    pub what: String,
    pub kind: Kind,
    /// Entries of one group were one command, and are undone and redone together.
    pub group: i64,
    /// It was undone and can be redone.
    pub undone: bool,
    /// You made it.
    pub yours: bool,
}

// --- get_history ---

pub struct GetHistory;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetHistoryInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: String,
    /// The most entries to give, newest first. 50 if left out, and at most 200.
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct GetHistoryOutput {
    pub path: String,
    /// The changes, newest first. The undone ones, which redo_model brings back, come first.
    pub entries: Vec<EntryView>,
    /// There are older entries than these.
    pub more: bool,
}

impl Operation for GetHistory {
    const NAME: &'static str = "get_history";
    const DESCRIPTION: &'static str = "Gives a model file's change history: every change crabsim made to it, by anyone, newest first, each with its author, its time, what it did in the command language (such as `connect Split > Drill`), its kind (a change, the file written whole, or a change from outside crabsim that was adopted), its group, whether it was undone, and whether you made it. It gives who changed what, never the model's earlier text. A file keeps its last 200 changes, fewer for a large model, and a deleted file's history is gone.";
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = GetHistoryInput;
    type Output = GetHistoryOutput;

    fn run(catalogue: &Catalogue, input: GetHistoryInput) -> Result<GetHistoryOutput, OperationError> {
        let key = crate::editing::key_of(catalogue, &input.path)?;
        let limit = input.limit.unwrap_or(50).clamp(1, HISTORY_LIST_LIMIT);
        let agent = if catalogue.source().is_user() { None } else { Some(catalogue.process_lock()?.to_string()) };
        let (rows, more) = catalogue.with_history(|history| history.list(&key, limit).map_err(failed))?;
        Ok(GetHistoryOutput { path: input.path, entries: rows.iter().map(|row| row.view(agent.as_deref())).collect(), more })
    }
}

// --- undo_model and redo_model ---

pub struct UndoModel;
pub struct RedoModel;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepInput {
    /// A model file in the models folder, such as `line.json`.
    pub path: String,
    /// How many steps, 1 if left out. A step is one change, or every change one command made.
    pub steps: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct UndoModelOutput {
    pub path: String,
    /// SHA-256 of the file as it is now, for the next `expected_hash`.
    pub file_hash: String,
    /// The changes undone, newest first.
    pub undone: Vec<EntryView>,
    /// For an agent given fewer steps than it asked for: whose change it stopped at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RedoModelOutput {
    pub path: String,
    /// SHA-256 of the file as it is now, for the next `expected_hash`.
    pub file_hash: String,
    /// The changes redone, oldest first.
    pub redone: Vec<EntryView>,
    /// For an agent given fewer steps than it asked for: whose change it stopped at.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stopped: Option<String>,
}

fn step(catalogue: &Catalogue, way: Way, input: &StepInput) -> Result<(Vec<EntryView>, String, Option<String>), OperationError> {
    let steps = input.steps.unwrap_or(1);
    if steps == 0 {
        return Err(OperationError::invalid_input("`steps` is how many steps to take, 1 or more"));
    }
    let (file, _) = read_file(catalogue, &input.path)?;
    let mut editing = crate::editing::before_change(catalogue, &input.path, None)?;
    // Read under the edit state, so it is the text the history must end at.
    let (_, current) = read_file(catalogue, &input.path)?;
    let key = crate::editing::key_of(catalogue, &input.path)?;
    let (_, agent) = editing.author();
    let write = |text: &str| std::fs::write(&file, text).map_err(|error| OperationError::run_error(format!("`{}` can't be written: {error}", input.path)));
    let stepped = catalogue.with_history(|history| history.step(way, &key, steps, agent.as_deref(), &crate::edit::hash(&current), write))?;
    editing.record(&stepped.text);
    Ok((stepped.entries.iter().map(|row| row.view(agent.as_deref())).collect(), crate::edit::hash(&stepped.text), stepped.stopped))
}

impl Operation for UndoModel {
    const NAME: &'static str = "undo_model";
    const DESCRIPTION: &'static str = "Undoes the latest change to a model file: the file is again exactly as it was before it, text and hash, so an instance that was removed is back with its connections and its stream. `steps` undoes several. A step is one change, or everything one command made, such as a macro run. You can undo only your own latest changes, while you hold your claim on the file (claim_model): it stops at the first change by someone else, saying whose in `stopped`, and is refused with not_yours when the latest change isn't yours. With nothing to undo it is refused with nothing_to_undo. redo_model brings an undone change back, until the next change to the file, which drops what could be redone. A file changed outside crabsim is refused until adopt_model, after which one undo takes the outside change back. get_history shows the changes.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = StepInput;
    type Output = UndoModelOutput;

    fn run(catalogue: &Catalogue, input: StepInput) -> Result<UndoModelOutput, OperationError> {
        let (undone, file_hash, stopped) = step(catalogue, Way::Undo, &input)?;
        Ok(UndoModelOutput { path: input.path, file_hash, undone, stopped })
    }
}

impl Operation for RedoModel {
    const NAME: &'static str = "redo_model";
    const DESCRIPTION: &'static str = "Brings back the change to a model file that undo_model last undid: the file is again exactly as it was after it. `steps` redoes several, each one change or everything one command made. You can redo only your own changes, while you hold your claim on the file. Any new change to the file drops what could be redone, and with nothing to redo it is refused with nothing_to_undo.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = StepInput;
    type Output = RedoModelOutput;

    fn run(catalogue: &Catalogue, input: StepInput) -> Result<RedoModelOutput, OperationError> {
        let (redone, file_hash, stopped) = step(catalogue, Way::Redo, &input)?;
        Ok(RedoModelOutput { path: input.path, file_hash, redone, stopped })
    }
}
