//! Neo: the run's new pages, for `context.on('page')` and `page.on('popup')`. Playwright's server
//! attaches to every target of its browser and reports a page once its first real navigation
//! committed; Neo turns on CDP target discovery while a run listens for new pages, and reports
//! each new tab of the run's context the same way, as the `page` event on the context.
//!
//! ```text
//!  Runtime: Discovery  holders 0 → 1: Target.setDiscoverTargets {discover: true, page filter}
//!                      holders 1 → 0: {discover: false}             shared by the Runtime's runs
//!  run: Tabs::discover ─▶ events receiver ─▶ baseline = Target.getTargets ─▶ lease ─▶ watch
//!    Target.targetCreated / targetInfoChanged, url ""  ─▶ pending: the initial empty document
//!    … url ≠ ""                                        ─▶ report: register, emit `page`
//!    Target.targetDestroyed while pending              ─▶ report it closed
//! ```
//!
//! Measured on BrowserOS neo: `window.open` (with or without `noopener`) and `target=_blank`
//! report `targetCreated` with url `""` and `openerId`, then `targetInfoChanged` once the popup's
//! first document commits; a popup closed before that commit gets only `targetDestroyed`;
//! `Browser.createTab` reports `about:blank` at once; turning discovery on first replays a
//! `targetCreated` for every existing page target, before its reply.
//!
//! The baseline holds every page target that had a document before the run listened, so neither
//! that replay nor those tabs' later navigations read as new. It is read before the lease, so a
//! tab opened between the two reads as new, as it is. The watcher registers with the hub's
//! barrier: a page event that preceded a call's reply runs its listeners before that call's
//! caller continues. It ends with the run's events (`EventHub::close`), and lets go of discovery
//! before the run's tasks finish.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use browseros_cdp::{CdpClient, CdpError, CdpEvent};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};

use crate::events::{EventHub, Flushes};
use crate::tabs::Tabs;

/// Neo: the target types discovery reports. Tabs are `page` targets; without the filter, every
/// extension worker and browser UI target would cross the run's event ring too.
fn page_filter() -> Value {
    json!([{ "type": "page" }])
}

/// The `Runtime`'s discovery switch: how many of its runs listen for new pages. Discovery is a
/// setting of the browser connection, which every run of a `Runtime` shares, so it is on while
/// any run holds a [`Lease`] and off once the last lets go.
#[derive(Clone)]
pub(crate) struct Discovery {
    holders: Arc<Mutex<usize>>,
    /// The `Runtime`'s cleanup tracker, for a lease dropped without [`Lease::release`].
    cleanup: TaskTracker,
}

impl Default for Discovery {
    fn default() -> Self {
        Self::new(TaskTracker::new())
    }
}

/// One run's hold on discovery. [`Lease::release`] lets go; dropping an unreleased one lets go on
/// the `Runtime`'s cleanup tracker.
pub(crate) struct Lease {
    discovery: Discovery,
    cdp: CdpClient,
    released: bool,
}

impl Discovery {
    pub(crate) fn new(cleanup: TaskTracker) -> Self {
        Self {
            holders: Arc::default(),
            cleanup,
        }
    }

    /// Takes a hold, turning discovery on for the first holder. The lock spans the CDP call, so
    /// a concurrent run's acquire returns only once discovery is on, and a release's `false`
    /// can never land after a later acquire's `true`.
    ///
    /// # Errors
    ///
    /// The CDP error of `Target.setDiscoverTargets`; the hold is not taken.
    pub(crate) async fn acquire(&self, cdp: &CdpClient) -> Result<Lease, CdpError> {
        let mut holders = self.holders.lock().await;
        if *holders == 0 {
            cdp.send(
                "Target.setDiscoverTargets",
                json!({ "discover": true, "filter": page_filter() }),
                None,
            )
            .await?;
        }
        *holders += 1;
        Ok(Lease {
            discovery: self.clone(),
            cdp: cdp.clone(),
            released: false,
        })
    }

    /// How many runs hold discovery now.
    #[cfg(test)]
    pub(crate) async fn holders(&self) -> usize {
        *self.holders.lock().await
    }
}

impl Lease {
    /// Lets go, turning discovery off when this was the last hold. A failed switch-off is
    /// logged: the next acquire turns it on again either way.
    pub(crate) async fn release(mut self) {
        self.released = true;
        let_go(Arc::clone(&self.discovery.holders), self.cdp.clone()).await;
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.released || Handle::try_current().is_err() {
            return;
        }
        self.discovery.cleanup.spawn(let_go(
            Arc::clone(&self.discovery.holders),
            self.cdp.clone(),
        ));
    }
}

async fn let_go(holders: Arc<Mutex<usize>>, cdp: CdpClient) {
    let mut holders = holders.lock().await;
    *holders = holders.saturating_sub(1);
    if *holders == 0
        && let Err(error) = cdp
            .send(
                "Target.setDiscoverTargets",
                json!({ "discover": false }),
                None,
            )
            .await
    {
        debug!(%error, "turning target discovery off failed");
    }
}

/// What `Target.targetCreated`, `Target.targetInfoChanged` and `Target.getTargets` report of a
/// target.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetInfo {
    pub(crate) target_id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) url: String,
    /// The target whose frame opened this one, set even for `noopener` (measured).
    #[serde(default)]
    pub(crate) opener_id: Option<String>,
    #[serde(default)]
    pub(crate) browser_context_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetEvent {
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetDestroyed {
    target_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfos {
    target_infos: Vec<TargetInfo>,
}

/// Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `const isInitialEmptyPage = this._isMainFrame() && (this._page.mainFrame().url() === ':' || this._page.mainFrame().url() === '');`
/// A target's url is `""` until its first document commits (measured).
fn is_initial_empty(url: &str) -> bool {
    url.is_empty() || url == ":"
}

/// Turns the run's discovery on and watches it until the run's events end. Runs on the run's
/// task tracker, so `Dispatcher::finish` waits for the lease's release.
pub(crate) async fn run(tabs: Tabs, discovery: Discovery) {
    let hub = tabs.events().clone();
    let cdp = tabs.cdp().clone();
    // Subscribed first: every target event from here on reaches this receiver, the replay too.
    let events = cdp.events();
    let flushes = hub.watcher();
    let started = async {
        let baseline: TargetInfos = cdp
            .send_typed(
                "Target.getTargets",
                json!({ "filter": page_filter() }),
                None,
            )
            .await?;
        let lease = discovery.acquire(&cdp).await?;
        Ok::<_, CdpError>((baseline, lease))
    };
    let started = started.await;
    tabs.discovery_ready();
    let (baseline, lease) = match started {
        Ok(started) => started,
        Err(error) => {
            warn!(%error, "the run's target discovery did not start; no page events");
            return;
        }
    };
    let mut targets = Targets::default();
    for info in baseline.target_infos {
        if info.kind == "page" {
            targets.baseline(info);
        }
    }
    watch(events, flushes, &tabs, &hub, &mut targets).await;
    lease.release().await;
}

async fn watch(
    mut events: broadcast::Receiver<CdpEvent>,
    mut flushes: Flushes,
    tabs: &Tabs,
    hub: &EventHub,
    targets: &mut Targets,
) {
    let ended = hub.ended();
    loop {
        tokio::select! {
            biased;
            () = ended.cancelled() => return,
            event = events.recv() => match event {
                Ok(event) => targets.on(&event, tabs).await,
                Err(RecvError::Lagged(skipped)) => {
                    warn!(skipped, "the run's target watcher lagged; reporting pending pages");
                    targets.report_pending(tabs).await;
                }
                Err(RecvError::Closed) => return,
            },
            Some(ack) = flushes.recv() => {
                // A reply barrier (`events.rs`): every target event already in the ring preceded
                // the reply that asked for it.
                loop {
                    match events.try_recv() {
                        Ok(event) => targets.on(&event, tabs).await,
                        Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                        Err(TryRecvError::Lagged(_)) => targets.report_pending(tabs).await,
                    }
                }
                let _waiter = ack.send(());
            }
        }
    }
}

/// The watcher's view of the browser's page targets.
#[derive(Default)]
struct Targets {
    /// Targets never reported: those with a document before the run listened, and those
    /// already reported.
    known: HashSet<String>,
    /// New page targets still on their initial empty document.
    pending: HashMap<String, TargetInfo>,
}

impl Targets {
    /// A page target from before the run listened: known, unless it is itself a page still on
    /// its initial empty document, which reports once it commits, as a tab opened just now.
    fn baseline(&mut self, info: TargetInfo) {
        if is_initial_empty(&info.url) {
            self.pending.insert(info.target_id.clone(), info);
        } else {
            self.known.insert(info.target_id);
        }
    }

    async fn on(&mut self, event: &CdpEvent, tabs: &Tabs) {
        if event.session_id.is_some() {
            return;
        }
        match event.method.as_str() {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let Ok(TargetEvent { target_info: info }) =
                    serde_json::from_value::<TargetEvent>(event.params.clone())
                else {
                    return;
                };
                if info.kind != "page" || self.known.contains(&info.target_id) {
                    return;
                }
                if is_initial_empty(&info.url) {
                    self.pending.insert(info.target_id.clone(), info);
                } else {
                    self.pending.remove(&info.target_id);
                    self.known.insert(info.target_id.clone());
                    tabs.report_new(&info, false).await;
                }
            }
            "Target.targetDestroyed" => {
                let Ok(TargetDestroyed { target_id }) =
                    serde_json::from_value::<TargetDestroyed>(event.params.clone())
                else {
                    return;
                };
                if let Some(info) = self.pending.remove(&target_id) {
                    self.known.insert(target_id);
                    tabs.report_new(&info, true).await;
                }
            }
            _ => {}
        }
    }

    /// After lost events, a pending page may have committed or closed unseen: report each as it
    /// stands, as Playwright reports a page whose initialization ended either way.
    async fn report_pending(&mut self, tabs: &Tabs) {
        for (target, info) in self.pending.drain() {
            self.known.insert(target);
            tabs.report_new(&info, false).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_browser::{FakeBrowser, Replies};

    #[test]
    fn tests_that_only_an_empty_url_is_the_initial_document() {
        assert!(is_initial_empty(""));
        assert!(is_initial_empty(":"));
        assert!(!is_initial_empty("about:blank"));
        assert!(!is_initial_empty("https://a.test/"));
    }

    #[test]
    fn tests_that_target_events_decode_what_browseros_sends() {
        let event: TargetEvent = serde_json::from_value(json!({
            "targetInfo": {
                "targetId": "POPUP", "type": "page", "url": "", "openerId": "OPENER",
                "canAccessOpener": false, "openerFrameId": "OPENER", "attached": false,
                "browserContextId": "CONTEXT",
            }
        }))
        .unwrap();
        assert_eq!(event.target_info.target_id, "POPUP");
        assert_eq!(event.target_info.opener_id.as_deref(), Some("OPENER"));
        assert_eq!(
            event.target_info.browser_context_id.as_deref(),
            Some("CONTEXT")
        );
    }

    #[test]
    fn tests_that_the_baseline_keeps_only_committed_tabs_out() {
        let mut targets = Targets::default();
        for (id, url) in [("OLD", "https://a.test/"), ("OPENING", "")] {
            targets.baseline(TargetInfo {
                target_id: id.to_string(),
                kind: "page".to_string(),
                url: url.to_string(),
                opener_id: None,
                browser_context_id: None,
            });
        }
        assert!(targets.known.contains("OLD"));
        assert!(targets.pending.contains_key("OPENING"));
    }

    #[tokio::test]
    async fn tests_that_discovery_is_on_while_any_run_holds_it() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let discovery = Discovery::default();
        let first = discovery.acquire(&cdp).await.unwrap();
        let second = discovery.acquire(&cdp).await.unwrap();
        assert_eq!(discovery.holders().await, 2);
        first.release().await;
        assert_eq!(discovery.holders().await, 1);
        second.release().await;
        assert_eq!(discovery.holders().await, 0);
        let switches: Vec<Value> = browser
            .exchanges()
            .into_iter()
            .filter(|exchange| exchange.method() == "Target.setDiscoverTargets")
            .map(|exchange| exchange.command["params"]["discover"].clone())
            .collect();
        assert_eq!(switches, [json!(true), json!(false)], "one on, one off");
    }
}
