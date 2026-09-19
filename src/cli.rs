use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::groups::{ColumnNumber, GroupedData};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    // `PathBuf`, not `String`: on Unix a filename is arbitrary bytes, so a
    // `Vec<String>` field makes valid paths unrepresentable — clap rejects the
    // argument with "invalid UTF-8" before `run` ever sees it. A doc comment
    // would leak into `--help`, which the tests compare verbatim.
    pub filenames: Vec<PathBuf>,

    /// Column number to group by
    #[arg(short, long, default_value = "1")]
    pub column_number: ColumnNumber,
}

impl Cli {
    pub fn run(&self) -> Result<()> {
        let groups: GroupedData = GroupedData::from_files(&self.filenames, self.column_number)?;

        // Use a BufWriter to improve performance by reducing the number of write calls
        let stdout = io::stdout();
        let mut stream = io::BufWriter::new(stdout);

        for (group, rows) in groups.groups() {
            writeln!(stream, "{group}:\n")?;

            // `GroupedData` owns the grouping column's index, so it formats the
            // rows too: a `Display` impl on `Row` would have to be told which
            // field to omit by whoever happened to print it.
            groups.write_rows(rows, &mut stream)?;
            writeln!(stream)?;
        }

        // Flush explicitly: an error that only surfaces when the `BufWriter` is
        // dropped would be swallowed, and a closed stdout pipe would then go
        // unreported instead of reaching the broken-pipe handling in `main`.
        stream.flush()?;

        Ok(())
    }
}
