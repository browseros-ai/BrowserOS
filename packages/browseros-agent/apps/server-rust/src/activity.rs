//! `ServerActivity`: what OTA readiness (`GET /status` `can_update`) is computed from — active
//! chat runs plus in-flight `/mcp` POSTs. Each holds a [`BusyGuard`] for its whole lifetime.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Default)]
pub struct ServerActivity {
    // TS reads both counters on one event-loop turn. A single Rust counter
    // preserves that atomic idle observation: separate loads could straddle a
    // chat/MCP handoff and report idle although at least one was always active.
    active: AtomicUsize,
}

impl ServerActivity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the server busy until the guard drops. The guard is owned rather than borrowed
    /// because both reservations change hands: a chat run is registered by its request and
    /// driven by a spawned task, and an `/mcp` response body outlives the middleware that
    /// reserved it, so the server stays busy until the transport finishes or the caller leaves.
    pub fn reserve(self: &Arc<Self>) -> BusyGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        BusyGuard(self.clone())
    }

    pub fn can_update(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }
}

/// One reservation of [`ServerActivity`]; dropping it releases the reservation.
pub struct BusyGuard(Arc<ServerActivity>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}
