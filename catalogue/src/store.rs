//! The results store: one SQLite database, `<models folder>/.crabsim/results.sqlite`, keeping
//! every run's settings, its model's canonical text, each observation and each figure, so results
//! survive restarts and can be queried with SQL (ADR 0010). WAL mode lets a CLI and an MCP server
//! use it at once.

use crate::error::OperationError;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The store's file name inside `.crabsim`.
pub const STORE_FILE: &str = "results.sqlite";

/// The tables' version, kept in SQLite's `user_version`. A change to the tables raises it.
const TABLES_VERSION: i64 = 1;

/// The most rows `query_results` returns.
pub const QUERY_ROW_LIMIT: usize = 1_000;

/// How long `query_results` may take.
pub const QUERY_TIME_LIMIT: Duration = Duration::from_secs(5);

const TABLES: &str = "
CREATE TABLE IF NOT EXISTS runs (
    number INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT GENERATED ALWAYS AS ('run-' || number) VIRTUAL UNIQUE,
    model_path TEXT NOT NULL,
    model_text TEXT NOT NULL,
    model_hash TEXT NOT NULL,
    seed TEXT NOT NULL,
    length_s REAL NOT NULL,
    warm_up_s REAL NOT NULL,
    observations INTEGER NOT NULL,
    label TEXT,
    source TEXT NOT NULL,
    crabsim_version TEXT NOT NULL,
    started TEXT NOT NULL,
    finished TEXT,
    state TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS observations (
    run TEXT NOT NULL,
    number INTEGER NOT NULL,
    seed TEXT NOT NULL,
    events INTEGER NOT NULL,
    wall_time_s REAL NOT NULL,
    error TEXT,
    error_time_s REAL,
    PRIMARY KEY (run, number)
);
CREATE TABLE IF NOT EXISTS statistics (
    run TEXT NOT NULL,
    observation INTEGER NOT NULL,
    object TEXT NOT NULL,
    class TEXT NOT NULL,
    built_in_class TEXT NOT NULL,
    statistic TEXT NOT NULL,
    value REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS statistics_by_run ON statistics (run, statistic, object);
CREATE INDEX IF NOT EXISTS statistics_by_object ON statistics (run, object);
";

/// The tables and statistic names, as `query_results` describes them to an agent.
macro_rules! tables_description {
    () => {
        concat!(
            "Tables: ",
            "runs(id TEXT such as 'run-3', model_path, model_text: the canonical model file the run used, model_hash: SHA-256 of model_text so equal hashes mean the same model, seed TEXT, length_s, warm_up_s, observations, label, source: 'mcp' or 'cli', crabsim_version, started and finished: UTC ISO 8601, state: queued|running|done|failed|cancelled). ",
            "observations(run, number from 1, seed TEXT: the observation's own seed, events, wall_time_s, error, error_time_s: simulated seconds). ",
            "statistics(run, observation, object: instance name, class, built_in_class: Source|Station|Buffer|Drain, statistic, value REAL), one row per figure of an observation that ran to its end. ",
            "Statistics: mus_in, mus_out, working_share, blocked_share, idle_share, failed_share, paused_share (each 0 to 1 of the time measured after the warm-up), failures, availability (0 to 1), and for Drains throughput_per_hour and mean_lead_time_s. Times are in seconds. ",
            "Example: SELECT object, AVG(value) FROM statistics WHERE run = 'run-3' AND statistic = 'blocked_share' GROUP BY object HAVING AVG(value) > 0.3"
        )
    };
}

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<QueryResults>()]
}

/// The results store, open for writing.
pub struct Store {
    connection: Connection,
    path: PathBuf,
}

impl Store {
    /// Opens the store in `crabsim_folder`, creating the folder, the file and its tables if they
    /// don't exist yet.
    pub fn open(crabsim_folder: &Path) -> Result<Store, OperationError> {
        std::fs::create_dir_all(crabsim_folder)
            .map_err(|error| OperationError::run_error(format!("the results store's folder can't be made: {error}")))?;
        let path = crabsim_folder.join(STORE_FILE);
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > TABLES_VERSION {
            return Err(OperationError::run_error(format!(
                "the results store was written by a newer crabsim (tables version {version}, this one knows {TABLES_VERSION})"
            )));
        }
        connection.execute_batch(TABLES)?;
        connection.pragma_update(None, "user_version", TABLES_VERSION)?;
        Ok(Store { connection, path })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    /// Runs one read-only query on a connection of its own that can't write, capped at
    /// [`QUERY_ROW_LIMIT`] rows and [`QUERY_TIME_LIMIT`].
    pub fn query(&self, sql: &str) -> Result<QueryResultsOutput, OperationError> {
        let refused = |why: &str| OperationError::invalid_input(format!("{why}. Only one SELECT (or WITH ... SELECT) is allowed, and it can't change the store"));
        let first_word = without_comments(sql).split(|c: char| !c.is_ascii_alphabetic()).find(|word| !word.is_empty()).unwrap_or("").to_ascii_lowercase();
        if first_word != "select" && first_word != "with" {
            return Err(refused("the query doesn't start with SELECT or WITH"));
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(&self.path, flags)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "query_only", "ON")?;
        // No other databases, so a query can't read files outside the models folder.
        connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0)?;
        let deadline = Instant::now() + QUERY_TIME_LIMIT;
        connection.progress_handler(10_000, Some(move || Instant::now() > deadline))?;

        let mut statement = connection.prepare(sql).map_err(|error| OperationError::invalid_input(format!("the query can't be run: {error}")))?;
        if !statement.readonly() {
            return Err(refused("the query would change the store"));
        }
        let columns: Vec<String> = statement.column_names().into_iter().map(String::from).collect();
        let mut rows = statement.query([]).map_err(|error| query_failed(error, deadline))?;
        let mut out = Vec::new();
        let mut truncated = false;
        while let Some(row) = rows.next().map_err(|error| query_failed(error, deadline))? {
            if out.len() == QUERY_ROW_LIMIT {
                truncated = true;
                break;
            }
            let values = (0..columns.len())
                .map(|index| match row.get_ref(index) {
                    Ok(ValueRef::Null) | Err(_) => Value::Null,
                    Ok(ValueRef::Integer(n)) => Value::from(n),
                    Ok(ValueRef::Real(x)) => Value::from(x),
                    Ok(ValueRef::Text(text)) => Value::from(String::from_utf8_lossy(text).into_owned()),
                    Ok(ValueRef::Blob(bytes)) => Value::from(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                })
                .collect();
            out.push(values);
        }
        Ok(QueryResultsOutput { columns, rows: out, truncated })
    }
}

fn query_failed(error: rusqlite::Error, deadline: Instant) -> OperationError {
    if Instant::now() > deadline {
        OperationError::invalid_input(format!("the query took longer than {} s and was stopped: narrow it down", QUERY_TIME_LIMIT.as_secs()))
    } else {
        OperationError::invalid_input(format!("the query failed: {error}"))
    }
}

/// `sql` without its leading comments and white space.
fn without_comments(sql: &str) -> &str {
    let mut rest = sql.trim_start();
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, next)| next).trim_start();
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, next)| next).trim_start();
        } else {
            return rest;
        }
    }
}

// --- query_results ---

pub struct QueryResults;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryResultsInput {
    /// One SQLite SELECT statement on the tables `runs`, `observations` and `statistics`.
    pub sql: String,
}

#[derive(Serialize, JsonSchema)]
pub struct QueryResultsOutput {
    pub columns: Vec<String>,
    /// Each row's values, in the order of `columns`.
    pub rows: Vec<Vec<Value>>,
    /// More rows matched than the cap of 1,000: narrow the query or aggregate.
    pub truncated: bool,
}

impl Operation for QueryResults {
    const NAME: &'static str = "query_results";
    const DESCRIPTION: &'static str = concat!(
        "Runs one read-only SQL query (SQLite) on the results store, which holds every run's settings and results, by agents and people alike. Use it to compare runs or models side by side, or ask anything run_results doesn't. At most 1,000 rows and 5 seconds. ",
        tables_description!()
    );
    const EFFECT: Effect = Effect::ReadOnly;
    type Input = QueryResultsInput;
    type Output = QueryResultsOutput;

    fn run(catalogue: &Catalogue, input: QueryResultsInput) -> Result<QueryResultsOutput, OperationError> {
        catalogue.store()?.query(&input.sql)
    }
}
