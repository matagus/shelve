use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::Path;
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

/// A single CSV record, stored as its raw fields.
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
    /// Construct a row from an already-parsed record.
    #[must_use]
    pub fn new(data: csv::StringRecord) -> Self {
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

/// CSV records grouped by a single column's values.
#[derive(Debug)]
pub struct GroupedData {
    groups: BTreeMap<String, Vec<Row>>,
    /// 0-based index of the grouping column.
    index: usize,
    warned_missing_column: bool,
}

impl GroupedData {
    /// Create an empty grouping keyed by `column_number`.
    ///
    /// `column_number` is the 1-based number the CLI exposes. It is converted
    /// to a 0-based index exactly once, here, so no other code has to subtract
    /// one: the newtype cannot hold a zero, so the subtraction cannot underflow.
    #[must_use]
    pub fn new(column_number: ColumnNumber) -> Self {
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

            // Resolve the destination while `key` still borrows `record`. The
            // vector this returns borrows `self`, not the record, so that
            // borrow ends with the call and `record` is free to move into the
            // row on the next line.
            let rows = self.group_rows(key);
            rows.push(Row::new(record));
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

    /// Read CSV files and group their records by the chosen column.
    ///
    /// When `filenames` is empty, stdin is read instead. Records are split on
    /// `delimiter`, which matches [`csv::ReaderBuilder::delimiter`].
    ///
    /// # Errors
    ///
    /// Returns an error if any file cannot be opened or contains invalid CSV.
    pub fn from_files<P: AsRef<Path>>(
        filenames: &[P],
        column_number: ColumnNumber,
        no_headers: bool,
        delimiter: char,
    ) -> Result<Self> {
        let mut groups = GroupedData::new(column_number);
        let has_headers = !no_headers;
        let delim = delimiter as u8;

        if filenames.is_empty() {
            let stdin = std::io::stdin().lock();
            let mut rdr = csv::ReaderBuilder::new().has_headers(has_headers).delimiter(delim).from_reader(stdin);
            groups.process(&mut rdr)?;
        } else {
            for filename in filenames {
                let filename = filename.as_ref();
                // Name the file: with several arguments a bare "No such file or
                // directory" leaves no way to tell which one failed. The same
                // context covers parse errors, which are otherwise reported
                // without saying which input they came from.
                //
                // `display()` because the name may not be UTF-8.
                let file = File::open(filename).with_context(|| format!("cannot open '{}'", filename.display()))?;
                let mut rdr = csv::ReaderBuilder::new().has_headers(has_headers).delimiter(delim).from_reader(file);
                groups.process(&mut rdr).with_context(|| format!("cannot read '{}'", filename.display()))?;
            }
        }

        Ok(groups)
    }

    /// The rows belonging to `group_name`, creating the group if it is new.
    ///
    /// Borrowing the name keeps the hit path — the common case for grouped
    /// data — free of allocations. `entry()` cannot express this: it takes an
    /// owned key, so every row would have to build a `String` even when that
    /// key is already in the map. Only a genuinely new group pays, once, for
    /// the key it inserts.
    ///
    /// A new group therefore costs extra map descents. Closing that gap needs
    /// the raw entry API, which is not an option here: `BTreeMap::raw_entry_mut`
    /// does not exist on stable at all, and `HashMap::raw_entry_mut` is
    /// nightly-only — moving to `HashMap` to get it would trade a rare extra
    /// descent for the sorted group ordering this output depends on.
    ///
    /// The obvious `if let Some(rows) = self.groups.get_mut(name) { return rows }
    /// …entry(name)` form is also unavailable: returning the borrowed vector
    /// out of one arm while touching the map in the other is E0499 on stable
    /// borrowck (rust#51545, fixed only under Polonius). Testing membership
    /// first keeps every borrow short-lived at the cost of one more descent,
    /// and the `expect` below is unreachable — the key is present by
    /// construction on both paths.
    fn group_rows(&mut self, group_name: &str) -> &mut Vec<Row> {
        if !self.groups.contains_key(group_name) {
            self.groups.insert(group_name.to_owned(), Vec::new());
        }

        self.groups.get_mut(group_name).expect("the group was just inserted if it was missing")
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
    ///
    /// # Errors
    ///
    /// Returns an I/O error if writing to `out` fails.
    pub fn write_rows(&self, rows: &[Row], out: &mut impl Write) -> io::Result<()> {
        for row in rows {
            row.write_to(out, self.index)?;
        }

        Ok(())
    }

    /// Render the full grouped layout into any writable sink.
    ///
    /// Renders the full grouped layout: group header with colon, blank line,
    /// rows, trailing blank line, repeated for every group in key order.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if writing to `out` fails.
    pub fn write_to<W: Write>(&self, out: &mut W) -> io::Result<()> {
        for (group, rows) in self.groups() {
            writeln!(out, "{group}:\n")?;
            self.write_rows(rows, out)?;
            writeln!(out)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use csv::StringRecord;

    use super::{ColumnNumber, GroupedData, Row};
    use crate::testing::{build_csv, count_allocations, measurement_lock};

    /// Group `csv_text` by its first column, panicking on any parse failure.
    ///
    /// `csv::Reader` treats the first record as a header and skips it, so the
    /// fixtures from [`build_csv`] parse as one record fewer than they contain.
    /// Every use here compares two such calls, so the lost record cancels out.
    fn parse(csv_text: &str) -> GroupedData {
        let mut rdr = csv::Reader::from_reader(csv_text.as_bytes());
        let mut groups = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));
        groups.process(&mut rdr).expect("in-memory CSV reads");
        groups
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

        let _guard = measurement_lock();

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
        let _guard = measurement_lock();

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

    /// `write_to` renders the full grouped layout into any writable sink,
    /// making the output unit-testable without spawning the binary.
    #[test]
    fn write_to_renders_full_layout_into_buffer() {
        let groups = parse("key,a,b,c\ng1,keep-a,keep-b,keep-c\ng2,keep-d,keep-e,keep-f\n");
        let mut out: Vec<u8> = Vec::new();

        groups.write_to(&mut out).expect("writing to a Vec<u8> cannot fail");

        assert_eq!(
            String::from_utf8(out).expect("output is UTF-8"),
            "g1:\n\nkeep-a, keep-b, keep-c\n\ng2:\n\nkeep-d, keep-e, keep-f\n\n"
        );
    }

    /// Parsing more rows into groups that already exist must not add a
    /// per-row allocation.
    ///
    /// This is the same claim as `adding_rows_to_an_existing_group_does_not_allocate_a_key`,
    /// measured through the real `process` path rather than against the map
    /// directly, so a key allocation reintroduced into the reader loop is
    /// caught too. Both inputs group under the same two keys and differ only
    /// in row count, so their delta is the per-row cost: the `csv` reader's own
    /// record allocations, which are inherent, and nothing else. Measured here
    /// at about 3 per row; handing `entry()` an owned key adds a fourth, which
    /// is what the budget below rejects.
    #[test]
    fn parsing_more_rows_into_existing_groups_adds_no_key_allocation() {
        const FEW: usize = 128;
        const MANY: usize = 256;
        const STEP: usize = MANY - FEW;

        let _guard = measurement_lock();

        // Built outside the measured region: the comparison is about parsing.
        let few = build_csv(FEW, 4);
        let many = build_csv(MANY, 4);

        let (_, few_allocations) = count_allocations(|| parse(&few));
        let (_, many_allocations) = count_allocations(|| parse(&many));

        let delta = many_allocations.saturating_sub(few_allocations);
        assert!(
            delta <= STEP * 7 / 2,
            "parsing {STEP} extra rows into the same two groups cost {delta} allocations \
             ({FEW} rows: {few_allocations}, {MANY} rows: {many_allocations}); that exceeds the \
             3.5-per-row budget, so something beyond the reader is allocating once per row"
        );
    }

    /// Naming a group must cost nothing once that group exists.
    ///
    /// `entry()` takes an owned key, so routing rows through it makes the
    /// caller build a `String` for every row even when the key is already in
    /// the map: N rows across K groups cost N allocations where K would do.
    /// Two groups over 256 rows makes 254 of those keys pure waste.
    ///
    /// The keys and the rows are both built outside the measured region, so
    /// the count is the map's own cost and nothing else — no CSV reader, no
    /// formatting, no allocator noise from the fixture.
    #[test]
    fn adding_rows_to_an_existing_group_does_not_allocate_a_key() {
        const ROWS: usize = 256;

        let _guard = measurement_lock();

        let keys: Vec<String> = (0..ROWS).map(|row| format!("g{}", row % 2)).collect();
        let rows: Vec<Row> = (0..ROWS).map(|_| Row::new(StringRecord::from(vec!["g0", "a", "b", "c"]))).collect();
        let mut rows = rows.into_iter();

        let mut groups = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));

        let ((), allocations) = count_allocations(|| {
            for key in &keys {
                groups.group_rows(key).push(rows.next().expect("one row per key"));
            }
        });

        assert!(
            allocations <= ROWS / 4,
            "adding {ROWS} rows to 2 groups cost {allocations} allocations; only the 2 distinct \
             group names should allocate, not one per row"
        );
    }

    /// Print the exact counts that the budget tests above only bound.
    ///
    /// Those bounds are deliberately loose so they cannot flake on allocator
    /// details, which also makes them unable to answer "what does this cost
    /// now?". Getting the real number by tightening a bound until it panics,
    /// reading the panic message, and putting the bound back is a three-step
    /// dance that leaves the tree dirty if anything interrupts it. This measures
    /// the same operations without touching a bound, and CI runs it on every
    /// push, so the current numbers are already in the log of the latest run.
    ///
    /// Locally: `cargo test --bin shelve -- --ignored --nocapture`.
    ///
    /// Lines are `alloc op=<name> <inputs> total=<n>` so they can be grepped or
    /// diffed between two revisions. It asserts nothing on purpose: a count that
    /// merely moved is information, not a failure, and the bounded tests above
    /// are what decides when a move is a regression.
    #[test]
    #[ignore = "diagnostic: reports exact allocation counts and bounds nothing; run with -- --ignored --nocapture"]
    fn report_allocation_counts() {
        const WIDE_ROWS: usize = 200;
        const FEW: usize = 128;
        const MANY: usize = 256;
        const KEY_ROWS: usize = 256;

        let _guard = measurement_lock();

        // Fixtures built before the first measurement: the counts below are
        // about parsing and printing, not about assembling the inputs.
        let narrow = build_csv(WIDE_ROWS, 4);
        let wide = build_csv(WIDE_ROWS, 64);
        let few = build_csv(FEW, 4);
        let many = build_csv(MANY, 4);

        let (_, narrow_total) = count_allocations(|| parse(&narrow));
        let (_, wide_total) = count_allocations(|| parse(&wide));
        let (_, few_total) = count_allocations(|| parse(&few));
        let (_, many_total) = count_allocations(|| parse(&many));

        println!("alloc op=parse rows={WIDE_ROWS} cols=4 total={narrow_total}");
        println!(
            "alloc op=parse rows={WIDE_ROWS} cols=64 total={wide_total} width_delta={}",
            wide_total.saturating_sub(narrow_total)
        );
        println!("alloc op=parse rows={FEW} cols=4 total={few_total}");
        println!(
            "alloc op=parse rows={MANY} cols=4 total={many_total} row_delta={} extra_rows={}",
            many_total.saturating_sub(few_total),
            MANY - FEW
        );

        let groups = parse("key,a,b,c\ng1,keep-a,keep-b,keep-c\ng1,keep-d,keep-e,keep-f\n");
        let mut out: Vec<u8> = Vec::with_capacity(1 << 16);
        let ((), print_total) = count_allocations(|| {
            for (_, rows) in groups.groups() {
                groups.write_rows(rows, &mut out).expect("writing to a Vec<u8> cannot fail");
            }
        });
        println!("alloc op=write_rows rows=2 total={print_total}");

        let keys: Vec<String> = (0..KEY_ROWS).map(|row| format!("g{}", row % 2)).collect();
        let built: Vec<Row> = (0..KEY_ROWS).map(|_| Row::new(StringRecord::from(vec!["g0", "a", "b", "c"]))).collect();
        let mut built = built.into_iter();
        let mut grouped = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));
        let ((), key_total) = count_allocations(|| {
            for key in &keys {
                grouped.group_rows(key).push(built.next().expect("one row per key"));
            }
        });
        println!("alloc op=group_rows rows={KEY_ROWS} groups=2 total={key_total}");
    }
}
