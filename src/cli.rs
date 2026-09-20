use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

use shelve::{ColumnNumber, GroupedData};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    // `PathBuf`, not `String`: on Unix a filename is an arbitrary byte string,
    // and clap rejects any argument that is not valid UTF-8.
    pub filenames: Vec<PathBuf>,

    /// Column number to group by
    #[arg(short, long, default_value = "1")]
    pub column_number: ColumnNumber,

    /// Treat the first record as data instead of a header row
    #[arg(long)]
    pub no_headers: bool,

    /// Field delimiter character (default: ',')
    #[arg(short, long, default_value = ",")]
    pub delimiter: char,
}

impl Cli {
    pub fn run(&self) -> Result<()> {
        let groups: GroupedData =
            GroupedData::from_files(&self.filenames, self.column_number, self.no_headers, self.delimiter)?;

        // Use a BufWriter to improve performance by reducing the number of write calls
        let stdout = io::stdout();
        let mut stream = io::BufWriter::new(stdout);

        groups.write_to(&mut stream)?;

        // Flush explicitly: an error that only surfaces when the `BufWriter` is
        // dropped would be swallowed, and a closed stdout pipe would then go
        // unreported instead of reaching the broken-pipe handling in `main`.
        stream.flush()?;

        Ok(())
    }
}
