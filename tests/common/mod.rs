//! Helpers shared by integration tests.
//!
//! `src/testing.rs` is `#[cfg(test)]` inside the crate, so integration tests
//! (separate binaries) cannot see it. This module is the integration-side
//! counterpart: one home for the error alias, CR-normalising comparison, and
//! the run-and-compare helpers that every `tests/*.rs` would otherwise
//! re-implement.
//!
//! Use `mod common;` at the top of an integration test file to pull these in.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

pub type TestError = Box<dyn std::error::Error>;

/// `T` defaults to `()` so test bodies read like `-> TestResult`.
pub type TestResult<T = ()> = Result<T, TestError>;

// clap prints the invoked binary's name in its usage text, so on Windows the
// program reports `shelve.exe` while the committed fixtures say `shelve`.
// Normalising the *actual* output keeps one set of expected files.
#[allow(dead_code)]
fn normalize_usage(text: &str) -> String {
    text.replace("shelve.exe", "shelve")
}

/// A predicate that compares raw output bytes against an expected string,
/// normalising both CRLF → LF and the Windows binary-name difference.
///
/// The predicate is handed the raw output bytes, hence the UTF-8 check inside.
/// Line endings are normalised too: without a `.gitattributes`, checkout gives
/// Windows runners CRLF fixtures where the program emits LF.
#[allow(dead_code)]
pub fn output_matches(expected: String) -> impl Predicate<[u8]> {
    predicates::function::function(move |bytes: &[u8]| match std::str::from_utf8(bytes) {
        Ok(text) => normalize_usage(&text.replace("\r\n", "\n")) == expected.replace("\r\n", "\n"),
        Err(_) => false,
    })
    .fn_name("output_matches")
}

/// An `assert_cmd::Command` pre-loaded with the `shelve` binary.
///
/// Every integration test resolves the binary the same way unless it
/// deliberately needs a raw `std::process::Command` (e.g. for spawn or
/// closed-pipe tests).
#[allow(dead_code)]
pub fn bin() -> Result<Command, TestError> {
    Ok(Command::cargo_bin("shelve")?)
}

/// Run `shelve` with `args`, assert success, and compare stdout against the
/// contents of `expected_file` (with CRLF and binary-name normalisation).
#[allow(dead_code)]
pub fn run_ok(args: &[&str], expected_file: &str) -> TestResult {
    let expected = fs::read_to_string(expected_file)?;
    bin()?.args(args).assert().success().stdout(output_matches(expected));
    Ok(())
}

/// Like [`run_ok`], but feeds the contents of `stdin_file` to stdin first.
#[allow(dead_code)]
pub fn run_ok_stdin(stdin_file: &str, args: &[&str], expected_file: &str) -> TestResult {
    let input = fs::read_to_string(stdin_file)?;
    let expected = fs::read_to_string(expected_file)?;
    bin()?.args(args).write_stdin(input).assert().success().stdout(output_matches(expected));
    Ok(())
}
