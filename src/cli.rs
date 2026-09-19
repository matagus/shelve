use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};

use crate::groups::{ColumnNumber, GroupedData};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    pub filenames: Vec<String>,

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

            for row in rows {
                writeln!(stream, "{row}")?;
            }
            writeln!(stream)?;
        }

        // Flush explicitly: an error that only surfaces when the `BufWriter` is
        // dropped would be swallowed, and a closed stdout pipe would then go
        // unreported instead of reaching the broken-pipe handling in `main`.
        stream.flush()?;

        Ok(())
    }
}
