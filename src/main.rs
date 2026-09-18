use std::error::Error;
use std::io;
use std::process;

use clap::Parser;

mod cli;
mod groups;

use cli::Cli;

fn main() {
    let cli = Cli::parse();

    if let Err(err) = cli.run() {
        // A downstream reader closing early (`shelve file.csv | head`) is normal
        // behaviour, not a failure. Rust installs SIG_IGN for SIGPIPE, so the
        // write surfaces as EPIPE instead of killing the process; treat it as a
        // clean exit rather than reporting an error nobody is left to read.
        if is_broken_pipe(&*err) {
            return;
        }

        eprintln!("Error: {err}");
        process::exit(1);
    }
}

/// True when `err` is an I/O error caused by the write end of a closed pipe.
///
/// The explicit `'static` bound is required: in argument position a bare
/// `&dyn Error` defaults to the lifetime of the reference, and `downcast_ref`
/// can only inspect trait objects with a `'static` lifetime.
fn is_broken_pipe(err: &(dyn Error + 'static)) -> bool {
    err.downcast_ref::<io::Error>().is_some_and(|io_err| io_err.kind() == io::ErrorKind::BrokenPipe)
}
