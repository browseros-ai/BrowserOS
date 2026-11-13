//! Spike only (branch `feat/chromium-tree-spike`, never merged): how many bytes of CDP a
//! desktop snapshot moves, method by method.
//!
//! ```text
//!  neo-server's MeteredConnection ── record(method, sent, received) ──▶ TALLY (while ON)
//!  page.desktopSnapshot { meter: true } ── start() … the capture … take() ──▶ the op's result
//! ```
//!
//! The meter is off unless an op turns it on, because counting re-encodes every reply, which
//! costs time on a 50 MB accessibility tree: a run that measures time leaves it off, a run that
//! measures bytes turns it on. It counts the whole process, so it is only meaningful in a
//! container where one run is in flight.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use ts_rs::TS;

static ON: AtomicBool = AtomicBool::new(false);
static TALLY: Mutex<BTreeMap<String, MethodBytes>> = Mutex::new(BTreeMap::new());

/// One CDP method's traffic while the meter was on: its calls, the JSON bytes of their params
/// and of their replies (re-encoded by `serde_json`, so close to, not exactly, the wire).
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MethodBytes {
    /// The CDP method.
    pub method: String,
    /// Calls sent.
    pub calls: u64,
    /// Bytes of their params.
    pub sent: u64,
    /// Bytes of their replies.
    pub received: u64,
}

/// Whether the transport should count; checked before it encodes anything.
#[must_use]
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Adds one call's bytes, if the meter is on.
pub fn record(method: &str, sent: usize, received: usize) {
    if !enabled() {
        return;
    }
    let mut tally = TALLY.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = tally.entry(method.to_owned()).or_default();
    entry.calls += 1;
    entry.sent += sent as u64;
    entry.received += received as u64;
}

/// Clears the tally and turns the meter on.
pub(crate) fn start() {
    TALLY.lock().unwrap_or_else(PoisonError::into_inner).clear();
    ON.store(true, Ordering::Relaxed);
}

/// Turns the meter off and returns what it counted, by method.
pub(crate) fn take() -> Vec<MethodBytes> {
    ON.store(false, Ordering::Relaxed);
    let tally = std::mem::take(&mut *TALLY.lock().unwrap_or_else(PoisonError::into_inner));
    tally
        .into_iter()
        .map(|(method, bytes)| MethodBytes { method, ..bytes })
        .collect()
}
