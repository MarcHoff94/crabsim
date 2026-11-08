//! The results store as `query_results` sees it: its tables, and a query that can only read,
//! capped in rows and time.

mod common;

use common::Setup;
use serde_json::json;

#[test]
fn a_new_store_has_the_documented_tables() {
    let setup = Setup::new("tables");
    let tables = setup.ok("query_results", json!({ "sql": "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name" }));
    assert_eq!(tables["rows"], json!([["observations"], ["runs"], ["statistics"]]));
    let columns = |table: &str| {
        let info = setup.ok("query_results", json!({ "sql": format!("SELECT name FROM pragma_table_xinfo('{table}') ORDER BY cid") }));
        info["rows"].as_array().unwrap().iter().map(|row| row[0].as_str().unwrap().to_string()).collect::<Vec<_>>()
    };
    assert_eq!(
        columns("runs"),
        [
            "number", "id", "model_path", "model_text", "model_hash", "seed", "length_s", "warm_up_s", "observations", "label", "source",
            "crabsim_version", "started", "finished", "state", "process"
        ]
    );
    assert_eq!(columns("observations"), ["run", "number", "seed", "events", "wall_time_s", "error", "error_time_s"]);
    assert_eq!(columns("statistics"), ["run", "observation", "object", "class", "built_in_class", "statistic", "value"]);
}

#[test]
fn a_query_has_math_functions_for_intervals_and_rates() {
    let setup = Setup::new("math");
    let result = setup.ok("query_results", json!({ "sql": "SELECT sqrt(16.0), power(2, 10), ln(exp(1.0)), log10(1000), sqrt(2)" }));
    let row = result["rows"][0].as_array().unwrap();
    assert_eq!(&row[..4], [json!(4.0), json!(1024.0), json!(1.0), json!(3.0)]);
    assert!((row[4].as_f64().unwrap() - std::f64::consts::SQRT_2).abs() < 1e-12);
    assert_eq!(setup.ok("query_results", json!({ "sql": "SELECT sqrt(-1), sqrt(NULL)" }))["rows"], json!([[null, null]]));
    let query = setup.catalogue.operations().find(|o| o.name == "query_results").unwrap();
    assert!(query.description.contains("sqrt, power, ln, exp and log10"), "{}", query.description);
}

#[test]
fn a_query_gives_columns_and_rows() {
    let setup = Setup::new("query");
    let result = setup.ok("query_results", json!({ "sql": "-- a comment first\nSELECT 1 AS one, 'two' AS two, 2.5 AS three, NULL AS four" }));
    assert_eq!(result, json!({ "columns": ["one", "two", "three", "four"], "rows": [[1, "two", 2.5, null]], "truncated": false }));
}

#[test]
fn a_query_that_would_change_the_store_is_refused_and_changes_nothing() {
    let setup = Setup::new("read-only");
    setup.ok("query_results", json!({ "sql": "SELECT 1" }));
    let store = setup.folder.join(".crabsim/results.sqlite");
    let outside = setup.folder.join("attached.sqlite");
    for sql in [
        "INSERT INTO runs (model_path) VALUES ('x')".to_string(),
        "DELETE FROM statistics".to_string(),
        "DROP TABLE runs".to_string(),
        "UPDATE runs SET label = 'x'".to_string(),
        "CREATE TABLE extra (x)".to_string(),
        "PRAGMA journal_mode = DELETE".to_string(),
        "VACUUM".to_string(),
        "BEGIN EXCLUSIVE".to_string(),
        format!("ATTACH DATABASE '{}' AS other", outside.display()),
        "WITH x AS (SELECT 1) DELETE FROM runs".to_string(),
        "SELECT 1; DROP TABLE runs".to_string(),
        "/* SELECT */ DELETE FROM runs".to_string(),
    ] {
        let error = setup.err("query_results", json!({ "sql": sql }));
        assert_eq!(error["kind"], "invalid_input", "{sql}: {error}");
    }
    let tables = setup.ok("query_results", json!({ "sql": "SELECT count(*) FROM sqlite_master WHERE name IN ('runs', 'observations', 'statistics')" }));
    assert_eq!(tables["rows"], json!([[3]]));
    assert!(!outside.exists(), "no other database was made");
    assert!(store.is_file());
}

#[test]
fn a_query_stops_at_its_row_cap() {
    let setup = Setup::new("row-cap");
    let sql = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 5000) SELECT x FROM n";
    let result = setup.ok("query_results", json!({ "sql": sql }));
    assert_eq!(result["rows"].as_array().unwrap().len(), 1_000);
    assert_eq!(result["rows"][999], json!([1000]));
    assert_eq!(result["truncated"], true);

    let exactly = setup.ok("query_results", json!({ "sql": sql.replace("5000", "1000") }));
    assert_eq!(exactly["rows"].as_array().unwrap().len(), 1_000);
    assert_eq!(exactly["truncated"], false);
}

#[test]
fn a_query_that_runs_too_long_is_stopped() {
    let setup = Setup::new("time-cap");
    let endless = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT count(*) FROM n";
    // On a thread of its own, so a missing time cap fails the test instead of hanging it.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(setup.err("query_results", json!({ "sql": endless }))));
    let error = receiver.recv_timeout(std::time::Duration::from_secs(60)).expect("the query stops within its time cap");
    assert_eq!(error["kind"], "invalid_input");
    assert!(error["message"].as_str().unwrap().contains("longer than"), "{error}");
}

#[test]
fn a_query_that_isnt_sql_says_why() {
    let setup = Setup::new("bad-sql");
    let error = setup.err("query_results", json!({ "sql": "SELECT nothing FROM nowhere" }));
    assert_eq!(error["kind"], "invalid_input");
    assert!(error["message"].as_str().unwrap().contains("nowhere"), "{error}");
}
