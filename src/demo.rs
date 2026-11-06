//! A small example model, run from the command line.

use crate::flows::FailureProfile;
use crate::materialflow_objects::{Drain, SingleStation, Source};
use crate::random_numbers::Distribution;
use crate::simulation_run::SimulationRun;
use crate::visualization::terminal::{render_table, InPlaceTerminalView};
use crate::visualization::topology::TopologyVisualizer;
use crate::visualization::advance_with_view;
use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};

const RUN_LENGTH: Duration = Duration::from_secs(3_600);

/// The example model, returned with its Drain so a caller can read what has left it.
pub fn build_model(seed: u64) -> (SimulationRun, Rc<RefCell<Drain>>) {
    // availability=0.9, mttr=60s → mtbf=540s derived automatically
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

    st1.borrow_mut().succ = vec![st2.clone()];
    st2.borrow_mut().succ = vec![st3.clone(), st5.clone()];
    // Branch 1: st3 -> st4 -> 3 parallel -> st7 -> drain
    st3.borrow_mut().succ = vec![st4.clone()];
    st4.borrow_mut().succ = vec![st4a.clone(), st4b.clone(), st4c.clone()];
    st4a.borrow_mut().succ = vec![st7.clone()];
    st4b.borrow_mut().succ = vec![st7.clone()];
    st4c.borrow_mut().succ = vec![st7.clone()];
    st7.borrow_mut().succ = vec![drain.clone()];
    // Branch 2: st5 -> 3 parallel -> drain
    st5.borrow_mut().succ = vec![st5a.clone(), st5b.clone(), st5c.clone()];
    st5a.borrow_mut().succ = vec![drain.clone()];
    st5b.borrow_mut().succ = vec![drain.clone()];
    st5c.borrow_mut().succ = vec![drain.clone()];

    let mut run = SimulationRun::new(seed);
    run.add_object(source);
    for station in [st1, st2, st3, st4, st4a, st4b, st4c, st7, st5, st5a, st5b, st5c] {
        run.add_object(station);
    }
    run.add_object(drain.clone());
    (run, drain)
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
    log::info!("exit times: {:#?}", drain.borrow().exits);
    Ok(())
}
