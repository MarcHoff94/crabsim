fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--inplace" || a == "-i") {
        std::env::set_var("CRABSIM_LIVE_INPLACE", "1");
    }
    if args.iter().any(|a| a == "--renderer-window") {
        std::env::set_var("CRABSIM_RENDERER", "1");
    }
    if let Some(i) = args.iter().position(|a| a == "--speed") {
        if let Some(val) = args.get(i + 1) { std::env::set_var("CRABSIM_RENDERER_SPEED", val); }
    }
    crabsim::run_demo();
}
