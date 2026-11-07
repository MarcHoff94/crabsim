//! A small example model, run from the command line.

use crate::flows::FailureProfile;
use crate::model::{ModelDescription, Position};
use crate::random_numbers::Distribution;
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;
use crate::visualization::terminal::{render_table, InPlaceTerminalView};
use crate::visualization::topology::TopologyVisualizer;
use crate::visualization::advance_with_view;
use std::time::Instant;

const RUN_LENGTH: SimTime = SimTime::from_secs(3_600);

/// The example model, built into a run.
pub fn build_model(seed: u64) -> SimulationRun {
    model().build(seed)
}

/// The example model, with names and positions for a view: a Source, a line that splits into two
/// branches with parallel Stations, and a Drain. Positions are in metres, laid out by hand.
pub fn model() -> ModelDescription {
    // availability=0.9, mttr=60s → mtbf=540s derived automatically
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
    let place = |model: &mut ModelDescription, id: u32, name: &str, x: f64, y: f64| {
        model.set_name(id, name);
        model.set_position(id, Position { x, y, z: 0.0 });
        id
    };
    let source = model.add_source(SimTime::from_secs(5));
    place(&mut model, source, "Source", 0.0, 0.0);
    let station = |model: &mut ModelDescription, name: &str, process_secs: u64, x: f64, y: f64| {
        let id = model.add_station(SimTime::from_secs(process_secs), failures.clone());
        place(model, id, name, x, y)
    };
    let st1 = station(&mut model, "Station 1", 10, 4.0, 0.0);
    let st2 = station(&mut model, "Station 2", 15, 8.0, 0.0);
    let st3 = station(&mut model, "Station 3", 20, 12.0, 4.0);
    let st4 = station(&mut model, "Station 4", 25, 16.0, 4.0);
    let st4a = station(&mut model, "Station 4a", 12, 20.0, 7.0);
    let st4b = station(&mut model, "Station 4b", 15, 20.0, 4.0);
    let st4c = station(&mut model, "Station 4c", 18, 20.0, 1.0);
    let st7 = station(&mut model, "Station 7", 30, 24.0, 4.0);
    let st5 = station(&mut model, "Station 5", 18, 12.0, -4.0);
    let st5a = station(&mut model, "Station 5a", 10, 16.0, -1.0);
    let st5b = station(&mut model, "Station 5b", 14, 16.0, -4.0);
    let st5c = station(&mut model, "Station 5c", 16, 16.0, -7.0);
    let drain = model.add_drain();
    place(&mut model, drain, "Drain", 28.0, 0.0);

    let connections = [
        (source, st1),
        (st1, st2),
        (st2, st3),
        (st2, st5),
        // Branch 1: st3 -> st4 -> 3 parallel -> st7 -> drain
        (st3, st4),
        (st4, st4a),
        (st4, st4b),
        (st4, st4c),
        (st4a, st7),
        (st4b, st7),
        (st4c, st7),
        (st7, drain),
        // Branch 2: st5 -> 3 parallel -> drain
        (st5, st5a),
        (st5, st5b),
        (st5, st5c),
        (st5a, drain),
        (st5b, drain),
        (st5c, drain),
    ];
    for (from, to) in connections {
        model.connect(from, to);
    }
    model
}

/// Runs the example model for an hour of simulation time and prints the result.
pub fn run_demo(inplace: bool) -> Result<(), crate::error::RunError> {
    let _ = env_logger::try_init();

    let mut run = build_model(0);
    let topology = TopologyVisualizer::render_network_topology(run.objects());
    run.init()?;

    let start = Instant::now();
    if inplace {
        advance_with_view(&mut run, RUN_LENGTH, &mut InPlaceTerminalView::new(topology))?;
    } else {
        run.advance_to(RUN_LENGTH).map_err(|failure| failure.error)?;
        println!("{topology}{}", render_table(&run.state()));
    }
    println!("simulation runtime: {:.2} ms", start.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
