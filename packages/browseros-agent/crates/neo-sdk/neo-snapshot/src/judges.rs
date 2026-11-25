//! What every format's tests judge the Rust printing by, shared so each format reads the same
//! cases and draws the same trees.
//!
//! ```text
//!  neo-playwright-js build.ts ─▶ tests/data/cases/*.json ─▶ case_file: the committed cases, and
//!                            │                                the private sets of saved pages
//!                            └─▶ tests/data/*.js ──────────▶ bundles: today's renderer and
//!                                                             upstream's printer, in QuickJS
//!  generator: seeded random trees with every fact a page can carry, which both sides answer
//!
//!  each case's call, or a generated one ─▶ render(&snapshot, call) ─▶ Answer
//!                                              expected: held to today's text and to what
//!                                              today kept, in its form
//! ```
//!
//! Every judge crosses the crate's interface, `render`, so a case is held to what the snapshot
//! operation will receive: the text and what it may keep for the tab.
//!
//! The random runs are small, as the gate runs them on every change. A long run sets
//! `NEO_SNAPSHOT_DIFFERENTIAL_NODES`, `NEO_SNAPSHOT_DIFFERENTIAL_TREES` and
//! `NEO_SNAPSHOT_DIFFERENTIAL_SEED` (decimal, or hex after `0x`).

mod bundles;
mod case_file;
mod expected;
mod first_difference;
mod generator;

pub(crate) use bundles::Bundles;
pub(crate) use case_file::{CaseFile, NEO_CASES, PLAYWRIGHT_CASES};
pub(crate) use expected::Shown;
pub(crate) use first_difference::first_difference;
pub(crate) use generator::{EarlierKind, Generator, count, seed};
