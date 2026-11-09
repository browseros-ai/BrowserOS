//! Neo: A tab's presence: the one host-owned CDP session on a touched tab that turns focus emulation
//! on, so a background tab keeps painting for its captures and screenshots between runs. Runs
//! cannot provide it: each run's session detaches when the run ends (`runtime.rs`). It plays the
//! role browseros-core's long-lived per-tab attach played for claw-server's capture.
//!
//! Platform: focus emulation keeps background painting per attached session, but `document.hasFocus()` is a page-shared flag cleared by an emulating session detaching or disabling it (`designs/neo-host-events/grounding/probes.md`, BrowserOS neo 155 probes 1, 2 and P-A).
//! Measured headless and headed:
//! - a background tab paints only while some attached session has focus emulation on, counted
//!   per session: one session's detach or `enabled: false` does not stop another's painting;
//! - `document.hasFocus()` is one shared flag: any emulating session's detach, or any
//!   `enabled: false`, turns it false for the page, even while another session still emulates;
//! - a session that never enabled emulation (a screencast, a screenshot) attaches and detaches
//!   without changing focus or visibility.
//!
//! So exactly one host session per tab enables emulation, the presence, and never sends
//! `enabled: false`. A capture ensures the presence before it starts, and capture sessions enable
//! nothing. A presence detaches only
//! when the caller's predicate says no live session touched the tab and no run is in flight on
//! it ([`Presences::prune`]), or when the tab is gone: its detach clears page focus, so it must
//! never happen under a running script. A presence also outlives the captures of its tab, which
//! stop a grace period after their last viewer.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::{Mutex, OnceCell};
use tokio::task::JoinHandle;

use crate::host::cdp;
use crate::host::link::{Attached, HostLink, LinkError};
use crate::run::PageId;

/// Every tab's presence, by target. A cell is empty while its attach is in flight or after it
/// failed; a later [`Presences::ensure`] fills it.
pub(crate) struct Presences {
    link: Arc<HostLink>,
    tabs: Mutex<HashMap<PageId, Arc<OnceCell<Arc<Presence>>>>>,
}

/// One tab's presence, attached and emulating focus.
pub(crate) struct Presence {
    session: Attached,
}

impl Presences {
    pub(crate) fn new(link: Arc<HostLink>) -> Self {
        Self {
            link,
            tabs: Mutex::new(HashMap::new()),
        }
    }

    /// Keeps a screenshot operation owned after its caller times out, until the private
    /// socket replies or shuts down. Its process-wide slot lives for that whole operation.
    pub(super) fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.link.spawn(future)
    }

    /// The tab's presence, attached and emulating focus; concurrent first uses share one
    /// attach. A presence whose session is gone (its tab closed, or the socket was replaced) is
    /// replaced.
    pub(crate) async fn ensure(&self, target: &PageId) -> Result<Arc<Presence>, LinkError> {
        let cell = {
            let mut tabs = self.tabs.lock().await;
            let cell = tabs.entry(target.clone()).or_default();
            if cell
                .get()
                .is_some_and(|presence| presence.session.is_gone())
            {
                *cell = Arc::default();
            }
            Arc::clone(cell)
        };
        let link = Arc::clone(&self.link);
        let target = target.clone();
        // The attach runs on a task of its own, so a caller that gives up (a follower's switch
        // timeout, a viewer that left) cannot drop an attach the browser still answers: that
        // session would stay attached with nobody to detach it.
        let attaching = self.link.spawn(async move {
            cell.get_or_try_init(|| attach(&link, &target))
                .await
                .map(Arc::clone)
        });
        // The task only fails to finish when the runtime is shutting down.
        attaching.await.unwrap_or(Err(LinkError::Down))
    }

    /// Detaches every presence `needed` refuses, and forgets those whose session is already
    /// gone. `needed` is the ledger's rule: a live session touched the tab, or a run is in
    /// flight on it. A presence still attaching is left for the next prune.
    pub(crate) async fn prune(&self, needed: impl Fn(&PageId) -> bool) {
        let mut doomed = Vec::new();
        self.tabs
            .lock()
            .await
            .retain(|target, cell| match cell.get() {
                None => true,
                Some(presence) if presence.session.is_gone() => false,
                Some(presence) => {
                    let keep = needed(target);
                    if !keep {
                        doomed.push(Arc::clone(presence));
                    }
                    keep
                }
            });
        for presence in doomed {
            let link = Arc::clone(&self.link);
            self.link
                .spawn(async move { link.release(&presence.session).await });
        }
    }
}

impl Presence {
    /// The presence's session, for reads that need the tab painting (an after-call screenshot).
    pub(crate) fn session(&self) -> &Attached {
        &self.session
    }
}

async fn attach(link: &HostLink, target: &PageId) -> Result<Arc<Presence>, LinkError> {
    let session = link.attach(target).await?;
    if let Err(error) = cdp::emulate_focus(&session).await {
        // The session never turned emulation on, so detaching it changes nothing on the page.
        // The next ensure tries again. A foreground tab paints anyway.
        link.release(&session).await;
        return Err(error.into());
    }
    Ok(Arc::new(Presence { session }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;
    use tokio::time;
    use tokio_util::sync::CancellationToken;

    use super::{Presence, Presences};
    use crate::fake_host_cdp::FakeHostCdp;
    use crate::host::link::HostLink;
    use crate::run::PageId;

    async fn presences(browser: &FakeHostCdp) -> Presences {
        let link = HostLink::connect(browser.port(), CancellationToken::new())
            .await
            .unwrap_or_else(|error| panic!("connect the host link: {error}"));
        Presences::new(link)
    }

    fn page(target: &str) -> PageId {
        PageId::new(target.to_string())
    }

    async fn ensure(presences: &Presences, target: &str) -> Arc<Presence> {
        presences
            .ensure(&page(target))
            .await
            .unwrap_or_else(|error| panic!("ensure the presence on {target}: {error}"))
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_first_uses_share_one_session_that_turns_emulation_on() {
        let browser = FakeHostCdp::start().await;
        let presences = presences(&browser).await;

        let (first, second) = tokio::join!(
            ensure(&presences, "target-1"),
            ensure(&presences, "target-1")
        );

        assert!(Arc::ptr_eq(&first, &second), "one presence per tab");
        assert_eq!(
            browser.sent("Target.attachToTarget").len(),
            1,
            "one attach for both callers"
        );
        let emulation = browser.sent("Emulation.setFocusEmulationEnabled");
        assert_eq!(
            emulation
                .iter()
                .map(|command| (command.session.clone(), command.params.clone()))
                .collect::<Vec<_>>(),
            [(
                Some(first.session().session.as_str().to_string()),
                json!({ "enabled": true })
            )],
            "emulation turned on once, on the presence's own session"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn prune_detaches_only_the_presences_no_one_needs() {
        let browser = FakeHostCdp::start().await;
        let presences = presences(&browser).await;
        let kept = ensure(&presences, "target-1").await;
        let unneeded = ensure(&presences, "target-2").await;

        presences
            .prune(|target| target.as_str() == "target-1")
            .await;

        let detached = browser.wait_for("Target.detachFromTarget", 1).await;
        assert_eq!(
            detached
                .iter()
                .map(|command| command.params.clone())
                .collect::<Vec<_>>(),
            [json!({ "sessionId": unneeded.session().session.as_str() })],
            "only the unneeded presence was detached"
        );
        let again = ensure(&presences, "target-1").await;
        assert!(Arc::ptr_eq(&kept, &again), "a needed presence stays");
        ensure(&presences, "target-2").await;
        assert_eq!(
            browser.sent("Target.attachToTarget").len(),
            3,
            "a pruned tab gets a new presence when it is needed again"
        );
        assert!(
            browser
                .sent("Emulation.setFocusEmulationEnabled")
                .iter()
                .all(|command| command.params == json!({ "enabled": true })),
            "emulation is never turned off"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_presence_whose_tab_closed_is_replaced_without_a_detach() {
        let browser = FakeHostCdp::start().await;
        let presences = presences(&browser).await;
        let closed = ensure(&presences, "target-1").await;

        browser.detach(closed.session().session.as_str());
        time::timeout(Duration::from_secs(5), closed.session().gone())
            .await
            .unwrap_or_else(|_| panic!("the browser's detach never arrived"));
        let replaced = ensure(&presences, "target-1").await;
        presences.prune(|_| true).await;

        assert!(!Arc::ptr_eq(&closed, &replaced), "a fresh presence");
        assert_eq!(
            browser.sent("Emulation.setFocusEmulationEnabled").len(),
            2,
            "the fresh presence turned emulation on again"
        );
        assert!(
            browser.sent("Target.detachFromTarget").is_empty(),
            "nothing detached a session the browser already closed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_that_gives_up_does_not_strand_the_attach() {
        let browser = FakeHostCdp::start().await;
        let presences = presences(&browser).await;
        browser.delay_attaches(Duration::from_secs(10));

        let target = page("target-1");
        let gave_up = time::timeout(Duration::from_secs(1), presences.ensure(&target)).await;
        assert!(gave_up.is_err(), "the attach was still in flight");
        browser.delay_attaches(Duration::ZERO);
        browser
            .wait_for("Emulation.setFocusEmulationEnabled", 1)
            .await;
        ensure(&presences, "target-1").await;

        assert_eq!(
            browser.sent("Target.attachToTarget").len(),
            1,
            "the abandoned attach finished and became the tab's presence"
        );
    }
}
