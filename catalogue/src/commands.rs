//! The command language (spec issue 102): `edit_model`'s changes in people's words, such as
//! `add Mill as Drill; connect Split > Drill`, and a few commands for the window, such as
//! `goto 2d`. One parser, so the window's `:` line and agents (`run_commands`) read commands
//! exactly alike. Names are matched like the window's search: exactly, or by a part only one name
//! has.

use crate::edit::{Change, EditModelInput};
use crate::error::OperationError;
use crate::model_files::read_file;
use crate::{entry, Catalogue, Effect, Entry, Operation};
use crabsim::time::parse_time;
use crabsim::SimTime;
use crabsim_model::{Attribute, Model, BUILT_IN_CLASSES};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) fn entries() -> Vec<Entry> {
    vec![entry::<RunCommands>()]
}

/// Every command's first words, for completing them.
pub const VERBS: [&str; 20] = [
    "add", "connect", "disconnect", "rename", "remove", "set", "restore", "move", "derive", "class rename", "class duplicate",
    "class remove", "make class", "add failure", "remove failure", "rename failure", "goto", "speed", "seed", "open",
];

/// What a line asks for: changes to the model, or one thing for the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Changes(Vec<Planned>),
    Window(WindowCommand),
    /// A macro run alone that holds commands for the window too: lines to carry out in order,
    /// each model changes or one window command.
    Sequence(Vec<String>),
    /// Saving, removing or keeping a macro.
    Macro(MacroCommand),
}

/// A command about macros.
#[derive(Debug, Clone, PartialEq)]
pub enum MacroCommand {
    /// `macro name = command; command`.
    Save { name: String, commands: Vec<String> },
    /// `macro remove name`.
    Remove { name: String },
    /// `macro keep name`: the user keeps an agent's macro.
    Keep { name: String },
}

/// One command of a line and the `edit_model` changes it makes.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The command as typed.
    pub command: String,
    pub changes: Vec<Value>,
    /// The instance it places, for `add`.
    pub placed: Option<String>,
    /// Whether it removes an instance or a class, which the window asks about first.
    pub removes: bool,
    /// What it did beyond what it says, for the person who typed it, such as an instance getting
    /// its own copy of its class's failure profiles.
    pub notes: Vec<String>,
}

/// A command for the window rather than the model.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    GoTo(SimTime),
    Speed(f64),
    Seed(u64),
    Open(String),
    Validate,
}

/// What a line is read with: the instance selected, if any, and where `add` places with nothing
/// selected, such as the middle of the view.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub selected: Option<String>,
    pub at: Option<(f64, f64)>,
    /// The macros `@name` can run, by name with their commands.
    pub macros: Vec<(String, Vec<String>)>,
}

/// The model as the line has changed it so far: its instances, with class, position and length,
/// and its classes.
struct Names {
    instances: Vec<Placed>,
    classes: Vec<String>,
    /// Each instance's failure profiles as the line has set them so far, as a model file writes
    /// them.
    failures: std::collections::HashMap<String, Vec<Value>>,
}

#[derive(Clone)]
struct Placed {
    name: String,
    x: f64,
    y: f64,
    length: f64,
}

impl Names {
    fn of(model: &Model) -> Names {
        let instances = model
            .instances
            .iter()
            .enumerate()
            .map(|(index, instance)| {
                let position = instance.position.unwrap_or(crabsim::Position { x: 0.0, y: 0.0, z: 0.0 });
                let length = model.resolve(index).and_then(|resolved| resolved.attributes.dimensions).map_or(1.0, |size| size.length);
                Placed { name: instance.name.clone(), x: position.x, y: position.y, length }
            })
            .collect();
        let mut classes: Vec<String> = BUILT_IN_CLASSES.iter().map(|name| name.to_string()).collect();
        classes.extend(model.classes.iter().filter(|class| !class.refines_built_in()).map(|class| class.name.clone()));
        Names { instances, classes, failures: std::collections::HashMap::new() }
    }

    fn instance(&self, wanted: &str) -> Result<String, String> {
        let names: Vec<&str> = self.instances.iter().map(|placed| placed.name.as_str()).collect();
        find(&names, wanted, "instance")
    }

    fn class(&self, wanted: &str) -> Result<String, String> {
        let names: Vec<&str> = self.classes.iter().map(String::as_str).collect();
        find(&names, wanted, "class")
    }

    fn placed(&self, name: &str) -> Option<&Placed> {
        self.instances.iter().find(|placed| placed.name == name)
    }
}

/// The name `wanted` means: one exactly so, then one so ignoring case, then the only one holding
/// it. Several holding it, or none, is refused, naming them.
pub fn find(names: &[&str], wanted: &str, what: &str) -> Result<String, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Err(format!("which {what}?"));
    }
    if let Some(name) = names.iter().find(|name| **name == wanted) {
        return Ok(name.to_string());
    }
    let lower = wanted.to_lowercase();
    if let Some(name) = names.iter().find(|name| name.to_lowercase() == lower) {
        return Ok(name.to_string());
    }
    let holding: Vec<&&str> = names.iter().filter(|name| name.to_lowercase().contains(&lower)).collect();
    match holding.as_slice() {
        [only] => Ok(only.to_string()),
        [] => Err(format!("no {what} is called anything like `{wanted}`")),
        several => {
            let shown: Vec<String> = several.iter().take(5).map(|name| format!("`{name}`")).collect();
            let more = if several.len() > 5 { format!(" and {} more", several.len() - 5) } else { String::new() };
            Err(format!("`{wanted}` could be {}{more}: give more of its name", shown.join(", ")))
        }
    }
}

/// The attribute `wanted` names: exactly, or by a part only one attribute's name holds,
/// ignoring case, spaces and underscores, such as `proc` for `process_time`. Where several hold
/// it but only one begins with it, that one: `dwell` is `dwelling_time`, not
/// `stop_freezes_dwelling`.
pub fn attribute(wanted: &str) -> Result<Attribute, String> {
    let plain = |text: &str| text.to_lowercase().replace([' ', '_'], "");
    let wanted = plain(wanted);
    if let Some(exact) = Attribute::ALL.into_iter().find(|attribute| plain(attribute.name()) == wanted) {
        return Ok(exact);
    }
    let holding: Vec<Attribute> = Attribute::ALL.into_iter().filter(|attribute| !wanted.is_empty() && plain(attribute.name()).contains(&wanted)).collect();
    let beginning: Vec<Attribute> = holding.iter().copied().filter(|attribute| plain(attribute.name()).starts_with(&wanted)).collect();
    if let ([_, _, ..], [only]) = (holding.as_slice(), beginning.as_slice()) {
        return Ok(*only);
    }
    match holding.as_slice() {
        [only] => Ok(*only),
        [] => Err(format!(
            "no attribute is called anything like `{wanted}`: there are {}",
            Attribute::ALL.iter().map(|attribute| attribute.name()).collect::<Vec<_>>().join(", ")
        )),
        several => Err(format!("`{wanted}` could be {}", several.iter().map(|attribute| attribute.name()).collect::<Vec<_>>().join(", "))),
    }
}

/// A value as a model file writes it, from what was typed: a time as text, a path in its short
/// form, such as `4, arc 1.5 90`, or as JSON, anything else that reads as JSON as that, and the
/// rest as text. A path that reads as neither says what's wrong with its short form.
fn value(attribute: Attribute, typed: &str) -> Result<Value, String> {
    let typed = typed.trim();
    if matches!(attribute, Attribute::Interval | Attribute::ProcessTime | Attribute::DwellingTime) {
        return Ok(json!(typed));
    }
    if attribute == Attribute::Path {
        return match serde_json::from_str::<Value>(typed) {
            Ok(json) if json.is_array() => Ok(json),
            _ => crabsim_model::path::read_short_form(typed).map(|path| crabsim_model::path::to_json(&path)),
        };
    }
    Ok(serde_json::from_str(typed).unwrap_or_else(|_| json!(typed)))
}

/// "1st", "2nd", "3rd", "4th" and so on.
pub fn ordinal(place: usize) -> String {
    let suffix = match (place % 10, place % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{place}{suffix}")
}

/// A command, cut short for a message.
fn shown(command: &str) -> String {
    let command = command.trim();
    if command.chars().count() > 30 { format!("{}…", command.chars().take(29).collect::<String>()) } else { command.to_string() }
}

/// What a newly placed instance of `class` starts with: changes to make first and values set on
/// it. A Station works 10 s on each MU to start with: the model's own Station gets it where
/// nothing in the model is built on Station yet, and otherwise the Station placed does, so earlier
/// Stations keep their values. Other classes start with nothing.
pub fn starting_values(model: &Model, class: &str) -> (Vec<Value>, Option<Value>) {
    let Some((_, built_in)) = model.chain(class) else { return (Vec::new(), None) };
    let Some(values) = template(built_in) else { return (Vec::new(), None) };
    if model.class(built_in).is_some() {
        return (Vec::new(), None);
    }
    let built_on = |name: &str| model.chain(name).is_some_and(|(_, root)| root == built_in);
    let in_use = model.instances.iter().any(|instance| built_on(&instance.class)) || model.classes.iter().any(|class| built_on(&class.name));
    if in_use {
        return (Vec::new(), (class == built_in).then_some(values));
    }
    let changes = values
        .as_object()
        .into_iter()
        .flatten()
        .map(|(attribute, value)| json!({ "change": "set_attribute", "target": { "class": built_in }, "attribute": attribute, "value": value }))
        .collect();
    (changes, None)
}

/// The values building on a built-in class starts with: a Station works 10 s on each MU, where
/// the built-in Station's default is 0.
pub fn template(built_in: &str) -> Option<Value> {
    match built_in {
        "Station" => Some(json!({ "process_time": "00:00:10" })),
        _ => None,
    }
}

/// A position to the centimetre, as a model file writes it.
fn position(x: f64, y: f64) -> Value {
    let round = |metres: f64| (metres * 100.0).round() / 100.0;
    json!({ "x": round(x), "y": round(y) })
}

/// Two numbers, such as `12 4`.
fn two_numbers(text: &str) -> Option<(f64, f64)> {
    let mut numbers = text.split_whitespace().map(|word| word.parse::<f64>().ok().filter(|number| number.is_finite()));
    let pair = (numbers.next()??, numbers.next()??);
    numbers.next().is_none().then_some(pair)
}

/// Reads a line: commands separated by `;`. A line is either changes to the model, each command
/// giving one or more `edit_model` changes, or one command for the window.
pub fn parse(line: &str, model: &Model, context: &Context) -> Result<Parsed, String> {
    if let Some(rest) = line.trim().strip_prefix("macro ") {
        return macro_command(rest.trim()).map(Parsed::Macro);
    }
    let typed: Vec<&str> = line.split(';').map(str::trim).filter(|command| !command.is_empty()).collect();
    if typed.is_empty() {
        return Err("type a command, such as add Station".to_string());
    }
    let mut expanded = Vec::new();
    for (place, command) in typed.iter().enumerate() {
        expand(command, context, 0, &mut expanded).map_err(|why| format!("{} command ({}): {why}", ordinal(place + 1), shown(command)))?;
    }
    let commands: Vec<&str> = expanded.iter().map(String::as_str).collect();
    let window = |command: &str| -> Option<Result<WindowCommand, String>> {
        let (verb, rest) = command.split_once(' ').map_or((command, ""), |(verb, rest)| (verb, rest.trim()));
        Some(match verb {
            "goto" => parse_time(rest).map(WindowCommand::GoTo).ok_or_else(|| format!("`{rest}` isn't a time: type one such as 2d or 1:30:00")),
            "speed" => rest.parse::<f64>().ok().filter(|speed| *speed > 0.0).map(WindowCommand::Speed).ok_or_else(|| format!("`{rest}` isn't a speed: type one such as 100")),
            "seed" => rest.parse::<u64>().map(WindowCommand::Seed).map_err(|_| format!("`{rest}` isn't a seed: type a whole number")),
            "open" if !rest.is_empty() => Ok(WindowCommand::Open(rest.to_string())),
            "open" => Err("open which model file?".to_string()),
            "validate" => Ok(WindowCommand::Validate),
            _ => return None,
        })
    };
    if let [only] = commands.as_slice()
        && let Some(command) = window(only)
    {
        return command.map(Parsed::Window);
    }
    // A macro run alone may mix the two: its lines run in order, model changes side by side as
    // one batch.
    if typed.len() == 1 && typed[0].starts_with('@') && commands.iter().any(|command| window(command).is_some()) {
        let mut lines: Vec<String> = Vec::new();
        let mut batch: Vec<&str> = Vec::new();
        for command in &commands {
            if window(command).is_some() {
                if !batch.is_empty() {
                    lines.push(batch.join("; "));
                    batch.clear();
                }
                lines.push(command.to_string());
            } else {
                batch.push(command);
            }
        }
        if !batch.is_empty() {
            lines.push(batch.join("; "));
        }
        return Ok(Parsed::Sequence(lines));
    }
    if let Some(command) = commands.iter().find(|command| window(command).is_some()) {
        return Err(format!(
            "`{}` is for the window, and a line is either changes to the model or one of goto, speed, seed, open and validate: give it a line of its own",
            shown(command)
        ));
    }
    let mut names = Names::of(model);
    let mut last_placed: Option<String> = None;
    let mut planned = Vec::new();
    for (place, command) in commands.iter().enumerate() {
        // `$last` is the instance the last `add` placed, so far in this line.
        let command = &if command.contains("$last") {
            match &last_placed {
                Some(last) => command.replace("$last", last),
                None => return Err(format!("{} command ({}): $last is the instance the last add placed, and none has yet", ordinal(place + 1), shown(command))),
            }
        } else {
            command.to_string()
        };
        let read = read_command(command, model, context, &mut names, &mut last_placed)
            .map_err(|why| format!("{} command ({}): {why}", ordinal(place + 1), shown(command)))?;
        planned.push(read);
    }
    Ok(Parsed::Changes(planned))
}

/// `macro name = …`, `macro remove name` or `macro keep name`.
fn macro_command(rest: &str) -> Result<MacroCommand, String> {
    if let Some((name, commands)) = rest.split_once('=') {
        let commands: Vec<String> = commands.split(';').map(str::trim).filter(|command| !command.is_empty()).map(str::to_string).collect();
        if commands.is_empty() {
            return Err("a macro needs at least one command, such as macro line = add Station; connect $selected > $last".to_string());
        }
        return Ok(MacroCommand::Save { name: name.trim().to_string(), commands });
    }
    match rest.split_once(' ') {
        Some(("remove", name)) => Ok(MacroCommand::Remove { name: name.trim().to_string() }),
        Some(("keep", name)) => Ok(MacroCommand::Keep { name: name.trim().to_string() }),
        _ => Err("write it as macro name = command; command, macro remove name, or macro keep name".to_string()),
    }
}

/// How deep macros may run macros, so one that runs itself stops.
const MACRO_DEPTH: usize = 8;

/// `command`, or the commands of the macro `@name` runs, its arguments and `$selected` filled in,
/// macros within it run in turn.
fn expand(command: &str, context: &Context, depth: usize, into: &mut Vec<String>) -> Result<(), String> {
    let Some(call) = command.strip_prefix('@') else {
        into.push(command.to_string());
        return Ok(());
    };
    if depth >= MACRO_DEPTH {
        return Err(format!("macros run each other more than {MACRO_DEPTH} deep, as one that runs itself does"));
    }
    let (name, rest) = call.split_once(char::is_whitespace).map_or((call, ""), |(name, rest)| (name, rest.trim()));
    let (_, commands) = context
        .macros
        .iter()
        .find(|(known, _)| known == name)
        .ok_or_else(|| format!("there is no macro `{name}`: list_macros lists them"))?;
    // Arguments are split at commas if there are any, so a name may hold spaces, and otherwise
    // at spaces.
    let arguments: Vec<&str> = if rest.contains(',') { rest.split(',').map(str::trim).collect() } else { rest.split_whitespace().collect() };
    for line in commands {
        let mut filled = line.clone();
        // From the highest, so $1 doesn't eat the start of $12.
        for place in (1..=9).rev() {
            let marker = format!("${place}");
            if filled.contains(&marker) {
                let argument = arguments.get(place - 1).ok_or_else(|| format!("@{name} needs at least {place} argument{}", if place == 1 { "" } else { "s" }))?;
                filled = filled.replace(&marker, argument);
            }
        }
        if filled.contains("$selected") {
            let selected = context.selected.as_deref().ok_or_else(|| format!("@{name} uses $selected: select an instance first"))?;
            filled = filled.replace("$selected", selected);
        }
        for inner in filled.split(';').map(str::trim).filter(|inner| !inner.is_empty()) {
            expand(inner, context, depth + 1, into)?;
        }
    }
    Ok(())
}

/// The macros a line runs by name, at its top level, for counting their runs.
pub fn macros_run(line: &str) -> Vec<String> {
    line.split(';')
        .filter_map(|command| command.trim().strip_prefix('@'))
        .map(|call| call.split_whitespace().next().unwrap_or_default().to_string())
        .collect()
}

fn read_command(command: &str, model: &Model, context: &Context, names: &mut Names, last_placed: &mut Option<String>) -> Result<Planned, String> {
    let (verb, rest) = match command.split_once(' ') {
        Some(("class", rest)) => match rest.trim().split_once(' ') {
            Some((what, rest)) => (format!("class {what}"), rest.trim()),
            None => (format!("class {}", rest.trim()), ""),
        },
        Some(("make", rest)) if rest.trim_start().starts_with("class ") => ("make class".to_string(), rest.trim_start()["class ".len()..].trim()),
        Some((verb, rest)) => (verb.to_string(), rest.trim()),
        None => (command.to_string(), ""),
    };
    let planned = |changes: Vec<Value>| Planned { command: command.to_string(), changes, placed: None, removes: false, notes: Vec::new() };
    if let Some((what, rest)) = command.split_once(' ')
        && matches!(what, "add" | "remove" | "rename")
        && let Some(rest) = rest.trim_start().strip_prefix("failure ")
    {
        // `$selected` stands for the instance selected, as in a macro, which may be several by
        // name in the window.
        let rest = match (&context.selected, rest.contains("$selected")) {
            (Some(selected), true) => rest.replace("$selected", selected),
            (None, true) => return Err("$selected is the instance selected, and none is".to_string()),
            _ => rest.to_string(),
        };
        let (changes, notes) = failure_command(what, rest.trim(), model, names)?;
        return Ok(Planned { notes, removes: what == "remove", ..planned(changes) });
    }
    let split = |text: &str, word: &str| -> Result<(String, String), String> {
        text.split_once(&format!(" {word} "))
            .map(|(left, right)| (left.trim().to_string(), right.trim().to_string()))
            .ok_or_else(|| format!("write it as {verb} … {word} …"))
    };
    match verb.as_str() {
        "add" => {
            let (rest, at) = match rest.rsplit_once(" at ") {
                Some((left, right)) => match two_numbers(right) {
                    Some(at) => (left.trim(), Some(at)),
                    None => return Err(format!("`{}` isn't a place: type two numbers in metres, such as at 12 4", right.trim())),
                },
                None => (rest, None),
            };
            let (class, name) = match rest.split_once(" as ") {
                Some((class, name)) => (names.class(class)?, Some(name.trim().to_string())),
                None => (names.class(rest)?, None),
            };
            if model.is_mu_class(&class) {
                return Err(format!("`{class}` is an MU class, which isn't placed: a Source makes its MUs once its mu_class is set to it, such as set In.mu_class {class}"));
            }
            let name = name.unwrap_or_else(|| next_name(&class, names));
            let length = model.resolve_class(&class).and_then(|resolved| resolved.attributes.dimensions).map_or(1.0, |size| size.length);
            // To the right of the last placed, else of the selection, else where the window says.
            let beside = last_placed.clone().or_else(|| context.selected.clone()).and_then(|name| names.placed(&name).cloned());
            let (x, y) = match (at, beside) {
                (Some(at), _) => at,
                (None, Some(beside)) => (beside.x + beside.length / 2.0 + 2.0 + length / 2.0, beside.y),
                (None, None) => context.at.unwrap_or((0.0, 0.0)),
            };
            let (mut changes, values) = starting_values(model, &class);
            let mut add = json!({ "change": "add_instance", "name": name, "class": class, "position": position(x, y) });
            if let Some(values) = values {
                add["attributes"] = values;
            }
            changes.push(add);
            names.instances.push(Placed { name: name.clone(), x, y, length });
            *last_placed = Some(name.clone());
            Ok(Planned { placed: Some(name), ..planned(changes) })
        }
        "connect" | "disconnect" => {
            // `>` reads with or without spaces around it, as A>B or A > B.
            let (from, to) = rest
                .split_once('>')
                .map(|(from, to)| (from.trim().to_string(), to.trim().to_string()))
                .ok_or_else(|| format!("write it as {verb} … > …"))?;
            Ok(planned(vec![json!({ "change": verb, "from": names.instance(&from)?, "to": names.instance(&to)? })]))
        }
        "rename" => {
            let (name, to) = split(rest, "to")?;
            let name = names.instance(&name)?;
            if let Some(placed) = names.instances.iter_mut().find(|placed| placed.name == name) {
                placed.name = to.clone();
            }
            Ok(planned(vec![json!({ "change": "rename_instance", "name": name, "to": to })]))
        }
        "remove" => {
            let name = names.instance(rest)?;
            names.instances.retain(|placed| placed.name != name);
            Ok(Planned { removes: true, ..planned(vec![json!({ "change": "remove_instance", "name": name })]) })
        }
        "set" | "restore" => {
            let (target, attribute, typed) = address(rest, names)?;
            let change = if verb == "set" {
                if typed.trim().is_empty() {
                    return Err(format!("set it to what? Write it as set {}.{} 0:45", target_name(&target), attribute.name()));
                }
                json!({ "change": "set_attribute", "target": target, "attribute": attribute.name(), "value": value(attribute, &typed)? })
            } else {
                json!({ "change": "restore_inheritance", "target": target, "attribute": attribute.name() })
            };
            Ok(planned(vec![change]))
        }
        "move" => {
            let (name, to, relative) = match (rest.rsplit_once(" to "), rest.rsplit_once(" by ")) {
                (Some((name, to)), _) => (name, to, false),
                (None, Some((name, by))) => (name, by, true),
                _ => return Err("write it as move … to 12 4, or move … by 2 0".to_string()),
            };
            let name = names.instance(name)?;
            let (dx, dy) = two_numbers(to).ok_or_else(|| format!("`{}` isn't a place: type two numbers in metres", to.trim()))?;
            let placed = names.instances.iter_mut().find(|placed| placed.name == name).expect("found by name");
            let (x, y) = if relative { (placed.x + dx, placed.y + dy) } else { (dx, dy) };
            (placed.x, placed.y) = (x, y);
            Ok(planned(vec![json!({ "change": "move_instance", "name": name, "position": position(x, y) })]))
        }
        "derive" => {
            let (parent, name) = split(rest, "as")?;
            let parent = names.class(&parent)?;
            let (mut changes, values) = starting_values(model, &parent);
            let mut add = json!({ "change": "add_class", "name": name, "parent": parent });
            if let Some(values) = values {
                add["attributes"] = values;
            }
            changes.push(add);
            names.classes.push(name);
            Ok(planned(changes))
        }
        "class rename" => {
            let (name, to) = split(rest, "to")?;
            let name = names.class(&name)?;
            if let Some(class) = names.classes.iter_mut().find(|class| **class == name) {
                *class = to.clone();
            }
            Ok(planned(vec![json!({ "change": "rename_class", "name": name, "to": to })]))
        }
        "class duplicate" => {
            let (name, to) = split(rest, "as")?;
            let name = names.class(&name)?;
            names.classes.push(to.clone());
            Ok(planned(vec![json!({ "change": "duplicate_class", "name": name, "to": to })]))
        }
        "class remove" => {
            let name = names.class(rest)?;
            Ok(Planned { removes: true, ..planned(vec![json!({ "change": "remove_class", "name": name, "delete_dependents": true })]) })
        }
        "make class" => {
            let (name, from) = split(rest, "from")?;
            let instance = names.instance(&from)?;
            let found = model.instances.iter().find(|candidate| candidate.name == instance).ok_or_else(|| format!("`{instance}` is placed in this line, so make the class after it's saved"))?;
            let mut changes = vec![json!({ "change": "add_class", "name": name, "parent": found.class, "attributes": found.attributes.to_json() })];
            for attribute in Attribute::ALL.into_iter().filter(|&attribute| found.attributes.is_set(attribute)) {
                changes.push(json!({ "change": "restore_inheritance", "target": { "instance": instance }, "attribute": attribute.name() }));
            }
            changes.push(json!({ "change": "set_class", "name": instance, "class": name }));
            names.classes.push(name);
            Ok(planned(changes))
        }
        other => Err(format!("there is no command `{other}`: there are {}", VERBS.join(", "))),
    }
}

/// A failure profile's name at the start of `text`, in quotes or as one word, and what follows.
fn profile_name(text: &str) -> Result<(String, &str), String> {
    let text = text.trim_start();
    if let Some(quoted) = text.strip_prefix('"') {
        let (name, rest) = quoted.split_once('"').ok_or("a name in quotes needs its closing quote")?;
        if name.trim().is_empty() {
            return Err("a failure profile needs a name".to_string());
        }
        return Ok((name.to_string(), rest));
    }
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    if name.is_empty() {
        return Err("which failure profile? Give its name, in quotes if it has spaces".to_string());
    }
    Ok((name.to_string(), rest))
}

/// Instances named in a comma-separated list.
fn instance_list(text: &str, names: &Names) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for wanted in text.split(',').map(str::trim) {
        let name = names.instance(wanted)?;
        if !found.contains(&name) {
            found.push(name);
        }
    }
    Ok(found)
}

/// An instance's failure profiles as the line has them so far, and, if it inherits them, the
/// class it does from.
fn failures_of(model: &Model, names: &Names, instance: &str) -> (Vec<Value>, Option<String>) {
    if let Some(list) = names.failures.get(instance) {
        return (list.clone(), None);
    }
    let Some(index) = model.instances.iter().position(|candidate| candidate.name == instance) else { return (Vec::new(), None) };
    let Some(resolved) = model.resolve(index) else { return (Vec::new(), None) };
    let as_json = |attributes: crabsim_model::Attributes| attributes.to_json()["failures"].as_array().cloned().unwrap_or_default();
    let list = as_json(crabsim_model::Attributes { failures: resolved.attributes.failures.clone(), ..Default::default() });
    match resolved.origin(Attribute::Failures) {
        crabsim_model::Origin::Class(class) if !list.is_empty() => (list, Some(class.clone())),
        _ => (list, None),
    }
}

/// A profile's stream within its object, or its place in the list.
fn stream_of(profile: &Value, place: usize) -> u64 {
    profile["stream"].as_u64().unwrap_or(place as u64)
}

/// `add failure "Jam" to A, B availability 95% repair 2:00`, `remove failure "Jam" from A, B` and
/// `rename failure "Jam" of A, B to "Belt jam"`: each instance named gets its whole list set, so a
/// class's list it inherited is copied to it first.
fn failure_command(what: &str, rest: &str, model: &Model, names: &mut Names) -> Result<(Vec<Value>, Vec<String>), String> {
    let (profile, rest) = profile_name(rest)?;
    let rest = rest.trim();
    let mut changes = Vec::new();
    let mut notes = Vec::new();
    let (instances, change): (Vec<String>, Box<dyn Fn(&str, &mut Vec<Value>) -> Result<(), String>>) = match what {
        "add" => {
            let rest = rest.strip_prefix("to ").ok_or("write it as add failure \"Jam\" to Station 3 availability 95% repair 2:00")?;
            let (targets, values) = rest.split_once(" availability ").ok_or("give its availability, such as availability 95%")?;
            let (availability, repair) = values.split_once(" repair ").ok_or("give its repair time, such as repair 2:00")?;
            let availability = availability.trim();
            let share = match availability.strip_suffix('%') {
                Some(percent) => percent.trim().parse::<f64>().ok().map(|percent| percent / 100.0),
                None => availability.parse::<f64>().ok(),
            }
            .filter(|share| *share > 0.0 && *share < 1.0)
            .ok_or_else(|| format!("`{availability}` isn't an availability: type one above 0% and below 100%, such as 95%"))?;
            // An optional `on working time` at the end names its time basis.
            let (repair, basis) = match repair.rsplit_once(" on ") {
                Some((repair, basis)) => {
                    let word = basis.trim().strip_suffix(" time").unwrap_or(basis.trim());
                    let basis = crabsim_model::time_basis_named(word)
                        .ok_or_else(|| format!("`{}` isn't a time basis: write on simulation time, on working time or on processing time", basis.trim()))?;
                    (repair, basis)
                }
                None => (repair, crabsim_model::TimeBasis::Simulation),
            };
            let repair = repair.trim();
            let time = parse_time(repair).filter(|time| *time > SimTime::ZERO).ok_or_else(|| format!("`{repair}` isn't a repair time: type one such as 2:00"))?;
            let repair = crabsim::time::format_time_exact(time);
            let profile = profile.clone();
            (
                instance_list(targets, names)?,
                Box::new(move |instance: &str, list: &mut Vec<Value>| {
                    if list.iter().any(|existing| existing["name"] == profile.as_str()) {
                        return Err(format!("{instance} already has a failure profile named \"{profile}\""));
                    }
                    let stream = list.iter().enumerate().map(|(place, existing)| stream_of(existing, place) + 1).max().unwrap_or(0);
                    let mut added = json!({ "name": profile, "stream": stream, "availability": share, "repair_time": { "constant": repair } });
                    if basis != crabsim_model::TimeBasis::Simulation {
                        added["time_basis"] = json!(crabsim_model::time_basis_name(basis));
                    }
                    list.push(added);
                    Ok(())
                }),
            )
        }
        "remove" => {
            let targets = rest.strip_prefix("from ").ok_or("write it as remove failure \"Jam\" from Station 3")?;
            let profile = profile.clone();
            (
                instance_list(targets, names)?,
                Box::new(move |instance: &str, list: &mut Vec<Value>| {
                    let place = list.iter().position(|existing| existing["name"] == profile.as_str()).ok_or_else(|| format!("{instance} has no failure profile named \"{profile}\""))?;
                    // The others keep their streams, so their failures stay as they were.
                    for (at, existing) in list.iter_mut().enumerate() {
                        if existing.get("stream").is_none() {
                            existing["stream"] = json!(at);
                        }
                    }
                    list.remove(place);
                    Ok(())
                }),
            )
        }
        _ => {
            let rest = rest.strip_prefix("of ").ok_or("write it as rename failure \"Jam\" of Station 3 to \"Belt jam\"")?;
            let (targets, to) = rest.rsplit_once(" to ").ok_or("write it as rename failure \"Jam\" of Station 3 to \"Belt jam\"")?;
            let (to, after) = profile_name(to)?;
            if !after.trim().is_empty() {
                return Err(format!("`{}` follows the new name: put a name with spaces in quotes", after.trim()));
            }
            let profile = profile.clone();
            (
                instance_list(targets, names)?,
                Box::new(move |instance: &str, list: &mut Vec<Value>| {
                    if list.iter().any(|existing| existing["name"] == to.as_str()) {
                        return Err(format!("{instance} already has a failure profile named \"{to}\""));
                    }
                    let existing = list.iter_mut().find(|existing| existing["name"] == profile.as_str()).ok_or_else(|| format!("{instance} has no failure profile named \"{profile}\""))?;
                    existing["name"] = json!(to);
                    Ok(())
                }),
            )
        }
    };
    for instance in instances {
        let (mut list, inherited_from) = failures_of(model, names, &instance);
        let copied = list.len();
        change(&instance, &mut list)?;
        if let Some(class) = inherited_from {
            notes.push(format!("{instance} now has its own failures ({copied} copied from {class})"));
        }
        changes.push(json!({ "change": "set_attribute", "target": { "instance": instance }, "attribute": "failures", "value": list }));
        names.failures.insert(instance, list);
    }
    Ok((changes, notes))
}

/// `Station 3.process_time 0:45` as the instance (or else class) it names, the attribute, and
/// what follows. The name may hold spaces and dots, so each dot is tried until one is followed
/// by an attribute and preceded by a name.
fn address(text: &str, names: &Names) -> Result<(Value, Attribute, String), String> {
    let mut last = Err("write it as name.attribute, such as Station 3.process_time".to_string());
    for (at, _) in text.match_indices('.') {
        let (name, rest) = (text[..at].trim(), &text[at + 1..]);
        let (wanted, typed) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(attribute, typed)| (attribute, typed));
        let Ok(attribute) = attribute(wanted) else { continue };
        match names.instance(name) {
            Ok(instance) => return Ok((json!({ "instance": instance }), attribute, typed.to_string())),
            Err(why) => match names.class(name) {
                Ok(class) => return Ok((json!({ "class": class }), attribute, typed.to_string())),
                Err(_) => last = Err(why),
            },
        }
    }
    last
}

fn target_name(target: &Value) -> &str {
    target["instance"].as_str().or_else(|| target["class"].as_str()).unwrap_or_default()
}

/// "Mill 3", one more than the highest "Mill n" so far.
fn next_name(class: &str, names: &Names) -> String {
    let prefix = format!("{class} ");
    let highest = names.instances.iter().filter_map(|placed| placed.name.strip_prefix(&prefix)?.parse::<u64>().ok()).max().unwrap_or(0);
    format!("{class} {}", highest + 1)
}

/// What the last command of a line could become, as whole lines: its first words, a macro's name
/// after `@` from `macros`, an instance or class name after the first words or after `>`, `to`,
/// `as` or `from`, or an attribute after a dot.
pub fn complete(line: &str, model: &Model, macros: &[String]) -> Vec<String> {
    let (done, last) = line.rsplit_once(';').map_or(("", line), |(done, last)| (done, last));
    let lead = if done.is_empty() { String::new() } else { format!("{done}; ") };
    let command = last.trim_start();
    let words = |text: &str| -> Vec<String> { VERBS.iter().filter(|verb| verb.starts_with(text) && **verb != text).map(|verb| format!("{lead}{verb} ")).collect() };
    // A macro's name after @.
    if let Some(typed) = command.strip_prefix('@').filter(|typed| !typed.contains(' ')) {
        return macros.iter().filter(|name| name.starts_with(typed) && name.as_str() != typed).map(|name| format!("{lead}@{name} ")).collect();
    }
    let Some(verb) = VERBS.iter().filter(|verb| command.starts_with(&format!("{verb} "))).max_by_key(|verb| verb.len()) else {
        return words(command);
    };
    let names = Names::of(model);
    let mut instance_names: Vec<&str> = names.instances.iter().map(|placed| placed.name.as_str()).collect();
    let class_names: Vec<&str> = names.classes.iter().map(String::as_str).collect();
    // Where the name being typed begins.
    let start = [">", " to ", " as ", " from "]
        .iter()
        .filter_map(|separator| command.rfind(separator).map(|at| at + separator.len()))
        .max()
        .unwrap_or(verb.len() + 1);
    let partial = &command[start.min(command.len())..];
    // A name after `>` begins after any spaces, which the completed line keeps as typed.
    let spaces = partial.len() - partial.trim_start().len();
    let (before, partial) = command.split_at(start.min(command.len()) + spaces);
    // A value after `set object.attribute `: a direction, or a word of a path's short form.
    let value_typed = partial.match_indices('.').find_map(|(dot, _)| {
        let (attribute, typed) = partial[dot + 1..].split_once(' ')?;
        Attribute::ALL.iter().any(|known| known.name() == attribute).then_some((attribute, typed))
    });
    if let Some((attribute, typed)) = value_typed
        && *verb == "set"
    {
        let word_at = typed.rfind([' ', ',']).map_or(0, |at| at + 1);
        let word = &typed[word_at..];
        let choices: &[&str] = match attribute {
            "direction" => &["forward", "backward"],
            "path" => crabsim_model::path::next_words(&typed[typed[..word_at].rfind(',').map_or(0, |at| at + 1)..word_at]),
            _ => &[],
        };
        if word.is_empty() && attribute == "path" {
            return Vec::new();
        }
        let kept = &partial[..partial.len() - word.len()];
        return choices
            .iter()
            .filter(|choice| choice.starts_with(&word.to_lowercase()) && **choice != word)
            .map(|choice| format!("{lead}{before}{kept}{choice} "))
            .collect();
    }
    if let Some(dot) = partial.rfind('.')
        && matches!(*verb, "set" | "restore")
    {
        let typed = &partial[dot + 1..];
        return Attribute::ALL
            .iter()
            .filter(|attribute| attribute.name().starts_with(typed))
            .map(|attribute| format!("{lead}{before}{}.{} ", &partial[..dot], attribute.name()))
            .collect();
    }
    let classes_first = matches!(*verb, "add" | "derive" | "class rename" | "class duplicate" | "class remove") && start == verb.len() + 1;
    let pool: Vec<&str> = if classes_first {
        // MU classes aren't placed.
        class_names.into_iter().filter(|class| *verb != "add" || !model.is_mu_class(class)).collect()
    } else {
        instance_names.extend(class_names);
        instance_names
    };
    let lower = partial.trim().to_lowercase();
    pool.iter()
        .filter(|name| name.to_lowercase().contains(&lower) && **name != partial.trim())
        .map(|name| format!("{lead}{before}{name}"))
        .collect()
}

// --- run_commands ---

pub struct RunCommands;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCommandsInput {
    /// The model file to change, in the models folder, such as `line.json`.
    pub path: String,
    /// One line of commands separated by `;`, such as `add Mill as Drill; connect Split > Drill;
    /// set Drill.process_time 0:20`, applied all or nothing.
    pub commands: String,
    /// The instance `add` places beside when nothing was placed before it in the line, as the
    /// window's selection is. Left out, `add` places at `at`.
    pub selected: Option<String>,
    /// Where `add` places with nothing to place beside, in metres. (0, 0) if left out.
    pub at: Option<[f64; 2]>,
    /// The hash of the file as you read it: the line is refused as a conflict if the file has
    /// changed since.
    pub expected_hash: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunCommandsOutput {
    pub path: String,
    /// SHA-256 of the file as saved.
    pub file_hash: String,
    /// How many commands were carried out.
    pub commands: usize,
    /// The instances `add` placed, in order.
    pub placed: Vec<String>,
    /// What the commands did beyond what they say, such as an instance getting its own copy of
    /// the failure profiles it inherited.
    pub notes: Vec<String>,
    /// What the commands removed beyond what they named, such as a removed instance's
    /// connections.
    pub also_removed: crate::edit::AlsoRemoved,
}

impl Operation for RunCommands {
    const NAME: &'static str = "run_commands";
    const DESCRIPTION: &'static str = "Changes a model file with a line of commands in people's words, the same language as the window's : line, applied as one edit_model batch, all or nothing. Commands are separated by ; and are: add <class> [as <name>] [at <x> <y>] (placed beside `selected` or the instance added before it in the line, its length plus 2 m to the right, else at `at`; a new Station's process time starts at 10 s as in the window); connect <a> > <b>; disconnect <a> > <b>; rename <a> to <b>; remove <a> (with its connections); set <name>.<attribute> <value> (on an instance, or else a class, a value written as in a model file, such as 0:45); restore <name>.<attribute>; move <a> to <x> <y> or by <dx> <dy> (metres); derive <class> as <name>; class rename <a> to <b>; class duplicate <a> as <b>; class remove <a> (with what depends on it); make class <name> from <instance>; add failure <profile> to <a>[, <b>…] availability <95%> repair <2:00> [on working time] (a constant repair time, on simulation time unless it says on working or on processing time; each instance gets its own copy, with a stream of its own, and one inheriting its class's profiles gets a copy of them first, said in notes); remove failure <profile> from <a>[, <b>…]; rename failure <profile> of <a>[, <b>…] to <name> (a profile's name in quotes if it has spaces). Names are matched exactly, or by a part only one name holds, ignoring case, and attributes by part of their name, such as proc for process_time. A command that can't apply refuses the line naming it by place, such as 3rd command. goto, speed, seed, open and validate are for the window: use start_run, read_model or validate_model instead. An agent needs its claim, as for edit_model.";
    const EFFECT: Effect = Effect::Destructive;
    const CHANGES_MODELS: bool = true;
    type Input = RunCommandsInput;
    type Output = RunCommandsOutput;

    fn run(catalogue: &Catalogue, input: RunCommandsInput) -> Result<RunCommandsOutput, OperationError> {
        let (_, text) = read_file(catalogue, &input.path)?;
        let model = crabsim_model::load(&text).map_err(|problems| OperationError::invalid_model(&format!("`{}`", input.path), &problems))?;
        let macros = crate::macros::Held::open(catalogue)?.macros.iter().map(|r#macro| (r#macro.name.clone(), r#macro.commands.clone())).collect();
        let context = Context { selected: input.selected, at: input.at.map(|[x, y]| (x, y)), macros };
        let planned = match parse(&input.commands, &model, &context).map_err(OperationError::invalid_input)? {
            Parsed::Changes(planned) => planned,
            Parsed::Macro(command) => {
                match command {
                    MacroCommand::Save { name, commands } => {
                        crate::macros::save(catalogue, &name, commands, None)?;
                    }
                    MacroCommand::Remove { name } => crate::macros::remove(catalogue, &name)?,
                    MacroCommand::Keep { name } if catalogue.source().is_user() => {
                        crate::macros::keep(catalogue, Some(&name), None, None)?;
                    }
                    MacroCommand::Keep { .. } => return Err(OperationError::invalid_input("keeping a macro is the user's own: ask the user to keep it")),
                }
                return Ok(RunCommandsOutput { path: input.path, file_hash: crate::edit::hash(&text), commands: 1, placed: Vec::new(), notes: Vec::new(), also_removed: Default::default() });
            }
            Parsed::Sequence(_) => {
                return Err(OperationError::invalid_input("that macro also holds commands for the window, such as goto: run its model changes alone, or the macro in the window"));
            }
            Parsed::Window(command) => {
                let instead = match command {
                    WindowCommand::GoTo(_) | WindowCommand::Speed(_) | WindowCommand::Seed(_) => "start_run, which runs a model to a length with a seed",
                    WindowCommand::Open(_) => "read_model",
                    WindowCommand::Validate => "validate_model",
                };
                return Err(OperationError::invalid_input(format!("that command is for the window: use {instead} instead")));
            }
        };
        // Which command each change came from, to name it in a refusal.
        let mut from_command = Vec::new();
        let mut changes = Vec::new();
        for (place, command) in planned.iter().enumerate() {
            for change in &command.changes {
                from_command.push(place);
                changes.push(serde_json::from_value::<Change>(change.clone()).expect("the parser writes changes edit_model reads"));
            }
        }
        // The history words the line as it was typed, macros filled in.
        let wording = crate::edit::shortened(&planned.iter().map(|command| command.command.as_str()).collect::<Vec<_>>().join("; "), changes.len());
        let edited = crate::edit::edit(catalogue, EditModelInput { path: input.path, changes, expected_hash: input.expected_hash }, Some(wording)).map_err(|mut error| {
            error.message = name_commands(&error.message, &from_command, &planned);
            error
        })?;
        let run = macros_run(&input.commands);
        if !run.is_empty() {
            crate::macros::Held::open(catalogue)?.ran(&run);
        }
        Ok(RunCommandsOutput {
            path: edited.path,
            file_hash: edited.file_hash,
            commands: planned.len(),
            placed: planned.iter().filter_map(|command| command.placed.clone()).collect(),
            notes: planned.iter().flat_map(|command| command.notes.clone()).collect(),
            also_removed: edited.also_removed,
        })
    }
}

/// An `edit_model` refusal with each `changes/n (kind)` named by the command it came from.
fn name_commands(message: &str, from_command: &[usize], planned: &[Planned]) -> String {
    let mut named = String::new();
    let mut rest = message;
    while let Some(at) = rest.find("changes/") {
        named.push_str(&rest[..at]);
        let after = &rest[at + "changes/".len()..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        let place = after[..digits].parse::<usize>().ok().and_then(|change| from_command.get(change).copied());
        let tail = &after[digits..];
        // Drop the change's kind in brackets, since the command says what it was.
        let tail = tail.strip_prefix(" (").and_then(|tail| tail.split_once(')').map(|(_, tail)| tail)).unwrap_or(tail);
        match place {
            Some(place) => named.push_str(&format!("{} command ({})", ordinal(place + 1), shown(&planned[place].command))),
            None => named.push_str("changes/"),
        }
        rest = tail;
    }
    named.push_str(rest);
    named
}
