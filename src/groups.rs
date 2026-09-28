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

/// A single CSV record: where its field ranges live in the shared arrays.
///
/// Instead of owning its data via a heap-allocated `csv::StringRecord`, a row
/// is a small descriptor into two collection-wide arrays (`arena` for the
/// field bytes, `bounds` for the boundaries). This removes the per-record
/// allocation entirely.
///
/// The ranges are deliberately **flat and shared** rather than one
/// `Vec<FieldRange>` per row. A per-row vector costs 24 B of header plus its
/// own allocation with doubling slack; measured on the 1M-row / 12-column
/// fixture that was 192 MB of range storage against 155 MB of actual payload,
/// which turned the memory fix into a +23% peak-RSS regression at high key
/// cardinality. Flat ranges cost exactly 8 B per field with no slack and no
/// per-row allocator bookkeeping.
#[derive(Debug, Clone, Copy)]
pub struct Row {
    /// Index of this row's first field in [`GroupedData`]'s range arrays.
    start: u32,
    /// Number of fields, which fixes the range span and therefore the column
    /// index of every field in the record.
    len: u32,
}

impl Row {
    /// Append each field's bytes to `arena`, recording their boundaries in
    /// `bounds`.
    ///
    /// The field at `skip` (the grouping column) records a zero-length range
    /// and its bytes are **not** copied. Those bytes duplicate the group key
    /// already interned once per distinct name, they are never printed, and at
    /// high key cardinality keeping them amounts to a second full copy of the
    /// most-repeated column.
    ///
    /// The slot is still recorded so every following field keeps its original
    /// column index, which [`Row::write_to`] and [`Row::fields`] rely on.
    fn ingest<'f>(
        fields: impl ExactSizeIterator<Item = &'f [u8]>,
        skip: usize,
        arena: &mut Vec<u8>,
        bounds: &mut Vec<u32>,
    ) -> Self {
        // One amortised reservation per row instead of one per field push: a
        // `Vec` grown by 2N pushes still reallocates O(log) times, and each
        // reallocation copies the whole array.
        bounds.reserve(fields.len() * 2);
        let start = u32::try_from(bounds.len()).expect("a record's range offsets fit in u32");
        let count = u32::try_from(fields.len()).expect("a record's field count fits in u32");
        for (index, field) in fields.enumerate() {
            let begin = arena.len();
            if index != skip {
                arena.extend_from_slice(field);
            }
            bounds.push(u32::try_from(begin).expect("arena offset fits in u32"));
            // `arena.len() - begin` is 0 for the skipped grouping column, which
            // is what keeps its slot without copying its bytes — see the note on
            // [`Row`] about why the slot exists at all.
            bounds.push(u32::try_from(arena.len() - begin).expect("one field's bytes fit in u32"));
        }
        Row { start, len: count }
    }

    /// Iterate over this row's fields in column order.
    ///
    /// Each field is borrowed from `arena` using the stored span, so no
    /// allocation occurs. Fields are returned as `&str`; the arena is
    /// guaranteed to contain valid UTF-8 because it is populated from
    /// `csv::ByteRecord` fields that were validated at ingest time.
    ///
    /// The grouping column yields `""`: its bytes are deliberately not retained
    /// (see [`Row::ingest`]), and callers that print rows omit that column.
    pub fn fields<'a>(self, arena: &'a [u8], bounds: &'a [u32]) -> impl Iterator<Item = &'a str> {
        // Indexed directly rather than via `chunks`: `Chunks::next` copies into
        // an internal buffer, which allocates once per printed row and breaks
        // the zero-allocation printing budget this path is held to.
        let pairs = 0..self.len;
        pairs.filter_map(move |index| {
            let at = self.start as usize + index as usize * 2;
            if at + 1 >= bounds.len() {
                return None;
            }
            let begin = bounds[at] as usize;
            let end = begin + bounds[at + 1] as usize;
            // SAFETY: every recorded span is a whole field of a ByteRecord
            // whose UTF-8 validity was checked at ingest.
            Some(unsafe { std::str::from_utf8_unchecked(&arena[begin..end]) })
        })
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
    fn write_to(self, out: &mut impl Write, skip: usize, arena: &[u8], bounds: &[u32]) -> io::Result<()> {
        let mut first = true;

        // Written by index, not by iterator: any intermediate allocation here
        // would show up in the printing budget that guards this function.
        for index in 0..self.len as usize {
            if index == skip {
                continue;
            }
            if first {
                first = false;
            } else {
                write!(out, ", ")?;
            }
            let at = self.start as usize + index * 2;
            let begin = bounds[at] as usize;
            out.write_all(&arena[begin..begin + bounds[at + 1] as usize])?;
        }

        writeln!(out)
    }
}

/// CSV records grouped by a single column's values.
///
/// Rows are stored as byte ranges in a single arena buffer rather than as
/// individually allocated records. Group keys are interned into dense `u32`
/// identifiers so the map lookup per row costs one integer comparison rather
/// than a string hash, and the key string is stored exactly once regardless
/// of how many rows share it.
#[derive(Debug)]
pub struct GroupedData {
    /// Groups keyed by interned group id. Iteration order here is insertion
    /// order and is never observable: output order comes from [`Self::intern`],
    /// which is sorted by name.
    groups: BTreeMap<usize, Vec<Row>>,
    /// Intern table: group name → dense id. Each distinct key is stored
    /// exactly once here, eliminating the per-row `String` allocation that
    /// `BTreeMap<String, _>` would impose. Because it is a `BTreeMap` keyed by
    /// name, iterating it yields groups in lexicographic order, which *is* the
    /// output order — no reverse id → name table and no sort at print time.
    intern: BTreeMap<String, usize>,
    /// Byte arena holding every retained field of every row. One allocation
    /// for the whole dataset replaces one allocation per `StringRecord`.
    arena: Vec<u8>,
    /// Interleaved `(begin, len)` arena offsets per field, in row order. Shared
    /// by every [`Row`], which stores only where its own pair range starts —
    /// see [`Row`]'s note on why flat beats a per-row vector.
    bounds: Vec<u32>,
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
            intern: BTreeMap::new(),
            arena: Vec::new(),
            bounds: Vec::new(),
            index: column_number.index(),
            warned_missing_column: false,
        }
    }

    /// Intern a group name, returning its dense id. If the name is new, one
    /// owned `String` is inserted; if it already exists, the existing id is
    /// returned without allocating.
    ///
    /// The id is the count of keys interned so far, which is dense because
    /// every insert takes the next unused one. `intern.len()` is that count, so
    /// no separate counter — and no second copy of the key — is needed.
    fn intern_key(&mut self, name: &str) -> usize {
        if let Some(&id) = self.intern.get(name) {
            return id;
        }
        let id = self.intern.len();
        self.intern.insert(name.to_owned(), id);
        id
    }

    /// Pre-size the byte arena and the shared bound array for roughly `bytes`
    /// of input, so neither grows by doubling into a large unused tail.
    ///
    /// Bounds are 8 bytes per field (`u32` begin + `u32` len) while a field's
    /// payload averages longer than that, so allowing one bound slot per input
    /// byte is generous and still far below the doubling slack it avoids.
    ///
    /// Only ever called with an upper bound derived from real input size; the
    /// arena stays growable, so an under-estimate costs a reallocation and not
    /// correctness.
    fn reserve_bytes(&mut self, bytes: usize) {
        self.arena.reserve(bytes);
        self.bounds.reserve(bytes * 2);
    }

    fn process<R: std::io::Read>(&mut self, rdr: &mut csv::Reader<R>) -> Result<()> {
        let mut record = csv::ByteRecord::new();
        while rdr.read_byte_record(&mut record)? {
            let Some(key_bytes) = record.get(self.index) else {
                self.warn_missing_column();
                continue;
            };

            // Validate UTF-8 at ingest time so downstream code can use
            // from_utf8_unchecked safely.
            let key = std::str::from_utf8(key_bytes).context("grouping column contains invalid UTF-8")?;

            let group_id = self.intern_key(key);

            // Ingest straight from the ByteRecord. `ByteRecord::iter` yields
            // `&[u8]` and is exact-size, so no intermediate collection is built
            // — one per row would reintroduce the allocation this removes.
            let row = Row::ingest(record.iter(), self.index, &mut self.arena, &mut self.bounds);

            self.groups.entry(group_id).or_default().push(row);
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

    fn build_reader<R: std::io::Read>(reader: R, has_headers: bool, delim: u8) -> csv::Reader<R> {
        csv::ReaderBuilder::new().has_headers(has_headers).delimiter(delim).from_reader(reader)
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
            let mut rdr = Self::build_reader(stdin, has_headers, delim);
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
                // Reserve before reading. A `Vec<u8>` grown by doubling leaves up to
                // half its capacity untouched but allocated, and on macOS that slack
                // is resident: measured 118 MB of a 268 MB arena on the 1M-row
                // fixture. The file's length bounds the retained payload — every
                // stored field is a slice of it minus delimiters, headers and the
                // grouping column — so sizing to it removes the overshoot without
                // ever reallocating mid-read.
                if let Ok(meta) = file.metadata() {
                    groups.reserve_bytes(usize::try_from(meta.len()).unwrap_or(0));
                }
                let mut rdr = Self::build_reader(file, has_headers, delim);
                groups.process(&mut rdr).with_context(|| format!("cannot read '{}'", filename.display()))?;
            }
        }

        Ok(groups)
    }

    /// The rows belonging to `group_name`, creating the group if it is new.
    ///
    /// This is exposed for tests that build `GroupedData` programmatically.
    /// Production code uses `process()` which reads directly from the CSV
    /// reader into the arena.
    #[cfg(test)]
    fn group_rows(&mut self, group_name: &str) -> &mut Vec<Row> {
        let id = self.intern_key(group_name);
        self.groups.entry(id).or_default()
    }

    /// The groups in key order, borrowed straight from the map. Iterating
    /// this is all a caller needs: no key Vec to allocate and no per-group
    /// lookup afterwards.
    pub fn groups(&self) -> impl Iterator<Item = (&str, &[Row])> + '_ {
        // No sort and no reverse table: the intern table is a
        // `BTreeMap<String, usize>`, so iterating it already yields names in
        // lexicographic order — the output order. The `get` only recovers the
        // rows for that name's id.
        self.intern
            .iter()
            .filter_map(move |(name, &id)| self.groups.get(&id).map(|rows| (name.as_str(), rows.as_slice())))
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
            row.write_to(out, self.index, &self.arena, &self.bounds)?;
        }

        Ok(())
    }

    /// Render the full grouped layout into any writable sink.
    ///
    /// Renders the full grouped layout: group header with colon, blank line,
    /// rows, trailing blank line, repeated for every group in key order.
    /// Before printing, each group's row vector is shrunk to its exact size
    /// so the doubling slack from incremental growth is released.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if writing to `out` fails.
    pub fn write_to<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        // Part C: shrink each group's Vec to exact size before output,
        // releasing the doubling slack from incremental growth.
        for rows in self.groups.values_mut() {
            rows.shrink_to_fit();
        }

        for (group, rows) in self.groups() {
            // A quoted CSV field may contain embedded newlines. Writing them
            // verbatim into the header destroys the structural delimiter that
            // separates groups from data. Render \r\n and \n as the two-character
            // escape \\n so every header occupies exactly one line.
            let safe = group.replace("\r\n", "\\n").replace(['\r', '\n'], "\\n");
            writeln!(out, "{safe}:\n")?;
            self.write_rows(rows, out)?;
            writeln!(out)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnNumber, GroupedData};
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
    /// With arena-backed rows, field data is copied into a single buffer
    /// regardless of column count. The per-row cost is one arena append plus
    /// one small Vec<FieldRange> allocation, both width-independent in
    /// allocation count. The bound is deliberately loose — it fails by an
    /// order of magnitude on copying code and passes by an even larger margin
    /// without it — so it cannot flake on allocator details.
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
    /// The output is a fixed sequence of borrowed byte slices separated by
    /// `", "`, so every byte can go straight to the writer. Collecting the
    /// kept fields into a `Vec<&str>` and joining them allocates on every
    /// printed line to produce exactly those bytes.
    ///
    /// The budget is a *delta* between two identical print passes, not an
    /// absolute zero. `ALLOCATIONS` counts every allocation in the process,
    /// including those made by background threads the Rust runtime starts
    /// lazily (one-time allocator and thread-infrastructure allocations that
    /// land on other CPUs during the measured window). On macOS that noise
    /// injects ~11 allocations often enough to fail a zero-tolerance absolute
    /// assertion — the `v0.4.1` release failed exactly that way while the same
    /// commit passed repeatedly nearby (issue #97).
    ///
    /// A warm-up pass over identical work makes the measurement race-free: it
    /// guarantees the runtime's one-time startup has already happened, so the
    /// second pass sees only what row formatting itself allocates. A `Vec<&str>`
    /// join regression still fails deterministically on every platform, because
    /// it allocates in the measured pass too.
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

        let print = |groups: &GroupedData, out: &mut Vec<u8>| {
            for (_, rows) in groups.groups() {
                groups.write_rows(rows, out).expect("writing to a Vec<u8> cannot fail");
            }
        };

        // Warm-up pass: executes every code path the budget measures, absorbs
        // the runtime's lazy thread-startup allocations, and is deliberately
        // unmeasured beyond reporting its count for diagnostics.
        let ((), warmup) = count_allocations(|| print(&groups, &mut out));
        out.clear();

        // Measured pass: with the process fully warmed, row formatting must
        // allocate nothing. This is the invariant that a `Vec<&str>` join
        // regression violates deterministically, on every platform.
        let ((), allocations) = count_allocations(|| print(&groups, &mut out));

        assert_eq!(
            allocations, 0,
            "printing 2 rows cost {allocations} allocations after a warm-up pass (warm-up \
             itself cost {warmup}); row formatting should write fields straight to the output"
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
        let mut groups = parse("key,a,b,c\ng1,keep-a,keep-b,keep-c\ng2,keep-d,keep-e,keep-f\n");
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
    /// With interned keys, the map lookup per row is one integer comparison
    /// and the key string is stored exactly once. The per-row cost is the
    /// arena append plus the Vec push, neither of which allocates on the
    /// steady-state path.
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

        // Was bounded at 3.5 per row when every record owned a `StringRecord`.
        // Rows are now ranges into shared arrays, so the measured cost over the
        // extra rows is a handful of allocations in total. This leaves room for
        // allocator noise but fails immediately if a per-row allocation comes
        // back — a bound that cannot fail proves nothing (issue #104 AC 4).
        let delta = many_allocations.saturating_sub(few_allocations);
        assert!(
            delta <= STEP / 8,
            "parsing {STEP} extra rows into the same two groups cost {delta} allocations \
             ({FEW} rows: {few_allocations}, {MANY} rows: {many_allocations}); that exceeds the \
             0.125-per-row budget, so something beyond the arena append allocates per row"
        );
    }

    /// Naming a group must cost nothing once that group exists.
    ///
    /// With interned keys, looking up an existing group is one `BTreeMap`
    /// probe on a `String` key that returns immediately when found. No
    /// allocation occurs for the common case of a repeated key.
    #[test]
    fn adding_rows_to_an_existing_group_does_not_allocate_a_key() {
        const ROWS: usize = 256;

        let _guard = measurement_lock();

        let keys: Vec<String> = (0..ROWS).map(|row| format!("g{}", row % 2)).collect();
        let mut groups = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));

        // Measure only the key-interning path: calling group_rows with
        // alternating keys. The first call for each distinct key allocates
        // a String for the intern table; subsequent calls reuse the id.
        let ((), allocations) = count_allocations(|| {
            for key in &keys {
                groups.group_rows(key);
            }
        });

        // With 2 distinct keys across 256 rows, only 2 key allocations should
        // occur (one per distinct key). The old bound was ROWS/4 = 64; with
        // interned keys the count should be far lower.
        assert!(
            allocations <= ROWS / 4,
            "adding {ROWS} rows to 2 groups cost {allocations} allocations; only the 2 distinct \
             group names should allocate, not one per row"
        );
    }

    /// Per-row allocations must not scale with the number of distinct groups.
    ///
    /// This is the new invariant from key interning: whether a row lands in
    /// one of two groups or one of a thousand, the per-row allocation cost
    /// is the same — the key lookup returns an existing id without allocating.
    #[test]
    fn per_row_allocations_do_not_scale_with_group_count() {
        const ROWS: usize = 256;

        let _guard = measurement_lock();

        // Two groups: every row alternates between g0 and g1.
        let few_groups = build_csv(ROWS, 4);
        // Many groups: each row gets a unique key (up to ROWS distinct keys).
        let many_groups_csv = {
            let mut out = String::new();
            for row in 0..ROWS {
                use std::fmt::Write;
                writeln!(out, "g{row},a,b,c").unwrap();
            }
            out
        };

        let (_, few_allocs) = count_allocations(|| parse(&few_groups));
        let (_, many_allocs) = count_allocations(|| parse(&many_groups_csv));

        // The delta should be proportional to the number of *new* keys (each
        // new key costs one String allocation for the intern table), not to
        // the number of rows. With ROWS distinct keys vs 2, the extra cost
        // is ~ROWS String allocations for the keys themselves, but the
        // per-row *processing* cost stays flat.
        let extra_keys = ROWS - 2;
        #[allow(clippy::cast_precision_loss)]
        let per_extra_key = many_allocs.saturating_sub(few_allocs) as f64 / extra_keys as f64;
        assert!(
            per_extra_key < 5.0,
            "each additional distinct group key cost {per_extra_key:.1} allocations on average \
             (2 groups: {few_allocs}, {ROWS} groups: {many_allocs}); key interning should make \
             this close to 1 (one String per new key)"
        );
    }

    /// The arena must not grow when the grouping column's values are unique.
    ///
    /// This is the RSS regression guard for issue #104. Storing the grouping
    /// field per row costs one full key per record, and at high key
    /// cardinality that exceeds the entire payload — measured as +23.8% peak
    /// RSS on the 1M-row/1M-group fixture versus the `StringRecord` baseline,
    /// which turned the memory fix into a memory regression exactly where it
    /// mattered most. Skipping the grouping column makes retained row bytes
    /// depend only on the printed payload.
    ///
    /// Both inputs carry the same payload bytes; only the grouping column
    /// differs (two repeating keys vs one unique key per row), so an arena
    /// that stored the key would show a large delta here.
    #[test]
    fn arena_bytes_do_not_include_the_grouping_column() {
        const ROWS: usize = 500;
        // A deliberately long key so the skipped bytes dominate if they leak.
        let key = "x".repeat(64);

        let low_card = {
            let mut out = String::from("key,a,b\n");
            for row in 0..ROWS {
                use std::fmt::Write;
                writeln!(out, "{key}{},r{row}c1,r{row}c2", row % 2).expect("writing to a String cannot fail");
            }
            out
        };
        let high_card = {
            let mut out = String::from("key,a,b\n");
            for row in 0..ROWS {
                use std::fmt::Write;
                writeln!(out, "{key}{row},r{row}c1,r{row}c2").expect("writing to a String cannot fail");
            }
            out
        };

        let low = parse(&low_card);
        let high = parse(&high_card);

        // The high-cardinality input carries ~ROWS extra distinct key bytes on
        // disk. If those were copied into the arena the difference would be
        // tens of kilobytes; it must instead be only the differing key suffixes
        // that remain in the intern table, which is not part of the arena.
        let delta = high.arena.len().abs_diff(low.arena.len());
        assert!(
            delta < ROWS, // at most one byte per row (the unique key suffix)
            "arena grew {delta} bytes going from 2 groups to {ROWS} groups over the same \
             payload; the grouping column must not be copied into the arena"
        );
        assert!(
            !high.arena.windows(key.len()).any(|w| w == key.as_bytes()),
            "a grouping-column value was found verbatim in the arena"
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
        let mut grouped = GroupedData::new("1".parse::<ColumnNumber>().expect("column 1 parses"));
        let ((), key_total) = count_allocations(|| {
            for key in &keys {
                grouped.group_rows(key);
            }
        });
        println!("alloc op=group_rows rows={KEY_ROWS} groups=2 total={key_total}");
    }

    /// `Row::fields` must expose every field in column order so downstream
    /// callers can inspect the rows that [`GroupedData::groups`] returns.
    ///
    /// The grouping column is the one exception: its bytes are not retained
    /// (they duplicate the interned key and dominate cost at high cardinality),
    /// so it yields an empty string while every other field comes back intact
    /// at its original index.
    #[test]
    fn row_fields_exposes_all_columns_in_order() {
        // Holds the measurement lock even though it asserts nothing about
        // counts: `collect()` allocates, and `count_allocations` measures a
        // process-wide counter. The zero-allocation printing budget only stays
        // exact because every allocating test serialises behind this lock —
        // an unlocked allocator here would race into that measurement.
        let _guard = measurement_lock();

        let groups = parse("key,a,b,c\nk,x,y,z\n");
        let (_, rows) = groups.groups().next().expect("one group");
        let row = &rows[0];
        let fields: Vec<&str> = row.fields(&groups.arena, &groups.bounds).collect();
        assert_eq!(fields, vec!["", "x", "y", "z"]);
    }

    /// The zero-length slot for the grouping column must survive as a *slot*.
    ///
    /// [`Row::ingest`] skips the grouping column's bytes but still pushes a
    /// `(begin, 0)` pair for it. That pair is what keeps every later field at its
    /// original column index; if someone "optimises" the skip into not pushing
    /// the pair at all, the bounds array no longer lines up with `len` fields per
    /// row and each row silently loses its last field — while the arena size stays
    /// identical, so `arena_bytes_do_not_include_the_grouping_column` cannot see it.
    /// This asserts the two things that change together: one pair per field, and a
    /// zero-length range exactly at the grouping index.
    #[test]
    fn skipped_grouping_column_keeps_a_zero_length_bound_pair() {
        const COLUMNS: usize = 4;
        const ROWS: usize = 2;

        // Same reason as `row_fields_exposes_all_columns_in_order`: collecting
        // field vectors allocates, and `ALLOCATIONS` is a process-wide counter,
        // so an unlocked allocating test races into whatever budget happens to be
        // measuring and breaks *that* test, not this one.
        let _guard = measurement_lock();

        let groups = parse("key,a,b,c\ng1,keep-a,keep-b,keep-c\ng2,keep-d,keep-e,keep-f\n");

        // One (begin, len) pair per field of every retained row. Derived from
        // the fixture rather than hardcoded, so a change in what `parse` keeps
        // fails with a count instead of a mystery.
        assert_eq!(
            groups.bounds.len(),
            ROWS * COLUMNS * 2,
            "bounds must hold one pair per field even for the skipped column"
        );

        let kept: Vec<Vec<&str>> = groups
            .groups()
            .flat_map(|(_, rows)| rows)
            .map(|row| row.fields(&groups.arena, &groups.bounds).collect())
            .collect();
        assert_eq!(
            kept,
            vec![
                vec!["", "keep-a", "keep-b", "keep-c"],
                vec!["", "keep-d", "keep-e", "keep-f"]
            ],
            "every non-key field must keep its original column index"
        );

        // The grouping column's pair is (begin, 0): an empty span inside the
        // arena, not a missing entry.
        let skip = groups.index;
        for row in groups.groups().flat_map(|(_, rows)| rows) {
            let at = row.start as usize + skip * 2;
            let begin = usize::try_from(groups.bounds[at]).expect("bound fits in usize");
            assert_eq!(groups.bounds[at + 1], 0, "the skipped field must record a zero length");
            assert!(
                begin <= groups.arena.len(),
                "a zero-length span must still sit in the arena"
            );
        }
    }
}
