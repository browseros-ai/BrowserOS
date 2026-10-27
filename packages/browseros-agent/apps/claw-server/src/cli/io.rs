//! The only module permitted to write to the process streams.
//!
//! The workspace denies `print_stdout`/`print_stderr` as warnings and CI runs
//! `-D warnings`, so a crate-wide allow would silence the lint everywhere and a
//! stray `println!` in a command module could quietly corrupt `--json` output.
//! Allowing it here instead keeps every other module honest.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);

pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Tool output and command results. Always stdout, so `--json` stays pipeable.
pub fn out(text: &str) {
    println!("{text}");
}

/// The CLI's own notices: implicit pairing, session recovery, clamped values.
/// Always stderr, and suppressed by `--quiet`.
pub fn note(text: &str) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    eprintln!("{text}");
}

/// Errors. Never suppressed, because a silent failure is worse than a noisy one.
pub fn fail(text: &str) {
    eprintln!("{text}");
}

/// Whether decoration is wanted. Agents read piped output, so the default off a
/// terminal is plain, and `NO_COLOR` wins either way.
#[must_use]
pub fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}
