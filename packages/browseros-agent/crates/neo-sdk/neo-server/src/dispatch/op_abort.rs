//! The script's abort of one op it started: Playwright's `signal` option.
//!
//! ```text
//!  realm   signal fires ─▶ __neo_invoke("$neo.abort", { key, reason })   (bootstrap.js)
//!  bridge  key ─▶ the OpAbort it registered when the op was submitted ─▶ abort(reason)
//!  op      OpScope's waits race OpCtx::aborted ─▶ the leaf's timeout path reports the abort
//! ```
//!
//! An abort never stops an op from outside, as the run's stop does: the op ends itself through
//! the path its own timeout takes, so the error carries the op's call log and the door derives
//! delivery as for a timeout (neo-playwright's `op_env.rs`). A method that never waits on it
//! simply finishes; the script's promise still settles with whatever the op answered.

use std::sync::{Arc, OnceLock};

use tokio_util::sync::CancellationToken;

/// One op's abort switch. The bridge holds one clone under the realm's key and the op's state
/// holds another, so an abort reaches the op wherever it is waiting. Write-once: the first
/// reason wins and a repeated abort changes nothing.
#[derive(Clone, Debug, Default)]
pub(crate) struct OpAbort(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    reason: OnceLock<String>,
    requested: CancellationToken,
}

impl OpAbort {
    /// Aborts the op with `reason`, the text its error prints.
    pub(crate) fn abort(&self, reason: String) {
        if self.0.reason.set(reason).is_ok() {
            self.0.requested.cancel();
        }
    }

    /// The reason, once the op was aborted.
    pub(crate) fn reason(&self) -> Option<&str> {
        self.0.reason.get().map(String::as_str)
    }

    /// Resolves once the op is aborted.
    pub(crate) async fn requested(&self) {
        self.0.requested.cancelled().await;
    }
}

#[cfg(test)]
mod tests {
    use super::OpAbort;

    #[tokio::test]
    async fn the_first_reason_wins_and_wakes_the_waiter() {
        let abort = OpAbort::default();
        assert_eq!(abort.reason(), None);
        let waiter = abort.clone();
        let woken = tokio::spawn(async move { waiter.requested().await });
        abort.abort("foo bar".to_owned());
        abort.abort("later".to_owned());
        assert!(woken.await.is_ok());
        assert_eq!(abort.reason(), Some("foo bar"));
    }
}
