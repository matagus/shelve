# shelve

A command-line tool written in Rust for pretty-printing CSV files grouped by a specified column or field.

[![Crates.io](https://img.shields.io/crates/v/shelve.svg)](https://crates.io/crates/shelve)
[![Documentation](https://docs.rs/shelve/badge.svg)](https://docs.rs/shelve)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## Installation

```bash
cargo install shelve
```

## Usage

```bash
shelve --help

Usage: shelve [OPTIONS] <FILENAME>

Arguments:
  <FILENAME>

Options:
  -c, --column-number <COLUMN_NUMBER>  Column number to group by [default: 0]
  -h, --help                           Print help
  -V, --version                        Print version
```

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

Grouping by the `Status` column (column number 2):

```bash
shelve -c 2 example.csv

Done:

2, Fix bug B, Done, Jane Doe, Low
6, Write missing documentation for feature A, Done, Peter Foo, Medium

To Do:

4, Refactor code, To Do, Jane Doe, High
5, Deploy to production A and B, To Do, John Doe, Low
7, Fix bug C, To Do, Alice Bar, High

In Progress:

3, Write tests for feature A, In Progress, John Doe, Medium
8, Write tests for feature A, In Progress, John Doe, Low
```

Grouping by the `Priority` column (column number 4):

```bash
shelve -c 4 example.csv

High:

1, Implement feature A, In Progress, John Doe, High
4, Refactor code, To Do, Jane Doe, High
7, Fix bug C, To Do, Alice Bar, High

Medium:

3, Write tests for feature A, In Progress, John Doe, Medium
6, Write missing documentation for feature A, Done, Peter Foo, Medium

Low:

2, Fix bug B, Done, Jane Doe, Low
5, Deploy to production A and B, To Do, John Doe, Low
8, Write tests for feature A, In Progress, John Doe, Low
```

Grouping by the `Assignee` column (column number 3):

```bash
shelve -c 3 example.csv

Jane Doe:

2, Fix bug B, Done, Jane Doe, Low
4, Refactor code, To Do, Jane Doe, High

John Doe:

1, Implement feature A, In Progress, John Doe, High
3, Write tests for feature A, In Progress, John Doe, Medium
5, Deploy to production A and B, To Do, John Doe, Low
8, Write tests for feature A, In Progress, John Doe, Low

Peter Foo:

6, Write missing documentation for feature A, Done, Peter Foo, Medium

Alice Bar:

7, Fix bug C, To Do, Alice Bar, High
```

The command can also read input from `stdin`:

```bash
>> cat sample-files/tasks.csv | shelve -c 4

High:

1, Implement feature A, In Progress, John Doe, High
4, Refactor code, To Do, Jane Doe, High
7, Fix bug C, To Do, Alice Bar, High

Low:

2, Fix bug B, Done, Jane Doe, Low
5, Deploy to production A and B, To Do, John Doe, Low
8, Write tests for feature A, In Progress, John Doe, Low

Medium:

3, Write tests for feature A, In Progress, John Doe, Medium
6, Write missing documentation for feature A, Done, Peter Foo, Medium
```

Or reading multiple files at once:

```bash
shelve -c 4 sample-files/tasks.csv sample-files/more-tasks.csv
```


## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
