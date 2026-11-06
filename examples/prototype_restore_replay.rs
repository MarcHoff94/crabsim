//! PROTOTYPE, THROWAWAY: step 5 of the build order. Not for master.
//!
//! Tests the mechanism in ADR 0002 against the real run, and gets real cost figures for it:
//! a checkpoint is a clone of the run, and going back is restoring the nearest earlier checkpoint
//! and replaying to the instant wanted with recording off.
//!
//! cargo run --release --example prototype_restore_replay

use crabsim::random_numbers::Distribution;
use crabsim::{EventType, FailureProfile, ModelDescription, Occurrence, Recording, RunState, SimTime, SimulationRun};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

static LIVE: AtomicU64 = AtomicU64::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.store(LIVE.load(Relaxed) + layout.size() as u64, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.store(LIVE.load(Relaxed).saturating_sub(layout.size() as u64), Relaxed);
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.store(LIVE.load(Relaxed).saturating_sub(layout.size() as u64) + new_size as u64, Relaxed);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

const OBJECTS_PER_LINE: u32 = 14;

/// Copies of the demo line. The first Station of line `n` is object `n * 14 + 1`.
fn model(lines: u32, seed: u64) -> SimulationRun {
    let mut model = ModelDescription::new();
    for _ in 0..lines {
        let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
        let source = model.add_source(SimTime::from_secs(5));
        let mut station = |secs: u64| model.add_station(SimTime::from_secs(secs), failures.clone());
        let (st1, st2, st3, st4) = (station(10), station(15), station(20), station(25));
        let (a4, b4, c4, st7) = (station(12), station(15), station(18), station(30));
        let (st5, a5, b5, c5) = (station(18), station(10), station(14), station(16));
        let drain = model.add_drain();
        for (from, to) in [
            (source, st1), (st1, st2), (st2, st3), (st2, st5), (st3, st4), (st4, a4), (st4, b4), (st4, c4), (a4, st7),
            (b4, st7), (c4, st7), (st7, drain), (st5, a5), (st5, b5), (st5, c5), (a5, drain), (b5, drain), (c5, drain),
        ] {
            model.connect(from, to);
        }
    }
    model.build(seed)
}

fn advance(run: &mut SimulationRun, until: SimTime) -> Vec<Occurrence> {
    run.advance_to(until).map_err(|failure| failure.error).unwrap()
}

fn hours(h: u64) -> SimTime {
    SimTime::from_secs(h * 3_600)
}

// ---- 1. Is going back exact? ----

/// Takes a checkpoint every `spacing` events, then goes back to each target from the end of the
/// run, and compares the state there, the occurrences from there on, and the final state, against
/// a run that never went back.
fn check_going_back(name: &str, lines: u32, seed: u64, length: SimTime, spacing: usize) {
    let targets: Vec<SimTime> = {
        let mut probe = model(lines, seed);
        probe.init().unwrap();
        // Times between events, plus real instants, where several events can share a time.
        let mut targets: Vec<SimTime> = (1..=6).map(|k| SimTime::from_millis(length.as_millis() * k / 7 + 137 * k)).collect();
        for k in [2, 5] {
            advance(&mut probe, SimTime::from_millis(length.as_millis() * k / 11));
            targets.push(probe.next_event_time().unwrap());
        }
        targets.sort();
        targets
    };

    let mut reference = model(lines, seed);
    reference.set_recording(Recording::ALL);
    let mut all = reference.init().unwrap();
    let mut states: Vec<RunState> = Vec::new();
    for &target in &targets {
        all.extend(advance(&mut reference, target));
        states.push(reference.state());
    }
    all.extend(advance(&mut reference, length));
    let final_state = reference.state();

    let mut ahead = model(lines, seed);
    ahead.set_recording(Recording::ALL);
    ahead.init().unwrap();
    let mut checkpoints = Vec::new();
    let mut part_way_through_an_instant = 0;
    loop {
        if ahead.next_event_time() == Some(ahead.time()) {
            part_way_through_an_instant += 1;
        }
        checkpoints.push(ahead.clone());
        if ahead.advance_bounded(length, spacing).map_err(|failure| failure.error).unwrap().reached {
            break;
        }
    }
    assert_eq!(ahead.state(), final_state, "{name}: the run that took checkpoints differs");

    for (&target, state) in targets.iter().zip(&states).rev() {
        let nearest = checkpoints.iter().rposition(|c| c.time() <= target).unwrap();
        let mut restored = checkpoints[nearest].clone();
        restored.set_recording(Recording::NOTHING);
        assert!(advance(&mut restored, target).is_empty());
        assert_eq!(&restored.state(), state, "{name}: state differs after going back to {target:?}");

        restored.set_recording(Recording::ALL);
        let rest = advance(&mut restored, length);
        let expected: Vec<Occurrence> = all.iter().filter(|o| o.time > target).cloned().collect();
        assert_eq!(rest.len(), expected.len(), "{name}: occurrences differ after {target:?}");
        assert!(rest == expected, "{name}: occurrences differ after {target:?}");
        assert_eq!(restored.state(), final_state, "{name}: final state differs after going back to {target:?}");
        assert_eq!(restored.events_processed(), ahead.events_processed());
    }
    println!(
        "  ok  {name}: seed {seed}, a checkpoint every {spacing} events ({} taken, {part_way_through_an_instant} part way through an instant), {} targets, {} occurrences compared",
        checkpoints.len(),
        targets.len(),
        all.len()
    );
}

// ---- 2. Interventions and going back behind them ----

#[derive(Clone, Copy, Debug)]
enum Action {
    /// Stands in for Pause on, which isn't built: it takes effect at once, like a Pause would.
    Stop(u32),
    /// A warm-up ending, which the statistics design left open for step 6.
    ClearStatistics,
}

fn apply(run: &mut SimulationRun, action: Action) {
    match action {
        Action::Stop(id) => run.schedule_event(run.time(), EventType::StartFailure, id).unwrap(),
        Action::ClearStatistics => run.clear_statistics(),
    }
}

/// Same seed plus same log: a fresh run, applying each logged action at its time.
fn fresh_run_with(lines: u32, seed: u64, log: &[(SimTime, Action)], until: SimTime) -> RunState {
    let mut run = model(lines, seed);
    run.init().unwrap();
    for &(at, action) in log {
        advance(&mut run, at);
        apply(&mut run, action);
    }
    advance(&mut run, until);
    run.state()
}

/// A checkpoint has to know how much of the log is already in it. Times alone can't say, because
/// a checkpoint can land at the very time of an action, either side of it.
struct Checkpoint {
    run: SimulationRun,
    log_len: usize,
}

/// The run computing ahead of what is shown, as ADR 0002 has it.
struct Session {
    run: SimulationRun,
    checkpoints: Vec<Checkpoint>,
    log: Vec<(SimTime, Action)>,
    spacing: usize,
}

impl Session {
    fn compute_ahead_to(&mut self, until: SimTime) {
        loop {
            self.checkpoints.push(Checkpoint { run: self.run.clone(), log_len: self.log.len() });
            if self.run.advance_bounded(until, self.spacing).map_err(|failure| failure.error).unwrap().reached {
                return;
            }
        }
    }

    /// Abandons the computed future. Logged actions after `to` are gone, and so is every
    /// checkpoint from that future. Logged actions the replay passes get applied again.
    fn go_back_to(&mut self, to: SimTime) {
        self.log.retain(|&(at, _)| at <= to);
        let log_len = self.log.len();
        self.checkpoints.retain(|c| c.run.time() <= to && c.log_len <= log_len);
        let nearest = self.checkpoints.last().expect("the checkpoint from Init is always there");
        let mut run = nearest.run.clone();
        run.set_recording(Recording::NOTHING);
        for &(at, action) in &self.log[nearest.log_len..] {
            advance(&mut run, at);
            apply(&mut run, action);
        }
        advance(&mut run, to);
        self.run = run;
    }

    fn intervene(&mut self, at: SimTime, action: Action) {
        self.go_back_to(at);
        apply(&mut self.run, action);
        self.log.push((at, action));
    }
}

fn check_interventions(name: &str, lines: u32, seed: u64, spacing: usize) {
    let (end, station) = (hours(40), 1);
    let (warm_up, t1, t2, t3) = (hours(5), hours(12), hours(25), hours(30));
    let mut session = Session { run: model(lines, seed), checkpoints: Vec::new(), log: Vec::new(), spacing };
    session.run.init().unwrap();

    // The run is always well ahead of each time something is done at.
    session.compute_ahead_to(hours(8));
    session.intervene(warm_up, Action::ClearStatistics);
    session.compute_ahead_to(hours(15));
    session.intervene(t1, Action::Stop(station));
    session.compute_ahead_to(hours(28));
    session.intervene(t2, Action::Stop(station + 2));
    session.compute_ahead_to(end);
    let log = session.log.clone();
    assert_eq!(session.run.state(), fresh_run_with(lines, seed, &log, end), "{name}: same seed and log differ");

    // Going back to a time between them, with the later ones removed.
    let between = t1.saturating_add(hours(3));
    session.go_back_to(between);
    assert_eq!(session.log.len(), 2);
    session.compute_ahead_to(end);
    assert_eq!(session.run.state(), fresh_run_with(lines, seed, &log[..2], end), "{name}: going back behind the last one");

    // A new future from there, with a different intervention.
    session.go_back_to(t3);
    session.intervene(t3, Action::Stop(station + 3));
    session.compute_ahead_to(end);
    let new_log = session.log.clone();
    assert_eq!(session.run.state(), fresh_run_with(lines, seed, &new_log, end), "{name}: a new future");

    // Back behind the warm-up: nothing is left of any of them.
    session.go_back_to(hours(2));
    assert!(session.log.is_empty());
    session.compute_ahead_to(end);
    let state = session.run.state();
    assert_eq!(state.statistics_since, SimTime::ZERO);
    assert_eq!(state, fresh_run_with(lines, seed, &[], end), "{name}: going back behind everything");

    println!("  ok  {name}: seed {seed}, a checkpoint every {spacing} events, log of {} actions", log.len());
}

// ---- 3. What it costs ----

fn median(values: &mut [Duration]) -> Duration {
    values.sort();
    values[values.len() / 2]
}

fn median_u64(values: &mut [u64]) -> u64 {
    values.sort();
    values[values.len() / 2]
}

/// How much one checkpoint takes and how long a clone takes, at many points in a run.
fn checkpoint_costs(name: &str, lines: u32, length: SimTime) -> (u64, Duration) {
    let mut run = model(lines, 0);
    run.init().unwrap();
    let (mut bytes, mut times) = (Vec::new(), Vec::new());
    for k in 1..=200u64 {
        advance(&mut run, SimTime::from_millis(length.as_millis() * k / 200));
        let before = LIVE.load(Relaxed);
        let start = Instant::now();
        let checkpoint = run.clone();
        times.push(start.elapsed());
        bytes.push(LIVE.load(Relaxed) - before + size_of::<SimulationRun>() as u64);
        black_box(&checkpoint);
    }
    let (largest, typical, clone) = (*bytes.iter().max().unwrap(), median_u64(&mut bytes), median(&mut times));
    println!(
        "  {name:<12} {:>5} objects: {:>9.1} KB per checkpoint, {:>6.1} B per object (largest seen {:.1} KB), {:>7.1} us to take or restore one",
        lines * OBJECTS_PER_LINE,
        typical as f64 / 1e3,
        typical as f64 / (lines * OBJECTS_PER_LINE) as f64,
        largest as f64 / 1e3,
        clone.as_secs_f64() * 1e6
    );
    (typical, clone)
}

/// How long going back takes when the replay is `events` long: restore, then replay silently.
fn jump_back_time(lines: u32, warm: SimTime, events: usize) -> Duration {
    let mut times = Vec::new();
    let mut run = model(lines, 0);
    run.init().unwrap();
    advance(&mut run, warm);
    for round in 1..=9u64 {
        let mut later = warm.as_millis() * (10 + round) / 10;
        later += 1;
        advance(&mut run, SimTime::from_millis(later));
        let checkpoint = run.clone();
        let start = Instant::now();
        let mut restored = checkpoint.clone();
        restored.set_recording(Recording::NOTHING);
        let advance = restored.advance_bounded(SimTime::MAX, events).map_err(|failure| failure.error).unwrap();
        times.push(start.elapsed());
        black_box((&restored, advance));
    }
    median(&mut times)
}

/// A run going at full speed that takes a checkpoint every `spacing` events, keeping as many as
/// fit in the budget and dropping the oldest. None means no checkpoints at all: a headless run.
fn timed_run(lines: u32, length: SimTime, spacing: Option<usize>, budget_bytes: u64, checkpoint_bytes: u64) -> (Duration, u64) {
    let mut run = model(lines, 0);
    run.init().unwrap();
    let start = Instant::now();
    match spacing {
        None => {
            advance(&mut run, length);
        }
        Some(spacing) => {
            let most = (budget_bytes / checkpoint_bytes).max(1) as usize;
            let mut kept = std::collections::VecDeque::with_capacity(most + 1);
            while !run.advance_bounded(length, spacing).map_err(|failure| failure.error).unwrap().reached {
                kept.push_back(run.clone());
                if kept.len() > most {
                    kept.pop_front();
                }
            }
            black_box(&kept);
        }
    }
    (start.elapsed(), run.events_processed())
}

fn overhead(name: &str, lines: u32, length: SimTime, checkpoint_bytes: u64, budget_bytes: u64) {
    let spacings = [None, Some(1_000_000), Some(100_000), Some(10_000)];
    let mut times = vec![Vec::new(); spacings.len()];
    let mut events = 0;
    for _round in 0..7 {
        for (i, spacing) in spacings.iter().enumerate() {
            let (time, count) = timed_run(lines, length, *spacing, budget_bytes, checkpoint_bytes);
            times[i].push(time);
            events = count;
        }
    }
    let baseline = median(&mut times[0]).as_secs_f64();
    println!("  {name} over {:.0} events, with a {} MB budget:", events as f64, budget_bytes / 1_000_000);
    for (i, spacing) in spacings.iter().enumerate() {
        let time = median(&mut times[i]).as_secs_f64();
        let label = spacing.map_or("no checkpoints".to_string(), |s| format!("every {s} events"));
        println!("    {label:<22} {time:>7.3} s   {:>6.1}% slower", (time / baseline - 1.0) * 100.0);
    }
}

fn reach_of_budget(name: &str, checkpoint_bytes: u64, events_per_hour: f64) {
    for budget in [100_000_000u64, 1_000_000_000] {
        for spacing in [100_000u64, 1_000_000] {
            let count = budget / checkpoint_bytes;
            let hours_back = (count * spacing) as f64 / events_per_hour;
            println!(
                "  {name:<12} {:>5} MB, every {spacing:>7} events: {count:>6} checkpoints reach back {:>9.1} simulated days",
                budget / 1_000_000,
                hours_back / 24.0
            );
        }
    }
}

/// Taking checkpoints that are kept, as real ones are, so each lands on fresh memory. Section 3
/// drops each clone at once, so the next one reuses memory that is already touched.
fn kept_checkpoint_time(name: &str, lines: u32, warm: SimTime, count: usize) {
    let mut run = model(lines, 0);
    run.init().unwrap();
    advance(&mut run, warm);
    let mut times = Vec::new();
    for _round in 0..7 {
        let mut kept = Vec::with_capacity(count);
        let start = Instant::now();
        for _ in 0..count {
            kept.push(run.clone());
        }
        times.push(start.elapsed() / count as u32);
        let start = Instant::now();
        drop(kept);
        times.push(Duration::ZERO.max(Duration::ZERO));
        black_box(start.elapsed());
    }
    times.retain(|time| !time.is_zero());
    println!("  {name:<14} keeping {count} checkpoints: {:>8.1} us each", median(&mut times).as_secs_f64() * 1e6);
}

fn main() {
    if std::env::args().any(|arg| arg == "--kept") {
        kept_checkpoint_time("demo", 1, hours(24 * 30), 5_000);
        kept_checkpoint_time("2,800 objects", 200, hours(6), 200);
        return;
    }
    println!("1. Is going back exact? State, occurrences from there on, and final state, against a run that never went back");
    for seed in 0..5 {
        check_going_back("demo", 1, seed, hours(24 * 30), 100_000);
    }
    check_going_back("demo", 1, 7, hours(24 * 30), 1_000);
    check_going_back("demo", 1, 8, hours(24 * 30), 1_000_000);
    check_going_back("2,800 objects", 200, 0, hours(6), 100_000);
    check_going_back("2,800 objects", 200, 1, hours(6), 1_000_000);

    println!("\n2. Interventions and going back behind them, against a fresh run with the same seed and log");
    for spacing in [1_000, 100_000, 1_000_000] {
        check_interventions("demo", 1, 3, spacing);
    }
    check_interventions("2,800 objects", 200, 3, 1_000_000);

    println!("\n3. What one checkpoint costs (ADR 0002 estimated 10 to 20 KB for the demo)");
    let (demo_bytes, _) = checkpoint_costs("demo", 1, hours(24 * 365));
    let (large_bytes, _) = checkpoint_costs("2,800 objects", 200, hours(48));

    println!("\n4. How long going back takes: restore, then replay with recording off (ADR 0002 estimated 0.2 ms per 1,000 events)");
    for (name, lines, warm) in [("demo", 1, hours(24 * 30)), ("2,800 objects", 200, hours(6))] {
        for events in [1_000, 10_000, 100_000, 1_000_000] {
            let time = jump_back_time(lines, warm, events);
            println!("  {name:<14} a replay of {events:>9} events: {:>9.2} ms", time.as_secs_f64() * 1e3);
        }
    }

    println!("\n5. What taking checkpoints costs a run at full speed");
    overhead("demo", 1, hours(24 * 365), demo_bytes, 100_000_000);
    overhead("2,800 objects", 200, hours(48), large_bytes, 1_000_000_000);

    println!("\n6. How far back a memory budget reaches");
    reach_of_budget("demo", demo_bytes, 15_500_000.0 / (24.0 * 365.0));
    reach_of_budget("2,800 objects", large_bytes, 17_000_000.0 / 48.0);
}
