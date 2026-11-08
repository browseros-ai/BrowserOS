//! The provenance gate (`crates/NEO_STYLE.md`, "Provenance"). It reads the neo crates'
//! production sources (`neo-runtime/src`, `neo-playwright-injected/src`,
//! `neo-quickjs-injected/src`) and checks every provenance line against Playwright's own sources,
//! vendored unchanged at the pinned commit; checks coverage for the files listed in
//! `tests/provenance-enforced.txt`; and keeps `crates/NEO_DEVIATIONS.md`, the generated register of
//! deviations, current.
//!
//! ```text
//! neo sources ─scan─► items and tags ─► truth, every file: grammar, citation, quote, value
//!                                    └► coverage, enforced files: tags, header, prose, wording
//! tags' citations ─► vendored Playwright (four trees, one pin) ◄─ sync, from a checkout
//! tags' Deviation: and Unexplained: lines ─► crates/NEO_DEVIATIONS.md
//! ```
//!
//! Files are read at run time through `CARGO_MANIFEST_DIR`, never `env!` or `include_str!`:
//! worktrees share one Cargo build dir, so a test binary built for another worktree must
//! still read this checkout.

mod fixtures;
mod lex;
mod playwright;
mod rules;
mod source;

use std::env;
use std::path::{Path, PathBuf};

use rules::{Gate, report};

fn crates_dir() -> PathBuf {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default()).join("..")
}

fn gate() -> Gate {
    Gate::load(&crates_dir()).unwrap_or_else(|error| panic!("the neo crates load: {error}"))
}

#[test]
fn every_neo_source_says_where_it_comes_from() {
    let problems = gate().source_problems();
    assert!(problems.is_empty(), "{}", report(&problems));
}

#[test]
fn vendored_playwright_is_the_cited_files_at_the_pin() {
    let problems = gate()
        .vendor_problems()
        .unwrap_or_else(|error| panic!("the vendored trees read: {error}"));
    assert!(problems.is_empty(), "{}", report(&problems));
}

/// `NEO_RUNTIME_WRITE_FIXTURES=1` rewrites the register first, as the other neo-runtime
/// fixtures do.
#[test]
fn the_deviation_register_is_current() {
    let gate = gate();
    if env::var_os("NEO_RUNTIME_WRITE_FIXTURES").is_some() {
        gate.write_register()
            .unwrap_or_else(|error| panic!("the register is writable: {error}"));
    }
    let problems = gate.register_problems();
    assert!(problems.is_empty(), "{}", report(&problems));
}

/// Vendors exactly the Playwright files the provenance lines cite that no other neo vendor
/// tree holds, read from the pinned commit's objects, with Playwright's LICENSE and NOTICE and
/// a `VENDOR.md` of their sha256.
#[test]
#[ignore = "writes vendor/playwright/: NEO_PLAYWRIGHT=<checkout at the pin> cargo test -p neo-runtime --test provenance -- --ignored sync"]
fn sync() {
    let checkout = env::var_os("NEO_PLAYWRIGHT")
        .unwrap_or_else(|| panic!("set NEO_PLAYWRIGHT to a Playwright checkout at the pin"));
    gate()
        .sync(Path::new(&checkout))
        .unwrap_or_else(|error| panic!("sync: {error}"));
}
