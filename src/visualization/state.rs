use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
};

use crate::event::EcTime;
use crate::materialflow_objects::flows::{Drain, EventInterface, MaterialFlow, ProcessState, SingleStation, Source};

pub struct StateVisualizer;

impl StateVisualizer {
    pub fn print_materialflow_state(
        material_flows: &HashMap<u32, Rc<RefCell<dyn MaterialFlow>>>,
        current_time: EcTime,
        current_seq: u64,
    ) {
        let time_str = format!("State at time {} (seq={})", current_time.0, current_seq);
        let padding = 80_usize.saturating_sub(time_str.len());
        println!(
            "\n╔═════════════════════════════════════════════════════════════════════════════════╗"
        );
        println!(
            "║ {}{}║",
            time_str,
            " ".repeat(padding)
        );
        println!(
            "╚═════════════════════════════════════════════════════════════════════════════════╝"
        );

        let mut ids: Vec<u32> = material_flows.keys().cloned().collect();
        ids.sort_unstable();

        // Print current state table
        println!("\n┌─────┬──────────┬──────────┬────────────┬──────────────┬─────────────────┐");
        println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │", 
                 "ID", "Type", "Content", "State", "RemTime", "Blocked");
        println!("├─────┼──────────┼──────────┼────────────┼──────────────┼─────────────────┤");
        
        for id in ids {
            if let Some(obj_rc) = material_flows.get(&id) {
                let obj_borrow = obj_rc.borrow();
                let any: &dyn std::any::Any =
                    <dyn MaterialFlow as EventInterface>::as_any(&*obj_borrow);
                
                if let Some(st) = any.downcast_ref::<SingleStation>() {
                    let content = match &st.content {
                        Some(mu) => format!("MU{}", mu.id),
                        None => "-".to_string(),
                    };
                    let state = match st.state {
                        ProcessState::Processing => "Process",
                        ProcessState::Blocked => "Blocked",
                        ProcessState::Idle => "Idle",
                    };
                    let blocked = if st.blockedlist.is_empty() {
                        "-".to_string()
                    } else {
                        st.blockedlist.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
                    };
                    println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                        id, "Station", content, state, st.remain_proc_time.0, blocked);
                    continue;
                }
                if let Some(src) = any.downcast_ref::<Source>() {
                    let content = match &src.content {
                        Some(mu) => format!("MU{}", mu.id),
                        None => "-".to_string(),
                    };
                    let state = match src.state {
                        ProcessState::Processing => "Process",
                        ProcessState::Blocked => "Blocked",
                        ProcessState::Idle => "Idle",
                    };
                    println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                        id, "Source", content, state, src.remain_proc_time.0, "-");
                    continue;
                }
                if let Some(dr) = any.downcast_ref::<Drain>() {
                    let content = match &dr.content {
                        Some(mu) => format!("MU{}", mu.id),
                        None => "-".to_string(),
                    };
                    let blocked = if dr.blockedlist.is_empty() {
                        "-".to_string()
                    } else {
                        dr.blockedlist.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
                    };
                    println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                        id, "Drain", content, "-", "-", blocked);
                    continue;
                }
                println!("│ {:3} │ {:8} │ {:8} │ {:10} │ {:12} │ {:15} │",
                    id, "Unknown", "-", "-", "-", "-");
            }
        }
        println!("└─────┴──────────┴──────────┴────────────┴──────────────┴─────────────────┘\n");
    }
}
