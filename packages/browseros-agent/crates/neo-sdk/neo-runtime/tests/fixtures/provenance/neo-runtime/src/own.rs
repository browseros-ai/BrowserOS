//! Neo: a fixture module of Neo's own, left off the enforced list.
//!
//! It mirrors `server/frames.ts` in prose, which only a file off the list may do.

use std::time::Duration;

const POLL_MS: u64 = 10;

pub(crate) async fn wait() {
    tokio::time::sleep(Duration::from_millis(10)).await;
}
