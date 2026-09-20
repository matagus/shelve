//! A library for grouping CSV rows by a column and rendering the result.
//!
//! The primary entry point is [`GroupedData::from_files`], which reads one or
//! more CSV files (or stdin when no filenames are given), groups every record
//! by the chosen column, and returns a [`GroupedData`] that can render the
//! grouped layout into any [`Write`](std::io::Write) sink.
//!
//! # Column numbering
//!
//! Columns are numbered starting from 1, matching the CLI convention. The
//! [`ColumnNumber`] newtype wraps [`NonZeroUsize`](std::num::NonZeroUsize) so
//! that column zero is unrepresentable at the type level.

#![warn(missing_docs)]

mod groups;
#[cfg(test)]
mod testing;

pub use groups::{ColumnNumber, GroupedData, Row};
