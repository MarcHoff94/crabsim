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
//! reference model still matches the demo model and that a headless run's memory stays flat.

use crabsim::demo::build_model;
use crabsim::event::ScheduledEvent;
use crabsim::flows::FailureProfile;
use crabsim::model::ModelDescription;
use crabsim::report::{ObjectKind, Recording};
use crabsim::random_numbers::Distribution;
use crabsim::simulation_run::SimulationRun;
use crabsim::time::SimTime;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

const HOUR: SimTime = SimTime::from_secs(3_600);
const DAY: SimTime = SimTime::from_secs(86_400);
const YEAR: SimTime = SimTime::from_secs(365 * 86_400);

/// Copies of the demo line in the large model, which gives 6,006 objects: the target scale is
/// up to 6,000 objects.
const LARGE_MODEL_LINES: usize = 429;

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

/// Adds one copy of the demo line to a model. This is a copy of `crabsim::demo::build_model` rather
/// than a call to it, so a change to the demo can't quietly move the baseline. The check in
/// `check_model_matches_demo` says when the two part ways, and then it's a choice which to keep.
fn add_line(model: &mut ModelDescription) {
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let source = model.add_source(SimTime::from_secs(5));
    let mut station = |process_secs: u64| model.add_station(SimTime::from_secs(process_secs), failures.clone());
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
    let drain = model.add_drain();

    for (from, to) in [
        (source, st1),
        (st1, st2),
        (st2, st3),
        (st2, st5),
        (st3, st4),
        (st4, st4a),
        (st4, st4b),
        (st4, st4c),
        (st4a, st7),
        (st4b, st7),
        (st4c, st7),
        (st7, drain),
        (st5, st5a),
        (st5, st5b),
        (st5, st5c),
        (st5a, drain),
        (st5b, drain),
        (st5c, drain),
    ] {
        model.connect(from, to);
    }
}

fn reference_model(lines: usize, seed: u64) -> SimulationRun {
    let mut model = ModelDescription::new();
    for _ in 0..lines {
        add_line(&mut model);
    }
    model.build(seed)
}

// ---- Scenarios ----

#[derive(Clone, Copy)]
enum Occurrences {
    /// One advance over the whole length, recording and returning every occurrence.
    Kept,
    /// Advances an hour at a time, recording every occurrence and dropping what each returns.
    Dropped,
    /// A headless run: one advance over the whole length, recording nothing.
    Headless,
}

struct Scenario {
    name: &'static str,
    lines: usize,
    length: SimTime,
    length_label: &'static str,
    occurrences: Occurrences,
}

const SCENARIOS: [Scenario; 6] = [
    Scenario { name: "demo-headless", lines: 1, length: YEAR, length_label: "1 year", occurrences: Occurrences::Headless },
    // The same run over ten times the length: peak memory should stay where it is.
    Scenario { name: "demo-headless-10y", lines: 1, length: YEAR.times(10), length_label: "10 years", occurrences: Occurrences::Headless },
    Scenario {
        name: "large-headless",
        lines: LARGE_MODEL_LINES,
        length: SimTime::from_secs(2 * 86_400),
        length_label: "2 days",
        occurrences: Occurrences::Headless,
    },
    Scenario { name: "demo-kept", lines: 1, length: YEAR, length_label: "1 year", occurrences: Occurrences::Kept },
    Scenario { name: "demo-dropped", lines: 1, length: YEAR, length_label: "1 year", occurrences: Occurrences::Dropped },
    Scenario {
        name: "large-dropped",
        lines: LARGE_MODEL_LINES,
        length: SimTime::from_secs(2 * 86_400),
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
    mus_left: u64,
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
        Occurrences::Headless => {
            run.advance_to(scenario.length).map_err(|failure| failure.error).unwrap();
            0
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
    if !matches!(scenario.occurrences, Occurrences::Headless) {
        run.set_recording(Recording::ALL);
    }
    run.init().map_err(|error| format!("{error:?}")).unwrap();

    let allocations_before = ALLOCATIONS.load(Relaxed);
    let start = Instant::now();
    let occurrences = advance(&mut run, scenario);
    let wall = start.elapsed();
    let allocations = ALLOCATIONS.load(Relaxed) - allocations_before;
    let peak_heap_bytes = PEAK_BYTES.load(Relaxed) - bytes_before_model;

    let drains = run.state().objects.into_iter().filter(|object| object.kind == ObjectKind::Drain);
    let mus_left = drains.map(|drain| drain.statistics.mus_in).sum();
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

/// Peak heap of a headless run of the demo model, from before it is built until `length`.
fn headless_peak_heap(length: SimTime) -> u64 {
    let bytes_before_model = LIVE_BYTES.load(Relaxed);
    PEAK_BYTES.store(bytes_before_model, Relaxed);
    let mut run = reference_model(1, 0);
    run.init().map_err(|error| format!("{error:?}")).unwrap();
    run.advance_to(length).map_err(|failure| failure.error).unwrap();
    black_box(&run);
    PEAK_BYTES.load(Relaxed) - bytes_before_model
}

/// Memory stays flat as simulated length grows, unless the user asked for recording. Only the
/// event queue and the blocked lists can grow at all, and the model limits both.
fn check_memory_stays_flat() {
    let short = headless_peak_heap(DAY.times(5));
    let long = headless_peak_heap(DAY.times(20));
    assert!(long <= short + short / 10, "a headless run grew from {short} to {long} bytes over four times the length");
    println!("a headless run's memory stays flat: {short} bytes over 5 days, {long} over 20");
}

/// The reference model must stay the demo model unless someone decides otherwise.
fn check_model_matches_demo() {
    for seed in 0..3 {
        let mut demo = build_model(seed);
        let mut reference = reference_model(1, seed);
        demo.set_recording(Recording::ALL);
        reference.set_recording(Recording::ALL);
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
        check_memory_stays_flat();
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
