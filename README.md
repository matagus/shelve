# shelve

A command-line tool written in Rust for pretty-printing CSV files grouped by a specified column or field.

[![Crates.io](https://img.shields.io/crates/v/shelve.svg)](https://crates.io/crates/shelve)
[![Documentation](https://docs.rs/shelve/badge.svg)](https://docs.rs/shelve)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

![shelve demo](demo/shelve.gif)

## Installation

### Homebrew (macOS)

```bash
brew tap matagus/tap
brew install shelve
```

### Cargo

```bash
cargo install shelve
```

## Usage

```text
A simple command-line tool to pretty print CSV files grouped by a column

Usage: shelve [OPTIONS] [FILENAMES]...

Arguments:
  [FILENAMES]...  

Options:
  -c, --column-number <COLUMN_NUMBER>  Column number to group by [default: 1]
      --no-headers                     Treat the first record as data instead of a header row
  -d, --delimiter <DELIMITER>          Field delimiter character (default: ',') [default: ,]
  -h, --help                           Print help
  -V, --version                        Print version
```

> **Note:** This block is the verbatim output of `shelve --help`. A CI step
> checks that it stays in sync with the binary; if you add or change a flag,
> update both the code and this section (or run
> `scripts/update-readme-help.sh` to regenerate it).

### Strict rows

By default `shelve` requires every row to have the same number of fields as
the header. A short or long row aborts the entire run with exit 1 after the
whole input has been read:

```console
$ printf 'id,a,b\n1,x,y\n2,z\n' | shelve -c 1
Error: CSV error: record 2 (line: 3, byte: 13): found record with 2 fields, but the previous record has 3 fields
```

This catches malformed exports early. If your input has legitimate ragged
rows, pre-process it to pad or truncate fields before piping to `shelve`.

## Examples


Given the following CSV file containing data about tasks and their status:

```csv
Task ID,Task Title,Status,Assignee,Priority
1,Implement feature A,In Progress,John Doe,High
2,Fix bug B,Done,Jane Doe,Low
3,Write tests for feature A,In Progress,John Doe,Medium
4,Refactor code,To Do,Jane Doe,High
5,Deploy to production A and B,To Do,John Doe,Low
6,Write missing documentation for feature A,Done,Peter Foo,Medium
7,Fix bug C,To Do,Alice Bar,High
8,Write tests for feature A,In Progress,John Doe,Low
```

Grouping by the `Status` column (column number 3):

```bash
shelve -c 3 sample-files/tasks.csv

Done:

2, Fix bug B, Jane Doe, Low
6, Write missing documentation for feature A, Peter Foo, Medium

In Progress:

1, Implement feature A, John Doe, High
3, Write tests for feature A, John Doe, Medium
8, Write tests for feature A, John Doe, Low

To Do:

4, Refactor code, Jane Doe, High
5, Deploy to production A and B, John Doe, Low
7, Fix bug C, Alice Bar, High
```

Grouping by the `Priority` column (column number 5):

```bash
shelve -c 5 sample-files/tasks.csv

High:

1, Implement feature A, In Progress, John Doe
4, Refactor code, To Do, Jane Doe
7, Fix bug C, To Do, Alice Bar

Low:

2, Fix bug B, Done, Jane Doe
5, Deploy to production A and B, To Do, John Doe
8, Write tests for feature A, In Progress, John Doe

Medium:

3, Write tests for feature A, In Progress, John Doe
6, Write missing documentation for feature A, Done, Peter Foo
```

Grouping by the `Assignee` column (column number 4):

```bash
shelve -c 4 sample-files/tasks.csv

Alice Bar:

7, Fix bug C, To Do, High

Jane Doe:

2, Fix bug B, Done, Low
4, Refactor code, To Do, High

John Doe:

1, Implement feature A, In Progress, High
3, Write tests for feature A, In Progress, Medium
5, Deploy to production A and B, To Do, Low
8, Write tests for feature A, In Progress, Low

Peter Foo:

6, Write missing documentation for feature A, Done, Medium
```

The command can also read input from `stdin`:

```bash
cat sample-files/tasks.csv | shelve -c 5

High:

1, Implement feature A, In Progress, John Doe
4, Refactor code, To Do, Jane Doe
7, Fix bug C, To Do, Alice Bar

Low:

2, Fix bug B, Done, Jane Doe
5, Deploy to production A and B, To Do, John Doe
8, Write tests for feature A, In Progress, John Doe

Medium:

3, Write tests for feature A, In Progress, John Doe
6, Write missing documentation for feature A, Done, Peter Foo
```

Or reading multiple files at once:

```bash
shelve -c 5 sample-files/tasks.csv sample-files/more-tasks.csv
```

### Treating the first row as data (`--no-headers`)

If your file has no header row, pass `--no-headers` so the first line is
grouped as data instead of being consumed as column names:

```bash
shelve --no-headers -c 1 sample-files/tasks.csv
```

### Tab-separated and other delimiters (`-d`)

Use `-d` to change the field delimiter. For a TSV file:

```bash
shelve -d $'\t' -c 1 tests/inputs/tasks.tsv
```

Output:

```text
Alice:

Deploy, 1
Refactor, 3

Bob:

Triage, 2
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
