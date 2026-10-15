//! `ServerActivity`: what OTA readiness (`GET /status` `can_update`) is computed from — active
//! chat runs plus in-flight `/mcp` POSTs (an axum layer on `/mcp` increments around each request).
//! Owned by P3.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Default)]
pub struct ServerActivity {
    active_runs: AtomicUsize,
    in_flight_mcp_posts: AtomicUsize,
}

impl ServerActivity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Held for the lifetime of one chat run; dropping it decrements.
    pub fn run_started(&self) -> ActivityGuard<'_> {
        self.active_runs.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(&self.active_runs)
    }

    /// Held for the lifetime of one `/mcp` POST.
    pub fn mcp_post_started(&self) -> ActivityGuard<'_> {
        self.in_flight_mcp_posts.fetch_add(1, Ordering::SeqCst);
        ActivityGuard(&self.in_flight_mcp_posts)
    }

    /// The HTTP response body can outlive its handler future. An owned guard
    /// keeps OTA busy until the transport finishes or the caller disconnects.
    pub fn mcp_post_started_owned(self: &Arc<Self>) -> McpPostGuard {
        self.in_flight_mcp_posts.fetch_add(1, Ordering::SeqCst);
        McpPostGuard(self.clone())
    }

    pub fn can_update(&self) -> bool {
        self.active_runs.load(Ordering::SeqCst) == 0
            && self.in_flight_mcp_posts.load(Ordering::SeqCst) == 0
    }
}

pub struct ActivityGuard<'a>(&'a AtomicUsize);

/// A transport-owned reservation that survives the middleware-to-body handoff.
pub struct McpPostGuard(Arc<ServerActivity>);

impl Drop for McpPostGuard {
    fn drop(&mut self) {
        self.0.in_flight_mcp_posts.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
