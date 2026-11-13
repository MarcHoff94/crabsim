//! The headless CLI. Without a subcommand it runs a model file with a seed for a simulated length,
//! then prints each object's final statistics and how long the run took, saving nothing. An invalid
//! model file prints every problem in it and runs nothing.
//!
//! With a models folder it is people's front-end on the operation catalogue, the same one agents
//! use through the MCP server: `run` starts an experiment and waits for it, `runs` lists the runs
//! in the results store, `results` prints a run's results and `query` queries the store with SQL.
//! `edit` changes a model with a batch of changes, `attributes` shows an object's values and where
//! they come from, `adopt` accepts a file changed outside crabsim, and `editors` shows which agents
//! hold which model files and manages their claims: the person using the CLI is the user.
//! `describe` sums up a model, `graph` asks about its structure, and `export` writes it for graph
//! tools.

mod tables;

use clap::{Args, Parser, Subcommand};
use crabsim::time::{format_time_exact, parse_time};
use crabsim::{ModelDescription, ObjectKind, ObjectStatistics, RunState, SimTime};
use crabsim_catalogue::{Catalogue, OperationError, Settings, Source};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(
    name = "crabsim",
    about = "Runs a crabsim model file headless and prints its statistics, or, with a models folder, runs experiments and reads their results",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true,
    after_help = "Example: crabsim --model models/demo.json --length 30d"
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
    /// The model file to run, such as models/demo.json.
    #[arg(long, required = true)]
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
        /// Make this change even while an agent holds the file, which keeps its claim and reads
        /// the model again before its next change.
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
    /// Checks a model file: every problem, which stops it running, and every warning, what can
    /// run but is probably a mistake. Fails if it has problems.
    Validate {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
    },
    /// Accepts a model file changed outside crabsim, such as by a text editor or git, as it is,
    /// so crabsim may change it again.
    Adopt {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
    },
    /// Lists a model file's change history, newest first: who changed what, and when.
    History {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
        /// The most changes to list, 50 if left out.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Undoes the latest change to a model file, which is then exactly as it was before it.
    Undo {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
        /// How many steps. A step is one change, or everything one command made.
        #[arg(long, default_value_t = 1)]
        steps: usize,
        /// Undo also an agent's change or a change adopted from outside crabsim, which is
        /// otherwise only listed.
        #[arg(long)]
        yes: bool,
        /// Undo even while an agent holds the file, which keeps its claim and reads the model
        /// again before its next change.
        #[arg(long)]
        take_over: bool,
    },
    /// Brings back the change to a model file that was last undone.
    Redo {
        #[arg(long)]
        models: PathBuf,
        /// The model file, such as `line.json`.
        path: String,
        /// How many steps.
        #[arg(long, default_value_t = 1)]
        steps: usize,
        /// Redo even while an agent holds the file.
        #[arg(long)]
        take_over: bool,
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
    /// Shows which agents hold which model files, with their goals, and manages their claims, as
    /// the user.
    Editors {
        #[command(subcommand)]
        action: EditorsAction,
    },
    /// Sums up a model: its classes, Sources and Drains, where the flow branches and merges, how
    /// each Source reaches the Drains, loops, dead ends and problems.
    Describe {
        #[command(flatten)]
        on: WhichModel,
    },
    /// Answers a question about a model's structure.
    Graph {
        #[command(subcommand)]
        kind: GraphKind,
    },
    /// Writes a model as a graph for other tools into the models folder's `exports` folder,
    /// replacing an earlier export of it in the same format.
    Export {
        #[command(flatten)]
        on: WhichModel,
        /// `cypher`, `neo4j-csv` or `graphml`.
        #[arg(long)]
        format: String,
    },
}

/// Which model `describe`, `graph` and `export` are about: a file, as for `run`, or the model a
/// run ran.
#[derive(Args, Debug)]
struct WhichModel {
    #[arg(long)]
    models: PathBuf,
    /// The model file, such as `line.json`. Or give --run.
    #[arg(long)]
    model: Option<String>,
    /// A run, such as `run-3`: the model it ran.
    #[arg(long)]
    run: Option<String>,
}

#[derive(Subcommand, Debug)]
enum GraphKind {
    /// An instance's predecessors and successors.
    Neighbours {
        #[command(flatten)]
        on: WhichModel,
        name: String,
    },
    /// Every instance that can reach this one, nearest first.
    Upstream {
        #[command(flatten)]
        on: WhichModel,
        name: String,
        /// At most this many connections away.
        #[arg(long)]
        steps: Option<usize>,
    },
    /// Every instance this one can reach, nearest first.
    Downstream {
        #[command(flatten)]
        on: WhichModel,
        name: String,
        /// At most this many connections away.
        #[arg(long)]
        steps: Option<usize>,
    },
    /// The routes from one instance to another, shortest first, and how many there are, or with
    /// --weight or --figure, by the total of what their objects weigh.
    Paths {
        #[command(flatten)]
        on: WhichModel,
        from: String,
        to: String,
        /// Weigh each object by an attribute, resolved through inheritance, times in seconds:
        /// process_time, interval, starting_mus or capacity.
        #[arg(long, conflicts_with = "figure")]
        weight: Option<String>,
        /// Weigh each object by a figure of the run given with --run, the mean over its
        /// observations, such as working_share.
        #[arg(long)]
        figure: Option<String>,
        /// The largest totals first, as for a critical path, rather than the smallest.
        #[arg(long)]
        largest: bool,
    },
    /// The instances within some connections of one, either way, and the connections between them.
    SubgraphAround {
        #[command(flatten)]
        on: WhichModel,
        name: String,
        /// How many connections away, 2 if left out.
        #[arg(long)]
        steps: Option<usize>,
    },
    /// The instances that meet every criterion given.
    FindInstances {
        #[command(flatten)]
        on: WhichModel,
        /// Part of the name, ignoring case.
        #[arg(long)]
        name_contains: Option<String>,
        /// Of this class or one derived from it, such as `Station`.
        #[arg(long)]
        class: Option<String>,
        /// Part of the description, ignoring case.
        #[arg(long)]
        description_contains: Option<String>,
        /// A condition such as "process_time > 0:30", with =, <, >, <= or >=. Give it more than
        /// once for more.
        #[arg(long = "where")]
        conditions: Vec<String>,
        /// An attribute set on the instance itself rather than inherited.
        #[arg(long)]
        set_on_instance: Option<String>,
    },
    /// How a Source or Station picks the successor for its MU, with its successors in the order
    /// it ranks them.
    Distribution {
        #[command(flatten)]
        on: WhichModel,
        name: String,
    },
    /// The classes from the built-in classes down, or below one class.
    ClassTree {
        #[command(flatten)]
        on: WhichModel,
        /// Only this class and those derived from it.
        #[arg(long)]
        class: Option<String>,
        /// Also each class's own instances.
        #[arg(long)]
        instances: bool,
    },
}

#[derive(Subcommand, Debug)]
enum EditorsAction {
    /// Shows who holds a model file, or every file someone holds: each agent's claim with its
    /// goal and what it's doing, and the blocked agents.
    Show {
        #[arg(long)]
        models: PathBuf,
        path: Option<String>,
    },
    /// Ends an agent's claim on a model file at once.
    Revoke {
        #[arg(long)]
        models: PathBuf,
        path: String,
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
            let edited = catalogue.call("edit_model", input).map_err(take_over_hint)?;
            print!("{}", tables::edited(&edited));
            Ok(true)
        }),
        Some(Command::Attributes { models, path, name, class }) => catalogue(&models).and_then(|catalogue| {
            let target = if class { json!({ "class": name }) } else { json!({ "instance": name }) };
            let got = catalogue.call("get_attributes", json!({ "path": path, "target": target }))?;
            print!("{}", tables::attributes(&name, &got));
            Ok(true)
        }),
        Some(Command::Validate { models, path }) => catalogue(&models).and_then(|catalogue| {
            let validated = catalogue.call("validate_model", json!({ "path": path }))?;
            print!("{}", tables::validated(&validated));
            Ok(validated["valid"] == true)
        }),
        Some(Command::Adopt { models, path }) => catalogue(&models).and_then(|catalogue| {
            let adopted = catalogue.call("adopt_model", json!({ "path": path }))?;
            print!("{}", tables::adopted(&adopted));
            Ok(true)
        }),
        Some(Command::History { models, path, limit }) => catalogue(&models).and_then(|catalogue| {
            let mut input = json!({ "path": path });
            if let Some(limit) = limit {
                input["limit"] = json!(limit);
            }
            print!("{}", tables::history(&catalogue.call("get_history", input)?));
            Ok(true)
        }),
        Some(Command::Undo { models, path, steps, yes, take_over }) => catalogue(&models).and_then(|catalogue| {
            // An agent's change and a change from outside are only undone when asked for by name.
            if !yes {
                let history = catalogue.call("get_history", json!({ "path": path, "limit": 200 }))?;
                let asks = tables::undo_asks(&history, steps);
                if !asks.is_empty() {
                    return Err(OperationError::invalid_input(format!("this would undo what you didn't change yourself:\n{asks}Give --yes to undo it")));
                }
            }
            catalogue.set_take_over(take_over);
            let undone = catalogue.call("undo_model", json!({ "path": path, "steps": steps })).map_err(take_over_hint)?;
            print!("{}", tables::stepped(&undone, "undone", "Undone"));
            Ok(true)
        }),
        Some(Command::Redo { models, path, steps, take_over }) => catalogue(&models).and_then(|catalogue| {
            catalogue.set_take_over(take_over);
            let redone = catalogue.call("redo_model", json!({ "path": path, "steps": steps })).map_err(take_over_hint)?;
            print!("{}", tables::stepped(&redone, "redone", "Redone"));
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
        Some(Command::Editors { action }) => editors(action),
        Some(Command::Describe { on }) => catalogue(&on.models).and_then(|catalogue| {
            let described = catalogue.call("describe_model", which(on.model, on.run))?;
            print!("{}", tables::described(&described));
            Ok(true)
        }),
        Some(Command::Graph { kind }) => graph(kind),
        Some(Command::Export { on, format }) => catalogue(&on.models).and_then(|catalogue| {
            let mut input = which(on.model, on.run);
            input["format"] = json!(format);
            let exported = catalogue.call("export_model", input)?;
            print!("{}", tables::exported(&exported));
            Ok(true)
        }),
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

/// The user's view of the agents' claims, and their controls, each one operation. Taking and
/// releasing a file stay in the window: the CLI's own right lasts one command, through
/// `edit --take-over`.
fn editors(action: EditorsAction) -> Result<bool, OperationError> {
    match action {
        EditorsAction::Show { models, path } => {
            let input = path.as_ref().map_or_else(|| json!({}), |path| json!({ "path": path }));
            let catalogue = catalogue(&models)?;
            let editors = catalogue.call("get_editors", input)?;
            print!("{}", tables::editors(&editors));
        }
        EditorsAction::Revoke { models, path } => {
            let catalogue = catalogue(&models)?;
            let revoked = catalogue.call("revoke_claim", json!({ "path": path }))?;
            println!("{}'s claim on {path} ended", revoked["revoked"].as_str().unwrap_or("the agent"));
        }
        EditorsAction::Block { models, agent } => {
            let catalogue = catalogue(&models)?;
            catalogue.call("block_agent", json!({ "agent": agent }))?;
            println!("{agent} is blocked from changing models in this folder");
        }
        EditorsAction::Unblock { models, agent } => {
            let catalogue = catalogue(&models)?;
            catalogue.call("unblock_agent", json!({ "agent": agent }))?;
            println!("{agent} may claim and change models again");
        }
    }
    Ok(true)
}

/// `path` or `run` as an operation takes them. Neither or both is left for it to refuse.
fn which(path: Option<String>, run: Option<String>) -> Value {
    let mut input = json!({});
    if let Some(path) = path {
        input["path"] = json!(path);
    }
    if let Some(run) = run {
        input["run"] = json!(run);
    }
    input
}

/// A condition such as `process_time > 0:30`, as `find_instances` takes it.
fn condition(typed: &str) -> Result<Value, OperationError> {
    let found = (0..typed.len())
        .filter(|&at| typed.is_char_boundary(at))
        .find_map(|at| [">=", "<=", "=", "<", ">"].into_iter().find(|op| typed[at..].starts_with(op)).map(|op| (at, op)));
    let refused = || {
        OperationError::invalid_input(format!(
            "`{typed}` isn't a condition: write it as an attribute, =, <, >, <= or >=, and a value, such as \"process_time > 0:30\""
        ))
    };
    let (at, op) = found.ok_or_else(refused)?;
    let attribute = typed[..at].trim();
    let value = typed[at + op.len()..].trim();
    if attribute.is_empty() || value.is_empty() {
        return Err(refused());
    }
    let value = value.parse::<u64>().map_or_else(|_| json!(value), |number| json!(number));
    Ok(json!({ "attribute": attribute, "op": op, "value": value }))
}

/// A question about a model's structure, answered as tables.
fn graph(kind: GraphKind) -> Result<bool, OperationError> {
    let (on, query) = match kind {
        GraphKind::Neighbours { on, name } => (on, json!({ "kind": "neighbours", "name": name })),
        GraphKind::Upstream { on, name, steps } => (on, json!({ "kind": "upstream", "name": name, "steps": steps })),
        GraphKind::Downstream { on, name, steps } => (on, json!({ "kind": "downstream", "name": name, "steps": steps })),
        GraphKind::Paths { on, from, to, weight, figure, largest } => {
            let weight = match (weight, figure) {
                (Some(attribute), _) => json!({ "attribute": attribute }),
                (None, Some(figure)) => json!({ "figure": figure }),
                (None, None) => Value::Null,
            };
            let order = if largest { json!("largest") } else { Value::Null };
            (on, json!({ "kind": "paths", "from": from, "to": to, "weight": weight, "order": order }))
        }
        GraphKind::SubgraphAround { on, name, steps } => (on, json!({ "kind": "subgraph_around", "name": name, "steps": steps })),
        GraphKind::FindInstances { on, name_contains, class, description_contains, conditions, set_on_instance } => {
            let conditions = conditions.iter().map(|typed| condition(typed)).collect::<Result<Vec<_>, _>>()?;
            let query = json!({
                "kind": "find_instances",
                "name_contains": name_contains,
                "class": class,
                "description_contains": description_contains,
                "conditions": if conditions.is_empty() { Value::Null } else { json!(conditions) },
                "set_on_instance": set_on_instance,
            });
            (on, query)
        }
        GraphKind::Distribution { on, name } => (on, json!({ "kind": "distribution", "name": name })),
        GraphKind::ClassTree { on, class, instances } => (on, json!({ "kind": "class_tree", "class": class, "instances": instances })),
    };
    // Options not given are left out, as the operation's schema wants, rather than null.
    let query: serde_json::Map<String, Value> = query.as_object().expect("a query is an object").iter().filter(|(_, value)| !value.is_null()).map(|(key, value)| (key.clone(), value.clone())).collect();
    let catalogue = catalogue(&on.models)?;
    let mut input = which(on.model, on.run);
    input["query"] = Value::Object(query);
    let answer = catalogue.call("query_model", input)?;
    print!("{}", tables::answer(&answer));
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
    let Some(path) = &opts.model else { unreachable!("clap requires --model for a single run") };
    let (model, which) = match read_model(path) {
        Ok(model) => (model, path.display().to_string()),
        Err(problems) => {
            eprintln!("{} can't be run:", path.display());
            for problem in problems {
                eprintln!("{problem}");
            }
            return ExitCode::FAILURE;
        }
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

/// A refusal because an agent holds the file, with how to change it anyway.
fn take_over_hint(mut error: OperationError) -> OperationError {
    if error.kind == crabsim_catalogue::ErrorKind::NotEditor {
        error.message += ". Or give --take-over to make this one change while the agent keeps its right";
    }
    error
}
