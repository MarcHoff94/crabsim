//! A small example model, run by the headless CLI and drawn by the UI.

use crate::flows::FailureProfile;
use crate::materialflow_objects::{ExitStrategy, StartMode};
use crate::model::{ModelDescription, Position};
use crate::report::Capacity;
use crate::random_numbers::Distribution;
use crate::simulation_run::SimulationRun;
use crate::time::SimTime;

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

/// A second example, with Buffers and a Station with capacity: a line of three Stations with a
/// store of 10 places between the first two, an oven baking up to 6 MUs at once after the second, and a
/// cooling section, where every MU stays a minute and the newest is taken first once several may
/// leave. `models/buffered-line.json` is the same model as a file, and the benchmark runs it.
pub fn buffered_line() -> ModelDescription {
    let mut model = ModelDescription::new();
    let place = |model: &mut ModelDescription, id: u32, name: &str, x: f64| {
        model.set_name(id, name);
        model.set_position(id, Position { x, y: 0.0, z: 0.0 });
        id
    };
    let failures = |availability: f32, repair_secs: f64| FailureProfile::new(availability, Distribution::ERLANG { mean: repair_secs * 1_000.0, k: 2 }, 0);
    let source = model.add_source(SimTime::from_secs(5));
    place(&mut model, source, "Source", 0.0);
    let saw = model.add_station(SimTime::from_secs(4), failures(0.9, 60.0));
    place(&mut model, saw, "Saw", 4.0);
    let store = model.add_buffer(Capacity::Limited(10));
    place(&mut model, store, "Store", 8.0);
    let mill = model.add_station(SimTime::from_secs(5), failures(0.85, 90.0));
    place(&mut model, mill, "Mill", 12.0);
    // Added after the others, so it has the next stream, as a model file gives it, and the others
    // keep theirs.
    let oven = model.add_station(SimTime::from_secs(25), FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0));
    model.set_station_capacity(oven, 6, StartMode::Individual);
    place(&mut model, oven, "Oven", 16.0);
    model.set_stream(oven, 7);
    let cooling = model.add_buffer(Capacity::Unlimited);
    model.set_dwelling_time(cooling, SimTime::from_secs(60));
    model.set_exit_strategy(cooling, ExitStrategy::LastInFirstOut);
    place(&mut model, cooling, "Cooling", 20.0);
    let pack = model.add_station(SimTime::from_secs(4), failures(0.95, 30.0));
    place(&mut model, pack, "Pack", 24.0);
    let drain = model.add_drain();
    place(&mut model, drain, "Drain", 28.0);
    for (id, stream) in [(cooling, 4), (pack, 5), (drain, 6)] {
        model.set_stream(id, stream);
    }
    for (from, to) in [(source, saw), (saw, store), (store, mill), (mill, oven), (oven, cooling), (cooling, pack), (pack, drain)] {
        model.connect(from, to);
    }
    model
}

/// A line with conveyors (spec issue 133), for the benchmark's `conveyor-line` scenario and the
/// conveyor's tests: a Source making a 0.6 by 0.4 m tote every 5 s, a feed conveyor to a Saw that
/// fails now and then, a chain of two conveyors, the second slower, to a Mill, the slowest machine,
/// so totes accumulate in front of it, and an outfeed conveyor to the Drain. Every conveyor
/// accumulates.
pub fn conveyor_line() -> ModelDescription {
    let mut model = ModelDescription::new();
    let place = |model: &mut ModelDescription, id: u32, name: &str, x: f64| {
        model.set_name(id, name);
        model.set_position(id, Position { x, y: 0.0, z: 0.0 });
        id
    };
    let failures = |availability: f32, repair_secs: f64| FailureProfile::new(availability, Distribution::ERLANG { mean: repair_secs * 1_000.0, k: 2 }, 0);
    let tote = model.add_mu_kind(crate::model::MuKind { length: 0.6, width: 0.4, booking_point: 0.5 });
    let source = model.add_source(SimTime::from_secs(5));
    model.set_mu_kind(source, tote);
    place(&mut model, source, "In", 0.0);
    let feed = model.add_conveyor(10.0, 0.6, 0.5);
    model.set_conveyor_gap(feed, 0.1);
    place(&mut model, feed, "Feed", 6.5);
    let saw = model.add_station(SimTime::from_secs(4), failures(0.9, 60.0));
    place(&mut model, saw, "Saw", 13.0);
    let link = model.add_conveyor(6.0, 0.6, 0.8);
    place(&mut model, link, "Link", 17.0);
    let approach = model.add_conveyor(6.0, 0.6, 0.4);
    place(&mut model, approach, "Approach", 23.0);
    let mill = model.add_station(SimTime::from_secs(6), failures(0.85, 90.0));
    place(&mut model, mill, "Mill", 27.0);
    let outfeed = model.add_conveyor(8.0, 0.6, 0.8);
    place(&mut model, outfeed, "Outfeed", 32.0);
    let drain = model.add_drain();
    place(&mut model, drain, "Out", 37.0);
    for (from, to) in [(source, feed), (feed, saw), (saw, link), (link, approach), (approach, mill), (mill, outfeed), (outfeed, drain)] {
        model.connect(from, to);
    }
    model
}

/// A line with cross conveyors (spec issue 148), for the benchmark's `cross-line` scenario: Housings
/// every 8 s and Covers every 12 s are sawn, then a feed conveyor brings both to a cross conveyor,
/// the Fork. Housings, whose work plan goes on to the Mill, turn left there onto a spur, lying
/// crosswise, and after the Mill turn right at a second cross conveyor, the Corner, and left again
/// into the Join. Covers go straight on to Paint and straight through the Join. The Join's front
/// leads to the Drain. `models/cross-line.json` is the same model as a file.
pub fn cross_line() -> ModelDescription {
    use crate::materialflow_objects::Side;
    use crate::routing::PlanStep;
    let mut model = ModelDescription::new();
    let place = |model: &mut ModelDescription, id: u32, name: &str, x: f64, y: f64| {
        model.set_name(id, name);
        model.set_position(id, Position { x, y, z: 0.0 });
        id
    };
    let never = || FailureProfile::new(1.0, Distribution::CONSTANT { value: 0.0 }, 0);
    let failures = |availability: f32, repair_secs: f64| FailureProfile::new(availability, Distribution::ERLANG { mean: repair_secs * 1_000.0, k: 2 }, 0);
    let in_a = model.add_source(SimTime::from_secs(8));
    place(&mut model, in_a, "In A", 0.0, 1.0);
    let in_b = model.add_source(SimTime::from_secs(12));
    place(&mut model, in_b, "In B", 0.0, -1.0);
    let saw = model.add_station(SimTime::from_secs(4), never());
    place(&mut model, saw, "Saw", 2.5, 0.0);
    let feed = model.add_conveyor(8.0, 0.6, 0.5);
    place(&mut model, feed, "Feed", 8.0, 0.0);
    let fork = model.add_cross_conveyor(0.6, 0.6, 0.5);
    place(&mut model, fork, "Fork", 12.3, 0.0);
    let spur = model.add_conveyor(3.0, 0.6, 0.5);
    place(&mut model, spur, "Spur", 12.3, 1.8);
    let mill = model.add_station(SimTime::from_secs(7), failures(0.95, 60.0));
    place(&mut model, mill, "Mill", 12.3, 4.3);
    let over = model.add_conveyor(6.4, 0.6, 0.5);
    place(&mut model, over, "Over", 16.5, 4.3);
    let corner = model.add_cross_conveyor(0.6, 0.6, 0.5);
    place(&mut model, corner, "Corner", 20.0, 4.3);
    let down = model.add_conveyor(3.7, 0.6, 0.5);
    place(&mut model, down, "Down", 20.0, 2.15);
    let lane = model.add_conveyor(2.0, 0.6, 0.5);
    place(&mut model, lane, "Lane", 13.6, 0.0);
    let paint = model.add_station(SimTime::from_secs(10), failures(0.9, 45.0));
    place(&mut model, paint, "Paint", 15.6, 0.0);
    let dry = model.add_conveyor(3.1, 0.6, 0.5);
    place(&mut model, dry, "Dry", 18.15, 0.0);
    let join = model.add_cross_conveyor(0.6, 0.6, 0.5);
    place(&mut model, join, "Join", 20.0, 0.0);
    let outfeed = model.add_conveyor(4.0, 0.6, 0.5);
    place(&mut model, outfeed, "Outfeed", 22.3, 0.0);
    let out = model.add_drain();
    place(&mut model, out, "Out", 24.8, 0.0);
    // Destinations in the order a model file names them: its classes' plans, then its exits.
    let [to_saw, to_mill, to_out, to_paint] = [saw, mill, out, paint].map(|id| model.add_destination(&[id]));
    let step = |at| PlanStep { at, process_time: None };
    let housing = model.add_mu_kind_with_plan(crate::model::MuKind::default(), vec![step(to_saw), step(to_mill), step(to_out)]);
    let cover = model.add_mu_kind_with_plan(crate::model::MuKind::default(), vec![step(to_saw), step(to_paint), step(to_out)]);
    model.set_mu_kind(in_a, housing);
    model.set_mu_kind(in_b, cover);
    model.set_exit_destinations(fork, Side::Left, &[to_mill]);
    model.set_exit_destinations(corner, Side::Right, &[to_out]);
    model.set_exit_destinations(join, Side::Front, &[to_out]);
    for (from, to) in [(in_a, saw), (in_b, saw), (saw, feed), (spur, mill), (mill, over), (lane, paint), (paint, dry), (outfeed, out)] {
        model.connect(from, to);
    }
    model.connect_at(feed, None, fork, Some(Side::Back));
    model.connect_at(fork, Some(Side::Left), spur, None);
    model.connect_at(fork, Some(Side::Front), lane, None);
    model.connect_at(over, None, corner, Some(Side::Back));
    model.connect_at(corner, Some(Side::Right), down, None);
    model.connect_at(down, None, join, Some(Side::Left));
    model.connect_at(dry, None, join, Some(Side::Back));
    model.connect_at(join, Some(Side::Front), outfeed, None);
    model
}
