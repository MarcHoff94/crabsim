//! What user code costs a run (spec issue 156, PR 2): the demo model with a distribution hook at
//! every object with more than one successor, as WebAssembly and natively, against the same
//! model without code. `pass` answers default, so its run gives exactly the model's own results
//! and the difference is the call alone; `cycle` is round robin written as code.
//!
//! `cargo bench -p crabsim-code --bench calls`, with `-- --days N` for another length.

use crabsim::hooks::ObjectHooks;
use crabsim::{demo, ModelDescription, SimTime};
use crabsim_code::{Code, Library, Options};
use std::sync::Arc;
use std::time::Instant;

fn forks(model: &ModelDescription) -> Vec<u32> {
    (0..model.object_count()).filter(|&id| model.successors(id).len() > 1).collect()
}

fn with_code(library: Library, function: &str) -> ModelDescription {
    let mut model = demo::model();
    let binding = library.bind(function, &serde_json::Map::new(), &|_| None).expect("the function has no parameters");
    let code = Code::new(Arc::new(library), vec![binding], &model, Options::default()).expect("the module runs");
    model.set_hooks(Box::new(code));
    for fork in forks(&model) {
        model.set_object_hooks(fork, ObjectHooks { distribution: Some(0), ..ObjectHooks::default() });
    }
    model
}

struct Measured {
    events: u64,
    calls: u64,
    wall: f64,
    hash: u64,
}

fn measure(model: &ModelDescription, length: SimTime) -> Measured {
    let mut run = model.build(0);
    let start = Instant::now();
    run.init().unwrap();
    run.advance_to(length).unwrap();
    let wall = start.elapsed().as_secs_f64();
    let state = run.state();
    let calls = forks(model).iter().map(|&fork| state.objects[fork as usize].statistics.mus_out).sum();
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{state:?}").hash(&mut hasher);
    Measured { events: run.events_processed(), calls, wall, hash: hasher.finish() }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let days = args.iter().position(|arg| arg == "--days").and_then(|at| args.get(at + 1)).and_then(|days| days.parse().ok()).unwrap_or(30u64);
    let length = SimTime::from_secs(days * 86_400);
    let wasm = std::fs::read(crabsim_code::testing::routing()).expect("the example module builds");
    let library = || {
        let mut library = Library::new();
        library.load_wasm("routing", &wasm).unwrap();
        library
    };
    let native = || {
        let mut library = Library::new();
        library.add_native(crabsim_guest::native_module!("routing", [routing::pass, routing::cycle])).unwrap();
        library
    };
    // `cargo test --all-targets` runs this without `--bench`: then it only checks that a hook
    // answering default changes nothing, both ways.
    if !args.iter().any(|arg| arg == "--bench") {
        let day = SimTime::from_secs(86_400);
        let without = measure(&demo::model(), day).hash;
        assert_eq!(measure(&with_code(library(), "routing.pass"), day).hash, without, "pass in WebAssembly changes nothing");
        assert_eq!(measure(&with_code(native(), "routing.pass"), day).hash, without, "pass natively changes nothing");
        println!("calls: a hook answering default changes nothing, in WebAssembly and natively");
        return;
    }
    let scenarios: Vec<(&str, ModelDescription)> = vec![
        ("without code", demo::model()),
        ("pass, WebAssembly", with_code(library(), "routing.pass")),
        ("pass, native", with_code(native(), "routing.pass")),
        ("cycle, WebAssembly", with_code(library(), "routing.cycle")),
        ("cycle, native", with_code(native(), "routing.cycle")),
    ];
    println!("The demo model for {days} days, a distribution hook at each of its {} forks\n", forks(&demo::model()).len());
    println!("{:<20} {:>12} {:>12} {:>14} {:>14} {:>12}", "scenario", "events/s", "sim s/wall s", "calls", "calls/s", "ns a call");
    let mut baseline: Option<(f64, u64)> = None;
    for (name, model) in &scenarios {
        // The best of three, after one to warm up.
        measure(model, SimTime::from_secs(86_400));
        let best = (0..3).map(|_| measure(model, length)).min_by(|a, b| a.wall.total_cmp(&b.wall)).unwrap();
        let (base_wall, base_hash) = *baseline.get_or_insert((best.wall, best.hash));
        let per_call = if best.calls > 0 && *name != "without code" { format!("{:.0}", (best.wall - base_wall) * 1e9 / best.calls as f64) } else { "-".to_string() };
        println!(
            "{:<20} {:>11.2}M {:>12.0} {:>14} {:>13.2}M {:>12}{}",
            name,
            best.events as f64 / best.wall / 1e6,
            length.as_secs_f64() / best.wall,
            best.calls,
            best.calls as f64 / best.wall / 1e6,
            per_call,
            if name.starts_with("pass") && best.hash != base_hash { "  (results differ!)" } else { "" }
        );
    }
}
