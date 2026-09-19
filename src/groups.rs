use std::collections::BTreeMap;
use std::fs::File;
use std::num::NonZeroUsize;
use std::str::FromStr;

use anyhow::{Context, Result};

/// The grouping column picked on the command line, counting from 1.
///
/// Wrapping [`NonZeroUsize`] makes column zero unrepresentable, so the 1-based
/// (CLI) to 0-based (index) conversion has exactly one owner — [`ColumnNumber::index`]
/// — and can no longer underflow there. `0` is rejected while parsing the
/// argument, which turns it into a clap usage error instead of a runtime check
/// in a different module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnNumber(NonZeroUsize);

impl ColumnNumber {
    /// The 0-based index this column number refers to.
    #[must_use]
    pub fn index(self) -> usize {
        self.0.get() - 1
    }
}

impl FromStr for ColumnNumber {
    type Err = <NonZeroUsize as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        NonZeroUsize::from_str(s).map(Self)
    }
}

impl std::fmt::Display for ColumnNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
    /// to a 0-based index exactly once, here, so no other code has to subtract
    /// one: the newtype cannot hold a zero, so the subtraction cannot underflow.
    fn new(column_number: ColumnNumber) -> Self {
        GroupedData {
            groups: BTreeMap::new(),
            index: column_number.index(),
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

    /// `column_number` is the 1-based grouping column from the CLI. It is typed
    /// so that the zero the CLI must not accept cannot be spelled here either.
    pub fn from_files(filename_vec: &[String], column_number: ColumnNumber) -> Result<Self> {
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
