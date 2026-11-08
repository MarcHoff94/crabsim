//! Operation outputs as tables for people: what agents read as JSON.

use serde_json::Value;

/// Rows under headers, each column as wide as its widest cell, numbers to the right.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.chars().count()).collect();
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let numeric: Vec<bool> = (0..headers.len())
        .map(|column| !rows.is_empty() && rows.iter().all(|row| row.get(column).is_none_or(|cell| cell.is_empty() || looks_numeric(cell))))
        .collect();
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(column, cell)| if numeric[column] { format!("{cell:>w$}", w = widths[column]) } else { format!("{cell:<w$}", w = widths[column]) })
            .collect();
        padded.join("  ").trim_end().to_string() + "\n"
    };
    let mut text = line(headers.to_vec());
    for row in rows {
        text += &line(row.iter().map(String::as_str).collect());
    }
    text
}

fn looks_numeric(cell: &str) -> bool {
    cell.trim_end_matches(" %").trim_end_matches(" s").parse::<f64>().is_ok()
}

/// A JSON value as a cell.
fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// A query's result as a table.
pub fn query(result: &Value) -> String {
    let headers: Vec<&str> = result["columns"].as_array().into_iter().flatten().map(|column| column.as_str().unwrap_or_default()).collect();
    let rows: Vec<Vec<String>> =
        result["rows"].as_array().into_iter().flatten().map(|row| row.as_array().into_iter().flatten().map(cell).collect()).collect();
    let mut text = table(&headers, &rows);
    text += &format!("{} row{}", rows.len(), if rows.len() == 1 { "" } else { "s" });
    if result["truncated"] == true {
        text += ", stopped at the cap: narrow the query or aggregate";
    }
    text + "\n"
}

/// A figure's amount as people read it: shares as percentages, times as they are.
fn amount(statistic: &str, value: &Value) -> String {
    match value {
        Value::Number(number) => {
            let x = number.as_f64().unwrap_or_default();
            if statistic.ends_with("_share") || statistic == "availability" {
                format!("{:.1} %", x * 100.0)
            } else if statistic.ends_with("_s") {
                crabsim::time::format_time_exact(crabsim::SimTime::from_secs_f64(x))
            } else {
                // Results come rounded to 4 significant digits already.
                format!("{x}")
            }
        }
        other => cell(other),
    }
}

/// A run's results, as `run_results` gives them: its settings, its observations, then each
/// object's figures.
pub fn results(results: &Value) -> String {
    let label = results["label"].as_str().map_or(String::new(), |label| format!(" \"{label}\""));
    let mut text = format!(
        "{}{label}: {}, seed {}, {} observation{} of {} after a warm-up of {}, {}\n",
        cell(&results["run"]),
        cell(&results["model"]),
        cell(&results["seed"]),
        results["observations"],
        if results["observations"] == 1 { "" } else { "s" },
        cell(&results["length"]),
        cell(&results["warm_up"]),
        cell(&results["state"]),
    );
    text += &format!(
        "Started {} by {}, finished {}, crabsim {} ({}), model SHA-256 {}\n\n",
        cell(&results["started"]),
        cell(&results["agent"]),
        if results["finished"].is_null() { "not yet".to_string() } else { cell(&results["finished"]) },
        cell(&results["crabsim_version"]),
        cell(&results["crabsim_build"]),
        cell(&results["model_hash"]).chars().take(12).collect::<String>(),
    );
    let failed = results["failed_observations"].as_array().map_or(0, Vec::len);
    let ended = results["observation_runs"].as_array().map_or_else(|| "".to_string(), |runs| format!("{} of ", runs.len()));
    text += &format!(
        "{ended}{} observations ended, {}: {} events and {:.3} s of wall time between them\n",
        results["observations"],
        if failed == 0 { "none failed".to_string() } else { format!("{failed} failed") },
        results["events"],
        results["wall_time_s"].as_f64().unwrap_or_default(),
    );
    // Every observation when asked for, otherwise those that failed.
    let shown = if results["observation_runs"].is_array() { &results["observation_runs"] } else { &results["failed_observations"] };
    let observations: Vec<Vec<String>> = shown
        .as_array()
        .into_iter()
        .flatten()
        .map(|observation| {
            let error = match (&observation["error"], &observation["error_time"]) {
                (Value::String(error), Value::String(time)) => format!("at {time}: {error}"),
                _ => String::new(),
            };
            vec![
                cell(&observation["observation"]),
                cell(&observation["seed"]),
                cell(&observation["events"]),
                format!("{:.3} s", observation["wall_time_s"].as_f64().unwrap_or_default()),
                error,
            ]
        })
        .collect();
    if !observations.is_empty() {
        text += &table(&["Observation", "Seed", "Events", "Wall time", "Error"], &observations);
    }
    for object in results["objects"].as_array().into_iter().flatten() {
        let why: Vec<String> = object["why"].as_array().into_iter().flatten().map(|why| cell(why).replace('_', " ")).collect();
        text += &format!("\n{} ({}, {}): {}\n", cell(&object["name"]), cell(&object["class"]), cell(&object["built_in_class"]), why.join(", "));
        let mut rows = Vec::new();
        for statistic in crabsim_catalogue::STATISTICS {
            let key = statistic;
            let Some(figure) = object["figures"].get(key) else { continue };
            let interval = match figure["ci95"].as_array() {
                Some(bounds) => format!("{} to {}", amount(key, &bounds[0]), amount(key, &bounds[1])),
                None => String::new(),
            };
            rows.push(vec![key.to_string(), amount(key, &figure["mean"]), interval, amount(key, &figure["min"]), amount(key, &figure["max"])]);
        }
        text += &table(&["Statistic", "Mean", "95% interval", "Min", "Max"], &rows);
    }
    text
}

/// What `edit_model` did.
pub fn edited(edited: &Value) -> String {
    let count = edited["changes"].as_u64().unwrap_or_default();
    let mut text = format!("{count} change{} applied to {}, now SHA-256 {}\n", if count == 1 { "" } else { "s" }, cell(&edited["path"]), cell(&edited["file_hash"]));
    let also = &edited["also_removed"];
    for (what, key) in [("instances", "instances"), ("classes", "classes")] {
        let names: Vec<String> = also[key].as_array().into_iter().flatten().map(cell).collect();
        if !names.is_empty() {
            text += &format!("Also removed the {what} {}\n", names.join(", "));
        }
    }
    let connections: Vec<String> = also["connections"].as_array().into_iter().flatten().map(|c| format!("{} -> {}", cell(&c["from"]), cell(&c["to"]))).collect();
    if !connections.is_empty() {
        text += &format!("Also removed the connections {}\n", connections.join(", "));
    }
    text
}

/// What `get_attributes` gives, as a table.
pub fn attributes(name: &str, got: &Value) -> String {
    let chain: Vec<String> = got["chain"].as_array().into_iter().flatten().map(cell).collect();
    let mut text = format!("{name} ({})\n", chain.join(", "));
    if got["changed_outside"] == true {
        text += "The file was changed outside crabsim, or crabsim never saved it: crabsim adopt it before changing it\n";
    }
    let rows: Vec<Vec<String>> = got["attributes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|attribute| {
            let from = match attribute["from"].as_str().unwrap_or_default() {
                "class" => format!("class {}", cell(&attribute["class"])),
                "built_in" => "built in".to_string(),
                "not_set" => "not set".to_string(),
                other => other.to_string(),
            };
            let value = attribute.get("value").map_or(String::new(), |value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
            vec![cell(&attribute["attribute"]), value, from]
        })
        .collect();
    text += &table(&["Attribute", "Value", "From"], &rows);
    if let Some(successors) = got["successors"].as_array() {
        let names: Vec<String> = successors.iter().map(cell).collect();
        text += &format!("Successors, in the order tried: {}\n", if names.is_empty() { "none".to_string() } else { names.join(", ") });
    }
    text
}

/// What `adopt_model` did.
pub fn adopted(adopted: &Value) -> String {
    let mut text = format!("{} adopted as it is, SHA-256 {}\n", cell(&adopted["path"]), cell(&adopted["file_hash"]));
    let problems = adopted["problems"].as_array().map_or(0, Vec::len);
    if problems > 0 {
        text += &format!("It has {problems} problem{}: crabsim validate it, or write it again\n", if problems == 1 { "" } else { "s" });
    }
    text
}

/// The edit queue: each file's editor and waiting agents, and the blocked agents.
pub fn queue(queue: &Value) -> String {
    let files: Vec<Value> = match queue["files"].as_array() {
        Some(files) => files.clone(),
        None => vec![serde_json::json!({ "path": "", "editor": queue["editor"], "queue": queue["queue"] })],
    };
    let mut rows = Vec::new();
    for file in &files {
        let path = cell(&file["path"]);
        match &file["editor"] {
            Value::Null => rows.push(vec![path.clone(), "nobody edits".to_string(), String::new(), String::new(), String::new(), String::new()]),
            editor if editor["user"] == true => rows.push(vec![path.clone(), "editor".to_string(), "the user".to_string(), String::new(), String::new(), cell(&editor["since"])]),
            editor => rows.push(vec![path.clone(), "editor".to_string(), cell(&editor["agent"]), cell(&editor["client"]), cell(&editor["purpose"]), cell(&editor["since"])]),
        }
        for (place, waiting) in file["queue"].as_array().into_iter().flatten().enumerate() {
            rows.push(vec![path.clone(), format!("waiting {place}"), cell(&waiting["agent"]), cell(&waiting["client"]), cell(&waiting["purpose"]), cell(&waiting["since"])]);
        }
    }
    let mut text = if rows.is_empty() { "Nobody edits or waits for any model file.\n".to_string() } else { table(&["File", "Role", "Who", "Harness", "Purpose", "Since"], &rows) };
    let blocked: Vec<String> = queue["blocked"].as_array().into_iter().flatten().map(|agent| format!("{} ({})", cell(&agent["agent"]), cell(&agent["client"]))).collect();
    if !blocked.is_empty() {
        text += &format!("Blocked: {}\n", blocked.join(", "));
    }
    text
}

/// What `compare_runs` gives, as a table per object.
pub fn compared(compared: &Value) -> String {
    let side = |side: &Value| {
        let label = side["label"].as_str().map_or(String::new(), |label| format!(" \"{label}\""));
        format!("{}{label}", cell(&side["run"]))
    };
    let pairing = match compared["pairing"].as_str() {
        Some("paired") => "paired".to_string(),
        _ => format!("not paired, because {}", cell(&compared["why_unpaired"])),
    };
    let mut text = format!("{} minus {}: {pairing}\n", side(&compared["b"]), side(&compared["a"]));
    for warning in compared["warnings"].as_array().into_iter().flatten() {
        text += &format!("Warning: {}\n", cell(warning));
    }
    for dropped in compared["dropped"].as_array().into_iter().flatten() {
        text += &format!("Left out: {}\n", cell(&dropped["reason"]));
    }
    for object in compared["objects"].as_array().into_iter().flatten() {
        let common = object.get("why_not_common").map_or(String::new(), |why| format!(" (no common random numbers: {})", cell(why)));
        text += &format!("\n{}{common}\n", cell(&object["name"]));
        let mut rows = Vec::new();
        for statistic in crabsim_catalogue::STATISTICS {
            let Some(figure) = object["figures"].get(statistic) else { continue };
            let interval = match figure["ci95"].as_array() {
                Some(bounds) => format!("{} to {}", amount(statistic, &bounds[0]), amount(statistic, &bounds[1])),
                None => String::new(),
            };
            let relative = figure["relative"].as_f64().map_or(String::new(), |share| format!("{:+.1} %", share * 100.0));
            rows.push(vec![
                statistic.to_string(),
                amount(statistic, &figure["a"]),
                amount(statistic, &figure["b"]),
                amount(statistic, &figure["difference"]),
                interval,
                relative,
                if figure["clear"] == true { "yes".to_string() } else { "no".to_string() },
                cell(&figure["method"]),
            ]);
        }
        text += &table(&["Statistic", "a", "b", "Difference", "95% interval", "Relative", "Clear", "Method"], &rows);
    }
    let unchanged: Vec<String> = compared["unchanged"].as_array().into_iter().flatten().map(cell).collect();
    if !unchanged.is_empty() {
        text += &format!("\nUnchanged in every observation: {}\n", unchanged.join(", "));
    }
    for (which, key) in [("a", "only_in_a"), ("b", "only_in_b")] {
        let names: Vec<String> = compared[key].as_array().into_iter().flatten().map(cell).collect();
        if !names.is_empty() {
            text += &format!("\nOnly in {which}'s model: {}\n", names.join(", "));
        }
    }
    text
}

/// `count` and a noun, made plural unless there is one.
fn counted(count: u64, noun: &str) -> String {
    let plural = if noun.ends_with('s') { format!("{noun}es") } else { format!("{noun}s") };
    format!("{count} {}", if count == 1 { noun } else { &plural })
}

/// A list of names with its full count, such as "Sources (12): A, B, and 10 more".
fn listed(what: &str, list: &Value, none: &str) -> String {
    let count = list["count"].as_u64().unwrap_or_default();
    if count == 0 {
        return format!("{none}\n");
    }
    let names: Vec<String> = list["names"].as_array().into_iter().flatten().map(cell).collect();
    let more = count.saturating_sub(names.len() as u64);
    let more = if more > 0 { format!(", and {more} more") } else { String::new() };
    format!("{what} ({count}): {}{more}\n", names.join(", "))
}

/// What `describe_model` gives.
pub fn described(described: &Value) -> String {
    let mut text = String::new();
    if let Some(description) = described["description"].as_str() {
        text += &format!("{description}\n");
    }
    let problems = described["problems"]["count"].as_u64().unwrap_or_default();
    text += &format!(
        "{}, {}\n\n",
        counted(described["instances"].as_u64().unwrap_or_default(), "instance"),
        if problems == 0 { "valid".to_string() } else { format!("with {}", counted(problems, "problem")) }
    );
    let rows: Vec<Vec<String>> = described["classes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|class| vec![cell(&class["class"]), cell(&class["built_in_class"]), cell(&class["instances"])])
        .collect();
    text += &table(&["Class", "Built-in class", "Instances"], &rows);
    if let Some(more) = described["more_classes"].as_u64() {
        text += &format!("and {} more\n", counted(more, "class"));
    }
    text += "\n";
    text += &listed("Sources", &described["sources"], "No Sources");
    text += &listed("Drains", &described["drains"], "No Drains");
    for (what, key, width) in [("Branch points", "branch_points", "successor"), ("Merge points", "merge_points", "predecessor")] {
        let points = &described[key];
        let count = points["count"].as_u64().unwrap_or_default();
        if count == 0 {
            text += &format!("No {}\n", what.to_lowercase());
            continue;
        }
        let widest: Vec<String> = points["widest"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|point| format!("{} ({})", cell(&point["name"]), counted(point[format!("{width}s")].as_u64().unwrap_or_default(), width)))
            .collect();
        let more = count.saturating_sub(widest.len() as u64);
        let more = if more > 0 { format!(", and {more} more") } else { String::new() };
        text += &format!("{what} ({count}), widest first: {}{more}\n", widest.join(", "));
    }
    text += "\n";
    for flow in described["flows"].as_array().into_iter().flatten() {
        let source = cell(&flow["source"]);
        let drains = flow["drains"].as_array().map_or(&[][..], Vec::as_slice);
        if drains.is_empty() {
            text += &format!("{source} reaches no Drain\n");
        }
        for reach in drains {
            let fewest = reach["fewest_objects"].as_u64().unwrap_or_default();
            let past = match reach["most_objects"].as_u64() {
                Some(most) if most == fewest => counted(most, "object"),
                Some(most) => format!("{fewest} to {most} objects"),
                None => format!("at least {}, through a loop", counted(fewest, "object")),
            };
            text += &format!("{source} reaches {} past {past}\n", cell(&reach["drain"]));
        }
        if let Some(more) = flow["more_drains"].as_u64() {
            text += &format!("{source} reaches {} more\n", counted(more, "Drain"));
        }
    }
    let sources = described["sources"]["count"].as_u64().unwrap_or_default();
    let shown = described["flows"].as_array().map_or(0, Vec::len) as u64;
    if sources > shown {
        text += &format!("and the flows of {} more\n", counted(sources - shown, "Source"));
    }
    text += "\n";
    let loops = &described["loops"];
    match loops["count"].as_u64().unwrap_or_default() {
        0 => text += "No loops\n",
        count => {
            text += &format!("Loops ({count}):\n");
            for example in loops["examples"].as_array().into_iter().flatten() {
                let cycle: Vec<String> = example["cycle"].as_array().into_iter().flatten().map(cell).collect();
                let length = example["cycle_length"].as_u64().unwrap_or_default();
                let round = if cycle.len() as u64 == length { format!(" -> {}", cycle[0]) } else { " -> ...".to_string() };
                text += &format!("  {}{round}, in a group of {}\n", cycle.join(" -> "), counted(example["objects"].as_u64().unwrap_or_default(), "object"));
            }
        }
    }
    text += &listed("Unreachable", &described["unreachable"], "Nothing is unreachable");
    text += &listed("Dead ends", &described["dead_ends"], "No dead ends");
    if problems == 0 {
        text += "No problems\n";
    } else {
        text += &format!("\n{}:\n", counted(problems, "problem"));
        for problem in described["problems"]["first"].as_array().into_iter().flatten() {
            let pointer = problem["pointer"].as_str().filter(|pointer| !pointer.is_empty()).unwrap_or("/");
            let name = problem["name"].as_str().map_or(String::new(), |name| format!(" \"{name}\""));
            text += &format!("  {pointer}{name}: {}\n", cell(&problem["message"]));
        }
        let shown = described["problems"]["first"].as_array().map_or(0, Vec::len) as u64;
        if problems > shown {
            text += &format!("  and {} more: crabsim validate gives them all\n", problems - shown);
        }
    }
    text
}

/// Instances with their classes, and a distance when they have one.
fn instances(list: &Value) -> String {
    let with_distance = list.as_array().is_some_and(|list| list.iter().any(|node| node.get("distance").is_some()));
    let rows: Vec<Vec<String>> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| {
            let mut row = vec![cell(&node["name"]), cell(&node["class"]), cell(&node["built_in_class"])];
            if with_distance {
                row.insert(0, cell(&node["distance"]));
            }
            row
        })
        .collect();
    if rows.is_empty() {
        return "none\n".to_string();
    }
    if with_distance { table(&["Distance", "Name", "Class", "Built-in class"], &rows) } else { table(&["Name", "Class", "Built-in class"], &rows) }
}

/// "N instances", and that the list stops short when it does.
fn total(answer: &Value, shown: &str) -> String {
    let count = answer["count"].as_u64().unwrap_or_default();
    let cut = if answer["truncated"] == true { format!(", the first {} shown", answer[shown].as_array().map_or(0, Vec::len)) } else { String::new() };
    format!("{}{cut}\n", counted(count, "instance"))
}

/// What `query_model` answers, by its kind.
pub fn answer(answer: &Value) -> String {
    let path = |path: &Value| path.as_array().into_iter().flatten().map(cell).collect::<Vec<_>>().join(" -> ");
    match answer["kind"].as_str().unwrap_or_default() {
        "neighbours" => {
            let name = cell(&answer["name"]);
            format!(
                "Predecessors of {name}:\n{}\nSuccessors of {name}, in the order tried:\n{}",
                instances(&answer["predecessors"]),
                instances(&answer["successors"])
            )
        }
        kind @ ("upstream" | "downstream") => {
            let within = answer["steps"].as_u64().map_or(String::new(), |steps| format!(", within {}", counted(steps, "step")));
            let what = if kind == "upstream" { "Upstream" } else { "Downstream" };
            format!("{what} of {}{within}:\n{}{}", cell(&answer["name"]), instances(&answer["instances"]), total(answer, "instances"))
        }
        "paths" => {
            let (from, to) = (cell(&answer["from"]), cell(&answer["to"]));
            if answer["shortest"].is_null() {
                return format!("No route from {from} to {to}\n");
            }
            let mut text = format!("Shortest: {}\n", path(&answer["shortest"]));
            let others = answer["others"].as_array().map_or(&[][..], Vec::as_slice);
            if !others.is_empty() {
                text += "Others, shortest first:\n";
                for other in others {
                    text += &format!("  {}\n", path(other));
                }
            }
            let count = answer["count"].as_u64().unwrap_or_default();
            let more = if answer["more"] == true { "More than " } else { "" };
            text += &format!("{more}{} from {from} to {to}\n", counted(count, "path"));
            text
        }
        "subgraph_around" => {
            let steps = answer["steps"].as_u64().unwrap_or_default();
            let mut text = format!("Within {} of {}, either way:\n", counted(steps, "step"), cell(&answer["name"]));
            text += &instances(&answer["nodes"]);
            if answer["truncated"] == true {
                text += "and more, left out: give fewer steps\n";
            }
            text += "Connections between them:\n";
            for edge in answer["edges"].as_array().into_iter().flatten() {
                text += &format!("  {} -> {}\n", cell(&edge["from"]), cell(&edge["to"]));
            }
            text
        }
        "find_instances" => {
            let rows: Vec<Vec<String>> = answer["instances"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|found| {
                    let values: Vec<String> = found["values"].as_object().into_iter().flatten().map(|(key, value)| format!("{key} = {}", cell(value))).collect();
                    vec![cell(&found["name"]), cell(&found["class"]), cell(&found["built_in_class"]), values.join("; ")]
                })
                .collect();
            let found = if rows.is_empty() { String::new() } else { table(&["Name", "Class", "Built-in class", "Values"], &rows) };
            found + &total(answer, "instances")
        }
        "class_tree" => {
            let mut text = String::new();
            for class in answer["classes"].as_array().into_iter().flatten() {
                let indent = "  ".repeat(class["depth"].as_u64().unwrap_or_default() as usize);
                let kind = class["parent"].as_str().map_or("built in".to_string(), |parent| format!("derived from {parent}"));
                let sets: Vec<String> = class["sets"].as_object().into_iter().flatten().map(|(key, value)| format!("{key} = {}", cell(value))).collect();
                let sets = if sets.is_empty() { String::new() } else { format!(", sets {}", sets.join("; ")) };
                text += &format!(
                    "{indent}{} ({kind}){sets}: {} of its own, {} in all\n",
                    cell(&class["name"]),
                    counted(class["instances_direct"].as_u64().unwrap_or_default(), "instance"),
                    class["instances_total"].as_u64().unwrap_or_default()
                );
                if let Some(names) = class["instances"].as_array().filter(|names| !names.is_empty()) {
                    text += &format!("{indent}  {}\n", names.iter().map(cell).collect::<Vec<_>>().join(", "));
                }
            }
            if answer["truncated"] == true {
                text += "Instance names stop at 1,000: ask about one class with --class\n";
            }
            text
        }
        other => format!("{other}: {answer}\n"),
    }
}

/// A size as people read it.
fn size(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..1_000_000 => format!("{:.1} KB", bytes as f64 / 1e3),
        _ => format!("{:.1} MB", bytes as f64 / 1e6),
    }
}

/// What `export_model` wrote.
pub fn exported(exported: &Value) -> String {
    let mut text = String::new();
    for file in exported["files"].as_array().into_iter().flatten() {
        text += &format!("Wrote {} ({})\n", cell(&file["path"]), size(file["bytes"].as_u64().unwrap_or_default()));
    }
    text += &format!(
        "{}, {} and {}, as {}\n",
        counted(exported["instances"].as_u64().unwrap_or_default(), "instance"),
        counted(exported["classes"].as_u64().unwrap_or_default(), "class"),
        counted(exported["connections"].as_u64().unwrap_or_default(), "connection"),
        cell(&exported["format"])
    );
    text
}
