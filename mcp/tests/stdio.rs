//! The MCP server end to end: its binary spoken to over stdio as a harness does, with JSON-RPC
//! written out by hand, so the test doesn't share the server's MCP library.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

/// A models folder of its own, removed afterwards.
struct Models(std::path::PathBuf);

impl Models {
    fn new(name: &str) -> Models {
        let path = std::env::temp_dir().join(format!("crabsim-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Models(path)
    }
}

impl Drop for Models {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The server, with its replies read on a thread of their own, so a server that stops answering
/// fails the test instead of hanging it.
struct Server {
    child: Child,
    input: ChildStdin,
    replies: Receiver<Value>,
    next_id: u64,
    /// For a client that never sends `initialize`, as Claude Code since 2.1.317 (MCP 2026-07-28):
    /// the `_meta` it sends with every request, naming itself there.
    meta: Option<Value>,
}

impl Server {
    /// A client that connects with `initialize`, named `crabsim-test`.
    fn start(models: &Models) -> Server {
        let mut server = Server::spawn(models);
        let initialised = server.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "crabsim-test", "version": "1" } }),
        );
        assert_eq!(initialised["serverInfo"]["name"], "crabsim", "{initialised}");
        assert!(initialised["capabilities"]["tools"].is_object(), "{initialised}");
        server.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        server
    }

    /// A client that never sends `initialize` but names itself in every request's `_meta`, as
    /// Claude Code 2.1.317 does.
    fn start_without_initialize(models: &Models, client: &str) -> Server {
        let mut server = Server::spawn(models);
        server.meta = Some(json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": client, "version": "1" },
            "io.modelcontextprotocol/clientCapabilities": {},
        }));
        server.request("server/discover", json!({}));
        server
    }

    fn spawn(models: &Models) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_crabsim-mcp"))
            .arg("--models")
            .arg(&models.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the server starts");
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let (sender, replies) = channel();
        std::thread::spawn(move || {
            for line in output.lines() {
                let Ok(line) = line else { break };
                let message: Value = serde_json::from_str(&line).unwrap_or_else(|error| panic!("not JSON on stdout: {line}: {error}"));
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        Server { child, input, replies, next_id: 1, meta: None }
    }

    fn send(&mut self, message: Value) {
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
    }

    /// Sends a request and gives its result, failing on a protocol error.
    fn request(&mut self, method: &str, mut params: Value) -> Value {
        if let Some(meta) = &self.meta {
            params["_meta"] = meta.clone();
        }
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let reply = self.replies.recv_timeout(left).unwrap_or_else(|_| panic!("no reply to {method} within a minute"));
            if reply["id"] == id {
                assert!(reply.get("error").is_none(), "{method} gave a protocol error: {reply}");
                return reply["result"].clone();
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({ "name": tool, "arguments": arguments }))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const SMALL: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","attributes":{"interval":"10"}},
  {"name":"Work","class":"Station","attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain"}],"connections":[
  {"from":"In","to":"Work","exit":1,"entrance":1},
  {"from":"Work","to":"Out","exit":1,"entrance":1}]}"#;

#[test]
fn an_agent_lists_the_tools_validates_runs_and_reads_results_over_stdio() {
    let models = Models::new("agent");
    std::fs::write(models.0.join("small.json"), SMALL).unwrap();
    let mut server = Server::start(&models);

    // Every operation is a tool with its description, schemas and hints.
    let listed = server.request("tools/list", json!({}));
    let tools = listed["tools"].as_array().unwrap();
    let tool = |name: &str| tools.iter().find(|tool| tool["name"] == name).unwrap_or_else(|| panic!("no tool {name}: {listed}")).clone();
    assert_eq!(tools.len(), 25, "{listed}");
    for users_own in ["take_model", "revoke_claim", "block_agent", "unblock_agent"] {
        assert!(!tools.iter().any(|tool| tool["name"] == users_own), "{users_own} is the user's own");
    }
    for read_only in ["validate_model", "list_models", "read_model", "model_file_format", "get_attributes", "describe_model", "query_model", "get_editors", "compare_runs", "run_status", "run_results", "find_bottleneck", "query_results"] {
        assert_eq!(tool(read_only)["annotations"]["readOnlyHint"], true, "{read_only}");
    }
    for destructive in ["write_model", "edit_model", "delete_model", "export_model", "forget_run"] {
        assert_eq!(tool(destructive)["annotations"]["readOnlyHint"], false, "{destructive}");
        assert_eq!(tool(destructive)["annotations"]["destructiveHint"], true, "{destructive}");
    }
    assert_eq!(tool("start_run")["annotations"]["destructiveHint"], false);
    let start_run = tool("start_run");
    assert!(start_run["description"].as_str().unwrap().contains("splitmix64"));
    assert_eq!(start_run["inputSchema"]["type"], "object");
    assert_eq!(start_run["outputSchema"]["properties"]["run"]["type"], "string");

    // Validate, start a short run, poll it and read its results.
    let validated = server.call("validate_model", json!({ "path": "small.json" }));
    assert_eq!(validated["isError"], false, "{validated}");
    assert_eq!(validated["structuredContent"], json!({ "valid": true, "problems": [], "warnings": [], "changed_outside": true }), "written by the test, not by crabsim");

    let started = server.call("start_run", json!({ "model": "small.json", "seed": 3, "length": "2:00:00", "observations": 2 }));
    let run = started["structuredContent"]["run"].as_str().unwrap().to_string();
    assert_eq!(run, "run-1");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = server.call("run_status", json!({ "run": run }));
        if status["structuredContent"]["state"] == "done" {
            break;
        }
        assert!(Instant::now() < deadline, "{status}");
        std::thread::sleep(Duration::from_millis(50));
    }
    let results = server.call("run_results", json!({ "run": run }));
    let content = &results["structuredContent"];
    assert_eq!(content["source"], "mcp");
    assert_eq!(content["observations"], 2);
    assert_eq!(content["failed_observations"], json!([]));
    let out = content["objects"].as_array().unwrap().iter().find(|object| object["name"] == "Out").expect("a small model is shown whole");
    assert!(out["figures"]["mus_in"]["mean"].as_f64().unwrap() > 0.0, "{content}");
    // The text content holds the same, for clients that read only text.
    let text: Value = serde_json::from_str(results["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&text, content);

    // A claim on a file another agent holds is refused, naming it. Claims aren't queued.
    let other = crabsim_catalogue::Catalogue::open(crabsim_catalogue::Settings::new(&models.0, crabsim_catalogue::Source::Mcp)).unwrap();
    other.call("claim_model", json!({ "path": "small.json", "goal": "a tidier model", "doing": "renaming" })).unwrap();
    let in_use = server.call("claim_model", json!({ "path": "small.json", "goal": "check the name", "doing": "claiming" }));
    assert_eq!(in_use["isError"], true, "{in_use}");
    let error: Value = serde_json::from_str(in_use["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(error["kind"], "model_in_use");
    assert!(error["message"].as_str().unwrap().contains("a tidier model"), "{error}");
    drop(other);

    // A claim shows the agent by the name its harness gave when it connected.
    let claimed = server.call("claim_model", json!({ "path": "small.json", "goal": "check the name", "doing": "claiming" }));
    assert_eq!(claimed["structuredContent"]["file"]["claim"]["client"], "crabsim-test", "{claimed}");
    assert_eq!(claimed["structuredContent"]["file"]["claim"]["goal"], "check the name", "{claimed}");

    // A failure comes back as a tool result marked as an error, for the agent to read.
    let failed = server.call("read_model", json!({ "path": "../outside.json" }));
    assert_eq!(failed["isError"], true, "{failed}");
    let error: Value = serde_json::from_str(failed["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(error["kind"], "outside_models_folder");
    let invalid = server.call("validate_model", json!({}));
    assert_eq!(invalid["isError"], true);
    let unknown = server.call("no_such_tool", json!({}));
    assert_eq!(unknown["isError"], true);
}

#[test]
fn a_client_that_names_itself_in_each_request_is_shown_by_that_name() {
    let models = Models::new("discover");
    std::fs::write(models.0.join("small.json"), SMALL).unwrap();
    let mut server = Server::start_without_initialize(&models, "claude-code");
    let claimed = server.call("claim_model", json!({ "path": "small.json", "goal": "check the name", "doing": "claiming" }));
    assert_eq!(claimed["structuredContent"]["file"]["claim"]["client"], "claude-code", "{claimed}");
}

#[test]
fn the_server_refuses_to_start_on_a_folder_holding_source_code() {
    let models = Models::new("source");
    std::fs::write(models.0.join("Cargo.toml"), "[package]").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_crabsim-mcp")).arg("--models").arg(&models.0).stdin(Stdio::null()).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("source code"), "{output:?}");
    assert!(output.stdout.is_empty(), "nothing on the protocol's channel");
    assert!(!models.0.join(".crabsim").exists());
}
