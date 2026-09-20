//! Golden differential sweep: every committed fixture × every grouping column,
//! compared byte for byte against `tests/expected/golden/`.
//!
//! `tests/cli.rs` asserts one hand-picked column per fixture, which is the right
//! shape for a behaviour test and the wrong shape for a refactor guard: a change
//! to how a row is printed, or to which field the grouping column omits, can land
//! on a column nobody pinned and pass. This sweep is the complement — it is
//! derived from the fixtures on disk rather than from a list in the test, so a
//! new `tests/inputs/*.csv` is covered the moment it is committed, with no new
//! test to write.
//!
//! "The refactor did not change the output" is therefore `cargo test`, not a
//! shell harness invented per change.
//!
//! The goldens are generated, so they are only as good as the review that
//! accepted them. To regenerate after an intentional output change:
//!
//! ```sh
//! SHELVE_BLESS_GOLDEN=1 cargo test --test golden
//! git diff --stat tests/expected/golden
//! ```
//!
//! Always read that diff. Blessing without reading turns this file from a guard
//! into a transcription of whatever the binary happens to do.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type TestError = Box<dyn std::error::Error>;

/// `T` defaults to `()` so the test bodies read like the ones in `tests/cli.rs`.
type TestResult<T = ()> = Result<T, TestError>;

const INPUTS: &str = "tests/inputs";
const GOLDENS: &str = "tests/expected/golden";

/// Columns swept per fixture, counting from 1.
///
/// Every column goes through the same `write_to`, so the middle of a wide row
/// adds cases without adding coverage. The sweep takes the first few — where a
/// leading-separator bug would show — plus the last one, where a trailing
/// separator would. Fixtures narrower than this are swept in full.
const MAX_SWEPT_COLUMNS: usize = 5;

/// Fixtures the sweep deliberately does not run, and why. A file listed here
/// still has to be covered by a named test in `tests/cli.rs`; this is for
/// inputs whose correct behaviour is not "print groups to stdout".
const SKIPPED: &[(&str, &str)] = &[(
    "malformed.csv",
    "exits 1 by design; its error text is asserted in tests/cli.rs",
)];

/// True when the goldens should be rewritten instead of compared.
fn blessing() -> bool {
    std::env::var_os("SHELVE_BLESS_GOLDEN").is_some()
}

/// The number of fields in `path`'s header record.
///
/// Parsed with the `csv` reader rather than counted by splitting on commas,
/// because a quoted field may contain one — `malformed.csv` already does.
/// A file with no header at all reports zero columns and is swept zero times.
fn header_columns(path: &Path) -> TestResult<usize> {
    let mut reader = csv::Reader::from_path(path)?;
    Ok(reader.headers()?.len())
}

/// The columns to sweep for a fixture that is `total` columns wide.
fn columns_to_sweep(total: usize) -> Vec<usize> {
    let mut columns: Vec<usize> = (1..=total.min(MAX_SWEPT_COLUMNS)).collect();
    if total > MAX_SWEPT_COLUMNS {
        columns.push(total);
    }
    columns
}

/// The fixtures to sweep, in name order.
///
/// Sorted because `read_dir` order is filesystem-dependent, and a failure list
/// that changes order between runs is harder to diff.
fn fixtures() -> TestResult<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> =
        fs::read_dir(INPUTS)?.map(|entry| entry.map(|entry| entry.path())).collect::<Result<Vec<_>, _>>()?;

    paths.sort();
    paths.retain(|path| path.extension().is_some_and(|ext| ext == "csv"));

    Ok(paths)
}

#[test]
fn golden_output_matches_for_every_fixture_and_column() -> TestResult {
    let mut cases = 0usize;
    let mut swept_files = 0usize;
    let mut blessed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    if blessing() {
        fs::create_dir_all(GOLDENS)?;
    }

    for path in fixtures()? {
        let name = path.file_name().expect("a directory entry has a name").to_string_lossy().into_owned();

        if let Some((_, reason)) = SKIPPED.iter().find(|(skipped, _)| *skipped == name) {
            println!("golden: skipping {name} — {reason}");
            continue;
        }

        let total = header_columns(&path)?;
        let columns = columns_to_sweep(total);
        if columns.is_empty() {
            println!("golden: skipping {name} — no header record, so no column to group by");
            continue;
        }
        swept_files += 1;

        for column in columns {
            cases += 1;

            // `read_dir` on a relative directory yields relative paths, so a
            // failure below names the fixture the way a reader would type it.
            let out =
                Command::new(env!("CARGO_BIN_EXE_shelve")).arg("-c").arg(column.to_string()).arg(&path).output()?;

            let stem = path.file_stem().expect("a .csv path has a stem").to_string_lossy();
            let golden = Path::new(GOLDENS).join(format!("{stem}-c{column}.txt"));

            if !out.status.success() {
                failures.push(format!(
                    "{name} -c {column}: exited with {} and stderr {:?}",
                    out.status,
                    String::from_utf8_lossy(&out.stderr)
                ));
                continue;
            }

            // No fixture's output may contain a carriage return, not even the
            // CRLF one: the reader strips `\r` from the last field, so a CR here
            // means it leaked out of the input. Asserted before any
            // normalisation below could hide it.
            if out.stdout.contains(&b'\r') {
                failures.push(format!("{name} -c {column}: output contains a carriage return"));
                continue;
            }

            if blessing() {
                fs::write(&golden, &out.stdout)?;
                blessed += 1;
                continue;
            }

            let expected = match fs::read(&golden) {
                Ok(bytes) => bytes,
                Err(err) => {
                    failures.push(format!(
                        "{name} -c {column}: no golden at {} ({err}); create it with \
                         SHELVE_BLESS_GOLDEN=1 cargo test --test golden and review the diff",
                        golden.display()
                    ));
                    continue;
                }
            };

            // Defensive, matching tests/cli.rs: `eol=lf` in .gitattributes
            // already pins these files, so a CRLF golden would mean a checkout
            // that ignored it. Normalising the expectation keeps the sweep
            // portable without weakening the actual bytes, which were just
            // checked for stray CRs above.
            let expected: Vec<u8> = expected.iter().copied().filter(|b| *b != b'\r').collect();

            if out.stdout != expected {
                failures.push(format!(
                    "{name} -c {column}: output differs from {}\n--- expected ---\n{}\n--- actual ---\n{}",
                    golden.display(),
                    String::from_utf8_lossy(&expected),
                    String::from_utf8_lossy(&out.stdout)
                ));
            }
        }
    }

    if blessing() {
        println!("golden: blessed {blessed} files under {GOLDENS}/ — review `git diff` before committing");
        return Ok(());
    }

    // A sweep that quietly ran nothing would report success, so pin the shape of
    // what was found. The exact counts move as fixtures are added, which is the
    // point; zero, or a fixture that stopped producing its full column range,
    // is a bug in this file rather than in the program.
    assert!(
        swept_files >= 8,
        "only swept {swept_files} fixtures — did the {INPUTS}/ glob break?"
    );
    assert!(
        cases >= 30,
        "only swept {cases} cases — did the header-column count break?"
    );
    assert_eq!(
        header_columns(Path::new("tests/inputs/tasks.csv"))?,
        5,
        "tasks.csv is the 5-column reference fixture"
    );

    assert!(
        failures.is_empty(),
        "{} of {cases} golden cases differed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );

    println!("golden: {cases} cases across {swept_files} fixtures, all identical");
    Ok(())
}
