//! Neo: The store's behaviour, checked by the values, rows and counts a host can read back, never by
//! log wording or timing. Events are built by hand from neo-runtime's public types: no runtime
//! runs and no browser is needed.

mod apply;
mod migrations;
mod open;
mod query;
mod retention;
mod support;
mod writer;
