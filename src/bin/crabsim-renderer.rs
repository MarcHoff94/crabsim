use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};
use std::sync::mpsc::{self, Sender, Receiver};

use crabsim::visualization::state::{StateSnapshot, StateVisualizer};
use serde_json::Value;
// use crabsim::visualization::topology::TopologyVisualizer;

#[cfg(windows)]
fn enable_ansi_support() {
    #[allow(non_snake_case)]
    unsafe fn SetVT() {
        extern "system" {
            fn GetStdHandle(nStdHandle: i32) -> *mut core::ffi::c_void;
            fn GetConsoleMode(hConsoleHandle: *mut core::ffi::c_void, lpMode: *mut u32) -> i32;
            fn SetConsoleMode(hConsoleHandle: *mut core::ffi::c_void, dwMode: u32) -> i32;
        }
        const STD_OUTPUT_HANDLE: i32 = -11; // from winbase.h
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if !h.is_null() {
            let mut mode: u32 = 0;
            if GetConsoleMode(h, &mut mode) != 0 {
                let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
    unsafe { SetVT(); }
}

#[cfg(not(windows))]
fn enable_ansi_support() {}

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

    enable_ansi_support();

    // Reader thread: parse input lines and push to channel
    let (tx, rx): (Sender<Value>, Receiver<Value>) = mpsc::channel();
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
    });

    // Renderer state
    let mut topology_text: Option<String> = None;
    use std::collections::BTreeMap;
    let mut timeline: BTreeMap<u128, StateSnapshot> = BTreeMap::new();
    let mut current_time: u128 = 0;
    let mut latest_seq: u64 = 0;

    // Main render loop: 1 Hz frame rate; advance current_time by 1 each second
    let mut last_tick = Instant::now();
    let mut idle_final_ticks: u32 = 0; // count ticks at final time without new data
    let hold_open: bool = std::env::var("CRABSIM_RENDERER_HOLD").map(|v| v != "0").unwrap_or(true);
    loop {
        // Drain any incoming messages
        let mut got_msg = false;
        while let Ok(msg) = rx.try_recv() {
            got_msg = true;
            let kind = msg.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            if kind == "topology" {
                if let Some(t) = msg.get("text").and_then(|t| t.as_str()) {
                    topology_text = Some(t.to_string());
                }
                continue;
            }
            if kind == "snapshot" {
                if let Ok(snap) = serde_json::from_value::<StateSnapshot>(msg.get("snapshot").cloned().unwrap_or(Value::Null)) {
                    // Store snapshot keyed by its sim time
                    latest_seq = snap.current_seq;
                    if timeline.is_empty() { current_time = snap.current_time; }
                    timeline.insert(snap.current_time, snap);
                }
                continue;
            }
        }

        // Tick every 1 second of wall time
        let now = Instant::now();
        // Advance simulation time by 1 at a rate of 1/speed seconds wall time
        let per_tick = Duration::from_secs_f64(1.0 / speed);
        if now.duration_since(last_tick) >= per_tick {
            last_tick = now;
            // Advance simulation time by exactly 1 per tick once we have a starting point
            if !timeline.is_empty() {
                // Clamp to final snapshot time
                let final_time = *timeline.keys().next_back().unwrap_or(&current_time);
                if current_time < final_time { current_time = current_time.saturating_add(1); }
                if current_time > final_time { current_time = final_time; }

                // Track idle ticks after reaching final time; if we are at final time
                // and didn't receive any new messages this tick, increment counter.
                if current_time == final_time {
                    if got_msg { idle_final_ticks = 0; } else { idle_final_ticks = idle_final_ticks.saturating_add(1); }
                } else {
                    idle_final_ticks = 0;
                }
            }

            // Clear screen and draw current frame
            print!("\x1B[2J\x1B[H");
            // Choose the latest snapshot at or before current_time
            if let Some((_, snap)) = timeline.range(..=current_time).next_back() {
                let mut current = snap.clone();
                // Interpolate remaining time for Processing rows based on elapsed sim seconds since snapshot
                let elapsed = current_time.saturating_sub(snap.current_time);
                if elapsed > 0 {
                    for r in current.rows.iter_mut() {
                        if r.state == "Process" {
                            r.rem_time = r.rem_time.saturating_sub(elapsed);
                        }
                    }
                }
                current.current_time = current_time;
                if let Some(ref topo) = topology_text {
                    StateVisualizer::print_snapshot_with_topology(&current, topo);
                } else {
                    StateVisualizer::print_snapshot(&current);
                }
            } else {
                // No snapshot yet; display an empty frame with time 0
                let empty = StateSnapshot { current_time, current_seq: latest_seq, rows: Vec::new() };
                if let Some(ref topo) = topology_text {
                    StateVisualizer::print_snapshot_with_topology(&empty, topo);
                } else {
                    StateVisualizer::print_snapshot(&empty);
                }
            }
            let _ = std::io::Write::flush(&mut std::io::stdout());
        }
        // Sleep a bit to avoid busy loop
        thread::sleep(Duration::from_millis(50));

        // If we've been idle at final time for a few seconds (e.g., 3 seconds), stop rendering.
        if idle_final_ticks >= 60 { // 60 * 50ms sleeps ~= 3 seconds
            break;
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
