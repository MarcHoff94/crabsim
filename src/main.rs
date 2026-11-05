use clap::Parser;

/// Small CLI for crabsim demo
#[derive(Parser, Debug)]
#[command(name = "crabsim", about = "Discrete-event material-flow simulator demo")]
struct Opts {
    /// Update the state table in-place
    #[arg(short = 'i', long = "inplace")]
    inplace: bool,
}

fn main() {
    let opts = Opts::parse();
    if let Err(error) = crabsim::run_demo(opts.inplace) {
        eprintln!("the simulation stopped: {error}");
        std::process::exit(1);
    }
}
