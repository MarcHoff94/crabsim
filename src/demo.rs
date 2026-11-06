//! A small example model, run from the command line.

use crate::flows::FailureProfile;
use crate::model::ModelDescription;
use crate::random_numbers::Distribution;
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;
use crate::visualization::terminal::{render_table, InPlaceTerminalView};
use crate::visualization::topology::TopologyVisualizer;
use crate::visualization::advance_with_view;
use std::time::Instant;

const RUN_LENGTH: SimTime = SimTime::from_secs(3_600);

/// The example model, returned with its Drain's id so a caller can read what has left it.
pub fn build_model(seed: u64) -> (SimulationRun, u32) {
    // availability=0.9, mttr=60s → mtbf=540s derived automatically
    let failures = FailureProfile::new(0.9, Distribution::ERLANG { mean: 60_000.0, k: 2 }, 0);
    let mut model = ModelDescription::new();
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
    (model.build(seed), drain)
}

/// Runs the example model for an hour of simulation time and prints the result.
pub fn run_demo(inplace: bool) -> Result<(), crate::error::RunError> {
    let _ = env_logger::try_init();

    let (mut run, drain) = build_model(0);
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
    log::info!("exit times: {:#?}", run.object(drain).and_then(|object| object.as_drain()).map(|drain| &drain.exits));
    Ok(())
}
