use clap::Parser;
use std::error::Error;
use std::io::{self, Write};
use std::process;

mod groups;
use groups::GroupedData;
mod cli;
use cli::Cli;

fn main() {
    let cli = Cli::parse();

    let index = cli.column_number.unwrap() as usize;

    if let Err(err) = run(&cli.filenames, index) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}

fn run(filename_vec: &[String], index: usize) -> Result<(), Box<dyn Error>> {
    let groups: GroupedData = GroupedData::from_files(filename_vec, index)?;

    // Use a BufWriter to improve performance by reducing the number of write calls
    let stdout = io::stdout();
    let mut stream = io::BufWriter::new(stdout);

    for group in &groups.get_groups() {
        writeln!(stream, "{}:\n", group)?;

        if let Some(rows) = groups.get_rows(group) {
            for row in rows {
                writeln!(stream, "{}", row)?;
            }
        }
        writeln!(stream)?;
    }

    Ok(())
}
