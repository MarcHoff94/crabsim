//! The headless CLI: runs the demo model with a seed for a simulated length, then prints each
//! object's final statistics and how long the run took. It takes a model file once models can be
//! saved.

use clap::Parser;
use crabsim::time::{format_time, format_time_exact, parse_time};
use crabsim::{demo, ModelDescription, ObjectKind, ObjectStatistics, RunState, SimTime};
use std::process::ExitCode;
use std::time::Instant;

#[derive(Parser, Debug)]
#[command(name = "crabsim", about = "Runs crabsim's demo model headless and prints its statistics")]
struct Opts {
    /// Seed for the run's random numbers. The same seed gives the same results.
    #[arg(long, default_value_t = 0)]
    seed: u64,
    /// Simulated length, written as the UI shows time, such as `90`, `1:30`, `8:00:00` or
    /// `30d 12:00:00`.
    #[arg(long, default_value = "1d", value_parser = length)]
    length: SimTime,
}

fn length(typed: &str) -> Result<SimTime, String> {
    parse_time(typed).ok_or_else(|| "not a time: write it as 90, 1:30, 8:00:00 or 30d 12:00:00".to_string())
}

fn main() -> ExitCode {
    let _ = env_logger::try_init();
    let opts = Opts::parse();
    let model = demo::model();
    let mut run = model.build(opts.seed);
    println!("crabsim: the demo model, seed {}, for {}", opts.seed, format_time_exact(opts.length));

    let start = Instant::now();
    let outcome = run.init().and_then(|_| run.advance_to(opts.length).map_err(|failure| failure.error));
    let wall = start.elapsed();

    // A run that fails still gives back what it did up to the error.
    let state = run.state();
    print!("{}", report(&model, &state));
    let events = run.events_processed();
    println!(
        "Ran {events} events in {:.3} s, {:.2}M events per second",
        wall.as_secs_f64(),
        events as f64 / wall.as_secs_f64().max(f64::MIN_POSITIVE) / 1e6
    );
    match outcome {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("the run stopped at {}: {error}", format_time_exact(state.time));
            ExitCode::FAILURE
        }
    }
}

/// Each object's statistics as a table, then what only Drains keep.
fn report(model: &ModelDescription, state: &RunState) -> String {
    let mut text = format!("\nStatistics from {} to {}\n", format_time(state.statistics_since), format_time_exact(state.time));
    text += &format!(
        "{:>4}  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>13}\n",
        "ID", "Name", "Type", "MUs in", "MUs out", "Working", "Blocked", "Idle", "Failed", "Paused", "Failures", "Availability"
    );
    for object in &state.objects {
        let statistics = &object.statistics;
        let measured = statistics.measured_time();
        let share = |time: SimTime| match measured.as_millis() {
            0 => "-".to_string(),
            total => format!("{:.1} %", time.as_millis() as f64 * 100.0 / total as f64),
        };
        let availability = statistics.achieved_availability().map_or("-".to_string(), |a| format!("{:.1} %", a * 100.0));
        text += &format!(
            "{:>4}  {:<12} {:<8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>13}\n",
            object.id,
            model.name(object.id).unwrap_or("-"),
            kind(object.kind),
            statistics.mus_in,
            statistics.mus_out,
            share(statistics.time_busy),
            share(statistics.time_blocked),
            share(statistics.time_idle),
            share(statistics.time_failed),
            share(statistics.time_paused),
            statistics.failures,
            availability
        );
    }
    text += "\n";
    for object in state.objects.iter().filter(|object| object.statistics.drain.is_some()) {
        text += &drain_line(model.name(object.id).unwrap_or("Drain"), &object.statistics, state.time);
    }
    text
}

fn drain_line(name: &str, statistics: &ObjectStatistics, now: SimTime) -> String {
    let per_hour = statistics.mus_per_hour(now).map_or("-".to_string(), |rate| format!("{rate:.1}"));
    let lead_time = statistics.lead_time_mean().map_or("-".to_string(), format_time_exact);
    format!("{name}: {} MUs, {per_hour} an hour, mean lead time {lead_time}\n", statistics.mus_in)
}

fn kind(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Source => "Source",
        ObjectKind::Station => "Station",
        ObjectKind::Buffer => "Buffer",
        ObjectKind::Drain => "Drain",
    }
}
