use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::thread;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::sync::mpsc::{self, Sender, Receiver};

use crabsim::visualization::state::{StateSnapshot, StateVisualizer};
use serde_json::Value;
// use crabsim::visualization::topology::TopologyVisualizer;


fn main() {
    let mut args = std::env::args().skip(1);
    let mut connect = String::new();
    let mut speed: f64 = 5.0; // default 5x pacing

    while let Some(a) = args.next() {
        match a.as_str() {
            "--connect" => { if let Some(v) = args.next() { connect = v; } },
            "--speed" => { if let Some(v) = args.next() { speed = v.parse().unwrap_or(5.0); } },
            _ => {}
        }
    }

    if connect.is_empty() {
        eprintln!("renderer: missing --connect <host:port>");
        std::process::exit(1);
    }

    let stream = TcpStream::connect(&connect).expect("renderer: failed to connect to simulator");
    let reader = BufReader::new(stream);



    // Reader thread: parse input lines and push to channel
    let (tx, rx): (Sender<Value>, Receiver<Value>) = mpsc::channel();
    let stream_closed = Arc::new(AtomicBool::new(false));
    let stream_closed_worker = stream_closed.clone();
    thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(text) => {
                    if text.is_empty() { continue; }
                    if let Ok(v) = serde_json::from_str::<Value>(&text) { let _ = tx.send(v); }
                }
                Err(_e) => { break; }
            }
        }
        stream_closed_worker.store(true, Ordering::Relaxed);
    });

    // Renderer state
    let mut topology_text: Option<String> = None;
    use std::collections::BTreeMap;
    let mut timeline: BTreeMap<u128, StateSnapshot> = BTreeMap::new();
    let mut current_time: u128 = 0;
    let mut latest_seq: u64 = 0;

    // Main render loop: paced frame rate; advance current_time by step_ms per tick
    let mut last_tick = Instant::now();
    let hold_open: bool = std::env::var("CRABSIM_RENDERER_HOLD").map(|v| v != "0").unwrap_or(true);
    let step_ms: u128 = 1000; // advance 1 simulated second per tick
    loop {
        // Drain any incoming messages
        while let Ok(msg) = rx.try_recv() {
            let kind = msg.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind == "topology" {
                if let Some(t) = msg.get("text").and_then(|t| t.as_str()) {
                    topology_text = Some(t.to_string());
                }
                continue;
            }
            if kind == "snapshot" {
                if let Ok(snap) = serde_json::from_value::<StateSnapshot>(msg.get("snapshot").cloned().unwrap_or(Value::Null)) {
                    latest_seq = snap.current_seq;
                    if timeline.is_empty() { current_time = snap.current_time; }
                    timeline.insert(snap.current_time, snap);
                }
                continue;
            }
        }

        // Determine pacing behavior: paced vs. unpaced (-1 => fast-forward)
        let mut should_render = false;
        if speed < 0.0 {
            if !timeline.is_empty() {
                let final_time = *timeline.keys().next_back().unwrap_or(&current_time);
                if current_time < final_time { current_time = current_time.saturating_add(step_ms); }
                if current_time > final_time { current_time = final_time; }
                should_render = true;
            }
        } else {
            let now = Instant::now();
            let per_tick = Duration::from_secs_f64(1.0 / speed);
            if now.duration_since(last_tick) >= per_tick {
                last_tick = now;
                if !timeline.is_empty() {
                    let final_time = *timeline.keys().next_back().unwrap_or(&current_time);
                    if current_time < final_time { current_time = current_time.saturating_add(step_ms); }
                    if current_time > final_time { current_time = final_time; }
                }
                should_render = true;
            }
        }

        if should_render {
            print!("\x1B[2J\x1B[H");
            if let Some((_, snap)) = timeline.range(..=current_time).next_back() {
                let mut current = snap.clone();
                let elapsed = current_time.saturating_sub(snap.current_time);
                if elapsed > 0 {
                    for r in current.rows.iter_mut() {
                        if r.state == "Process" && r.interruption == "-" {
                            r.rem_time = r.rem_time.saturating_sub(elapsed);
                        }
                    }
                }
                current.current_time = current_time;
                if let Some(ref topo) = topology_text { StateVisualizer::print_snapshot_with_topology(&current, topo); }
                else { StateVisualizer::print_snapshot(&current); }
            } else {
                let empty = StateSnapshot { current_time, current_seq: latest_seq, rows: Vec::new() };
                if let Some(ref topo) = topology_text { StateVisualizer::print_snapshot_with_topology(&empty, topo); }
                else { StateVisualizer::print_snapshot(&empty); }
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());

            if !timeline.is_empty() {
                let final_time = *timeline.keys().next_back().unwrap_or(&current_time);
                // Only stop at final time once the input stream is closed.
                if current_time == final_time && stream_closed.load(Ordering::Relaxed) { break; }
            }
        }
    }

    // Graceful finish: keep the window open until user presses Enter (default),
    // controlled by CRABSIM_RENDERER_HOLD (set to 0 to disable).
    if hold_open {
        println!("\nSimulation finished. Press Enter to close the renderer...");
        use std::io::{stdin, Read};
        let _ = stdin().read(&mut [0u8]); // waits for any key/Enter
    }
}
