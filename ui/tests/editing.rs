//! The Editing tab's logic: the edit queue as the user sees it, every control of the user's, each
//! checked against the latest queue before it acts and read again after. Agents are played by
//! catalogues opened as `Source::Mcp` in this process, each an agent of its own. No window.

use crabsim_catalogue::{Catalogue, Settings, Source};
use crabsim_ui::editing::{ActRefusal, Holder, Queue, QueueAction};
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

fn claim(agent: &Catalogue, path: &str, purpose: &str) {
    agent.call("claim_model", json!({ "path": path, "purpose": purpose })).unwrap();
}

fn open(folder: &Folder) -> Workspace {
    let mut workspace = Workspace::open(&folder.0);
    assert!(workspace.open_model("line.json"));
    workspace
}

fn queue(workspace: &Workspace) -> Queue {
    workspace.edit_queue().unwrap()
}

/// Who holds the right on the open model, as the tab names them.
fn holder(workspace: &Workspace) -> Option<String> {
    queue(workspace).file("line.json").editor.map(|holder| match holder {
        Holder::User { .. } => "you".to_string(),
        Holder::Agent(agent) => agent.agent,
    })
}

fn waiting(workspace: &Workspace) -> Vec<String> {
    queue(workspace).file("line.json").queue.into_iter().map(|agent| agent.agent).collect()
}

fn act(workspace: &mut Workspace, action: QueueAction) -> Queue {
    let shown = queue(workspace);
    workspace.act(&shown, action).unwrap_or_else(|refusal| panic!("{refusal:?}"))
}

// ---------- Seeing the queue ----------

#[test]
fn the_queue_shows_who_edits_and_who_waits_with_harness_and_purpose() {
    let folder = Folder::new("see");
    let workspace = open(&folder);
    assert_eq!(holder(&workspace), None, "nobody edits yet");
    let (first, second) = (folder.agent(), folder.agent());
    claim(&first, "line.json", "adding a mill");
    claim(&second, "line.json", "tuning times");
    let shown = queue(&workspace);
    let file = shown.file("line.json");
    match file.editor {
        Some(Holder::Agent(agent)) => {
            assert_eq!((agent.agent.as_str(), agent.purpose.as_str()), ("agent-1", "adding a mill"));
            assert!(!agent.client.is_empty() && !agent.since.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(waiting(&workspace), ["agent-2"]);
    assert_eq!(shown.agent_holding("line.json").map(|agent| agent.purpose.as_str()), Some("adding a mill"), "for asking before taking the right");
}

#[test]
fn other_files_with_an_editor_or_a_queue_are_listed_apart() {
    let folder = Folder::new("others");
    let workspace = open(&folder);
    // An agent whose catalogue is gone counts as ended, so it has to stay alive here.
    let agent = folder.agent();
    claim(&agent, "other.json", "renaming things");
    let shown = queue(&workspace);
    let others: Vec<&str> = shown.others("line.json").iter().map(|file| file.path.as_str()).collect();
    assert_eq!(others, ["other.json"]);
    assert_eq!(shown.file("line.json").editor, None);
}

// ---------- The user's controls ----------

#[test]
fn taking_the_right_puts_the_agent_first_in_line_and_releasing_gives_it_back() {
    let folder = Folder::new("take");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "adding a mill");
    act(&mut workspace, QueueAction::Take);
    assert_eq!(holder(&workspace).as_deref(), Some("you"));
    assert_eq!(waiting(&workspace), ["agent-1"], "the agent interrupted waits first in line");
    act(&mut workspace, QueueAction::Release);
    assert_eq!(holder(&workspace).as_deref(), Some("agent-1"), "and gets its right back");
}

#[test]
fn giving_moving_and_removing_waiting_agents() {
    let folder = Folder::new("queue");
    let mut workspace = open(&folder);
    let agents = [folder.agent(), folder.agent(), folder.agent(), folder.agent()];
    for (agent, purpose) in agents.iter().zip(["a", "b", "c", "d"]) {
        claim(agent, "line.json", purpose);
    }
    assert_eq!(waiting(&workspace), ["agent-2", "agent-3", "agent-4"]);
    act(&mut workspace, QueueAction::Down("agent-2".into()));
    assert_eq!(waiting(&workspace), ["agent-3", "agent-2", "agent-4"]);
    act(&mut workspace, QueueAction::Up("agent-4".into()));
    assert_eq!(waiting(&workspace), ["agent-3", "agent-4", "agent-2"]);
    act(&mut workspace, QueueAction::Remove("agent-4".into()));
    assert_eq!(waiting(&workspace), ["agent-3", "agent-2"]);
    act(&mut workspace, QueueAction::Give("agent-2".into()));
    assert_eq!(holder(&workspace).as_deref(), Some("agent-2"));

    let shown = queue(&workspace);
    let refused = workspace.act(&shown, QueueAction::Up("agent-3".into())).unwrap_err();
    assert!(matches!(&refused, ActRefusal::Refused(why) if why.contains("first in line")), "the first can't move up: {refused:?}");
}

#[test]
fn blocking_and_unblocking_an_agent() {
    let folder = Folder::new("block");
    let mut workspace = open(&folder);
    let agent = folder.agent();
    claim(&agent, "line.json", "adding a mill");
    let after = act(&mut workspace, QueueAction::Block("agent-1".into()));
    assert_eq!(after.blocked, [("agent-1".to_string(), after.blocked[0].1.clone())]);
    assert_eq!(holder(&workspace), None, "a blocked agent loses its right");
    let after = act(&mut workspace, QueueAction::Unblock("agent-1".into()));
    assert!(after.blocked.is_empty());
}

// ---------- Checked before acting ----------

#[test]
fn an_action_on_a_queue_that_changed_since_it_was_shown_is_refused_saying_what_changed() {
    let folder = Folder::new("changed");
    let mut workspace = open(&folder);
    let shown = queue(&workspace);
    let agent = folder.agent();
    claim(&agent, "line.json", "adding a mill");

    let refused = workspace.act(&shown, QueueAction::Take).unwrap_err();
    match &refused {
        ActRefusal::Changed { now, what } => {
            assert!(what.contains("agent-1") && what.contains("adding a mill"), "{what}");
            assert!(matches!(now.file("line.json").editor, Some(Holder::Agent(_))), "the new state comes with it");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(holder(&workspace).as_deref(), Some("agent-1"), "nothing was done");

    let ActRefusal::Changed { now, .. } = refused else { unreachable!() };
    workspace.act(&now, QueueAction::Take).unwrap();
    assert_eq!(holder(&workspace).as_deref(), Some("you"), "acting on the new state goes through");
}

#[test]
fn a_change_to_who_waits_is_a_change_too() {
    let folder = Folder::new("waiting-changed");
    let mut workspace = open(&folder);
    let (first, second) = (folder.agent(), folder.agent());
    claim(&first, "line.json", "adding a mill");
    let shown = queue(&workspace);
    claim(&second, "line.json", "tuning times");
    match workspace.act(&shown, QueueAction::Give("agent-1".into())).unwrap_err() {
        ActRefusal::Changed { what, .. } => assert!(what.contains("agent-2"), "{what}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_change_to_another_file_doesnt_stop_an_action_on_the_open_one() {
    let folder = Folder::new("unrelated");
    let mut workspace = open(&folder);
    let shown = queue(&workspace);
    let agent = folder.agent();
    claim(&agent, "other.json", "renaming things");
    workspace.act(&shown, QueueAction::Take).unwrap();
    assert_eq!(holder(&workspace).as_deref(), Some("you"));
}

#[test]
fn with_no_model_open_there_is_no_queue_to_act_on() {
    let folder = Folder::new("none");
    let mut workspace = Workspace::open(&folder.0);
    let shown = workspace.edit_queue().unwrap();
    assert!(matches!(workspace.act(&shown, QueueAction::Take), Err(ActRefusal::Refused(_))));
}
