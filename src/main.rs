use std::io;
use std::process::ExitCode;

use clap::Parser;

mod cli;

use cli::Cli;

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => report(&err),
    }
}

/// Print `err` and turn it into an exit status.
///
/// `main` returns `ExitCode` instead of calling `process::exit`, so the runtime
/// still runs destructors and flushes stdio on the way out.
fn report(err: &anyhow::Error) -> ExitCode {
    // A downstream reader closing early (`shelve file.csv | head`) is normal
    // behaviour, not a failure. Rust installs SIG_IGN for SIGPIPE, so the write
    // surfaces as EPIPE instead of killing the process; treat it as a clean exit
    // rather than reporting an error nobody is left to read.
    if is_broken_pipe(err) {
        return ExitCode::SUCCESS;
    }

    // `{:#}` renders the whole source chain on one line, so a wrapped error
    // still explains what failed and why: "cannot open 'x.csv': No such file…".
    eprintln!("Error: {err:#}");

    ExitCode::FAILURE
}

/// True when `err`, or any error in its source chain, is an I/O error caused by
/// the write end of a closed pipe.
///
/// The chain is walked rather than only inspecting the outermost error, because
/// `csv::Error` and `anyhow` contexts both wrap the underlying `io::Error`.
fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|cause| cause.downcast_ref::<io::Error>().is_some_and(|io_err| io_err.kind() == io::ErrorKind::BrokenPipe))
}
