use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

use crate::event::EcTime;
use serde::{Deserialize, Serialize};
use crate::materialflow_objects::flows::{Drain, EventInterface, MaterialFlow, ProcessState, SingleStation, Source};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateRow {
    pub id: u32,
    pub typ: String,
    pub content: String,
    pub state: String,
    pub rem_time: u128,
    pub blocked: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateSnapshot {
    // Use primitive time for easy cross-process serialization
    pub current_time: u128,
    pub current_seq: u64,
    pub rows: Vec<StateRow>,
}

pub struct StateVisualizer;

impl StateVisualizer {
    pub fn collect_snapshot(
        material_flows: &HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
        current_time: EcTime,
        current_seq: u64,
    ) -> StateSnapshot {
        let mut ids: Vec<u32> = material_flows.keys().cloned().collect();
        ids.sort_unstable();
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(obj_rc) = material_flows.get(&id) {
                let obj_borrow = obj_rc.borrow();
                let any: &dyn std::any::Any =
                    <dyn MaterialFlow as EventInterface>::as_any(&*obj_borrow);
                if let Some(st) = any.downcast_ref::<SingleStation>() {
                    let content = match &st.content { Some(mu) => format!("MU{}", mu.id), None => "-".to_string() };
                    let state = match st.state { ProcessState::Processing => "Process", ProcessState::Blocked => "Blocked", ProcessState::Idle => "Idle", };
                    let blocked = if st.blockedlist.is_empty() { "-".to_string() } else { st.blockedlist.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",") };
                    rows.push(StateRow { id, typ: "Station".into(), content, state: state.into(), rem_time: st.remain_proc_time.0, blocked });
                    continue;
                }
                if let Some(src) = any.downcast_ref::<Source>() {
                    let content = match &src.content { Some(mu) => format!("MU{}", mu.id), None => "-".to_string() };
                    let state = match src.state { ProcessState::Processing => "Process", ProcessState::Blocked => "Blocked", ProcessState::Idle => "Idle", };
                    rows.push(StateRow { id, typ: "Source".into(), content, state: state.into(), rem_time: src.remain_proc_time.0, blocked: "-".into() });
                    continue;
                }
                if let Some(dr) = any.downcast_ref::<Drain>() {
                    let content = match &dr.content { Some(mu) => format!("MU{}", mu.id), None => "-".to_string() };
                    let blocked = if dr.blockedlist.is_empty() { "-".to_string() } else { dr.blockedlist.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",") };
                    rows.push(StateRow { id, typ: "Drain".into(), content, state: "-".into(), rem_time: 0, blocked });
                    continue;
                }
                rows.push(StateRow { id, typ: "Unknown".into(), content: "-".into(), state: "-".into(), rem_time: 0, blocked: "-".into() });
            }
        }
        StateSnapshot { current_time: current_time.0, current_seq, rows }
    }

    pub fn print_snapshot(snapshot: &StateSnapshot) {
        let time_str = format!("State at time {} (seq={})", snapshot.current_time, snapshot.current_seq);
        let padding = 80_usize.saturating_sub(time_str.len());
        println!("╔═════════════════════════════════════════════════════════════════════════════════╗");
        println!("║ {}{}║", time_str, " ".repeat(padding));
        println!("╚═════════════════════════════════════════════════════════════════════════════════╝");

        println!("\n┌─────┬──────────┬──────────┬────────────┬──────────────┬─────────────────┐");
        println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │", "ID", "Type", "Content", "State", "RemTime", "Blocked");
        println!("├─────┼──────────┼──────────┼────────────┼──────────────┼─────────────────┤");
        for r in &snapshot.rows {
            println!(
                "│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                r.id, r.typ, r.content, r.state, r.rem_time, r.blocked
            );
        }
        println!("└─────┴──────────┴──────────┴────────────┴──────────────┴─────────────────┘\n");
    }

    pub fn print_snapshot_with_topology(snapshot: &StateSnapshot, topology_text: &str) {
        let time_str = format!("State at time {} (seq={})", snapshot.current_time, snapshot.current_seq);
        let padding = 80_usize.saturating_sub(time_str.len());
        println!("╔═════════════════════════════════════════════════════════════════════════════════╗");
        println!("║ {}{}║", time_str, " ".repeat(padding));
        println!("╚═════════════════════════════════════════════════════════════════════════════════╝");

        // Topology snapshot printed once per frame, static content
        if !topology_text.is_empty() {
            println!("{}", topology_text);
        }

        println!("\n┌─────┬──────────┬──────────┬────────────┬──────────────┬─────────────────┐");
        println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │", "ID", "Type", "Content", "State", "RemTime", "Blocked");
        println!("├─────┼──────────┼──────────┼────────────┼──────────────┼─────────────────┤");
        for r in &snapshot.rows {
            println!(
                "│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                r.id, r.typ, r.content, r.state, r.rem_time, r.blocked
            );
        }
        println!("└─────┴──────────┴──────────┴────────────┴──────────────┴─────────────────┘\n");
    }
    pub fn print_materialflow_state(
        material_flows: &HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
        current_time: EcTime,
        current_seq: u64,
    ) {
        let snapshot = Self::collect_snapshot(material_flows, current_time, current_seq);
        Self::print_snapshot(&snapshot);
    }
}
