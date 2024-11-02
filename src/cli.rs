use clap::Parser;
use std::error::Error;
use std::io::{self, Write};

use crate::groups::GroupedData;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    pub filenames: Vec<String>,

    /// Column number to group by
    #[arg(short, long, default_value = "1")]
    pub column_number: Option<u8>,
}

impl Cli {
    pub fn run(&self) -> Result<(), Box<dyn Error>> {
        let column_number: usize = self.column_number.unwrap_or(1).into();

        if column_number == 0 {
            return Err("Column number must be greater than 0".into());
        }

        let groups: GroupedData = GroupedData::from_files(&self.filenames, column_number)?;

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
}
