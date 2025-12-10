#![allow(dead_code)]

pub mod event;
pub use crate::materialflow_objects::flows; // keep single module re-export
pub use crate::materialflow_objects::flows::BaseObject;
pub mod error;
pub mod materialflow_objects;
pub mod simulator;
pub mod visualization;
pub mod random_numbers;

pub use crate::error::{InitError, MaterialflowError, RunTimeError};
pub use crate::event::{EcTime, EventTemplate, EventType};
pub use crate::materialflow_objects::{Drain, SingleStation, Source}; // Adjusted to use materialflow_objects for concrete types
pub use crate::materialflow_objects::flows::MoveableUnit; // Moved MoveableUnit to be imported from flows
pub use crate::simulator::{EventController, Simulator};

use std::{cell::RefCell, rc::Rc};
use std::net::TcpListener;
use std::process::Command;
use std::time::Instant;

pub fn run_demo() {
	env_logger::init();
    
	let mut simulator = Simulator::new();
	// Optional: enable in-place live rendering via env var
	if std::env::var("CRABSIM_LIVE_INPLACE").ok().filter(|v| v != "0").is_some() {
		simulator.live_inplace = true;
	}

	// Optional: start external renderer window and attach stream
	let use_renderer = std::env::var("CRABSIM_RENDERER").ok().filter(|v| v != "0").is_some();
	let speed_factor: String = std::env::var("CRABSIM_RENDERER_SPEED").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "5".to_string());
	let mut _listener: Option<TcpListener> = None;
	if use_renderer {
		if let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) {
			let addr = listener.local_addr().unwrap();
			let connect_arg = format!("{}:{}", addr.ip(), addr.port());
			// Resolve path to renderer binary next to current exe
			let renderer_prog = {
				let mut p = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("crabsim-renderer"));
				p.set_file_name(if cfg!(windows) { "crabsim-renderer.exe" } else { "crabsim-renderer" });
				p
			};
			// Spawn renderer: Windows -> choose launcher (powershell/cmd) via env CRABSIM_RENDERER_LAUNCHER; others -> spawn normally
			#[cfg(windows)]
			{
				let launcher = std::env::var("CRABSIM_RENDERER_LAUNCHER").unwrap_or_else(|_| "powershell".to_string());
				match launcher.as_str() {
					"cmd" => {
						let path = renderer_prog.to_string_lossy().to_string();
						let _ = Command::new("cmd")
							.args([
								"/C",
								"start",
								"CrabSim Renderer",
								"/MAX",
								&path,
								"--connect",
								&connect_arg,
								"--speed",
								&speed_factor,
							])
							.spawn();
					}
					_ => {
						// Default: PowerShell Start-Process (maximized)
						let ps_cmd = format!(
							"Start-Process -FilePath '{}' -ArgumentList @('--connect','{}','--speed','{}') -WindowStyle Maximized",
							renderer_prog.display(), connect_arg, speed_factor
						);
						let _ = Command::new("powershell")
							.args(["-NoProfile", "-Command", &ps_cmd])
							.spawn();
					}
				}
			}
			#[cfg(not(windows))]
			{
				let _ = Command::new(renderer_prog)
					.arg("--connect").arg(&connect_arg)
					.arg("--speed").arg(&speed_factor)
					.spawn();
			}
			// accept a single connection from renderer
			if let Ok((stream, _peer)) = listener.accept() {
				simulator.renderer_stream = Some(stream);
				_listener = Some(listener); // keep listener alive for scope if needed (not strictly necessary)
				// Immediately send topology to the renderer so it appears before the first state frame
				simulator.print_network_topology();
			}
		}
	}
	let mut eventcontroller = EventController::new(Some(EcTime::from(3600)));
	
	// Station 1 - first station
	let st1 = Rc::new(RefCell::new(SingleStation::new(
		1,
		EcTime::from(10),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	// Station 2 - splits into two branches
	let st2 = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(15),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	// Branch 1: Station 3
	let st3 = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(20),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	// Station 4 splits into 3 parallel stations
	let st4 = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(25),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st4a = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(12),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st4b = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(15),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st4c = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(18),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	// Station after st4 parallel merge
	let st7 = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(30),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	// Branch 2: Station 5 splits into 3 parallel stations
	let st5 = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(18),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st5a = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(10),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st5b = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(14),
		None,
		Vec::new(),
		Vec::new(),
	)));
	let st5c = Rc::new(RefCell::new(SingleStation::new(
		2,
		EcTime::from(16),
		None,
		Vec::new(),
		Vec::new(),
	)));
	
	let source = Rc::new(RefCell::new(Source::new(
		0,
		EcTime::from(5),
		vec![st1.clone()],
	)));
	let drain = Rc::new(RefCell::new(Drain::new(3, vec![st7.clone(), st5a.clone(), st5b.clone(), st5c.clone()])));
	
	// Set up successor relationships
	st1.borrow_mut().succ = vec![st2.clone()];
	st2.borrow_mut().succ = vec![st3.clone(), st5.clone()];  // Split into two branches
	
	// Branch 1: st3 -> st4 -> 3 parallel -> st7 -> drain
	st3.borrow_mut().succ = vec![st4.clone()];
	st4.borrow_mut().succ = vec![st4a.clone(), st4b.clone(), st4c.clone()];  // 3 parallel
	st4a.borrow_mut().succ = vec![st7.clone()];
	st4b.borrow_mut().succ = vec![st7.clone()];
	st4c.borrow_mut().succ = vec![st7.clone()];
	st7.borrow_mut().succ = vec![drain.clone()];
	
	// Branch 2: st5 -> 3 parallel -> drain
	st5.borrow_mut().succ = vec![st5a.clone(), st5b.clone(), st5c.clone()];  // 3 parallel
	st5a.borrow_mut().succ = vec![drain.clone()];
	st5b.borrow_mut().succ = vec![drain.clone()];
	st5c.borrow_mut().succ = vec![drain.clone()];
	
	// Set up predecessor relationships to match successors
	st1.borrow_mut().pred = vec![source.clone()];
	st2.borrow_mut().pred = vec![st1.clone()];
	st3.borrow_mut().pred = vec![st2.clone()];
	st4.borrow_mut().pred = vec![st3.clone()];
	st4a.borrow_mut().pred = vec![st4.clone()];
	st4b.borrow_mut().pred = vec![st4.clone()];
	st4c.borrow_mut().pred = vec![st4.clone()];
	st7.borrow_mut().pred = vec![st4a.clone(), st4b.clone(), st4c.clone()];
	st5.borrow_mut().pred = vec![st2.clone()];
	st5a.borrow_mut().pred = vec![st5.clone()];
	st5b.borrow_mut().pred = vec![st5.clone()];
	st5c.borrow_mut().pred = vec![st5.clone()];
	
	simulator.register_material_flow(source.clone());
	simulator.register_material_flow(st1.clone());
	simulator.register_material_flow(st2.clone());
	simulator.register_material_flow(st3.clone());
	simulator.register_material_flow(st4.clone());
	simulator.register_material_flow(st4a.clone());
	simulator.register_material_flow(st4b.clone());
	simulator.register_material_flow(st4c.clone());
	simulator.register_material_flow(st7.clone());
	simulator.register_material_flow(st5.clone());
	simulator.register_material_flow(st5a.clone());
	simulator.register_material_flow(st5b.clone());
	simulator.register_material_flow(st5c.clone());
	simulator.register_material_flow(drain.clone());
	simulator.add_drain(drain.clone());
	// Measure simulation runtime
	eventcontroller.init(&mut simulator);
	let start = Instant::now();
	let _simulator = eventcontroller.run(simulator).unwrap();
	let elapsed = start.elapsed();
	let ms = elapsed.as_secs_f64() * 1000.0;
	let with_renderer = use_renderer;
	let launcher = if cfg!(windows) { std::env::var("CRABSIM_RENDERER_LAUNCHER").unwrap_or_else(|_| "powershell".to_string()) } else { "n/a".to_string() };
	log::info!("simulation runtime: {:.2} ms (renderer={}, launcher={})", ms, with_renderer, launcher);
	println!("simulation runtime: {:.2} ms (renderer={}, launcher={})", ms, with_renderer, launcher);
	log::info!("exit interval: {:#?}", _simulator.drains[0].borrow().exits);
}

#[cfg(test)]
mod tests {
	use super::*;
    use crate::materialflow_objects::flows::EventInterface;
	use std::{cell::RefCell, rc::Rc};

	#[test]
	fn simple_flow_records_exit() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(Some(EcTime::from(86400)));

		let station = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(60),
			None,
			Vec::new(),
			Vec::new(),
		)));
		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(30),
			vec![station.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station.clone()])));

		station.borrow_mut().succ = vec![drain.clone()];

		simulator.register_material_flow(source.clone());
		simulator.register_material_flow(station.clone());
		simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.endtime = Some(EcTime::from(1000));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		assert!(!sim.drains.is_empty());
		let exits = &sim.drains[0].borrow().exits;
		assert!(
			exits.len() > 0,
			"expected at least one exit recorded, got 0"
		);
	}

	#[test]
	fn selects_lowest_index_successor_when_all_allowed() {
		let simulator = Simulator::new();

		// predecessor with a MU ready to move
		let mut pred = SingleStation::new(0, EcTime::from(10), None, Vec::new(), Vec::new());
		pred.content = Some(MoveableUnit { id: 1, location_id: 0 });

		// two successors; both allow entrance by default
		let succ0 = Rc::new(RefCell::new(SingleStation::new(
			1,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));
		let succ1 = Rc::new(RefCell::new(SingleStation::new(
			2,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));

		pred.succ = vec![succ0.clone(), succ1.clone()];

		// call helper
		let moved = crate::flows::try_to_move_to_successor(&mut pred, &simulator).unwrap();
		assert!(moved.is_some(), "expected an event template when move succeeds");

		// ensure successor 0 received the MU
		assert!(succ0.borrow().content.is_some(), "succ0 should have received the MU");
		assert!(succ1.borrow().content.is_none(), "succ1 should be empty");
		// predecessor should have cleared its content
		assert!(pred.content.is_none(), "predecessor content should be taken");
		// succ_pointer should have advanced to 1 (next index)
		assert_eq!(pred.succ_pointer, 1usize);
	}

	#[test]
	fn skips_blocked_successor_and_selects_next_lowest() {
		let simulator = Simulator::new();

		// predecessor with a MU ready to move
		let mut pred = SingleStation::new(10, EcTime::from(10), None, Vec::new(), Vec::new());
		pred.content = Some(MoveableUnit { id: 42, location_id: 10 });

		// first successor denies entrance
		let succ0 = Rc::new(RefCell::new(SingleStation::new(
			11,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));
		succ0.borrow_mut().entrancelocked = true;

		// second successor allows entrance
		let succ1 = Rc::new(RefCell::new(SingleStation::new(
			12,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));

		pred.succ = vec![succ0.clone(), succ1.clone()];

		// call helper
		let moved = crate::flows::try_to_move_to_successor(&mut pred, &simulator).unwrap();
		assert!(moved.is_some(), "expected an event template when move succeeds to succ1");

		// succ0 should not have the MU, succ1 should
		assert!(succ0.borrow().content.is_none(), "succ0 should not have received the MU");
		assert!(succ1.borrow().content.is_some(), "succ1 should have received the MU");
		// predecessor should have cleared its content
		assert!(pred.content.is_none(), "predecessor content should be taken");
		// succ_pointer should have advanced to 0 (since selected index was 1 -> (1+1)%2 == 0)
		assert_eq!(pred.succ_pointer, 0usize);

		// first successor should have recorded blocked predecessor id
		assert!(succ0.borrow().blockedlist.contains(&pred.base.object_id));
	}

	#[test]
	fn blocked_flow_retries() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(Some(EcTime::from(86400)));

		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(60),
			None,
			Vec::new(),
			Vec::new(),
		)));
		let mut busy_b = SingleStation::new(
			0,
			EcTime::from(75),
			None,
			Vec::new(),
			vec![station_a.clone()],
		);
		busy_b.content = Some(MoveableUnit {
			id: 9999,
			location_id: 0,
		});
		busy_b.entrancelocked = true;
		busy_b.state = crate::flows::ProcessState::Processing;
		let station_b = Rc::new(RefCell::new(busy_b));

		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(30),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b.clone()])));

		station_a.borrow_mut().succ = vec![station_b.clone()];
		station_b.borrow_mut().succ = vec![drain.clone()];

		let _s_id = simulator.register_material_flow(source.clone());
		let _a_id = simulator.register_material_flow(station_a.clone());
		let b_id = simulator.register_material_flow(station_b.clone());
		let _d_id = simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.add_event(
			EcTime::from(100),
			EventTemplate {
				eventtype: EventType::MUExit,
				exec_time: EcTime::from(0),
				object_id: b_id,
				seq: 0,
			},
		);

		eventcontroller.endtime = Some(EcTime::from(1000));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		assert!(!sim.drains.is_empty());
		let exits = &sim.drains[0].borrow().exits;
		assert!(
			exits.len() > 0,
			"expected at least one exit recorded after blocked retry"
		);
	}

	#[test]
	fn blockedlist_dedup() {
		let mut s = SingleStation::new(1, EcTime::from(10), None, Vec::new(), Vec::new());
		s.add_blocked(5);
		s.add_blocked(5);
		assert_eq!(
			s.blockedlist.len(),
			1,
			"blockedlist should contain deduped entry"
		);

		let mut d = Drain::new(2, Vec::new());
		d.add_blocked(7);
		d.add_blocked(7);
		assert_eq!(
			d.blockedlist.len(),
			1,
			"drain blockedlist should contain deduped entry"
		);
	}

	#[test]
	fn drain_fifo_order() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(Some(EcTime::from(86400)));

		let drain = Rc::new(RefCell::new(Drain::new(0, Vec::new())));
		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(10),
			vec![drain.clone()],
		)));

		simulator.register_material_flow(source.clone());
		simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.endtime = Some(EcTime::from(200));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		let drain_ref = sim.drains[0].borrow();
		assert!(!drain_ref.exit_ids.is_empty());
		for (i, &id) in drain_ref.exit_ids.iter().enumerate() {
			assert_eq!(id as usize, i + 1, "MU ids should be emitted in FIFO order");
		}
	}

	#[test]
	fn ordering_across_stations_preserved() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(Some(EcTime::from(86400)));

		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));
		let station_b = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(30),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));
		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(0),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b.clone()])));

		station_a.borrow_mut().succ = vec![station_b.clone()];
		station_b.borrow_mut().succ = vec![drain.clone()];

		simulator.register_material_flow(source.clone());
		simulator.register_material_flow(station_a.clone());
		simulator.register_material_flow(station_b.clone());
		simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.endtime = Some(EcTime::from(500));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		let exit_ids = &sim.drains[0].borrow().exit_ids;
		assert!(!exit_ids.is_empty());
		let endtime: EcTime = EcTime::from(500);
		let first_exit =
			source.borrow().proc_time + station_a.borrow().proc_time + station_b.borrow().proc_time;
		let steady_interval = std::cmp::max(
			source.borrow().proc_time,
			std::cmp::max(station_a.borrow().proc_time, station_b.borrow().proc_time),
		);
		let expected_drained = if endtime < first_exit {
			0usize
		} else {
			1usize + ((endtime - first_exit).0 / steady_interval.0) as usize
		};
		let drained = exit_ids.len();
		assert_eq!(
			drained, expected_drained,
			"expected {} drained MUs, got {}",
			expected_drained, drained
		);
		for (i, &id) in exit_ids.iter().enumerate() {
			assert_eq!(
				id as usize,
				i + 1,
				"MU ids should preserve source order across stations"
			);
		}
	}

	#[test]
	fn ordering_across_parallel_stations_preserved() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(Some(EcTime::from(86400)));

		// First station
		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));
		// Two parallel stations with double the process time
		let station_b1 = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(20),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));
		let station_b2 = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(20),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));

		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(20),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b1.clone(), station_b2.clone()])));

		// wire up successors
		station_a.borrow_mut().succ = vec![station_b1.clone(), station_b2.clone()];
		station_b1.borrow_mut().succ = vec![drain.clone()];
		station_b2.borrow_mut().succ = vec![drain.clone()];

		simulator.register_material_flow(source.clone());
		simulator.register_material_flow(station_a.clone());
		simulator.register_material_flow(station_b1.clone());
		simulator.register_material_flow(station_b2.clone());
		simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.endtime = Some(EcTime::from(500));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		let exit_ids = &sim.drains[0].borrow().exit_ids;
		assert!(!exit_ids.is_empty());
		// Ensure the drain observed MUs in source order (IDs increasing sequentially)
		for (i, &id) in exit_ids.iter().enumerate() {
			assert_eq!(id as usize, i + 1, "MU ids should preserve source order across parallel stations");
		}

	// Assumptions for the calculation below:
	// - Parallel branches are identical in processing time and are round-robin balanced
	// - In steady state the combined throughput of identical parallel branches
	//   can be approximated as branch_proc_time / num_branches (integer arithmetic)
	// - This is a simplification for tests (works for identical branches);
	//   unequal branch times require a different model (see next test)
	// Also check the calculated number of drained MUs assuming parallel stations have identical proc_time
		let endtime: EcTime = EcTime::from(500);
		let first_exit = source.borrow().proc_time + station_a.borrow().proc_time + station_b1.borrow().proc_time;
		// effective steady interval for parallel identical branches = branch_proc_time / num_branches
		let num_branches = 2u128;
		let branch_effective = EcTime::from((station_b1.borrow().proc_time.0) / num_branches);
		let steady_interval = std::cmp::max(source.borrow().proc_time, std::cmp::max(station_a.borrow().proc_time, branch_effective));
		let expected_drained = if endtime < first_exit {
			0usize
		} else {
			1usize + ((endtime - first_exit).0 / steady_interval.0) as usize
		};
		let drained = exit_ids.len();
		assert_eq!(drained, expected_drained, "expected {} drained MUs, got {}", expected_drained, drained);
	}

	#[test]
	fn throughput_unbalanced_parallel_stations() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(None);

		// First station
		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(10),
			None,
			Vec::new(),
			Vec::new(),
		)));
		// Two parallel stations with unequal process times
		let station_b1 = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(30),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));
		let station_b2 = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(10),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));

		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(20),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b1.clone(), station_b2.clone()])));

		// wire up successors
		station_a.borrow_mut().succ = vec![station_b1.clone(), station_b2.clone()];
		station_b1.borrow_mut().succ = vec![drain.clone()];
		station_b2.borrow_mut().succ = vec![drain.clone()];

		simulator.register_material_flow(source.clone());
		simulator.register_material_flow(station_a.clone());
		simulator.register_material_flow(station_b1.clone());
		simulator.register_material_flow(station_b2.clone());
		simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		eventcontroller.endtime = Some(EcTime::from(500));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		let exit_ids = &sim.drains[0].borrow().exit_ids;
		assert!(!exit_ids.is_empty());

	// For unbalanced parallel branches we only check throughput (drained count),
	// not strict ordering. Exact analytical throughput for round-robin routing
	// with unequal branch times is non-trivial; assert only that some units
	// were drained and that the drained count does not exceed a safe upper
	// bound (best-case: all work goes to the fastest branch every interval).
	let endtime: EcTime = EcTime::from(500);
	let first_exit = source.borrow().proc_time + station_a.borrow().proc_time + station_b1.borrow().proc_time;
	let b1 = station_b1.borrow().proc_time.0;
	let b2 = station_b2.borrow().proc_time.0;
	let duration_after_first = if endtime < first_exit { 0u128 } else { (endtime - first_exit).0 };
	let fastest = std::cmp::min(b1, b2);
	let max_additional = if fastest == 0 { 0usize } else { (duration_after_first / fastest) as usize };
	let expected_max = if endtime < first_exit { 0usize } else { 1usize + max_additional };
	let drained = exit_ids.len();
	assert!(drained > 0, "expected at least one drained MU, got 0");
	assert!(drained <= expected_max, "drained count {} exceeds best-case upper bound {}", drained, expected_max);
	}

	#[test]
	fn blocked_retry_while_predecessor_processing_does_not_move_mu() {
		let mut simulator = Simulator::new();

		// create station_2 (initially denying) and station_3 (allowing)
		let station_2 = Rc::new(RefCell::new(SingleStation::new(
			2,
			EcTime::from(5),
			None,
			Vec::new(),
			Vec::new(),
		)));
		let station_3 = Rc::new(RefCell::new(SingleStation::new(
			3,
			EcTime::from(5),
			None,
			Vec::new(),
			Vec::new(),
		)));

		station_2.borrow_mut().entrancelocked = true; // deny initially

		// station_1 will prefer station_2 (index 0) then station_3
		let station_1 = Rc::new(RefCell::new(SingleStation::new(
			1,
			EcTime::from(10),
			None,
			vec![station_2.clone(), station_3.clone()],
			Vec::new(),
		)));

		// register objects so object ids are assigned
		simulator.register_material_flow(station_1.clone());
		simulator.register_material_flow(station_2.clone());
		simulator.register_material_flow(station_3.clone());

		// place MU100 in station_1 and trigger its MUExit; it will try station_2
		// (denied -> station_2.blockedlist records station_1) then move to station_3
		station_1.borrow_mut().content = Some(MoveableUnit { id: 100, location_id: 1 });
		let _ = station_1
			.borrow_mut()
			.execute_event(&mut simulator, &EventType::MUExit)
			.unwrap();

		assert!(station_3.borrow().content.is_some(), "station_3 should have received MU100");
		assert!(station_2.borrow().blockedlist.contains(&station_1.borrow().base.object_id));

		// now station_1 starts processing MU101 (in progress)
		station_1.borrow_mut().content = Some(MoveableUnit { id: 101, location_id: 1 });
		station_1.borrow_mut().state = crate::flows::ProcessState::Processing;
		station_1.borrow_mut().entrancelocked = true;

		// station_2 becomes available and emits BlockedRetry
		station_2.borrow_mut().entrancelocked = false;
		let res_evts = station_2.borrow_mut().execute_event(&mut simulator, &EventType::MUExit);
		if let Ok(Some(evts)) = res_evts {
			// process emitted events; find the BlockedRetry for station_1
			let mut found = false;
			for evt in evts {
				if let EventType::BlockedRetry = evt.eventtype {
					if evt.object_id == station_1.borrow().base.object_id {
						found = true;
						// invoke the retry on station_1 while it's processing
						let res = station_1.borrow_mut().execute_event(&mut simulator, &evt.eventtype);
						assert!(res.is_ok());
						// station_1 should still hold MU101 and must not have transferred it
						assert!(station_1.borrow().content.is_some(), "station_1 should still hold MU101");
						assert!(station_2.borrow().content.is_none(), "station_2 should not have received MU101");
					}
				}
			}
			assert!(found, "expected a BlockedRetry event for station_1");
		} else {
			panic!("expected station_2 to emit BlockedRetry events");
		}
	}

	#[test]
	fn station_becomes_blocked_when_no_successor_accepts() {
		let mut simulator = Simulator::new();

		// successor denies entrance
		let succ = Rc::new(RefCell::new(SingleStation::new(
			2,
			EcTime::from(5),
			None,
			Vec::new(),
			Vec::new(),
		)));
		succ.borrow_mut().entrancelocked = true;

		// predecessor with an MU ready to move
		let pred = Rc::new(RefCell::new(SingleStation::new(
			1,
			EcTime::from(10),
			None,
			vec![succ.clone()],
			Vec::new(),
		)));

		// register to ensure object ids are set
		simulator.register_material_flow(pred.clone());
		simulator.register_material_flow(succ.clone());

		// place MU in pred and trigger MUExit
		pred.borrow_mut().content = Some(MoveableUnit { id: 200, location_id: 1 });
		let _ = pred
			.borrow_mut()
			.execute_event(&mut simulator, &EventType::MUExit)
			.unwrap();

		// Since the successor denied entrance, predecessor should become Blocked
		match pred.borrow().state {
			crate::flows::ProcessState::Blocked => {}
			_ => panic!("expected predecessor to be in Blocked state"),
		}
		// MU should still be present and entrance still locked
		assert!(pred.borrow().content.is_some());
		assert!(pred.borrow().entrancelocked);
		// successor should have recorded the blocked predecessor id
		assert!(succ.borrow().blockedlist.contains(&pred.borrow().base.object_id));
	}

	#[test]
	fn blocked_retry_full_flow() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(None);

		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(60),
			None,
			Vec::new(),
			Vec::new(),
		)));
		// busy station_b that initially denies entrance
		let mut busy_b = SingleStation::new(
			0,
			EcTime::from(75),
			None,
			Vec::new(),
			vec![station_a.clone()],
		);
		busy_b.content = Some(MoveableUnit {
			id: 9999,
			location_id: 0,
		});
		busy_b.entrancelocked = true;
		busy_b.state = crate::flows::ProcessState::Processing;
		let station_b = Rc::new(RefCell::new(busy_b));

		// station_c is free and can accept fallback MUs
		let station_c = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(30),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));

		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(30),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b.clone(), station_c.clone()])));

		station_a.borrow_mut().succ = vec![station_b.clone(), station_c.clone()];
		station_b.borrow_mut().succ = vec![drain.clone()];
		station_c.borrow_mut().succ = vec![drain.clone()];

		let _s_id = simulator.register_material_flow(source.clone());
		let _a_id = simulator.register_material_flow(station_a.clone());
		let b_id = simulator.register_material_flow(station_b.clone());
		let _c_id = simulator.register_material_flow(station_c.clone());
		let _d_id = simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		// schedule busy station_b to finish at time 100 so it will emit BlockedRetry
		eventcontroller.add_event(
			EcTime::from(100),
			EventTemplate {
				eventtype: EventType::MUExit,
				exec_time: EcTime::from(0),
				object_id: b_id,
				seq: 0,
			},
		);

		eventcontroller.endtime = Some(EcTime::from(1000));
		eventcontroller.init(&mut simulator);
		let sim = eventcontroller.run(simulator).unwrap();

		// Ensure something reached the drain after the blocked-retry flow
		assert!(!sim.drains.is_empty());
		let exits = &sim.drains[0].borrow().exits;
		assert!(exits.len() > 0, "expected at least one exit recorded after blocked retry flow");
	}

	#[test]
	fn source_continues_producing_after_blocked_retry() {
		let mut simulator = Simulator::new();
		let mut eventcontroller = EventController::new(None);

		let station_a = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(60),
			None,
			Vec::new(),
			Vec::new(),
		)));
		// busy station_b that initially denies entrance
		let mut busy_b = SingleStation::new(
			0,
			EcTime::from(75),
			None,
			Vec::new(),
			vec![station_a.clone()],
		);
		busy_b.content = Some(MoveableUnit {
			id: 9999,
			location_id: 0,
		});
		busy_b.entrancelocked = true;
		busy_b.state = crate::flows::ProcessState::Processing;
		let station_b = Rc::new(RefCell::new(busy_b));

		// station_c is free and can accept fallback MUs
		let station_c = Rc::new(RefCell::new(SingleStation::new(
			0,
			EcTime::from(30),
			None,
			Vec::new(),
			vec![station_a.clone()],
		)));

		let source = Rc::new(RefCell::new(Source::new(
			0,
			EcTime::from(30),
			vec![station_a.clone()],
		)));
		let drain = Rc::new(RefCell::new(Drain::new(0, vec![station_b.clone(), station_c.clone()])));

		station_a.borrow_mut().succ = vec![station_b.clone(), station_c.clone()];
		station_b.borrow_mut().succ = vec![drain.clone()];
		station_c.borrow_mut().succ = vec![drain.clone()];

		let _s_id = simulator.register_material_flow(source.clone());
		let _a_id = simulator.register_material_flow(station_a.clone());
		let b_id = simulator.register_material_flow(station_b.clone());
		let _c_id = simulator.register_material_flow(station_c.clone());
		let _d_id = simulator.register_material_flow(drain.clone());
		simulator.add_drain(drain.clone());

		// schedule busy station_b to finish at time 100 so it will emit BlockedRetry
		eventcontroller.add_event(
			EcTime::from(100),
			EventTemplate {
				eventtype: EventType::MUExit,
				exec_time: EcTime::from(0),
				object_id: b_id,
				seq: 0,
			},
		);

		eventcontroller.endtime = Some(EcTime::from(1000));
		eventcontroller.init(&mut simulator);
		let _sim = eventcontroller.run(simulator).unwrap();

		// After the blocked-retry flow the source should have created more than two MUs
		// (i.e., mu_id >= 3)
		assert!(source.borrow().mu_id >= 3, "expected source to have created at least 3 MUs");
	}
}

