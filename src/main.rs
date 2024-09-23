use clap::Parser;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::{self, Write};
use std::process;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    filenames: Vec<String>,

    /// Column number to group by
    #[arg(short, long, default_value = "0")]
    column_number: Option<u8>,
}

fn main() {
    let cli = Cli::parse();

    let column_index = cli.column_number.unwrap() as usize;

    if let Err(err) = run(&cli.filenames, column_index) {
        println!("Error: {}", err);
        process::exit(1);
    }
}

fn run(filename_vec: &[String], column_index: usize) -> Result<(), Box<dyn Error>> {
    // Create a HashMap to store groups
    let mut groups: HashMap<String, Vec<Vec<String>>> = HashMap::new();

    if filename_vec.is_empty() {
        let stdin = std::io::stdin().lock();
        let mut rdr = csv::Reader::from_reader(stdin);
        process_reader(&mut rdr, &mut groups, column_index)?;
    } else {
        for filename in filename_vec {
            // Create a CSV reader
            let mut rdr = csv::Reader::from_reader(File::open(filename)?);
            process_reader(&mut rdr, &mut groups, column_index)?;
        }
    }

    // Use a BufWriter to improve performance by reducing the number of write calls
    let stdout = io::stdout();
    let mut stream = io::BufWriter::new(stdout);

    // Sort groups by key
    let mut sorted_groups: Vec<_> = groups.into_iter().collect();
    sorted_groups.sort_by(|a, b| a.0.cmp(&b.0));

    for (key, group) in sorted_groups {
        writeln!(stream, "{}:\n", key)?;

        for row in group {
            let row_to_display = match column_index {
                0 => row[1..].join(", "),
                _ => row[..column_index].join(", ") + ", " + &row[column_index..].join(", "),
            };
            writeln!(stream, "{}", row_to_display)?;
        }
        writeln!(stream)?;
    }

    Ok(())
}

fn process_reader<R: std::io::Read>(
    rdr: &mut csv::Reader<R>,
    groups: &mut HashMap<String, Vec<Vec<String>>>,
    column_index: usize,
) -> Result<(), Box<dyn Error>> {
    for result in rdr.records() {
        let record = result?;

        // Check if the column index is within bounds
        if column_index >= record.len() {
            return Err(format!("Column index {} is out of bounds", column_index).into());
        }

        let key = record[column_index].to_string();
        let group = groups.entry(key).or_insert_with(Vec::new);
        group.push(record.iter().map(|s| s.to_string()).collect());
    }
    Ok(())
}
