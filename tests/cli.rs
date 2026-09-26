mod common;

use common::{TestResult, output_matches, run_ok, run_ok_stdin};
use std::fs;

// Unix-only, for the non-UTF-8 filename tests below.
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

//---------------------------------------------------
#[test]
fn test_help() -> TestResult {
    run_ok(&["--help"], "tests/expected/help.txt")
}

#[test]
fn test_version() -> TestResult {
    let expected = format!("shelve {}\n", env!("CARGO_PKG_VERSION"));
    common::bin()?.arg("--version").assert().success().stdout(expected);
    Ok(())
}

// Column zero cannot be represented by the argument type, so clap rejects it
// while parsing: a usage error with exit code 2, not a runtime `Error:` with 1.
#[test]
fn test_zero_column() -> TestResult {
    common::bin()?
        .args(["-c", "0", "tests/inputs/tasks.csv"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("invalid value '0' for '--column-number"));
    Ok(())
}

// A column number above the old `u8` cap is a grouping miss, not a parse error:
// the warning path handles it and the run still succeeds.
#[test]
fn test_column_beyond_u8_range() -> TestResult {
    let input = fs::read_to_string("tests/inputs/tasks.csv")?;
    common::bin()?
        .args(["-c", "256"])
        .write_stdin(input)
        .assert()
        .success()
        .stdout("")
        .stderr(predicates::str::contains("column 256 is missing"));
    Ok(())
}

#[test]
fn test_default_column() -> TestResult {
    run_ok(&["tests/inputs/tasks.csv"], "tests/expected/default-column.txt")
}

#[test]
fn test_first_column() -> TestResult {
    run_ok(&["-c", "1", "tests/inputs/tasks.csv"], "tests/expected/column-1.txt")
}

#[test]
fn test_2nd_column() -> TestResult {
    run_ok(&["-c", "2", "tests/inputs/tasks.csv"], "tests/expected/column-2.txt")
}

#[test]
fn test_3rd_column() -> TestResult {
    run_ok(&["-c", "3", "tests/inputs/tasks.csv"], "tests/expected/column-3.txt")
}

#[test]
fn test_4th_column() -> TestResult {
    run_ok(&["-c", "4", "tests/inputs/tasks.csv"], "tests/expected/column-4.txt")
}

#[test]
fn test_5th_column() -> TestResult {
    run_ok(&["-c", "5", "tests/inputs/tasks.csv"], "tests/expected/column-5.txt")
}

#[test]
fn test_tw0_files() -> TestResult {
    run_ok(
        &["-c", "5", "tests/inputs/tasks.csv", "tests/inputs/more-tasks.csv"],
        "tests/expected/two-files.txt",
    )
}

#[test]
fn test_read_from_stdin() -> TestResult {
    run_ok_stdin("tests/inputs/tasks.csv", &["-c", "5"], "tests/expected/stdin.txt")
}

#[test]
fn test_unexpected_argument() -> TestResult {
    let expected = fs::read_to_string("tests/expected/unexpected-argument.txt")?;
    common::bin()?.args(["--foobar", "tests/inputs/tasks.csv"]).assert().failure().stderr(output_matches(expected));
    Ok(())
}

// With no filename argument the input must come from stdin; an empty stream is
// a valid empty list, not an error about a missing argument.
#[test]
fn test_no_filename_arg_reads_stdin() -> TestResult {
    run_ok_stdin("tests/inputs/empty.csv", &[], "tests/expected/empty.txt")
}

// A column number that is not a number is rejected by the parser, so it never
// reaches the grouping logic.
#[test]
fn test_non_integer_column_index() -> TestResult {
    common::bin()?
        .args(["-c", "abc", "tests/inputs/tasks.csv"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("invalid digit found in string"));
    Ok(())
}

// test a case where -c option is higher than the number of columns
#[test]
fn test_too_high_column() -> TestResult {
    run_ok_stdin("tests/inputs/tasks.csv", &["-c", "20"], "tests/expected/empty.txt")
}

// ...but it must not stay silent about it. Empty stdout plus a zero exit code
// is otherwise indistinguishable from an empty input file.
#[test]
fn test_too_high_column_warns_on_stderr() -> TestResult {
    let input = fs::read_to_string("tests/inputs/tasks.csv")?;
    common::bin()?
        .args(["-c", "20"])
        .write_stdin(input)
        .assert()
        .success()
        .stderr(predicates::str::contains("column 20 is missing"));
    Ok(())
}

// The warning fires once, not once per skipped row.
#[test]
fn test_too_high_column_warns_only_once() -> TestResult {
    let input = fs::read_to_string("tests/inputs/tasks.csv")?;
    let output = common::bin()?.args(["-c", "20"]).write_stdin(input).output()?;
    let warnings = String::from_utf8(output.stderr)?.matches("Warning:").count();
    assert_eq!(warnings, 1, "expected exactly one warning");
    Ok(())
}

// A failing path has to be named, otherwise a multi-file invocation gives no
// clue which argument could not be opened.
#[test]
fn test_missing_file_is_named_in_error() -> TestResult {
    common::bin()?
        .arg("tests/inputs/no-such-file.csv")
        .assert()
        .failure()
        .stderr(predicates::str::contains("tests/inputs/no-such-file.csv"));
    Ok(())
}

// The report keeps its whole source chain: the context says which file failed
// and the cause says why. Printing only the outermost error would drop one of
// the two halves. The OS message is matched loosely ("file specified" on
// Windows, "No such file or directory" on Unix), so assert the shared os error
// code plus both halves of the wrapped message.
#[test]
fn test_missing_file_error_includes_cause() -> TestResult {
    common::bin()?
        .arg("tests/inputs/no-such-file.csv")
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cannot open"))
        .stderr(predicates::str::contains("os error 2"));
    Ok(())
}

// A parse failure gets the same treatment: without context it is impossible to
// tell which of several inputs was malformed.
#[test]
fn test_malformed_file_is_named_in_error() -> TestResult {
    common::bin()?
        .arg("tests/inputs/malformed.csv")
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot read 'tests/inputs/malformed.csv'"))
        .stderr(predicates::str::contains("CSV error"));
    Ok(())
}

// A `String` filename cannot hold a non-UTF-8 path, and clap rejected the whole
// argument before `shelve` ran. It has to reach `File::open` instead.
#[cfg(unix)]
#[test]
fn test_non_utf8_filename_is_named_in_error() -> TestResult {
    let path = OsString::from_vec(b"tests/inputs/no-such-\xff-file.csv".to_vec());

    common::bin()?
        .arg(path)
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("cannot open 'tests/inputs/no-such-"));
    Ok(())
}

// The argument must also work when the file exists. Reusing the committed
// fixture under a non-UTF-8 name keeps the expectation identical.
#[cfg(unix)]
#[test]
fn test_non_utf8_filename_reads_committed_fixture() -> TestResult {
    let mut name: Vec<u8> = format!("shelve-non-utf8-fixture-{}.csv", std::process::id()).into_bytes();
    name.push(0xff);
    let path = std::env::temp_dir().join(OsString::from_vec(name));

    let expected = fs::read_to_string("tests/expected/default-column.txt")?;
    // Read the fixture first: a missing input must still fail loudly, unlike
    // the platform refusing to create the odd name below.
    let csv = fs::read("tests/inputs/tasks.csv")?;

    // APFS and HFS+ refuse to create a non-UTF-8 filename, so on macOS there
    // is nothing to read here; argument parsing is covered by the test above.
    if let Err(err) = fs::write(&path, &csv) {
        eprintln!("skipping: this filesystem cannot hold a non-UTF-8 filename: {err}");
        return Ok(());
    }

    // Capture rather than assert directly, so the temp file is still removed on failure.
    let out = common::bin()?.arg(&path).output()?;
    fs::remove_file(&path)?;

    assert!(
        out.status.success(),
        "shelve exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout)?, expected);
    Ok(())
}

// ---------------------------------------------------------------
// Edge cases around the input bytes and the shape of a row, rather
// than around argument parsing. Each one pins a behaviour that is
// easy to break without touching the code path the tests above cover.
// ---------------------------------------------------------------

// The grouping column is the only column, so every printed row omits its one
// field and comes out as a bare newline. The group headers and the blank line
// that follows each group still have to be there: "no columns left to print"
// must not collapse the layout.
#[test]
fn test_single_column_file_prints_headers_and_blank_rows() -> TestResult {
    run_ok(
        &["-c", "1", "tests/inputs/single-column.csv"],
        "tests/expected/single-column.txt",
    )
}

// Rows that share a key but are not adjacent in the input must still land in
// one group. Grouping is by map lookup, not by run-length, so interleaved keys
// are the case that a "group while the key is unchanged" implementation would
// silently split.
#[test]
fn test_non_adjacent_duplicate_keys_group_together() -> TestResult {
    run_ok(
        &["-c", "3", "tests/inputs/duplicate-keys.csv"],
        "tests/expected/duplicate-keys.txt",
    )
}

// A CRLF file is what a Windows user actually has. The csv reader strips the
// trailing `\r` from the last field, so it must not reach the output either:
// a stray carriage return is invisible in a terminal and corrupts a redirect.
#[test]
fn test_crlf_input_leaves_no_carriage_return_in_output() -> TestResult {
    let out = common::bin()?.args(["-c", "3", "tests/inputs/crlf.csv"]).output()?;
    assert!(out.status.success(), "shelve exited with {}", out.status);

    let stdout = String::from_utf8(out.stdout)?;
    assert!(
        !stdout.contains('\r'),
        "output still contains a carriage return:\n{stdout:?}"
    );

    let expected = fs::read_to_string("tests/expected/crlf-column-3.txt")?;
    assert_eq!(stdout, expected);
    Ok(())
}

// A UTF-8 BOM is invisible and common (Excel writes it). It belongs to the
// header record, which the reader consumes, so it must neither shift the
// columns nor leak into a group name: the output has to be byte-identical to
// the same file without the BOM, which is exactly what this compares against.
#[test]
fn test_utf8_bom_does_not_shift_columns_or_leak() -> TestResult {
    run_ok(&["tests/inputs/utf8-bom.csv"], "tests/expected/default-column.txt")
}

// A header with no records is an empty list, not an error — and not a warning
// either. The "column is missing" warning fires on records that lack the
// grouping column; with no records there is nothing to warn about, and a
// warning here would be indistinguishable from real data loss.
#[test]
fn test_header_only_file_is_empty_not_an_error() -> TestResult {
    common::bin()?.arg("tests/inputs/header-only.csv").assert().success().stdout("").stderr("");
    Ok(())
}

// A row far wider than the fixtures above. Printing omits one field and joins
// the rest, so width is where an off-by-one in the separator logic shows up:
// at 32 columns a leading, trailing or doubled ", " is unmissable.
#[test]
fn test_very_wide_row() -> TestResult {
    run_ok(&["-c", "1", "tests/inputs/wide-row.csv"], "tests/expected/wide-row.txt")
}

// Non-ASCII keys have to survive both as map keys and as printed group names.
#[test]
fn test_unicode_in_grouping_column() -> TestResult {
    run_ok(
        &["-c", "2", "tests/inputs/unicode-keys.csv"],
        "tests/expected/unicode-keys.txt",
    )
}

// Groups are ordered by UTF-8 bytes, because that is `BTreeMap<String, _>`'s
// order: "Zürich" (0x5A…) before "äteam" (0xC3 0xA4…) before "東京" (0xE4…)
// before "🎉 party" (0xF0…). Note the capital sorts first — this is byte order,
// not a human or locale collation. It is the one ordering guarantee the output
// makes, and swapping the map for a case-insensitive or natural-sort one would
// break every fixture above without failing a single assertion in them.
#[test]
fn test_groups_are_ordered_by_utf8_bytes() -> TestResult {
    let out = common::bin()?.args(["-c", "2", "tests/inputs/unicode-keys.csv"]).output()?;
    let stdout = String::from_utf8(out.stdout)?;
    let headers: Vec<&str> = stdout.lines().filter_map(|line| line.strip_suffix(':')).collect();

    assert_eq!(
        headers,
        vec!["Zürich", "äteam", "東京", "🎉 party"],
        "group order changed"
    );
    Ok(())
}

// `shelve big.csv | head -1` is a normal way to use this program, and the
// reader closing early is not a failure. Rust ignores SIGPIPE, so the write
// surfaces as EPIPE and `main` maps it to a zero exit; without that, every
// pipe into `head` or `less` would print an error nobody is left to read.
//
// Unix-only: Windows has no SIGPIPE and maps ERROR_BROKEN_PIPE differently, and
// the timing below was not verified there.
#[cfg(unix)]
#[test]
fn test_closed_stdout_exits_zero_without_an_error() -> TestResult {
    use std::fmt::Write as _;
    use std::io::Write as _;
    use std::process::Stdio;

    // The output has to exceed the 64 KiB pipe buffer. `shelve` reads all of
    // stdin before writing anything, so a small result would land in the buffer
    // and the program would exit 0 without ever seeing the closed read end —
    // the test would pass while asserting nothing.
    let mut input = String::from("id,filler\n");
    let filler = "x".repeat(80);
    for row in 0..20_000 {
        writeln!(input, "{row},{filler}").expect("writing to a String cannot fail");
    }

    // `std::process::Command` rather than `assert_cmd`'s: the assertion is about
    // a closed pipe, so the test needs the live child, and `assert_cmd::Command`
    // keeps `spawn` private in favour of its own collect-and-compare helpers.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_shelve"))
        .args(["-c", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Feeding stdin before closing stdout cannot deadlock: the child drains it
    // into memory and only starts writing once this returns.
    child.stdin.take().expect("stdin was piped").write_all(input.as_bytes())?;

    // Close the read end, which is what `| head -1` does after its first line.
    drop(child.stdout.take());

    let out = child.wait_with_output()?;
    let stderr = String::from_utf8(out.stderr)?;
    assert!(
        out.status.success(),
        "a closed stdout must exit 0, got {}: {stderr}",
        out.status
    );
    assert!(
        !stderr.contains("Error:"),
        "a closed stdout is not an error, but stderr said: {stderr}"
    );
    Ok(())
}

// The other half of the same contract, and a different mechanism. With fd 1
// closed outright — `shelve file.csv >&-` — there is no pipe and so no EPIPE:
// Rust's runtime substitutes `/dev/null` for any standard descriptor that is
// closed at startup, so every write succeeds into nowhere and the run is clean.
//
// Pinned because the outcome (exit 0, silent) is identical to the broken-pipe
// case above while the cause is not, which makes it easy to "fix" one and
// quietly break the other. Driven through `/bin/sh` because `Stdio` has no
// variant for "closed", and a child that merely inherits or is handed
// `/dev/null` would assert nothing.
#[cfg(unix)]
#[test]
fn test_stdout_closed_entirely_exits_zero() -> TestResult {
    // `sh -c <script> <argv0> <argv1> <argv2>` binds $0/$1/$2 to the trailing
    // operands, so the program and its fixture stay data rather than being
    // interpolated into the script text.
    let out = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "\"$1\" \"$2\" >&-",
            "sh",
            env!("CARGO_BIN_EXE_shelve"),
            "tests/inputs/tasks.csv",
        ])
        .stderr(std::process::Stdio::piped())
        .output()?;

    let stderr = String::from_utf8(out.stderr)?;
    assert!(
        out.status.success(),
        "a closed stdout must exit 0, got {}: {stderr}",
        out.status
    );
    assert_eq!(stderr, "", "nothing may be printed to stderr");
    Ok(())
}

/// A custom delimiter (`\t`) must be respected for both parsing and grouping.
/// This is the acceptance test for issue #17: `-d` / `--delimiter`.
#[test]
fn test_custom_delimiter_tab() -> TestResult {
    run_ok(
        &["-d", "\t", "-c", "1", "tests/inputs/tasks.tsv"],
        "tests/expected/delimiter-tsv.txt",
    )
}

/// Without `--no-headers`, the first record is consumed as a header row.
/// With it, every record is data — including the first one. The fixture
/// has three rows and no header; grouping on column 3 must produce all
/// three rows across two groups, not silently drop the first.
#[test]
fn test_no_headers_treats_first_record_as_data() -> TestResult {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_shelve"))
        .args(["--no-headers", "-c", "3", "tests/inputs/headerless.csv"])
        .output()?;

    assert!(out.status.success(), "exited with {}", out.status);
    let stdout = String::from_utf8(out.stdout)?;

    // All three input rows must appear in the output.
    assert!(stdout.contains("1, Deploy"), "first row missing: {stdout}");
    assert!(stdout.contains("2, Triage"), "second row missing: {stdout}");
    assert!(stdout.contains("3, Refactor"), "third row missing: {stdout}");

    // Two groups, sorted by key.
    assert!(stdout.contains("Jane:"), "Jane group missing");
    assert!(stdout.contains("John:"), "John group missing");

    Ok(())
}

/// A quoted CSV field containing an embedded newline must not corrupt the
/// group header. The header line is the structural delimiter of the grouped
/// layout; a raw newline inside it creates a stray line that parsers mistake
/// for a separate group. The fix renders embedded newlines as the two-character
/// sequence `\n` inside headers only (row fields remain unescaped to preserve
/// the zero-allocation print path).
#[test]
fn test_multiline_group_header_does_not_corrupt_layout() -> TestResult {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_shelve"))
        .args(["-c", "2", "tests/inputs/multiline-field.csv"])
        .output()?;

    assert!(out.status.success(), "exited with {}", out.status);
    let stdout = String::from_utf8(out.stdout)?;

    // Every header line must end with exactly ":" followed by a newline.
    // No header may contain a raw newline.
    for line in stdout.lines() {
        if let Some(name) = line.strip_suffix(':') {
            assert!(!name.contains('\n'), "header line contains embedded newline: {line:?}");
        }
    }

    // The two multiline keys must appear with \n rendered literally.
    assert!(
        stdout.contains("first\\nsecond:"),
        "expected escaped header, got:\n{stdout}"
    );
    assert!(
        stdout.contains("another\\nmultiline:"),
        "expected escaped header, got:\n{stdout}"
    );

    Ok(())
}
