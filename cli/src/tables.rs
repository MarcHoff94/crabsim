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
