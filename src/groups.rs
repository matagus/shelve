use std::collections::BTreeMap;
use std::fs::File;

use anyhow::{Context, Result};

#[derive(Debug)]
pub struct Row {
    data: Vec<String>,
    /// 0-based index of the grouping column, omitted when printing.
    index: usize,
}

impl std::fmt::Display for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // Skip the grouping column while printing rather than cloning the
        // whole row and shifting every entry to remove one field.
        let values: Vec<&str> =
            self.data.iter().enumerate().filter(|(i, _)| *i != self.index).map(|(_, value)| value.as_str()).collect();
        write!(f, "{}", values.join(", "))
    }
}

#[derive(Debug)]
pub struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
    /// 0-based index of the grouping column.
    index: usize,
    warned_missing_column: bool,
}

impl Row {
    fn new(data: Vec<String>, index: usize) -> Self {
        Row { data, index }
    }
}

impl GroupedData {
    /// `column_number` is the 1-based number the CLI exposes. It is converted
    /// to a 0-based index exactly once, here, so no other code has to
    /// subtract one (or risk underflowing on an unvalidated zero).
    fn new(column_number: usize) -> Self {
        GroupedData {
            groups: BTreeMap::new(),
            index: column_number - 1,
            warned_missing_column: false,
        }
    }

    fn process<R: std::io::Read>(&mut self, rdr: &mut csv::Reader<R>) -> Result<()> {
        for result in rdr.records() {
            let record = result?;

            let Some(key) = record.get(self.index) else {
                self.warn_missing_column();
                continue;
            };

            let row = Row::new(record.iter().map(ToString::to_string).collect(), self.index);
            self.add(key, row);
        }

        Ok(())
    }

    /// Report the first record that lacks the grouping column. Only the first
    /// occurrence is printed, so a wide input cannot emit one line per row.
    ///
    /// Skipping is still the right outcome, but it has to be announced:
    /// otherwise empty stdout with exit code 0 is indistinguishable from an
    /// empty input file.
    fn warn_missing_column(&mut self) {
        if self.warned_missing_column {
            return;
        }
        self.warned_missing_column = true;

        eprintln!(
            "Warning: column {} is missing from at least one record; those rows were skipped",
            self.index + 1
        );
    }

    /// `column_number` is the 1-based grouping column from the CLI.
    pub fn from_files(filename_vec: &[String], column_number: usize) -> Result<Self> {
        let mut groups = GroupedData::new(column_number);

        if filename_vec.is_empty() {
            let stdin = std::io::stdin().lock();
            let mut rdr = csv::Reader::from_reader(stdin);
            groups.process(&mut rdr)?;
        } else {
            for filename in filename_vec {
                // Name the file: with several arguments a bare "No such file or
                // directory" leaves no way to tell which one failed. The same
                // context covers parse errors, which are otherwise reported
                // without saying which input they came from.
                let file = File::open(filename).with_context(|| format!("cannot open '{filename}'"))?;
                let mut rdr = csv::Reader::from_reader(file);
                groups.process(&mut rdr).with_context(|| format!("cannot read '{filename}'"))?;
            }
        }

        Ok(groups)
    }

    pub fn add(&mut self, group_name: &str, row: Row) {
        self.groups.entry(group_name.to_string()).or_default().push(row);
    }

    /// The groups in key order, borrowed straight from the map. Iterating
    /// this is all a caller needs: no key Vec to allocate and no per-group
    /// lookup afterwards.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &[Row])> + '_ {
        self.groups.iter().map(|(name, rows)| (name.as_str(), rows.as_slice()))
    }
}
