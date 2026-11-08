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
        "Started {} by {}, finished {}, crabsim {}, model SHA-256 {}\n\n",
        cell(&results["started"]),
        cell(&results["source"]),
        if results["finished"].is_null() { "not yet".to_string() } else { cell(&results["finished"]) },
        cell(&results["crabsim_version"]),
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
    let mut text = format!("{count} change{} applied to {}, now SHA-256 {}\n", if count == 1 { "" } else { "s" }, cell(&edited["path"]), cell(&edited["hash"]));
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
    let mut text = format!("{} adopted as it is, SHA-256 {}\n", cell(&adopted["path"]), cell(&adopted["hash"]));
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
