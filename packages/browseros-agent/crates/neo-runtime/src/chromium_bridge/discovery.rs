//! Neo: the run's new pages, for `context.on('page')` and `page.on('popup')`. Playwright's server
//! attaches to every target of its browser and reports a page once its first real navigation
//! committed; Neo turns on CDP target discovery while a run listens for new pages, and reports
//! each new tab of the run's context the same way, as the `page` event on the context.
//!
//! ```text
//!  Runtime: Switch     holders 0 → 1: Target.setDiscoverTargets {discover: true, page filter}
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

use browseros_cdp::{CdpError, CdpEvent};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tracing::warn;

use crate::browser_representations::tabs::Tabs;
use crate::chromium_bridge::browser_connection_settings::{Setting, Switch};
use crate::chromium_bridge::cdp::{self, Command, target};
use crate::events::{EventHub, Flushes};

/// Neo: the target types discovery reports. Tabs are `page` targets; without the filter, every
/// extension worker and browser UI target would cross the run's event ring too.
fn page_filter() -> Vec<target::FilterEntry> {
    vec![target::FilterEntry {
        r#type: Some("page".to_string()),
        exclude: None,
    }]
}

/// Neo: target discovery as a setting the `Runtime`'s runs share (`browser_connection_settings.rs`): on for page
/// targets while any run listens for new pages, off once the last lets go.
pub(crate) static DISCOVERY: Setting = Setting {
    method: target::SetDiscoverTargets::METHOD,
    params: discovery_params,
};

fn discovery_params(on: bool, _scope: Option<&str>) -> Value {
    // Serializing this plain struct cannot fail; if it ever did, the browser would refuse the
    // empty params and the switch would report it.
    serde_json::to_value(target::SetDiscoverTargets {
        discover: on,
        filter: on.then(page_filter),
    })
    .unwrap_or_default()
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
pub(crate) fn is_initial_empty(url: &str) -> bool {
    url.is_empty() || url == ":"
}

/// Turns the run's discovery on and watches it until the run's events end. Runs on the run's
/// task tracker, so `Dispatcher::finish` waits for the lease's release.
pub(crate) async fn run(tabs: Tabs, discovery: Switch) {
    let hub = tabs.events().clone();
    let cdp = tabs.cdp().clone();
    // Subscribed first: every target event from here on reaches this receiver, the replay too.
    let events = cdp.events();
    let flushes = hub.watcher();
    let started = async {
        let baseline = cdp::Browser::new(cdp.clone())
            .send(target::GetTargets {
                filter: Some(page_filter()),
            })
            .await?;
        let baseline: TargetInfos = serde_json::from_value(baseline)?;
        let lease = discovery.acquire(&cdp, None).await?;
        Ok::<_, CdpError>((baseline, lease))
    };
    let started = started.await;
    tabs.discovery_ready();
    let (baseline, lease) = match started {
        Ok(started) => started,
        Err(error) => {
            warn!(%error, "the run's target discovery did not start; no page events");
            hub.note(format!(
                "neo-runtime could not watch for new pages ({error}): no page or popup events"
            ));
            return;
        }
    };
    let mut targets = Targets::default();
    // The run's own page is never new to the run, whatever its document.
    targets.known.insert(tabs.run_page().as_str().to_string());
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
                Ok(event) => report(targets.on(&event), tabs).await,
                Err(RecvError::Lagged(skipped)) => {
                    warn!(skipped, "the run's target watcher lagged; reporting pending pages");
                    report_pending(targets, tabs).await;
                }
                Err(RecvError::Closed) => return,
            },
            Some(ack) = flushes.recv() => {
                // A reply barrier (`events.rs`): every target event already in the ring preceded
                // the reply that asked for it.
                loop {
                    match events.try_recv() {
                        Ok(event) => report(targets.on(&event), tabs).await,
                        Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                        Err(TryRecvError::Lagged(_)) => report_pending(targets, tabs).await,
                    }
                }
                let _waiter = ack.send(());
            }
        }
    }
}

/// A new page for the run: its target, and whether it closed before its first commit.
type Report = (TargetInfo, bool);

async fn report(report: Option<Report>, tabs: &Tabs) {
    if let Some((info, closed)) = report {
        tabs.report_new(&info, closed).await;
    }
}

/// After lost events, a pending page may have committed or closed unseen: each reports as it
/// stands, as Playwright reports a page whose initialization ended either way.
async fn report_pending(targets: &mut Targets, tabs: &Tabs) {
    for info in targets.drain_pending() {
        tabs.report_new(&info, false).await;
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
        if self.known.contains(&info.target_id) {
            return;
        }
        if is_initial_empty(&info.url) {
            self.pending.insert(info.target_id.clone(), info);
        } else {
            self.known.insert(info.target_id);
        }
    }

    /// The report one browser-level target event makes, if any: a page target's first non-empty
    /// URL (its first commit), or its end while still on its initial empty document. Each target
    /// reports at most once.
    fn on(&mut self, event: &CdpEvent) -> Option<Report> {
        if event.session_id.is_some() {
            return None;
        }
        match event.method.as_str() {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let TargetEvent { target_info: info } =
                    serde_json::from_value::<TargetEvent>(event.params.clone()).ok()?;
                if info.kind != "page" || self.known.contains(&info.target_id) {
                    return None;
                }
                if is_initial_empty(&info.url) {
                    self.pending.insert(info.target_id.clone(), info);
                    return None;
                }
                self.pending.remove(&info.target_id);
                self.known.insert(info.target_id.clone());
                Some((info, false))
            }
            "Target.targetDestroyed" => {
                let TargetDestroyed { target_id } =
                    serde_json::from_value::<TargetDestroyed>(event.params.clone()).ok()?;
                let info = self.pending.remove(&target_id)?;
                self.known.insert(target_id);
                Some((info, true))
            }
            _ => None,
        }
    }

    /// Every pending page, known from now on.
    fn drain_pending(&mut self) -> Vec<TargetInfo> {
        let pending: Vec<TargetInfo> = self.pending.drain().map(|(_, info)| info).collect();
        self.known
            .extend(pending.iter().map(|info| info.target_id.clone()));
        pending
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio_util::task::TaskTracker;

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

    fn target_event(method: &str, target: &str, kind: &str, url: &str) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params: json!({ "targetInfo": {
                "targetId": target, "type": kind, "url": url, "openerId": "OPENER",
                "browserContextId": "CONTEXT",
            }}),
            session_id: None,
        }
    }

    fn destroyed(target: &str) -> CdpEvent {
        CdpEvent {
            method: "Target.targetDestroyed".to_string(),
            params: json!({ "targetId": target }),
            session_id: None,
        }
    }

    /// What each event of a sequence reports: `Some((target, closed))` or `None`.
    fn reports(targets: &mut Targets, events: &[CdpEvent]) -> Vec<Option<(String, bool)>> {
        events
            .iter()
            .map(|event| {
                targets
                    .on(event)
                    .map(|(info, closed)| (info.target_id, closed))
            })
            .collect()
    }

    #[test]
    fn tests_that_a_new_page_reports_once_at_its_first_commit_or_its_end() {
        let created = "Target.targetCreated";
        let changed = "Target.targetInfoChanged";
        let report = |target: &str, closed| Some((target.to_string(), closed));
        for (name, events, expected) in [
            (
                "window.open: empty, then committed, then navigated again",
                vec![
                    target_event(created, "P", "page", ""),
                    target_event(changed, "P", "page", "https://a.test/popup"),
                    target_event(changed, "P", "page", "https://a.test/next"),
                ],
                vec![None, report("P", false), None],
            ),
            (
                "closed before its first commit",
                vec![
                    target_event(created, "Q", "page", ""),
                    destroyed("Q"),
                    destroyed("Q"),
                ],
                vec![None, report("Q", true), None],
            ),
            (
                "createTab reports about:blank at once",
                vec![target_event(created, "N", "page", "about:blank")],
                vec![report("N", false)],
            ),
            (
                "a tab from before the run, a worker, and a reported page that closes",
                vec![
                    target_event(changed, "OLD", "page", "https://a.test/moved"),
                    target_event(created, "W", "service_worker", "https://a.test/sw.js"),
                    target_event(created, "R", "page", "https://a.test/r"),
                    destroyed("R"),
                ],
                vec![None, None, report("R", false), None],
            ),
        ] {
            let mut targets = Targets::default();
            targets.baseline(TargetInfo {
                target_id: "OLD".to_string(),
                kind: "page".to_string(),
                url: "https://a.test/".to_string(),
                opener_id: None,
                browser_context_id: None,
            });
            assert_eq!(reports(&mut targets, &events), expected, "{name}");
        }
    }

    #[test]
    fn tests_that_a_session_event_and_a_lag_are_handled_as_the_watcher_needs() {
        let mut targets = Targets::default();
        let mut attached = target_event("Target.targetInfoChanged", "S", "page", "https://a/");
        attached.session_id = Some(browseros_cdp::SessionId::from("session"));
        assert!(
            targets.on(&attached).is_none(),
            "a session's own target events"
        );
        assert!(
            targets
                .on(&target_event("Target.targetCreated", "L", "page", ""))
                .is_none()
        );
        let drained = targets.drain_pending();
        assert_eq!(drained.len(), 1, "a pending page reports after lost events");
        let committed = target_event("Target.targetInfoChanged", "L", "page", "https://a/");
        assert!(targets.on(&committed).is_none(), "and only once");
    }

    #[tokio::test]
    async fn tests_that_discovery_is_on_while_any_run_holds_it() {
        let browser = FakeBrowser::start(Replies::default()).await;
        let cdp = browser.cdp().await;
        let discovery = Switch::new(&DISCOVERY, TaskTracker::new());
        let first = discovery.acquire(&cdp, None).await.unwrap();
        let second = discovery.acquire(&cdp, None).await.unwrap();
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
