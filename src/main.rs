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
    #[arg(long = "speed", default_value = "5.0", allow_hyphen_values = true)]
    speed: String,

    /// Hold renderer window open after finish (set 0 to disable)
    #[arg(long = "hold", default_value = "1")]
    hold: String,

    /// Choose the renderer launcher (powershell or cmd)
    #[arg(long = "renderer-launcher")]
    renderer_launcher: Option<String>,
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
    if let Some(launcher) = opts.renderer_launcher {
        std::env::set_var("CRABSIM_RENDERER_LAUNCHER", launcher);
    }
    crabsim::run_demo();
}
