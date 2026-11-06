//! The reference benchmark: events per second, simulated time per wall second and peak memory.
//!
//! ```text
//! cargo bench --bench reference
//! cargo bench --bench reference -- --reps 9 --seed 3 --scenario demo-dropped
//! ```
//!
//! It runs in the bench profile, which is the tuned release profile, with no logger and no view.
//! Compare the numbers from before and after a change on the same machine with little else
//! running. If `events` or `MUs left` differ too, the change altered behaviour, not only speed.
//!
//! `cargo test --all-targets` runs this file without `--bench`, and then it only checks that the
//! reference model still matches the demo model, with no timing.

use crabsim::demo::build_model;
use crabsim::event::ScheduledEvent;
use crabsim::flows::{FailureProfile, MaterialFlow};
use crabsim::materialflow_objects::{Drain, SingleStation, Source};
use crabsim::random_numbers::Distribution;
use crabsim::simulation_run::SimulationRun;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::RefCell;
use std::hint::black_box;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

const HOUR: Duration = Duration::from_secs(3_600);
const DAY: Duration = Duration::from_secs(86_400);
const YEAR: Duration = Duration::from_secs(365 * 86_400);

/// Copies of the demo line in the large model, which gives 2,800 objects: the target scale is
/// up to a few thousand objects.
const LARGE_MODEL_LINES: usize = 200;

// ---- Memory ----

static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

/// Counts heap use around the system allocator. The benchmark runs on one thread, so plain loads
/// and stores are enough, and they add no locked instructions to what is being measured.
struct CountingAllocator;

fn add_live_bytes(bytes: usize) {
    let live = LIVE_BYTES.load(Relaxed) + bytes as u64;
    LIVE_BYTES.store(live, Relaxed);
    if live > PEAK_BYTES.load(Relaxed) {
        PEAK_BYTES.store(live, Relaxed);
    }
}

fn remove_live_bytes(bytes: usize) {
    LIVE_BYTES.store(LIVE_BYTES.load(Relaxed).saturating_sub(bytes as u64), Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.store(ALLOCATIONS.load(Relaxed) + 1, Relaxed);
        add_live_bytes(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        remove_live_bytes(layout.size());
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.store(ALLOCATIONS.load(Relaxed) + 1, Relaxed);
        // A copying realloc briefly holds both blocks, but large ones are usually remapped rather
        // than copied. Counting both overstated the demo-kept peak by half against the OS's figure.
        remove_live_bytes(layout.size());
        add_live_bytes(new_size);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// ---- The reference model ----

/// Adds one copy of the demo line to a run. This is a copy of `crabsim::demo::build_model` rather
/// than a call to it, so a change to the demo can't quietly move the baseline. The check in
/// `check_model_matches_demo` says when the two part ways, and then it's a choice which to keep.
fn add_line(run: &mut SimulationRun) {
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let station = |process_secs: u64| {
        Rc::new(RefCell::new(SingleStation::new(
            0,
            Duration::from_secs(process_secs),
            failures.clone(),
            None,
            Vec::new(),
            Vec::new(),
        )))
    };
    let st1 = station(10);
    let st2 = station(15);
    let st3 = station(20);
    let st4 = station(25);
    let st4a = station(12);
    let st4b = station(15);
    let st4c = station(18);
    let st7 = station(30);
    let st5 = station(18);
    let st5a = station(10);
    let st5b = station(14);
    let st5c = station(16);
    let source = Rc::new(RefCell::new(Source::new(0, Duration::from_secs(5), vec![st1.clone()])));
    let drain = Rc::new(RefCell::new(Drain::new(0, Vec::new())));
    let drain_as_successor: Rc<RefCell<dyn MaterialFlow>> = drain.clone();

    st1.borrow_mut().succ = vec![st2.clone()];
    st2.borrow_mut().succ = vec![st3.clone(), st5.clone()];
    st3.borrow_mut().succ = vec![st4.clone()];
    st4.borrow_mut().succ = vec![st4a.clone(), st4b.clone(), st4c.clone()];
    st4a.borrow_mut().succ = vec![st7.clone()];
    st4b.borrow_mut().succ = vec![st7.clone()];
    st4c.borrow_mut().succ = vec![st7.clone()];
    st7.borrow_mut().succ = vec![drain_as_successor.clone()];
    st5.borrow_mut().succ = vec![st5a.clone(), st5b.clone(), st5c.clone()];
    st5a.borrow_mut().succ = vec![drain_as_successor.clone()];
    st5b.borrow_mut().succ = vec![drain_as_successor.clone()];
    st5c.borrow_mut().succ = vec![drain_as_successor];

    run.add_object(source);
    for station in [st1, st2, st3, st4, st4a, st4b, st4c, st7, st5, st5a, st5b, st5c] {
        run.add_object(station);
    }
    run.add_object(drain);
}

fn reference_model(lines: usize, seed: u64) -> SimulationRun {
    let mut run = SimulationRun::new(seed);
    for _ in 0..lines {
        add_line(&mut run);
    }
    run
}

// ---- Scenarios ----

#[derive(Clone, Copy)]
enum Occurrences {
    /// One advance over the whole length, which returns every occurrence. What a run does today.
    Kept,
    /// Advances an hour at a time and drops what each returns. The nearest thing today to a
    /// headless run, though the occurrences are still recorded and the Drain still lists exits.
    Dropped,
}

struct Scenario {
    name: &'static str,
    lines: usize,
    length: Duration,
    length_label: &'static str,
    occurrences: Occurrences,
}

const SCENARIOS: [Scenario; 3] = [
    Scenario { name: "demo-kept", lines: 1, length: YEAR, length_label: "1 year", occurrences: Occurrences::Kept },
    Scenario { name: "demo-dropped", lines: 1, length: YEAR, length_label: "1 year", occurrences: Occurrences::Dropped },
    Scenario {
        name: "large-dropped",
        lines: LARGE_MODEL_LINES,
        length: Duration::from_secs(2 * 86_400),
        length_label: "2 days",
        occurrences: Occurrences::Dropped,
    },
];

/// One repetition of a scenario.
struct Measurement {
    wall: Duration,
    peak_heap_bytes: u64,
    allocations: u64,
    events: u64,
    occurrences: u64,
    mus_left: usize,
}

fn advance(run: &mut SimulationRun, scenario: &Scenario) -> u64 {
    match scenario.occurrences {
        Occurrences::Kept => {
            let occurrences = run.advance_to(scenario.length).map_err(|failure| failure.error).unwrap();
            black_box(&occurrences);
            let count = occurrences.len() as u64;
            // Part of the cost is giving that memory back, and it would be paid by any caller.
            drop(occurrences);
            count
        }
        Occurrences::Dropped => {
            let mut count = 0;
            let mut until = HOUR;
            loop {
                let occurrences = run.advance_to(until).map_err(|failure| failure.error).unwrap();
                black_box(&occurrences);
                count += occurrences.len() as u64;
                if until >= scenario.length {
                    return count;
                }
                until = (until + HOUR).min(scenario.length);
            }
        }
    }
}

fn measure(scenario: &Scenario, seed: u64) -> Measurement {
    let bytes_before_model = LIVE_BYTES.load(Relaxed);
    PEAK_BYTES.store(bytes_before_model, Relaxed);

    let mut run = reference_model(scenario.lines, seed);
    run.init().map_err(|error| format!("{error:?}")).unwrap();

    let allocations_before = ALLOCATIONS.load(Relaxed);
    let start = Instant::now();
    let occurrences = advance(&mut run, scenario);
    let wall = start.elapsed();
    let allocations = ALLOCATIONS.load(Relaxed) - allocations_before;
    let peak_heap_bytes = PEAK_BYTES.load(Relaxed) - bytes_before_model;

    let mus_left = run.state().objects.iter().map(|object| object.mus_left).sum();
    Measurement { wall, peak_heap_bytes, allocations, events: run.events_processed(), occurrences, mus_left }
}

fn run_scenario(scenario: &Scenario, seed: u64, reps: usize) {
    let mut measurements: Vec<Measurement> = (0..reps).map(|_| measure(scenario, seed)).collect();
    for later in &measurements[1..] {
        assert_eq!(
            (later.events, later.occurrences, later.mus_left),
            (measurements[0].events, measurements[0].occurrences, measurements[0].mus_left),
            "the same seed must give the same result",
        );
    }
    measurements.sort_by_key(|measurement| measurement.wall);
    let fastest = measurements[0].wall.as_secs_f64();
    let slowest = measurements[reps - 1].wall.as_secs_f64();
    let median = &measurements[reps / 2];
    let wall = median.wall.as_secs_f64();
    let events = median.events as f64;

    println!(
        "{:<14} {:>7} {:>6} {:>7.2}M/s {:>9.2}M {:>7.2}s {:>6.2}..{:<6.2} {:>8.2} {:>6.2} {:>10.1}MB {:>9} {:>11}",
        scenario.name,
        scenario.length_label,
        scenario.lines * 14,
        events / wall / 1e6,
        scenario.length.as_secs_f64() / wall / 1e6,
        wall,
        fastest,
        slowest,
        median.allocations as f64 / events,
        median.occurrences as f64 / events,
        median.peak_heap_bytes as f64 / 1e6,
        median.mus_left,
        median.events,
    );
}

/// The reference model must stay the demo model unless someone decides otherwise.
fn check_model_matches_demo() {
    for seed in 0..3 {
        let (mut demo, _) = build_model(seed);
        let mut reference = reference_model(1, seed);
        assert_eq!(demo.init().unwrap(), reference.init().unwrap());
        let demo_occurrences = demo.advance_to(DAY).map_err(|failure| failure.error).unwrap();
        let reference_occurrences = reference.advance_to(DAY).map_err(|failure| failure.error).unwrap();
        assert_eq!(demo_occurrences, reference_occurrences, "the reference model has parted from the demo model");
        assert_eq!(demo.state(), reference.state());
    }
    println!("reference model matches the demo model");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|arg| arg == "--bench") {
        check_model_matches_demo();
        return;
    }

    let mut reps = 5;
    let mut seed = 0;
    let mut chosen: Vec<&Scenario> = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bench" => {}
            "--reps" => reps = args.next().and_then(|value| value.parse().ok()).expect("--reps takes a number"),
            "--seed" => seed = args.next().and_then(|value| value.parse().ok()).expect("--seed takes a number"),
            "--scenario" => {
                let name = args.next().expect("--scenario takes a name");
                chosen.push(
                    SCENARIOS
                        .iter()
                        .find(|scenario| scenario.name == name)
                        .unwrap_or_else(|| panic!("unknown scenario {name}")),
                );
            }
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(reps > 0, "--reps must be at least 1");
    if chosen.is_empty() {
        chosen = SCENARIOS.iter().collect();
    }

    println!("seed {seed}, {reps} repetitions, wall time and rates are the median; simulated time per wall second is a ratio");
    println!("event queue entry: {} bytes", size_of::<ScheduledEvent>());
    println!(
        "{:<14} {:>7} {:>6} {:>10} {:>10} {:>8} {:>14} {:>8} {:>6} {:>12} {:>9} {:>11}",
        "scenario", "length", "objs", "events/s", "sim/wall", "wall", "fastest..slow", "alloc/ev", "occ/ev", "peak heap", "MUs left", "events"
    );
    for scenario in chosen {
        run_scenario(scenario, seed, reps);
    }
}
