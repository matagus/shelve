use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
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
    /// This row's fields, owned outright.
    ///
    /// A `StringRecord` already owns its data in a single allocation and
    /// iterates as `&str`, so copying every field into a `Vec<String>` would
    /// duplicate the entire input for nothing.
    data: csv::StringRecord,
}

impl Row {
    fn new(data: csv::StringRecord) -> Self {
        Row { data }
    }

    /// Write this row's fields to `out`, separated by `", "` and omitting the
    /// field at `skip`.
    ///
    /// `skip` is a parameter rather than a field because it belongs to the
    /// collection: every row in every group omits the same column and
    /// [`GroupedData`] already stores which one, so a per-row copy duplicated
    /// one `usize` per record for no reader's benefit.
    ///
    /// Each kept field goes straight to the writer. Collecting them into a
    /// `Vec<&str>` and joining the result would allocate twice per printed line
    /// to produce exactly these bytes.
    fn write_to(&self, out: &mut impl Write, skip: usize) -> io::Result<()> {
        let mut first = true;

        for (index, value) in self.data.iter().enumerate() {
            if index == skip {
                continue;
            }
            if first {
                first = false;
            } else {
                write!(out, ", ")?;
            }
            write!(out, "{value}")?;
        }

        writeln!(out)
    }
}

#[derive(Debug)]
pub struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
    /// 0-based index of the grouping column.
    index: usize,
    warned_missing_column: bool,
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

            // Own the key before the record moves into the row: `add` needs
            // both, and the key was borrowed from the record that is about to
            // become the row's data.
            let key = key.to_owned();
            let row = Row::new(record);
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

    /// The group name is taken by value. The caller has to own it anyway — it
    /// is borrowed from the record the row takes ownership of — and receiving
    /// it owned means the map entry no longer clones the name on every row,
    /// including rows for a group that already exists.
    pub fn add(&mut self, group_name: String, row: Row) {
        self.groups.entry(group_name).or_default().push(row);
    }

    /// The groups in key order, borrowed straight from the map. Iterating
    /// this is all a caller needs: no key Vec to allocate and no per-group
    /// lookup afterwards.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &[Row])> + '_ {
        self.groups.iter().map(|(name, rows)| (name.as_str(), rows.as_slice()))
    }

    /// Print `rows` one per line, without the grouping column.
    ///
    /// Formatting lives here rather than in a `Display` impl on [`Row`] because
    /// the index to omit is this struct's state: a row cannot name it on its own
    /// any more, and the caller should not have to thread it back in by hand.
    pub fn write_rows(&self, rows: &[Row], out: &mut impl Write) -> io::Result<()> {
        for row in rows {
            row.write_to(out, self.index)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{ColumnNumber, GroupedData};

    /// Counts allocations, so a test can assert on the *shape* of the work and
    /// not only on its output. This is installed for the unit-test binary only;
    /// the integration tests under `tests/` are a separate binary and are
    /// unaffected.
    struct CountingAllocator;

    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAllocator = CountingAllocator;

    /// Run `f` and report how many allocations it made. Every measurement is a
    /// fresh count, so callers never have to reason about what ran before.
    fn count_allocations<R>(f: impl FnOnce() -> R) -> (R, usize) {
        ALLOCATIONS.store(0, Ordering::SeqCst);
        let value = f();
        (value, ALLOCATIONS.load(Ordering::SeqCst))
    }

    fn parse(csv_text: &str) -> GroupedData {
        let mut rdr = csv::Reader::from_reader(csv_text.as_bytes());
        let mut groups = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));
        groups.process(&mut rdr).expect("in-memory CSV reads");
        groups
    }

    /// A `columns`-wide CSV with `rows` records. Column 1 is the grouping key
    /// and only ever takes two values, so every width builds the same two
    /// groups and the map cost is identical across the comparison.
    fn build_csv(rows: usize, columns: usize) -> String {
        let mut out = String::new();
        for row in 0..rows {
            write!(out, "g{}", row % 2).expect("writing to a String cannot fail");
            for column in 1..columns {
                write!(out, ",r{row}c{column}").expect("writing to a String cannot fail");
            }
            out.push('\n');
        }
        out
    }

    /// Parsing must not get more expensive per row as the input gets wider.
    ///
    /// Copying each field out of the `StringRecord` into a `Vec<String>` costs
    /// one allocation per field, so widening the row from 4 to 64 columns adds
    /// roughly `rows * 60` allocations. Storing the record itself makes the
    /// count essentially width-independent. The bound is deliberately loose —
    /// it fails by an order of magnitude on the copying code and passes by an
    /// even larger margin without it — so it cannot flake on allocator details.
    #[test]
    fn allocations_do_not_scale_with_column_count() {
        const ROWS: usize = 200;

        // Built outside the measured region: the comparison is about parsing.
        let narrow = build_csv(ROWS, 4);
        let wide = build_csv(ROWS, 64);

        let (_, narrow_allocations) = count_allocations(|| parse(&narrow));
        let (_, wide_allocations) = count_allocations(|| parse(&wide));

        let delta = wide_allocations.saturating_sub(narrow_allocations);
        assert!(
            delta <= ROWS * 2,
            "parsing {ROWS} rows of 64 columns cost {delta} more allocations than the same rows \
             of 4 columns (4 wide: {narrow_allocations}, 64 wide: {wide_allocations}); row \
             handling should not allocate once per field"
        );
    }

    /// Printing must cost nothing per row.
    ///
    /// The output is a fixed sequence of borrowed `&str` fields separated by
    /// `", "`, so every byte can go straight to the writer. Collecting the kept
    /// fields into a `Vec<&str>` and joining them allocates on every printed
    /// line to produce exactly those bytes.
    ///
    /// The buffer is sized outside the measured region so the assertion is about
    /// formatting alone, and the expected text is spelled out so the test also
    /// pins the bytes: dropping the allocation must not change the output. The
    /// first line is a header — `csv::Reader` consumes it — so the two records
    /// under it are the two rows that get printed.
    #[test]
    fn printing_rows_does_not_allocate() {
        let groups = parse("key,a,b,c\ng1,keep-a,keep-b,keep-c\ng1,keep-d,keep-e,keep-f\n");
        let mut out: Vec<u8> = Vec::with_capacity(1 << 16);

        let ((), allocations) = count_allocations(|| {
            for (_, rows) in groups.groups() {
                groups.write_rows(rows, &mut out).expect("writing to a Vec<u8> cannot fail");
            }
        });

        assert_eq!(
            allocations, 0,
            "printing 2 rows cost {allocations} allocations; row formatting should write fields straight to the output"
        );
        assert_eq!(
            String::from_utf8(out).expect("rows are written as UTF-8"),
            "keep-a, keep-b, keep-c\nkeep-d, keep-e, keep-f\n"
        );
    }
}
