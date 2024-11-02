use clap::Parser;
use std::process;

mod cli;
mod groups;
use cli::Cli;

fn main() {
    let cli = Cli::parse();

    if let Err(err) = cli.run() {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}
