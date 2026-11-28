//! The Status tab's logic (ADR 0030): where the open model stands, opening a model verifying it,
//! the user accepting what an agent proposed while the agent holds the file, and checking a
//! stage. An agent is played by a catalogue opened as `Source::Mcp`. No window.

use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::workspace::Workspace;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Source every 10 s and a Press of 8 s, working 80% of the time.
const LINE: &str = r#"{"format_version":2,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"attributes":{"interval":"10"}},
  {"name":"Press","class":"Station","position":{"x":3,"y":0},"attributes":{"process_time":"8"}},
  {"name":"Out","class":"Drain","position":{"x":6,"y":0}}],"connections":[
  {"from":"In","to":"Press","exit":1,"entrance":1},
  {"from":"Press","to":"Out","exit":1,"entrance":1}],
  "expectations":[{"id":"busy","text":"Press.working_share since start >= 70%","state":"accepted"}]}"#;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-status-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        user.call("write_model", json!({ "path": "line.json", "text": LINE })).unwrap();
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opening_a_model_verifies_it_and_the_status_says_where_it_stands() {
    let folder = Folder::new("open");
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    workspace.wait_for_runs();
    let status = workspace.status().unwrap();
    assert_eq!(status.stage, "verified", "{status:#?}");
    assert!(status.verification.current);
    let busy = &status.expectations[0];
    assert_eq!((busy.verdict.as_deref(), busy.verdict_text().as_str()), (Some("met"), "met"));
    assert!(status.next.starts_with("verified. Next"), "{}", status.next);
    // Opened again as it is, it isn't checked again.
    assert!(workspace.open_model("line.json"));
    assert!(!workspace.status().unwrap().checking());
    assert_eq!(workspace.check_stage("verification").unwrap(), vec!["run-1".to_string()], "the same check");
}

#[test]
fn the_user_accepts_what_an_agent_proposed_while_it_holds_the_file() {
    let folder = Folder::new("accept");
    let agent = Catalogue::open(Settings::new(&folder.0, Source::Mcp)).unwrap();
    agent.call("claim_model", json!({ "path": "line.json", "goal": "a faster line", "doing": "proposing" })).unwrap();
    agent
        .call("edit_model", json!({ "path": "line.json", "changes": [
            { "change": "add_expectation", "id": "out", "text": "model.throughput_per_hour since start >= 300/h" },
            { "change": "add_expectation", "id": "alone", "text": "Out.mus_in since start >= 1" },
            { "change": "add_request", "words": "ship 300 an hour", "expectations": ["out"] }
        ] }))
        .unwrap();
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    let status = workspace.status().unwrap();
    assert_eq!(status.to_accept(), ["r1", "alone"], "the request accepts its own");
    assert_eq!(status.expectations_of(&status.requests[0]).len(), 1);
    assert_eq!(status.unrequested().iter().map(|expectation| expectation.id.as_str()).collect::<Vec<_>>(), ["busy", "alone"]);
    workspace.accept(&status.to_accept()).unwrap();
    let status = workspace.status().unwrap();
    assert!(status.to_accept().is_empty(), "{status:#?}");
    assert!(status.requests[0].accepted);
    // The change was the user's, and the agent still holds its claim.
    let editors = workspace.editors().unwrap();
    assert!(editors.files.iter().any(|file| file.path == "line.json" && file.claim.is_some()));
    // Changed since its check, the verification isn't current until checked again.
    workspace.wait_for_runs();
    assert!(!workspace.status().unwrap().verification.current);
    workspace.check_stage("verification").unwrap();
    workspace.wait_for_runs();
    let status = workspace.status().unwrap();
    assert!(status.verification.current && status.stage == "verified", "{status:#?}");
    // Validation has nothing to check.
    assert!(!status.has_stage("validation"));
    assert!(workspace.check_stage("validation").unwrap_err().contains("no validation expectations"));
}

#[test]
fn an_expectation_the_model_cant_meet_says_why_before_anything_runs() {
    let folder = Folder::new("bounds");
    let user = Catalogue::open(Settings::new(&folder.0, Source::Cli)).unwrap();
    let fast = LINE.replace(r#""id":"busy","text":"Press.working_share since start >= 70%""#, r#""id":"fast","text":"Out.throughput_per_hour since start >= 400/h""#);
    user.call("write_model", json!({ "path": "fast.json", "text": fast })).unwrap();
    drop(user);
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("fast.json"));
    let status = workspace.status().unwrap();
    let why = status.expectations[0].cannot_be_met.as_deref().expect("a bound");
    assert!(why.contains("360/h"), "{why}");
    assert!(status.next.starts_with("crabsim works out that fast can't be met as the model is"), "{}", status.next);
}
