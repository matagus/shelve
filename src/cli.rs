use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;

use shelve::{ColumnNumber, GroupedData};

/// Characters that collide with CSV syntax and must not be accepted as a
/// field delimiter.
///
/// The quote character is the most dangerous: using it as the delimiter
/// silently produces garbage because the parser treats every field boundary
/// as a quoted-field toggle. Newlines break record framing, and whitespace
/// delimiters are ambiguous in most real-world CSVs.
const FORBIDDEN_DELIMITERS: &[char] = &['"', '\r', '\n'];

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
    #[arg(short, long, default_value = ",", value_parser = parse_delimiter)]
    pub delimiter: char,
}

/// Reject delimiter characters that collide with CSV syntax.
///
/// Returning a `String` error makes clap print it as a usage error (exit 2),
/// matching how column zero is already rejected at parse time via
/// [`ColumnNumber`].
fn parse_delimiter(s: &str) -> std::result::Result<char, String> {
    let ch = s.chars().next().ok_or_else(|| "delimiter must be a single character".to_owned())?;
    if s.len() != ch.len_utf8() {
        return Err("delimiter must be a single character".to_owned());
    }
    if FORBIDDEN_DELIMITERS.contains(&ch) {
        return Err(format!(
            "'{ch}' is not a valid delimiter: it collides with CSV syntax (quote or newline)"
        ));
    }
    Ok(ch)
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
