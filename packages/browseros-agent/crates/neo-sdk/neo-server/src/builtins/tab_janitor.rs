//! Closing agent tabs nobody uses any more.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::browser;
use browseros_core::{ProtocolSession, TabId};
use neo_service::Settings;
use neo_takeover::TabHold;
use neo_trace::{RunId, RunStatus};
use serde_json::json;
use tokio::time::{self, Instant};

use crate::browser::{BrowserLink, PageRef, TabRegistry};
use crate::dispatch::{Call, Caller, Dispatcher, OpReply, RunCtx, RunEnd, RunStart, StopReason};
use crate::host::HostHolds;

/// What one sweep did, as its run's result records it: the tabs it closed and the idle ones it
/// kept (refused by the door, the browser's last, or past the sweep's budget). A plain data bag.
struct Swept {
    closed: Vec<i64>,
    kept: Vec<i64>,
}

/// The method the janitor submits for each idle tab.
const CLOSE: &str = "page.close";
/// The `code` a sweep's run records, so the trace says who closed the tabs and why.
const SWEEP_CODE: &str = "// Neo's tab janitor: page.close on each idle agent-opened tab";
/// One sweep's budget. Each close is one CDP round trip, so this only bounds a browser that
/// stopped answering; the next sweep tries again.
const SWEEP_BUDGET: Duration = Duration::from_secs(30);

/// Closes the agent's tabs once nobody used them for the host's `tabs.idleCloseSeconds`
/// (decision 5; 30 minutes by default). With no sessions nothing else ever frees them, and a
/// hosted Neo runs for weeks.
///
/// A tab is in use, and left alone, while any of these holds (the settings design, section 2):
///
/// - a run that used it is still in flight (`TabRegistry::run_started` to `run_ended`);
/// - a lock holds it, which is how a host keeps the agent off a tab the owner is using;
/// - a fit is on it, which is how zuzu's handoff shows the tab to the owner, with no lock.
///
/// Its idle clock restarts when the last of them ends, so "idle" means that both the agent and
/// the owner stopped using it. A tab waiting for an approval is used by nobody, and stays the
/// host's to keep alive with a read. Only `TabOrigin::Agent` tabs are candidates (the agent's own
/// and their popups), so the owner's own tabs are never closed.
///
/// It never closes the browser's last tab: closing it ends the browser and Neo's server
/// (`TabJanitor::leaves_another`). "Last" counts every tab the browser lists, the owner's and
/// the one it opened at launch included. When the idle tabs are all that is left, the one used
/// most recently stays.
///
/// Each close is a `page.close` op that Neo submits through the dispatcher, in a run of its own
/// with no `requestId` and no `taskId` (synthesis graft 8), so the trace records what Neo closed.
/// A held tab is skipped rather than refused, which keeps a long takeover from leaving a refused
/// close in the trace every minute. A lock taken between the skip and the close is still the
/// door's to refuse: the close is an op like any other.
pub(crate) struct TabJanitor {
    dispatcher: Arc<Dispatcher>,
    link: Arc<BrowserLink>,
    tabs: Arc<TabRegistry>,
    holds: Arc<HostHolds>,
    settings: Settings,
}

impl TabJanitor {
    /// The janitor over the app's one dispatcher, whose trace records its runs, and the app's
    /// link, tab registry, the host's claims on tabs, and the host's settings, read at every
    /// pass so a changed idle time applies at the next one.
    #[must_use]
    pub(crate) fn new(
        dispatcher: Arc<Dispatcher>,
        link: Arc<BrowserLink>,
        tabs: Arc<TabRegistry>,
        holds: Arc<HostHolds>,
        settings: Settings,
    ) -> Self {
        Self {
            dispatcher,
            link,
            tabs,
            holds,
            settings,
        }
    }

    /// One pass, for the app's housekeeping to call every minute with `Instant::now()`. Returns
    /// the sweep's run, or `None` when nothing idle is free to close or the browser is not
    /// linked (no run is recorded then; the next pass tries again).
    pub(crate) async fn sweep(&self, now: Instant) -> Option<RunId> {
        // Asked before the registry's lock is taken: the host's parts have locks of their own.
        let holds: HashMap<TabId, TabHold> = self
            .tabs
            .agent_tabs()
            .into_iter()
            .filter_map(|tab| self.holds.hold(tab).map(|hold| (tab, hold)))
            .collect();
        let idle: Vec<PageRef> =
            self.tabs
                .idle_agent_tabs(self.settings.tab_idle_close(), now, &holds);
        if idle.is_empty() {
            return None;
        }
        let Some(view) = self.link.view() else {
            tracing::debug!(
                tabs = idle.len(),
                "idle agent tabs wait for the browser link"
            );
            return None;
        };
        // Nothing would close when the idle tabs are all the browser has and only one is left
        // to keep: no run then, so an idle keeper tab does not leave a run in the trace every
        // minute.
        let open = time::timeout(SWEEP_BUDGET, self.browser_tabs())
            .await
            .ok()
            .flatten()?;
        let closable = idle.iter().filter(|page| open.contains(&page.tab)).count();
        let others = open
            .iter()
            .filter(|tab| {
                !idle.iter().any(|page| page.tab == **tab) && !self.tabs.has_closed(**tab)
            })
            .count();
        if closable == 0 || (others == 0 && closable == 1) {
            return None;
        }
        let deadline = Instant::now() + SWEEP_BUDGET;
        let (run, guard) = RunCtx::start(
            &self.dispatcher,
            RunStart {
                id: RunId::new(),
                code: SWEEP_CODE.to_owned(),
                request_id: None,
                task_id: None,
                host_data: None,
                view,
                deadline,
                default_page: None,
            },
        );
        let Swept { closed, kept } = self.close_each(&run, &idle, deadline).await;
        // The ops' tasks end right after answering; drained so their records precede the run's.
        let mut ops = run.close_ops();
        while ops.join_next().await.is_some() {}
        let status = if run.stop_reason() == Some(StopReason::Deadline) {
            RunStatus::Timeout
        } else {
            RunStatus::Ok
        };
        guard.finish(RunEnd {
            status,
            error: None,
            result: Some(json!({ "closed": closed, "kept": kept })),
            replay: None,
        });
        Some(run.id())
    }

    /// Closes `idle`, the longest idle first, in `run`. One close at a time, each decided on a
    /// listing taken after the previous close was answered: the sweep's own closes can never
    /// take the last two tabs together, and the tab it keeps last is the one used most recently.
    async fn close_each(&self, run: &Arc<RunCtx>, idle: &[PageRef], deadline: Instant) -> Swept {
        let mut closed: Vec<TabId> = Vec::new();
        let mut kept = Vec::new();
        for page in idle {
            if run.stop_reason().is_some() {
                kept.push(page.tab.0);
                continue;
            }
            let Ok(Some(open)) = time::timeout_at(deadline, self.browser_tabs()).await else {
                kept.push(page.tab.0);
                continue;
            };
            if !open.contains(&page.tab) {
                continue;
            }
            if !self.leaves_another(&open, page.tab, &closed) {
                tracing::info!(run = %run.id(), tab = page.tab.0, "kept an idle agent tab: it is the browser's last, and closing it would end the browser and Neo");
                kept.push(page.tab.0);
                continue;
            }
            let call = Call {
                name: CLOSE.to_owned(),
                input: json!({ "page": page }),
                caller: Caller::Neo,
                site: None,
                abort: None,
            };
            let reply = self.dispatcher.submit(run, call).wait();
            tokio::pin!(reply);
            // Every op answers on every path, so after the stop its reply still arrives, as
            // `neo / run_timeout`.
            let reply: OpReply = tokio::select! {
                reply = &mut reply => reply,
                () = time::sleep_until(deadline) => {
                    run.stop(StopReason::Deadline);
                    reply.await
                }
            };
            match reply.outcome {
                Ok(_) => {
                    tracing::info!(run = %run.id(), tab = page.tab.0, "closed an idle agent tab");
                    closed.push(page.tab);
                }
                Err(failure) => {
                    tracing::info!(run = %run.id(), tab = page.tab.0, error = %failure.error, "kept an idle agent tab");
                    kept.push(page.tab.0);
                }
            }
        }
        Swept {
            closed: closed.iter().map(|tab| tab.0).collect(),
            kept,
        }
    }

    /// Every tab the browser has open, from its own inventory (`Browser.getTabs`): the owner's
    /// tabs, the tab it opened at launch and internal pages included, which core's page list
    /// leaves out in part. `None` when the browser did not answer; the janitor then closes
    /// nothing, since it cannot tell which tab is the last.
    async fn browser_tabs(&self) -> Option<Vec<TabId>> {
        let session = self.link.linked().await;
        let listed: Result<browser::GetTabsResult, _> = ProtocolSession::root(session.connection())
            .send("Browser.getTabs", json!({}))
            .await;
        match listed {
            Ok(listed) => Some(listed.tabs.iter().map(|tab| TabId(tab.tab_id)).collect()),
            Err(error) => {
                tracing::debug!(%error, "the browser did not list its tabs; no idle tab closed");
                None
            }
        }
    }

    /// Whether closing `tab` leaves the browser another tab. The browser quits when its last tab
    /// closes, and Neo's server with it: the browser restarts both a few seconds later with a
    /// new browser id, and a host that keeps one tab of its own, as zuzu does, would see that
    /// every idle time (zuzu's last-tab probe, 2026-11-12). So another tab must be in `open`, a
    /// listing taken just now, and must not be one this sweep closed (`closed`) or one Neo saw
    /// close: Chromium answers a close before the tab leaves its listing, so a listing can
    /// still show a tab that is already gone.
    ///
    /// What this cannot rule out is someone else closing that other tab in the round trip
    /// between the listing and this close: the owner closing their own tab, or a script's
    /// `page.close()`, at that same moment.
    fn leaves_another(&self, open: &[TabId], tab: TabId, closed: &[TabId]) -> bool {
        open.iter()
            .any(|other| *other != tab && !closed.contains(other) && !self.tabs.has_closed(*other))
    }
}
