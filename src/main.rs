use clap::Parser;

/// Small CLI for crabsim demo
#[derive(Parser, Debug)]
#[command(name = "crabsim", about = "Discrete-event material-flow simulator demo")]
struct Opts {
    /// Update the state table in-place
    #[arg(short = 'i', long = "inplace")]
    inplace: bool,

    /// Launch external renderer window
    #[arg(long = "renderer-window")]
    renderer_window: bool,

    /// Renderer pacing factor (sim seconds per wall-second)
    #[arg(long = "speed", default_value = "5.0")]
    speed: String,

    /// Hold renderer window open after finish (set 0 to disable)
    #[arg(long = "hold", default_value = "1")]
    hold: String,
}

fn main() {
    let opts = Opts::parse();
    if opts.inplace {
        std::env::set_var("CRABSIM_LIVE_INPLACE", "1");
    }
    if opts.renderer_window {
        std::env::set_var("CRABSIM_RENDERER", "1");
    }
    if !opts.speed.is_empty() {
        std::env::set_var("CRABSIM_RENDERER_SPEED", &opts.speed);
    }
    if !opts.hold.is_empty() {
        std::env::set_var("CRABSIM_RENDERER_HOLD", &opts.hold);
    }
    crabsim::run_demo();
}
