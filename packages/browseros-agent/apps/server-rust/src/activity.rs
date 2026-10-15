//! `ServerActivity`: what OTA readiness (`GET /status` `can_update`) is computed from — active
//! chat runs plus in-flight `/mcp` POSTs (an axum layer on `/mcp` increments around each request).
//! Owned by P3.

use std::sync::atomic::{AtomicUsize, Ordering};

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

    pub fn can_update(&self) -> bool {
        self.active_runs.load(Ordering::SeqCst) == 0
            && self.in_flight_mcp_posts.load(Ordering::SeqCst) == 0
    }
}

pub struct ActivityGuard<'a>(&'a AtomicUsize);

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
