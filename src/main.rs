use clap::Parser;
use kairo::cli::{run_cli, Cli};

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run_cli(cli) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
