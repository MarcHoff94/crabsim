//! The headless CLI. Without a subcommand it runs a model file, or the built-in demo model, with a
//! seed for a simulated length, then prints each object's final statistics and how long the run
//! took, saving nothing. An invalid model file prints every problem in it and runs nothing.
//!
//! With a models folder it is people's front-end on the operation catalogue, the same one agents
//! use through the MCP server: `run` starts an experiment and waits for it, `runs` lists the runs
//! in the results store, `results` prints a run's results and `query` queries the store with SQL.
//! `edit` changes a model with a batch of changes, `attributes` shows an object's values and where
//! they come from, `adopt` accepts a file changed outside crabsim, and `queue` shows and controls
//! who may change a model: the person using the CLI is the user, who controls the edit queue.

mod tables;

use clap::{Args, Parser, Subcommand};
use crabsim::time::{format_time_exact, parse_time};
use crabsim::{demo, ModelDescription, ObjectKind, ObjectStatistics, RunState, SimTime};
use crabsim_catalogue::{Catalogue, OperationError, Settings, Source};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(
    name = "crabsim",
    about = "Runs a crabsim model headless and prints its statistics, or, with a models folder, runs experiments and reads their results",
    args_conflicts_with_subcommands = true
)]
struct Opts {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    single: SingleRun,
}

/// One run, printed and not saved.
#[derive(Args, Debug)]
struct SingleRun {
    /// A model file to run. Without one, the built-in demo model runs.
    #[arg(long)]
    model: Option<PathBuf>,
    /// Seed for the run's random numbers. The same seed gives the same results.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Simulated length, written as the UI shows time, such as `90`, `1:30`, `8:00:00` or
    /// `30d 12:00:00`.
    #[arg(long, default_value = "1d", value_parser = length)]
    length: SimTime,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Runs an experiment on a model file in a models folder, waits for it, prints its results
    /// and keeps them in the folder's results store.
    Run {
        /// The models folder. It must not hold source code.
        #[arg(long)]
        models: PathBuf,
        /// The model file, as a path inside the models folder, such as `line.json`.
        #[arg(long)]
        model: String,
        /// The experiment's seed. Observation k gets its own seed from it.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Simulated length of each observation after the warm-up, such as `8:00:00` or `30d`.
        #[arg(long, default_value = "1d")]
        length: String,
        /// Simulated time at the start of each observation that its statistics leave out.
        #[arg(long)]
        warm_up: Option<String>,
        /// How many observations, run in parallel, one per CPU core. 10 if left out, as for
        /// agents.
        #[arg(long)]
        observations: Option<u32>,
        /// A name for the run, such as `baseline`.
        #[arg(long)]
        label: Option<String>,
    },
    /// Lists the runs in a models folder's results store, newest first, by people and agents.
    Runs {
        #[arg(long)]
        models: PathBuf,
    },
    /// Prints a run's results: by default every Drain and the most blocked and failed objects.
    Results {
        #[arg(long)]
        models: PathBuf,
        /// The run's id, such as `run-3`.
        run: String,
        /// An instance to show instead of the summary. Give it more than once for more.
        #[arg(long = "object")]
        objects: Vec<String>,
        /// A class whose instances to show instead of the summary. Give it more than once for more.
        #[arg(long = "class")]
        classes: Vec<String>,
        /// Also every observation: its seed, events, wall time and any error.
        #[arg(long)]
        values: bool,
    },
    /// Queries the results store with one read-only SQL SELECT.
    Query {
        #[arg(long)]
        models: PathBuf,
        /// Such as "SELECT id, label, state FROM runs".
        sql: String,
    },
    /// Changes a model file with a batch of changes, all or nothing, as edit_model does: a JSON
    /// list such as [{"change": "set_attribute", "target": {"instance": "Mill 1"},
    /// "attribute": "process_time", "value": "0:45"}].
    Edit {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
        /// A file holding the changes as JSON, or `-` to read them from stdin.
        changes: String,
        /// The hash of the file as you read it: refused if it changed since.
        #[arg(long)]
        expected_hash: Option<String>,
        /// Make this change even while an agent holds the editor right, which it keeps.
        #[arg(long)]
        take_over: bool,
    },
    /// Shows every attribute of an instance or class, its value and where it comes from.
    Attributes {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
        /// The instance's name, or the class's with --class.
        name: String,
        /// The name is a class's.
        #[arg(long)]
        class: bool,
    },
    /// Accepts a model file changed outside crabsim, such as by a text editor or git, as it is,
    /// so crabsim may change it again.
    Adopt {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
    },
    /// Compares two runs, such as a model and a variant: each figure's difference b minus a with
    /// its 95% interval, paired where the runs drew the same random numbers.
    Compare {
        #[arg(long)]
        models: PathBuf,
        /// The base run, such as `run-3`.
        a: String,
        /// The run compared with it.
        b: String,
        /// An instance to compare instead of the default. Give it more than once for more.
        #[arg(long = "object")]
        objects: Vec<String>,
        /// A class whose instances to compare instead of the default.
        #[arg(long = "class")]
        classes: Vec<String>,
    },
    /// Shows who may change model files and who waits, and controls it, as the user.
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
}

#[derive(Subcommand, Debug)]
enum QueueAction {
    /// Shows a model file's editor and queue, or every file's, and the blocked agents.
    Show {
        #[arg(long)]
        models: PathBuf,
        path: Option<String>,
    },
    /// Takes the right to change a model file, from any agent, until `queue release`.
    Take {
        #[arg(long)]
        models: PathBuf,
        path: String,
    },
    /// Gives back the right to change a model file, to the first waiting agent.
    Release {
        #[arg(long)]
        models: PathBuf,
        path: String,
    },
    /// Gives the right to change a model file to a waiting agent, such as `agent-3`, now.
    Give {
        #[arg(long)]
        models: PathBuf,
        path: String,
        agent: String,
    },
    /// Moves a waiting agent to another place in a model file's queue, 0 first.
    Move {
        #[arg(long)]
        models: PathBuf,
        path: String,
        agent: String,
        place: usize,
    },
    /// Takes an agent out of a model file's queue, or its right away if it edits.
    Remove {
        #[arg(long)]
        models: PathBuf,
        path: String,
        agent: String,
    },
    /// Blocks an agent session from changing models in the folder until unblocked.
    Block {
        #[arg(long)]
        models: PathBuf,
        agent: String,
    },
    /// Lets a blocked agent claim and change models again.
    Unblock {
        #[arg(long)]
        models: PathBuf,
        agent: String,
    },
}

fn length(typed: &str) -> Result<SimTime, String> {
    parse_time(typed).ok_or_else(|| "not a time: write it as 90, 1:30, 8:00:00 or 30d 12:00:00".to_string())
}

fn main() -> ExitCode {
    let _ = env_logger::try_init();
    let opts = Opts::parse();
    let outcome = match opts.command {
        None => return single_run(&opts.single),
        Some(Command::Run { models, model, seed, length, warm_up, observations, label }) => catalogue(&models).and_then(|catalogue| {
            let mut input = json!({ "model": model, "seed": seed, "length": length });
            if let Some(observations) = observations {
                input["observations"] = json!(observations);
            }
            if let Some(warm_up) = warm_up {
                input["warm_up"] = json!(warm_up);
            }
            if let Some(label) = label {
                input["label"] = json!(label);
            }
            run(&catalogue, input)
        }),
        Some(Command::Runs { models }) => catalogue(&models).and_then(|catalogue| runs(&catalogue)),
        Some(Command::Results { models, run, objects, classes, values }) => catalogue(&models).and_then(|catalogue| {
            let mut input = json!({ "run": run, "values": values });
            if !objects.is_empty() {
                input["objects"] = json!(objects);
            }
            if !classes.is_empty() {
                input["classes"] = json!(classes);
            }
            let results = catalogue.call("run_results", input)?;
            print!("{}", tables::results(&results));
            Ok(true)
        }),
        Some(Command::Query { models, sql }) => catalogue(&models).and_then(|catalogue| {
            let result = catalogue.call("query_results", json!({ "sql": sql }))?;
            print!("{}", tables::query(&result));
            Ok(true)
        }),
        Some(Command::Edit { models, path, changes, expected_hash, take_over }) => catalogue(&models).and_then(|catalogue| {
            let text = if changes == "-" {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text).map(|_| text)
            } else {
                std::fs::read_to_string(&changes)
            }
            .map_err(|error| OperationError::invalid_input(format!("the changes can't be read from `{changes}`: {error}")))?;
            let changes: Value = serde_json::from_str(&text).map_err(|error| OperationError::invalid_input(format!("the changes aren't JSON: {error}")))?;
            let mut input = json!({ "path": path, "changes": changes });
            if let Some(expected_hash) = expected_hash {
                input["expected_hash"] = json!(expected_hash);
            }
            catalogue.set_take_over(take_over);
            let edited = catalogue.call("edit_model", input).map_err(|mut error| {
                if error.kind == crabsim_catalogue::ErrorKind::NotEditor {
                    error.message += ". Or give --take-over to make this one change while the agent keeps its right";
                }
                error
            })?;
            print!("{}", tables::edited(&edited));
            Ok(true)
        }),
        Some(Command::Attributes { models, path, name, class }) => catalogue(&models).and_then(|catalogue| {
            let target = if class { json!({ "class": name }) } else { json!({ "instance": name }) };
            let got = catalogue.call("get_attributes", json!({ "path": path, "target": target }))?;
            print!("{}", tables::attributes(&name, &got));
            Ok(true)
        }),
        Some(Command::Adopt { models, path }) => catalogue(&models).and_then(|catalogue| {
            let adopted = catalogue.call("adopt_model", json!({ "path": path }))?;
            print!("{}", tables::adopted(&adopted));
            Ok(true)
        }),
        Some(Command::Compare { models, a, b, objects, classes }) => catalogue(&models).and_then(|catalogue| {
            let mut input = json!({ "a": a, "b": b });
            if !objects.is_empty() {
                input["objects"] = json!(objects);
            }
            if !classes.is_empty() {
                input["classes"] = json!(classes);
            }
            let compared = catalogue.call("compare_runs", input)?;
            print!("{}", tables::compared(&compared));
            Ok(true)
        }),
        Some(Command::Queue { action }) => queue(action),
    };
    match outcome {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// The user's controls of the edit queue, each one operation, then the queue as it is after.
fn queue(action: QueueAction) -> Result<bool, OperationError> {
    let (models, operation, input, path) = match action {
        QueueAction::Show { models, path } => {
            let input = path.as_ref().map_or_else(|| json!({}), |path| json!({ "path": path }));
            let catalogue = catalogue(&models)?;
            let queue = catalogue.call("get_edit_queue", input)?;
            print!("{}", tables::queue(&queue));
            return Ok(true);
        }
        QueueAction::Take { models, path } => (models, "take_model", json!({ "path": path }), path),
        QueueAction::Release { models, path } => (models, "release_model", json!({ "path": path }), path),
        QueueAction::Give { models, path, agent } => (models, "give_model", json!({ "path": path, "agent": agent }), path),
        QueueAction::Move { models, path, agent, place } => (models, "move_in_queue", json!({ "path": path, "agent": agent, "place": place }), path),
        QueueAction::Remove { models, path, agent } => (models, "remove_from_queue", json!({ "path": path, "agent": agent }), path),
        QueueAction::Block { models, agent } => {
            let catalogue = catalogue(&models)?;
            catalogue.call("block_agent", json!({ "agent": agent }))?;
            println!("{agent} is blocked from changing models in this folder");
            return Ok(true);
        }
        QueueAction::Unblock { models, agent } => {
            let catalogue = catalogue(&models)?;
            catalogue.call("unblock_agent", json!({ "agent": agent }))?;
            println!("{agent} may claim and change models again");
            return Ok(true);
        }
    };
    let catalogue = catalogue(&models)?;
    catalogue.call(operation, input)?;
    let queue = catalogue.call("get_edit_queue", json!({ "path": path }))?;
    print!("{}", tables::queue(&json!({ "files": [{ "path": path, "editor": queue["editor"], "queue": queue["queue"] }], "blocked": queue["blocked"] })));
    Ok(true)
}

fn catalogue(models: &std::path::Path) -> Result<Catalogue, OperationError> {
    Catalogue::open(Settings::new(models, Source::Cli))
}

/// Starts a run, shows its progress until it finishes, then prints its results. `Ok(false)` if
/// it failed.
fn run(catalogue: &Catalogue, input: Value) -> Result<bool, OperationError> {
    let started = catalogue.call("start_run", input)?;
    let id = started["run"].as_str().expect("start_run gives an id").to_string();
    eprintln!("{id} started");
    let mut shown = String::new();
    let status = loop {
        let status = catalogue.call("run_status", json!({ "run": id }))?;
        let state = status["state"].as_str().unwrap_or_default();
        if ["done", "failed", "cancelled"].contains(&state) {
            break status;
        }
        let running: Vec<String> = status["running"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|running| format!("{} at {}", running["observation"], running["reached"].as_str().unwrap_or_default()))
            .collect();
        let line = format!("{id}: {} of {} observations ended, running {}", status["ended"], status["observations"], running.join(", "));
        if line != shown {
            eprintln!("{line}");
            shown = line;
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let results = catalogue.call("run_results", json!({ "run": id }))?;
    print!("{}", tables::results(&results));
    Ok(status["state"] == "done")
}

/// Lists the runs in the store, newest first.
fn runs(catalogue: &Catalogue) -> Result<bool, OperationError> {
    let sql = "SELECT id, coalesce(label, ''), model_path, observations, length_s, source, state, started FROM runs ORDER BY number DESC";
    let result = catalogue.call("query_results", json!({ "sql": sql }))?;
    let rows: Vec<Vec<String>> = result["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| {
            let cell = |index: usize| match &row[index] {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            let length = row[4].as_f64().map_or(String::new(), |secs| format_time_exact(SimTime::from_secs_f64(secs)));
            vec![cell(0), cell(1), cell(2), cell(3), length, cell(5), cell(6), cell(7)]
        })
        .collect();
    if rows.is_empty() {
        println!("No runs yet.");
    } else {
        print!("{}", tables::table(&["Run", "Label", "Model", "Observations", "Length", "By", "State", "Started"], &rows));
    }
    Ok(true)
}

/// The CLI as it always was: one run, printed, nothing saved.
fn single_run(opts: &SingleRun) -> ExitCode {
    let (model, which) = match &opts.model {
        None => (demo::model(), "the demo model".to_string()),
        Some(path) => match read_model(path) {
            Ok(model) => (model, path.display().to_string()),
            Err(problems) => {
                eprintln!("{} can't be run:", path.display());
                for problem in problems {
                    eprintln!("{problem}");
                }
                return ExitCode::FAILURE;
            }
        },
    };
    let mut run = model.build(opts.seed);
    println!("crabsim: {which}, seed {}, for {}", opts.seed, format_time_exact(opts.length));

    let start = Instant::now();
    let outcome = run.init().and_then(|_| run.advance_to(opts.length).map_err(|failure| failure.error));
    let wall = start.elapsed();

    // A run that fails still gives back what it did up to the error.
    let state = run.state();
    print!("{}", report(&model, &state));
    let events = run.events_processed();
    println!(
        "Ran {events} events in {:.3} s, {:.2}M events per second",
        wall.as_secs_f64(),
        events as f64 / wall.as_secs_f64().max(f64::MIN_POSITIVE) / 1e6
    );
    match outcome {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("the run stopped at {}: {error}", format_time_exact(state.time));
            ExitCode::FAILURE
        }
    }
}

/// The model in the file at `path`, or every problem with reading it.
fn read_model(path: &PathBuf) -> Result<ModelDescription, Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|error| vec![format!("it can't be read: {error}")])?;
    let model = crabsim_model::load(&text).map_err(|problems| problems.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    Ok(model.to_description())
}

/// Each object's statistics as a table, then what only Drains keep.
fn report(model: &ModelDescription, state: &RunState) -> String {
    let mut text = format!("\nStatistics from {} to {}\n", crabsim::time::format_time(state.statistics_since), format_time_exact(state.time));
    text += &format!(
        "{:>4}  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>13}\n",
        "ID", "Name", "Type", "MUs in", "MUs out", "Working", "Blocked", "Idle", "Failed", "Paused", "Failures", "Availability"
    );
    for object in &state.objects {
        let statistics = &object.statistics;
        let measured = statistics.measured_time();
        let share = |time: SimTime| match measured.as_millis() {
            0 => "-".to_string(),
            total => format!("{:.1} %", time.as_millis() as f64 * 100.0 / total as f64),
        };
        let availability = statistics.achieved_availability().map_or("-".to_string(), |a| format!("{:.1} %", a * 100.0));
        text += &format!(
            "{:>4}  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>13}\n",
            object.id,
            model.name(object.id).unwrap_or("-"),
            kind(object.kind),
            statistics.mus_in,
            statistics.mus_out,
            share(statistics.time_busy),
            share(statistics.time_blocked),
            share(statistics.time_idle),
            share(statistics.time_failed),
            share(statistics.time_paused),
            statistics.failures,
            availability
        );
    }
    text += "\n";
    for object in state.objects.iter().filter(|object| object.statistics.drain.is_some()) {
        text += &drain_line(model.name(object.id).unwrap_or("Drain"), &object.statistics, state.time);
    }
    text
}

fn drain_line(name: &str, statistics: &ObjectStatistics, now: SimTime) -> String {
    let per_hour = statistics.mus_per_hour(now).map_or("-".to_string(), |rate| format!("{rate:.1}"));
    let lead_time = statistics.lead_time_mean().map_or("-".to_string(), format_time_exact);
    format!("{name}: {} MUs, {per_hour} an hour, mean lead time {lead_time}\n", statistics.mus_in)
}

fn kind(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Source => "Source",
        ObjectKind::Station => "Station",
        ObjectKind::Buffer => "Buffer",
        ObjectKind::Drain => "Drain",
    }
}
