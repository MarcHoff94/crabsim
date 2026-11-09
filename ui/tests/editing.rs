//! The Editing tab's logic: the agents' claims as the user sees them, and every control of the
//! user's, each checked against the latest state before it acts and read again after (ADR 0013).
//! Agents are played by catalogues opened as `Source::Mcp` in this process, each an agent of its
//! own. No window.

use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::editing::{ActRefusal, EditAction, Editors};
use crabsim_ui::workspace::Workspace;
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LINE: &str = r#"{"format_version":1,"instances":[
  {"name":"In","class":"Source","position":{"x":0,"y":0},"successors":["Out"],"attributes":{"interval":"10"}},
  {"name":"Out","class":"Drain","position":{"x":5,"y":0}}]}"#;

struct Folder(PathBuf);

impl Folder {
    /// `line.json` and `other.json`, saved by crabsim.
    fn new(name: &str) -> Folder {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("crabsim-ui-editing-{name}-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let user = Catalogue::open(Settings::new(&path, Source::Cli)).unwrap();
        for file in ["line.json", "other.json"] {
            user.call("write_model", json!({ "path": file, "text": LINE })).unwrap();
        }
        Folder(path)
    }

    fn agent(&self) -> Catalogue {
        Catalogue::open(Settings::new(&self.0, Source::Mcp)).unwrap()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn claim(agent: &Catalogue, path: &str, goal: &str, doing: &str) {
    agent.call("claim_model", json!({ "path": path, "goal": goal, "doing": doing })).unwrap();
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    workspace
}

fn editors(workspace: &Workspace) -> Editors {
    workspace.editors().unwrap()
}

/// Who may change the open model, as the tab names them, and whether an agent's claim is paused.
fn holder(workspace: &Workspace) -> (Option<String>, bool) {
    let file = editors(workspace).file("line.json");
    let paused = file.claim.as_ref().is_some_and(|claim| claim.paused);
    let who = if file.user_since.is_some() { Some("you".to_string()) } else { file.claim.map(|claim| claim.agent) };
    (who, paused)
}

fn act(workspace: &mut Workspace, action: EditAction) -> Editors {
    let shown = editors(workspace);
    workspace.act(&shown, action).unwrap_or_else(|refusal| panic!("{refusal:?}"))
}

// ---------- Seeing the claims ----------

#[test]
fn a_claim_shows_its_agent_harness_goal_and_what_it_is_doing() {
    let folder = Folder::new("see");
    let workspace = open(&folder);
    assert_eq!(holder(&workspace), (None, false), "nobody holds it yet");
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");
    claim(&agent, "line.json", "find the bottleneck", "tuning times");
    let shown = editors(&workspace);
    let claim = shown.file("line.json").claim.expect("claimed");
    assert_eq!((claim.agent.as_str(), claim.goal.as_str(), claim.doing.as_str()), ("agent-1", "find the bottleneck", "tuning times"));
    assert!(!claim.client.is_empty() && !claim.since.is_empty() && !claim.last_active.is_empty() && !claim.paused);
    let trail: Vec<&str> = claim.trail.iter().map(|(doing, _)| doing.as_str()).collect();
    assert_eq!(trail, ["tuning times", "adding a mill"], "newest first");
    assert_eq!(shown.agent_holding("line.json").map(|claim| claim.goal.as_str()), Some("find the bottleneck"), "for asking before taking it");
}

#[test]
fn other_files_held_are_listed_apart() {
    let folder = Folder::new("others");
    let workspace = open(&folder);
    // An agent whose catalogue is gone counts as ended, so it has to stay alive here.
    let agent = folder.agent();
    claim(&agent, "other.json", "tidy names", "renaming things");
    let shown = editors(&workspace);
    let others: Vec<&str> = shown.others("line.json").iter().map(|file| file.path.as_str()).collect();
    assert_eq!(others, ["other.json"]);
    assert_eq!(shown.file("line.json").claim, None);
}

// ---------- The marker on the open model ----------

#[test]
fn the_marker_says_which_agent_is_editing_the_open_model_and_what_it_is_doing() {
    let folder = Folder::new("marker");
    let mut workspace = open(&folder);
    assert_eq!(editors(&workspace).marker("line.json"), None, "nobody holds it");
    let agent = folder.agent();
    claim(&agent, "other.json", "tidy names", "renaming things");
    assert_eq!(editors(&workspace).marker("line.json"), None, "a claim on another file says nothing here");
    claim(&agent, "line.json", "find the bottleneck", "adding Mill 2 after Station 3");
    let shown = editors(&workspace);
    let client = shown.file("line.json").claim.unwrap().client;
    assert_eq!(shown.marker("line.json"), Some(format!("agent-1 ({client}) is editing this model: adding Mill 2 after Station 3")));

    act(&mut workspace, EditAction::Take);
    assert_eq!(
        editors(&workspace).marker("line.json"),
        Some(format!("agent-1 ({client}) is paused while you hold this model. It was: adding Mill 2 after Station 3")),
        "paused while the user holds it"
    );
    act(&mut workspace, EditAction::Revoke("line.json".into()));
    assert_eq!(editors(&workspace).marker("line.json"), None, "the user alone holding it needs no marker");
}

#[test]
fn the_marker_of_a_claim_that_says_nothing_about_what_it_does_ends_at_the_model() {
    let shown = Editors::from_json(&json!({ "files": [{ "path": "line.json", "claim": { "agent": "agent-4", "client": "copilot", "goal": "tidy", "doing": "" } }] }));
    assert_eq!(shown.marker("line.json").as_deref(), Some("agent-4 (copilot) is editing this model"));
}

// ---------- The user's controls ----------

#[test]
fn taking_pauses_the_agents_claim_and_releasing_gives_it_back() {
    let folder = Folder::new("take");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");
    act(&mut workspace, EditAction::Take);
    assert_eq!(holder(&workspace), (Some("you".to_string()), true), "the agent's claim is paused");
    act(&mut workspace, EditAction::Release);
    assert_eq!(holder(&workspace), (Some("agent-1".to_string()), false), "and comes back");
}

#[test]
fn revoking_ends_the_agents_claim() {
    let folder = Folder::new("revoke");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");
    act(&mut workspace, EditAction::Revoke("line.json".into()));
    assert_eq!(holder(&workspace), (None, false));
    let shown = editors(&workspace);
    assert!(matches!(workspace.act(&shown, EditAction::Revoke("line.json".into())), Err(ActRefusal::Refused(_))), "nothing left to revoke");
}

#[test]
fn blocking_and_unblocking_an_agent() {
    let folder = Folder::new("block");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");
    let after = act(&mut workspace, EditAction::Block("agent-1".into()));
    assert_eq!(after.blocked, [("agent-1".to_string(), after.blocked[0].1.clone())]);
    assert_eq!(holder(&workspace), (None, false), "a blocked agent's claim ends");
    let after = act(&mut workspace, EditAction::Unblock("agent-1".into()));
    assert!(after.blocked.is_empty());
}

#[test]
fn a_claim_on_another_file_can_be_revoked_and_its_agent_blocked_with_no_model_open() {
    let folder = Folder::new("revoke-other");
    let mut workspace = Workspace::open(&folder.0);
    let agent = folder.agent();
    claim(&agent, "other.json", "tidy names", "renaming things");
    claim(&agent, "line.json", "tidy names", "renaming things");
    let shown = editors(&workspace);
    let after = workspace.act(&shown, EditAction::Revoke("other.json".into())).unwrap();
    assert_eq!(after.file("other.json").claim, None);
    assert!(after.file("line.json").claim.is_some(), "only that file's claim ends");
    let after = workspace.act(&after, EditAction::Block("agent-1".into())).unwrap();
    assert_eq!((after.files.len(), after.blocked.len()), (0, 1));
}

#[test]
fn revoking_another_files_claim_is_refused_when_its_holder_changed_since_it_was_shown() {
    let folder = Folder::new("revoke-other-changed");
    let mut workspace = open(&folder);
    let first = folder.agent();
    claim(&first, "other.json", "tidy names", "renaming things");
    let shown = editors(&workspace);
    first.call("release_model", json!({ "path": "other.json" })).unwrap();
    let second = folder.agent();
    claim(&second, "other.json", "add a mill", "placing it");
    let refused = workspace.act(&shown, EditAction::Revoke("other.json".into())).unwrap_err();
    assert!(matches!(&refused, ActRefusal::Changed { what, .. } if what.contains("other.json") && what.contains("agent-2")), "{refused:?}");
    assert!(editors(&workspace).file("other.json").claim.is_some(), "nothing was done");
}

// ---------- Checked before acting ----------

#[test]
fn an_action_on_a_state_that_changed_since_it_was_shown_is_refused_saying_what_changed() {
    let folder = Folder::new("changed");
    let mut workspace = open(&folder);
    let shown = editors(&workspace);
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");

    let refused = workspace.act(&shown, EditAction::Take).unwrap_err();
    match &refused {
        ActRefusal::Changed { now, what } => {
            assert!(what.contains("agent-1") && what.contains("find the bottleneck"), "{what}");
            assert!(now.file("line.json").claim.is_some(), "the new state comes with it");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(holder(&workspace), (Some("agent-1".to_string()), false), "nothing was done");

    let ActRefusal::Changed { now, .. } = refused else { unreachable!() };
    workspace.act(&now, EditAction::Take).unwrap();
    assert_eq!(holder(&workspace).0.as_deref(), Some("you"), "acting on the new state goes through");
}

#[test]
fn an_agent_saying_what_it_does_now_is_no_change_to_act_on() {
    let folder = Folder::new("doing-changed");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "find the bottleneck", "adding a mill");
    let shown = editors(&workspace);
    claim(&agent, "line.json", "find the bottleneck", "tuning times");
    workspace.act(&shown, EditAction::Take).unwrap();
}

#[test]
fn a_change_to_another_file_doesnt_stop_an_action_on_the_open_one() {
    let folder = Folder::new("unrelated");
    let mut workspace = open(&folder);
    let shown = editors(&workspace);
    let agent = folder.agent();
    claim(&agent, "other.json", "tidy names", "renaming things");
    workspace.act(&shown, EditAction::Take).unwrap();
    assert_eq!(holder(&workspace).0.as_deref(), Some("you"));
}

#[test]
fn with_no_model_open_there_is_nothing_to_act_on() {
    let folder = Folder::new("none");
    let mut workspace = Workspace::open(&folder.0);
    let shown = workspace.editors().unwrap();
    assert!(matches!(workspace.act(&shown, EditAction::Take), Err(ActRefusal::Refused(_))));
}
