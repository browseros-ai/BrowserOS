//! `ServerActivity`: what OTA readiness (`GET /status` `can_update`) is computed from — active
//! chat runs plus in-flight `/mcp` POSTs (an axum layer on `/mcp` increments around each request).

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

    /// Held for the lifetime of one chat run; dropping it decrements.
    pub fn run_started(&self) -> ActivityGuard<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(&self.active)
    }

    /// Owned form of [`Self::run_started`]: a chat run is registered by the request and driven by
    /// a spawned task, so its reservation must move between them without a gap.
    pub fn run_started_owned(self: &Arc<Self>) -> McpPostGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        McpPostGuard(self.clone())
    }

    /// Held for the lifetime of one `/mcp` POST.
    pub fn mcp_post_started(&self) -> ActivityGuard<'_> {
        self.active.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(&self.active)
    }

    /// The HTTP response body can outlive its handler future. An owned guard
    /// keeps OTA busy until the transport finishes or the caller disconnects.
    pub fn mcp_post_started_owned(self: &Arc<Self>) -> McpPostGuard {
        self.active.fetch_add(1, Ordering::SeqCst);
        McpPostGuard(self.clone())
    }

    pub fn can_update(&self) -> bool {
        self.active.load(Ordering::SeqCst) == 0
    }
}

pub struct ActivityGuard<'a>(&'a AtomicUsize);

/// A transport-owned reservation that survives the middleware-to-body handoff.
pub struct McpPostGuard(Arc<ServerActivity>);

impl Drop for McpPostGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
