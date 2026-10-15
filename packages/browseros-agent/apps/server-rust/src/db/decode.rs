//! Lenient column reads for rows the TS server may have written.
//!
//! SQLite stores each value's own type, not the column's: a JS number that is not an integer
//! lands as REAL in an INTEGER column, and drizzle hands it back unchanged. sqlx refuses to decode
//! REAL as `i64`, which would fail a whole list over one odd row, so numeric reads here fall back
//! across storage classes the way `Number(value)` would. Booleans follow drizzle's boolean mode,
//! `Number(value) === 1`.

use sea_orm::{DbErr, QueryResult};

/// One result row, read by column name.
pub(super) struct Columns<'a> {
    row: &'a QueryResult,
    prefix: &'a str,
}

impl<'a> Columns<'a> {
    pub(super) fn new(row: &'a QueryResult, prefix: &'a str) -> Self {
        Self { row, prefix }
    }

    pub(super) fn text(&self, column: &str) -> Result<String, DbErr> {
        self.opt_text(column)?.ok_or_else(|| null_column(column))
    }

    pub(super) fn opt_text(&self, column: &str) -> Result<Option<String>, DbErr> {
        self.row.try_get::<Option<String>>(self.prefix, column)
    }

    pub(super) fn int(&self, column: &str) -> Result<i64, DbErr> {
        self.opt_int(column)?.ok_or_else(|| null_column(column))
    }

    /// An integer column. A REAL value is truncated toward zero: the field is an integer here,
    /// where TS would have echoed the fraction.
    pub(super) fn opt_int(&self, column: &str) -> Result<Option<i64>, DbErr> {
        match self.row.try_get::<Option<i64>>(self.prefix, column) {
            Ok(value) => Ok(value),
            Err(integer_error) => self
                .row
                .try_get::<Option<f64>>(self.prefix, column)
                .map(|value| value.map(|number| number as i64))
                .map_err(|_| integer_error),
        }
    }

    pub(super) fn real(&self, column: &str) -> Result<f64, DbErr> {
        match self.row.try_get::<Option<f64>>(self.prefix, column) {
            Ok(value) => value.ok_or_else(|| null_column(column)),
            Err(real_error) => self
                .row
                .try_get::<Option<i64>>(self.prefix, column)
                .map_err(|_| real_error)?
                .map(|number| number as f64)
                .ok_or_else(|| null_column(column)),
        }
    }

    /// drizzle's `integer({ mode: 'boolean' })` read: `Number(value) === 1`, so NULL is false.
    pub(super) fn boolean(&self, column: &str) -> Result<bool, DbErr> {
        match self.row.try_get::<Option<i64>>(self.prefix, column) {
            Ok(value) => Ok(value == Some(1)),
            Err(integer_error) => self
                .row
                .try_get::<Option<f64>>(self.prefix, column)
                .map(|value| value == Some(1.0))
                .map_err(|_| integer_error),
        }
    }
}

fn null_column(column: &str) -> DbErr {
    DbErr::Type(format!("column {column} is NULL"))
}
