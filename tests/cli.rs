use assert_cmd::Command;
use std::fs;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// --------------------------------------------------
fn run(args: &[&str], expected_file: &str) -> TestResult {
    let expected = fs::read_to_string(expected_file)?;
    Command::cargo_bin("shelve")?.args(args).assert().success().stdout(expected);
    Ok(())
}

fn run_reading_from_stdin(stdin_file: &str, args: &[&str], expected_file: &str) -> TestResult {
    let input = fs::read_to_string(stdin_file)?;
    let expected = fs::read_to_string(expected_file)?;
    Command::cargo_bin("shelve")?.args(args).write_stdin(input).assert().success().stdout(expected);
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

#[test]
fn test_zero_column() -> TestResult {
    Command::cargo_bin("shelve")?
        .args(&["-c", "0", "tests/inputs/tasks.csv"])
        .assert()
        .failure()
        .stderr("Error: Column number must be greater than 0\n");
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
    Command::cargo_bin("shelve")?.args(&["--foobar", "tests/inputs/tasks.csv"]).assert().failure().stderr(expected);
    Ok(())
}

// test a case where -c option is higher than the number of columns
#[test]
fn test_too_high_column() -> TestResult {
    run_reading_from_stdin("tests/inputs/tasks.csv", &["-c", "20"], "tests/expected/empty.txt")
}
