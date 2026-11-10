//! THROWAWAY spike instrument (proto-hook-pin). Never ships.
//!
//! Stands in for a `beforeAction` hook on `Frame.fill`: a fill whose value is exactly
//! [`PLACEHOLDER`] waits [`HOLD_MS`] at the point the [`VARIANT`] chooses, then types
//! [`SECRET`] instead. Tests set these statics; with [`ENABLED`] false nothing changes.
#![allow(missing_docs, clippy::all, clippy::pedantic)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

pub const PLACEHOLDER: &str = "{{password}}";
pub const SECRET: &str = "SECRET-123";

/// 0 = V0 (today, hold before the action), 1 = VS (document snapshot), 2 = VP (element pin),
/// 3 = VG (hold inside loop A after resolve).
pub static VARIANT: AtomicU8 = AtomicU8::new(0);
pub static ENABLED: AtomicBool = AtomicBool::new(false);
pub static HOLD_MS: AtomicU64 = AtomicU64::new(0);
/// Extra hold between the page's focus/select (`needsinput`) and `Input.insertText`, all variants.
pub static AFTER_FOCUS_HOLD_MS: AtomicU64 = AtomicU64::new(0);
/// Counts holds entered, so a test knows the call is held.
pub static HELD: AtomicU64 = AtomicU64::new(0);
pub static AFTER_FOCUS_HELD: AtomicU64 = AtomicU64::new(0);

static SENDS: Mutex<BTreeMap<&'static str, u64>> = Mutex::new(BTreeMap::new());

pub fn variant() -> u8 {
    VARIANT.load(Ordering::SeqCst)
}

pub fn active(value: &str) -> bool {
    ENABLED.load(Ordering::SeqCst) && value == PLACEHOLDER
}

pub fn hold() -> std::time::Duration {
    std::time::Duration::from_millis(HOLD_MS.load(Ordering::SeqCst))
}

pub fn after_focus_hold() -> std::time::Duration {
    std::time::Duration::from_millis(AFTER_FOCUS_HOLD_MS.load(Ordering::SeqCst))
}

/// Counts one CDP command sent by the runtime (any session, browser-level included).
pub fn count_send(method: &'static str) {
    if let Ok(mut sends) = SENDS.lock() {
        *sends.entry(method).or_default() += 1;
    }
}

/// Takes and clears the CDP command tally.
pub fn take_sends() -> BTreeMap<String, u64> {
    SENDS
        .lock()
        .map(|mut sends| {
            std::mem::take(&mut *sends)
                .into_iter()
                .map(|(method, count)| (method.to_string(), count))
                .collect()
        })
        .unwrap_or_default()
}

/// VS: every frame's document identity when the hold began.
#[derive(Default)]
pub(crate) struct ProtoSnapshot {
    pub(crate) frames: Vec<ProtoFrame>,
}

pub(crate) struct ProtoFrame {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) loader: String,
    /// The frame's cached world and its commit count, kept alive so the pointer stays unique.
    pub(crate) world: Option<(
        std::sync::Arc<crate::chromium_bridge::playwright_injected_methods::World>,
        u64,
    )>,
}
