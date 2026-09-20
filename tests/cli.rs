use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

// Unix-only, for the non-UTF-8 filename tests below.
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// clap prints the invoked binary's name in its usage text, so on Windows the
// program reports `shelve.exe` while the committed fixtures say `shelve`.
// Normalising the *actual* output keeps one set of expected files.
fn normalize_usage(text: &str) -> String {
    text.replace("shelve.exe", "shelve")
}

// The predicate is handed the raw output bytes, hence the UTF-8 check inside.
fn output_matches(expected: String) -> impl Predicate<[u8]> {
    predicates::function::function(move |bytes: &[u8]| match std::str::from_utf8(bytes) {
        Ok(text) => normalize_usage(text) == expected,
        Err(_) => false,
    })
    .fn_name("output_matches")
}

// --------------------------------------------------
fn run(args: &[&str], expected_file: &str) -> TestResult {
    let expected = fs::read_to_string(expected_file)?;
    Command::cargo_bin("shelve")?.args(args).assert().success().stdout(output_matches(expected));
    Ok(())
}

fn run_reading_from_stdin(stdin_file: &str, args: &[&str], expected_file: &str) -> TestResult {
    let input = fs::read_to_string(stdin_file)?;
    let expected = fs::read_to_string(expected_file)?;
    Command::cargo_bin("shelve")?.args(args).write_stdin(input).assert().success().stdout(output_matches(expected));
    Ok(())
}

//---------------------------------------------------
#[test]
fn test_help() -> TestResult {
    run(&["--help"], "tests/expected/help.txt")
}

#[test]
fn test_version() -> TestResult {
    let expected = format!("shelve {}\n", env!("CARGO_PKG_VERSION"));
    Command::cargo_bin("shelve")?.arg("--version").assert().success().stdout(expected);
    Ok(())
}

// Column zero cannot be represented by the argument type, so clap rejects it
// while parsing: a usage error with exit code 2, not a runtime `Error:` with 1.
#[test]
fn test_zero_column() -> TestResult {
    Command::cargo_bin("shelve")?
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
    Command::cargo_bin("shelve")?
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
    run(&["tests/inputs/tasks.csv"], "tests/expected/default-column.txt")
}

#[test]
fn test_first_column() -> TestResult {
    run(&["-c", "1", "tests/inputs/tasks.csv"], "tests/expected/column-1.txt")
}

#[test]
fn test_2nd_column() -> TestResult {
    run(&["-c", "2", "tests/inputs/tasks.csv"], "tests/expected/column-2.txt")
}

#[test]
fn test_3rd_column() -> TestResult {
    run(&["-c", "3", "tests/inputs/tasks.csv"], "tests/expected/column-3.txt")
}

#[test]
fn test_4th_column() -> TestResult {
    run(&["-c", "4", "tests/inputs/tasks.csv"], "tests/expected/column-4.txt")
}

#[test]
fn test_5th_column() -> TestResult {
    run(&["-c", "5", "tests/inputs/tasks.csv"], "tests/expected/column-5.txt")
}

#[test]
fn test_tw0_files() -> TestResult {
    run(
        &["-c", "5", "tests/inputs/tasks.csv", "tests/inputs/more-tasks.csv"],
        "tests/expected/two-files.txt",
    )
}

#[test]
fn test_read_from_stdin() -> TestResult {
    run_reading_from_stdin("tests/inputs/tasks.csv", &["-c", "5"], "tests/expected/stdin.txt")
}

#[test]
fn test_unexpected_argument() -> TestResult {
    let expected = fs::read_to_string("tests/expected/unexpected-argument.txt")?;
    Command::cargo_bin("shelve")?
        .args(["--foobar", "tests/inputs/tasks.csv"])
        .assert()
        .failure()
        .stderr(output_matches(expected));
    Ok(())
}

// With no filename argument the input must come from stdin; an empty stream is
// a valid empty list, not an error about a missing argument.
#[test]
fn test_no_filename_arg_reads_stdin() -> TestResult {
    run_reading_from_stdin("tests/inputs/empty.csv", &[], "tests/expected/empty.txt")
}

// A column number that is not a number is rejected by the parser, so it never
// reaches the grouping logic.
#[test]
fn test_non_integer_column_index() -> TestResult {
    Command::cargo_bin("shelve")?
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
    run_reading_from_stdin("tests/inputs/tasks.csv", &["-c", "20"], "tests/expected/empty.txt")
}

// ...but it must not stay silent about it. Empty stdout plus a zero exit code
// is otherwise indistinguishable from an empty input file.
#[test]
fn test_too_high_column_warns_on_stderr() -> TestResult {
    let input = fs::read_to_string("tests/inputs/tasks.csv")?;
    Command::cargo_bin("shelve")?
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
    let output = Command::cargo_bin("shelve")?.args(["-c", "20"]).write_stdin(input).output()?;
    let warnings = String::from_utf8(output.stderr)?.matches("Warning:").count();
    assert_eq!(warnings, 1, "expected exactly one warning");
    Ok(())
}

// A failing path has to be named, otherwise a multi-file invocation gives no
// clue which argument could not be opened.
#[test]
fn test_missing_file_is_named_in_error() -> TestResult {
    Command::cargo_bin("shelve")?
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
    Command::cargo_bin("shelve")?
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
    Command::cargo_bin("shelve")?
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

    Command::cargo_bin("shelve")?
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
    let out = Command::cargo_bin("shelve")?.arg(&path).output()?;
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
