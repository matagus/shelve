use clap::Parser;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
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

    let index = cli.column_number.unwrap() as usize;

    if let Err(err) = run(&cli.filenames, index) {
        eprintln!("Error: {}", err);
        process::exit(1);
    }
}

#[derive(Debug)]
struct Row {
    data: Vec<String>,
}

#[derive(Debug)]
struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
}

impl Row {
    fn new(data: Vec<String>) -> Self {
        Row { data }
    }

    fn join_without_col(&self, separator: &str, index: usize) -> String {
        let mut data = self.data.clone();
        data.remove(index);
        data.join(separator)
    }
}

impl GroupedData {
    fn new() -> Self {
        GroupedData {
            groups: BTreeMap::new(),
        }
    }

    fn add(&mut self, group_name: &str, row: Row) {
        match self.groups.entry(group_name.to_string()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(row);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![row]);
            }
        }
    }

    fn get_groups(&self) -> Vec<&String> {
        self.groups.keys().collect()
    }

    fn get_rows(&self, group_name: &str) -> Option<&Vec<Row>> {
        self.groups.get(group_name)
    }
}

fn run(filename_vec: &[String], index: usize) -> Result<(), Box<dyn Error>> {
    let mut groups: GroupedData = GroupedData::new();

    if filename_vec.is_empty() {
        let stdin = std::io::stdin().lock();
        let mut rdr = csv::Reader::from_reader(stdin);
        process_reader(&mut rdr, &mut groups, index)?;
    } else {
        for filename in filename_vec {
            // Create a CSV reader
            let mut rdr = csv::Reader::from_reader(File::open(filename)?);
            process_reader(&mut rdr, &mut groups, index)?;
        }
    }

    // Use a BufWriter to improve performance by reducing the number of write calls
    let stdout = io::stdout();
    let mut stream = io::BufWriter::new(stdout);

    for group in &groups.get_groups() {
        writeln!(stream, "{}:\n", group)?;

        if let Some(rows) = groups.get_rows(group) {
            for row in rows {
                let display_row = row.join_without_col(", ", index);
                writeln!(stream, "{}", display_row)?;
            }
        }
        writeln!(stream)?;
    }

    Ok(())
}

fn process_reader<R: std::io::Read>(
    rdr: &mut csv::Reader<R>,
    groups: &mut GroupedData,
    index: usize,
) -> Result<(), Box<dyn Error>> {
    for result in rdr.records() {
        let record = result?;

        // Check if the column index is within bounds
        if index >= record.len() {
            return Err(format!("Column index {} is out of bounds", index).into());
        }

        let key = record[index].to_string();
        let row = Row::new(record.iter().map(|s| s.to_string()).collect());
        groups.add(&key, row);
    }
    Ok(())
}
