use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

type TestResult = Result<(), Box<dyn std::error::Error>>;

// --------------------------------------------------
#[test]
fn dies_no_args() -> TestResult {
    Command::cargo_bin("shelve")?
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage: shelve"));
    Ok(())
}

// --------------------------------------------------
fn run(args: &[&str], expected_file: &str) -> TestResult {
    let expected = fs::read_to_string(expected_file)?;
    Command::cargo_bin("shelve")?
        .args(args)
        .assert()
        .success()
        .stdout(expected);
    Ok(())
}

//---------------------------------------------------
#[test]
fn test_help() -> TestResult {
    run(&["--help"], "tests/expected/help.txt")
}

#[test]
fn test_version() -> TestResult {
    run(&["--version"], "tests/expected/version.txt")
}

#[test]
fn test_default_column() -> TestResult {
    run(
        &["tests/inputs/tasks.csv"],
        "tests/expected/default-column.txt",
    )
}

#[test]
fn test_first_column() -> TestResult {
    run(
        &["-c", "1", "tests/inputs/tasks.csv"],
        "tests/expected/column-1.txt",
    )
}

#[test]
fn test_2nd_column() -> TestResult {
    run(
        &["-c", "2", "tests/inputs/tasks.csv"],
        "tests/expected/column-2.txt",
    )
}

#[test]
fn test_3rd_column() -> TestResult {
    run(
        &["-c", "3", "tests/inputs/tasks.csv"],
        "tests/expected/column-3.txt",
    )
}

#[test]
fn test_4th_column() -> TestResult {
    run(
        &["-c", "4", "tests/inputs/tasks.csv"],
        "tests/expected/column-4.txt",
    )
}
