use clap::Parser;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::process;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    filename: String,

    /// Column number to group by
    #[arg(short, long, default_value = "0")]
    column_number: Option<u8>,
}

fn main() {
    let cli = Cli::parse();

    let column_index = cli.column_number.unwrap() as usize;

    if let Err(err) = run(&cli.filename, column_index) {
        println!("Error: {}", err);
        process::exit(1);
    }
}

fn run(file_path: &str, column_index: usize) -> Result<(), Box<dyn Error>> {
    // Open the CSV file
    let file = File::open(file_path)?;

    // Create a CSV reader
    let mut rdr = csv::Reader::from_reader(file);

    // Create a HashMap to store groups
    let mut groups: HashMap<String, Vec<Vec<String>>> = HashMap::new();

    // Iterate over each record
    for result in rdr.records() {
        let record = result?;

        // Check if the column index is within bounds
        if column_index >= record.len() {
            return Err(format!("Column index {} is out of bounds", column_index).into());
        }

        // Get the key for grouping
        let key = record[column_index].to_string();

        // Insert the record into the appropriate group
        groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push(record.iter().map(|s| s.to_string()).collect());
    }

    // Sort groups by key
    let mut sorted_groups: Vec<_> = groups.into_iter().collect();
    sorted_groups.sort_by(|a, b| a.0.cmp(&b.0));

    for (key, group) in sorted_groups {
        println!("{}:\n", key);
        for row in group {
            let row_to_display = match column_index {
                0 => row[1..].join(", "),
                _ => row[..column_index].join(", ") + ", " + &row[column_index..].join(", "),
            };
            println!("{}", row_to_display);
        }
        println!();
    }

    Ok(())
}
