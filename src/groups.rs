use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fs::File;

use anyhow::{Context, Result};

#[derive(Debug)]
pub struct Row {
    data: Vec<String>,
    index: usize,
}

impl std::fmt::Display for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut cloned_data = self.data.clone();
        cloned_data.remove(self.index - 1);
        write!(f, "{}", cloned_data.join(", "))
    }
}

#[derive(Debug)]
pub struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
    index: usize,
    warned_missing_column: bool,
}

impl Row {
    fn new(data: Vec<String>, index: usize) -> Self {
        Row { data, index }
    }
}

impl GroupedData {
    fn new(index: usize) -> Self {
        GroupedData {
            groups: BTreeMap::new(),
            index,
            warned_missing_column: false,
        }
    }

    fn process<R: std::io::Read>(&mut self, rdr: &mut csv::Reader<R>) -> Result<()> {
        for result in rdr.records() {
            let record = result?;

            match record.get(self.index - 1) {
                Some(key) => {
                    let row = Row::new(record.iter().map(ToString::to_string).collect(), self.index);
                    self.add(key, row);
                }
                None => self.warn_missing_column(),
            }
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
            self.index
        );
    }

    pub fn from_files(filename_vec: &[String], index: usize) -> Result<Self> {
        let mut groups = GroupedData::new(index);

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
        match self.groups.entry(group_name.to_string()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().push(row);
            }
            Entry::Vacant(entry) => {
                entry.insert(vec![row]);
            }
        }
    }

    pub fn get_groups(&self) -> Vec<&String> {
        self.groups.keys().collect()
    }

    pub fn get_rows(&self, group_name: &str) -> Option<&Vec<Row>> {
        self.groups.get(group_name)
    }
}
