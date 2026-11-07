//! Neo: a run's init scripts (`page.addInitScript`, `browserContext.addInitScript`): Playwright's
//! `Page.addScriptToEvaluateOnNewDocument` on every session that shows a page the run touched,
//! the tab's own and each out-of-process iframe's. Chromium attaches an iframe of another
//! process as it starts and holds it until its scripts are in (`Target.setAutoAttach` with
//! `waitForDebuggerOnStart`, then `Page.enable`, the scripts and `Runtime.runIfWaitingForDebugger`;
//! probe C1, `probes-c1.md`), as Playwright's `FrameSession` does. Scripts run in the page's own
//! world only, so this file never names another world (`tests/page_js_guard.rs`).
//!
//! Chromium never answers some requests on an iframe it removed, and the CDP client waits a
//! minute for them; one such wait would hold every later iframe at its start, and with it the
//! page's `load`. So each request on a session ends the moment Chromium detaches that session
//! ([`until_gone`]), as `frames.rs` `watch_host` ends a removed OOPIF's.
//!
//! A script a session took must never outlive the run, even when the call that added it was
//! dropped mid-wait (a cancelled run). So each request that installs a script or turns on
//! auto-attach runs on a task of its own, which publishes its reply's identifier in a plain-lock
//! [`Registry`] the moment it arrives, or undoes it when the run has ended meanwhile; run end
//! reads that registry and never waits on the setup's lock.
//!
//! The page JavaScript guard allows this file, and only it, to send
//! `Page.addScriptToEvaluateOnNewDocument` and `Page.removeScriptToEvaluateOnNewDocument`.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::mem;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex, MutexGuard, OnceLock, Weak};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use neo_runtime_spec::{MethodGroup, MethodMeta};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

use crate::auto_attach::{self, AttachKind};
use crate::page::PageSession;
use crate::progress::PlaywrightError;

/// Neo: the guid prefix of an init script's `Disposable`, which the SDK disposes by guid.
pub(crate) const DISPOSABLE_PREFIX: &str = "disposable@";

/// Neo: `Page.addInitScript`'s trace metadata (`neo-runtime-spec/spec/playwright/page.yml`).
pub(crate) static PAGE_ADD_INIT_SCRIPT_META: MethodMeta = MethodMeta {
    interface: "Page",
    method: "addInitScript",
    title: Some("Add init script"),
    subtitle: None,
    group: Some(MethodGroup::Configuration),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `BrowserContext.addInitScript`'s trace metadata
/// (`neo-runtime-spec/spec/playwright/browserContext.yml`).
pub(crate) static CONTEXT_ADD_INIT_SCRIPT_META: MethodMeta = MethodMeta {
    interface: "BrowserContext",
    method: "addInitScript",
    title: Some("Add init script"),
    subtitle: None,
    group: Some(MethodGroup::Configuration),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `Disposable.dispose`'s trace metadata (`neo-runtime-spec/spec/playwright/core.yml`).
pub(crate) static DISPOSABLE_DISPOSE_META: MethodMeta = MethodMeta {
    interface: "Disposable",
    method: "dispose",
    title: None,
    subtitle: None,
    group: None,
    render_params: &[],
    internal: true,
    flags: &[],
};

/// An init script command, routed by `dispatch.rs`.
#[derive(Debug)]
pub(crate) enum InitRoute {
    /// `Page.addInitScript`, on the call's page.
    Page(String),
    /// `BrowserContext.addInitScript`.
    Context(String),
    /// `Disposable.dispose` of the script with this guid.
    Dispose(String),
}

/// `addInitScript`'s params.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddInitScriptParams {
    source: String,
}

/// `addInitScript`'s `source`, as `meta`'s command decodes it.
///
/// # Errors
///
/// Playwright's plain `Error` for params that do not match.
pub(crate) fn source_of(
    meta: &MethodMeta,
    params: Option<Value>,
) -> Result<String, PlaywrightError> {
    serde_json::from_value::<AddInitScriptParams>(params.unwrap_or_else(|| json!({})))
        .map(|params| params.source)
        .map_err(|error| {
            PlaywrightError::error(format!("{}.{}: {error}", meta.interface, meta.method))
        })
}

/// The run's init scripts and the sessions that show them.
pub(crate) struct InitScripts {
    /// This run's random token in every guid, as `evaluate.rs`'s handles carry one.
    run: String,
    next: AtomicU64,
    /// Held across the CDP calls that change it, so a session gets each script once. Run end
    /// never waits on it.
    state: Mutex<State>,
    /// Every identifier a session answered and every session given auto-attach, as each reply
    /// arrives: what run end undoes.
    registry: Arc<SyncMutex<Registry>>,
    /// The run's CDP client, once a page showed its scripts.
    cdp: OnceLock<CdpClient>,
    /// Whether the run added a script, so a call need not touch the pages before.
    any: AtomicBool,
    /// Every session that shows a page the run touched, by id: the page's target and the token
    /// its detach cancels. The event reader keeps it without waiting, so a plain lock.
    sessions: SyncMutex<HashMap<String, Watched>>,
    /// Stops the event reader, at run end.
    stop: CancellationToken,
    /// Cancelled as run end starts: a setup still waiting on a reply stops waiting, while the
    /// request's own task keeps the reply and undoes it.
    ending: CancellationToken,
}

#[derive(Clone)]
struct Watched {
    target: String,
    gone: CancellationToken,
}

#[derive(Default)]
struct State {
    scripts: Vec<Script>,
    pages: Vec<ShownPage>,
}

/// What run end undoes, kept under a plain lock no request is awaited under.
#[derive(Default)]
struct Registry {
    /// Set at run end, under this lock: a reply that arrives after it undoes its own effect.
    closed: bool,
    installed: Vec<Installed>,
    /// Each session given auto-attach, the tab's first.
    attaching: Vec<Link>,
}

/// A script a session took: its identifier there.
struct Installed {
    link: Link,
    guid: String,
    identifier: String,
}

struct Script {
    guid: String,
    /// Wrapped as Playwright wraps it ([`wrap`]).
    source: String,
    /// The page's target id for `page.addInitScript`; `None` for the context's.
    page: Option<String>,
}

impl Script {
    fn reaches(&self, target: &str) -> bool {
        self.page.as_deref().is_none_or(|page| page == target)
    }
}

/// A page the run touched, with every session that shows it: the tab's own first, then each
/// out-of-process iframe Chromium attached for it.
struct ShownPage {
    target: String,
    sessions: Vec<Link>,
}

/// A session that shows a page the run touched, with the token its detach cancels.
#[derive(Clone)]
struct Link {
    session: SessionId,
    gone: CancellationToken,
}

impl Default for InitScripts {
    fn default() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT_RUN.fetch_add(1, Ordering::Relaxed));
        Self {
            run: format!("{:016x}", hasher.finish()),
            next: AtomicU64::new(1),
            state: Mutex::default(),
            registry: Arc::default(),
            cdp: OnceLock::new(),
            any: AtomicBool::new(false),
            sessions: SyncMutex::default(),
            stop: CancellationToken::new(),
            ending: CancellationToken::new(),
        }
    }
}

impl Drop for InitScripts {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Neo: the process-wide count that keeps two runs' tokens apart within one process.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

impl InitScripts {
    /// `page.addInitScript` (with `page`) or `browserContext.addInitScript` (without): the new
    /// script's `Disposable` guid. A page's script reaches that page, a context's every page the
    /// run touches, each from its next document on, as Playwright's do.
    ///
    /// # Errors
    ///
    /// `TargetClosedError` after the run ended or when the page's own session cannot take it.
    pub(crate) async fn add(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: Option<&Arc<PageSession>>,
        source: &str,
    ) -> Result<String, PlaywrightError> {
        if self.closed() {
            return Err(PlaywrightError::target_closed());
        }
        if let Some(page) = page {
            self.show(cdp, page, true, false).await?;
        }
        let guid = format!(
            "{DISPOSABLE_PREFIX}{}.{}",
            self.run,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let script = Script {
            guid: guid.clone(),
            source: wrap(source),
            page: page.map(|page| target_of(page)),
        };
        let mut state = self.state.lock().await;
        self.any.store(true, Ordering::Relaxed);
        for shown_page in &state.pages {
            if !script.reaches(&shown_page.target) {
                continue;
            }
            for (index, link) in shown_page.sessions.iter().enumerate() {
                let installed = self
                    .install(cdp, link, &script.guid, &script.source, false)
                    .await;
                // Playwright fails the call when a session of the page cannot take it; a
                // vanished iframe's session is not the page.
                if !installed && index == 0 && script.page.is_some() {
                    return Err(PlaywrightError::target_closed());
                }
            }
        }
        state.scripts.push(script);
        Ok(guid)
    }

    /// Whether the run added a script: before, a call has nothing to show.
    pub(crate) fn has_scripts(&self) -> bool {
        self.any.load(Ordering::Relaxed)
    }

    /// Shows the run's scripts on `page` once a call touches it: its session gets every script
    /// that reaches it, and Chromium attaches its out-of-process iframes as they start. Nothing
    /// happens while the run has no script; every failure leaves the call as it was.
    pub(crate) async fn ensure(self: &Arc<Self>, cdp: &CdpClient, page: &Arc<PageSession>) {
        let _shown = self.show(cdp, page, false, false).await;
    }

    /// [`Self::ensure`] for a page the run just opened: its first document already exists, so
    /// each script runs there too, as Playwright's `FrameSession` of a new page runs them
    /// (`runImmediately`).
    pub(crate) async fn show_new(self: &Arc<Self>, cdp: &CdpClient, page: &Arc<PageSession>) {
        let _shown = self.show(cdp, page, false, true).await;
    }

    async fn show(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: &Arc<PageSession>,
        always: bool,
        run_immediately: bool,
    ) -> Result<(), PlaywrightError> {
        let target = target_of(page);
        let mut state = self.state.lock().await;
        if self.closed() {
            return Err(PlaywrightError::target_closed());
        }
        if state.pages.iter().any(|shown| shown.target == target)
            || (!always && state.scripts.is_empty())
        {
            return Ok(());
        }
        page.prepare()
            .await
            .map_err(|_| PlaywrightError::target_closed())?;
        if self.cdp.set(cdp.clone()).is_ok() {
            // Subscribed before any auto-attach is on, so no iframe it attaches is missed.
            tokio::spawn(watch(Arc::downgrade(self), cdp.clone(), cdp.events()));
        }
        let link = Link {
            session: page.session_id().clone(),
            gone: self.watched(page.session_id(), &target),
        };
        for script in state
            .scripts
            .iter()
            .filter(|script| script.reaches(&target))
        {
            self.install(cdp, &link, &script.guid, &script.source, run_immediately)
                .await;
        }
        self.attach_iframes(cdp, &link).await;
        state.pages.push(ShownPage {
            target,
            sessions: vec![link],
        });
        Ok(())
    }

    fn closed(&self) -> bool {
        lock(&self.registry).is_none_or(|registry| registry.closed)
    }

    /// Playwright's `FrameSession._evaluateOnNewDocument` of `source` on `link`'s session, on a
    /// task of its own that keeps the reply's identifier: a call dropped mid-wait still leaves it
    /// in the registry, and a reply after run end removes the script at once. Whether the
    /// session took it.
    async fn install(
        &self,
        cdp: &CdpClient,
        link: &Link,
        guid: &str,
        source: &str,
        run_immediately: bool,
    ) -> bool {
        let (registry, cdp, link, guid, source) = (
            Arc::clone(&self.registry),
            cdp.clone(),
            link.clone(),
            guid.to_string(),
            source.to_string(),
        );
        let task = tokio::spawn(async move {
            let Some(identifier) = add_script(&cdp, &link, &source, run_immediately).await else {
                return false;
            };
            let late = match lock(&registry) {
                Some(mut registry) if !registry.closed => {
                    registry.installed.push(Installed {
                        link: link.clone(),
                        guid,
                        identifier: identifier.clone(),
                    });
                    false
                }
                _ => true,
            };
            if late {
                remove_script(&cdp, &link, &identifier).await;
                return false;
            }
            true
        });
        tokio::select! {
            installed = task => installed.unwrap_or(false),
            () = self.ending.cancelled() => false,
        }
    }

    /// Turns auto-attach on for `link`'s session, as run end turns it off: registered before it
    /// is sent, and on a task of its own that turns it off again when its reply comes after run
    /// end, so the session is left without it whichever request Chromium reads last.
    async fn attach_iframes(&self, cdp: &CdpClient, link: &Link) {
        match lock(&self.registry) {
            Some(mut registry) if !registry.closed => registry.attaching.push(link.clone()),
            _ => return,
        }
        let (registry, cdp, link) = (Arc::clone(&self.registry), cdp.clone(), link.clone());
        let task = tokio::spawn(async move {
            set_auto_attach(&cdp, &link, true).await;
            if lock(&registry).is_none_or(|registry| registry.closed) {
                set_auto_attach(&cdp, &link, false).await;
            }
        });
        tokio::select! {
            _attached = task => {}
            () = self.ending.cancelled() => {}
        }
    }

    /// The token `session`'s detach cancels, the session now kept as one that shows `target`.
    fn watched(&self, session: &SessionId, target: &str) -> CancellationToken {
        let Ok(mut sessions) = self.sessions.lock() else {
            // A poisoned lock watches nothing: the session's requests wait as any other's.
            return CancellationToken::new();
        };
        sessions
            .entry(session.as_str().to_string())
            .or_insert_with(|| Watched {
                target: target.to_string(),
                gone: CancellationToken::new(),
            })
            .gone
            .clone()
    }

    /// Playwright's `DisposableObject.dispose` of an init script: every session drops it.
    ///
    /// # Errors
    ///
    /// `TargetClosedError` for a script that is gone (disposed, or another run's), which
    /// Playwright's client takes as done.
    pub(crate) async fn dispose(&self, guid: &str) -> Result<(), PlaywrightError> {
        let mut state = self.state.lock().await;
        let before = state.scripts.len();
        state.scripts.retain(|script| script.guid != guid);
        if state.scripts.len() == before {
            return Err(PlaywrightError::target_closed());
        }
        drop(state);
        let Some(cdp) = self.cdp.get() else {
            return Ok(());
        };
        let gone: Vec<Installed> = match lock(&self.registry) {
            Some(mut registry) => {
                let (gone, kept) = mem::take(&mut registry.installed)
                    .into_iter()
                    .partition(|installed| installed.guid == guid);
                registry.installed = kept;
                gone
            }
            None => Vec::new(),
        };
        for installed in gone {
            remove_script(cdp, &installed.link, &installed.identifier).await;
        }
        Ok(())
    }

    /// Removes every script this run added from every session that shows it and stops
    /// attaching iframes for them, so no later document, in this run or another, runs one.
    /// Deviation: `server/page.ts` `Page._addInitScript`: `this.initScripts.push(initScript);` Playwright's scripts live as long as their page or context; Neo's live as long as the run.
    ///
    /// The event reader runs until the end, so an iframe held at its start meanwhile still runs.
    pub(crate) async fn close(&self) {
        self.ending.cancel();
        let (installed, attaching) = match lock(&self.registry) {
            Some(mut registry) => {
                registry.closed = true;
                (
                    mem::take(&mut registry.installed),
                    mem::take(&mut registry.attaching),
                )
            }
            None => (Vec::new(), Vec::new()),
        };
        if let Some(cdp) = self.cdp.get().cloned() {
            // On a task of its own, so a run end cut short still removes every script.
            let removal = tokio::spawn(async move {
                for installed in installed.iter().rev() {
                    remove_script(&cdp, &installed.link, &installed.identifier).await;
                }
                // The iframes' sessions first: turning auto-attach off on the tab's detaches them.
                for link in attaching.iter().rev() {
                    set_auto_attach(&cdp, link, false).await;
                }
            });
            let _removed = removal.await;
        }
        self.stop.cancel();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }

    /// A session of `target`'s page attached `child`: an out-of-process iframe gets the scripts
    /// that reach the page (at once when it was held at its start), and Chromium attaches its own
    /// iframes too; whatever it is, it then runs. Each attach is answered on its own task, so one
    /// iframe never waits on another's requests.
    async fn attached(self: Arc<Self>, cdp: CdpClient, attach: Attach) {
        let Attach {
            target,
            link,
            held,
            iframe,
        } = attach;
        let mut shown_here = false;
        if iframe && !self.closed() {
            let mut state = self.state.lock().await;
            let State { scripts, pages } = &mut *state;
            if let Some(page) = pages.iter_mut().find(|page| page.target == target) {
                let _enabled = until_gone(
                    &link.gone,
                    cdp.send("Page.enable", json!({}), Some(&link.session)),
                )
                .await;
                for script in scripts.iter().filter(|script| script.reaches(&target)) {
                    self.install(&cdp, &link, &script.guid, &script.source, held)
                        .await;
                }
                if !link.gone.is_cancelled() {
                    page.sessions.push(link.clone());
                    shown_here = true;
                }
            }
        }
        // Sent together, in this order, as Playwright's `FrameSession` sends its batch: a target
        // held at its start may answer `Target.setAutoAttach` only once it runs.
        // Playwright: `server/chromium/crPage.ts` `FrameSession._initialize`: `promises.push(this._client.send('Runtime.runIfWaitingForDebugger'));`
        let resume = until_gone(
            &link.gone,
            cdp.send(
                "Runtime.runIfWaitingForDebugger",
                json!({}),
                Some(&link.session),
            ),
        );
        if shown_here {
            let ((), _resumed) = tokio::join!(self.attach_iframes(&cdp, &link), resume);
        } else {
            let _resumed = resume.await;
        }
    }

    /// A session of `target`'s page went away: an iframe's with its frame; the tab's own with
    /// the tab, or for a session Neo attaches anew, which the next call shows the scripts again.
    async fn detached(&self, target: &str, session: &str) {
        let mut state = self.state.lock().await;
        state.pages.retain_mut(|page| {
            if page.target != target {
                return true;
            }
            if page
                .sessions
                .first()
                .is_some_and(|tab| tab.session.as_str() == session)
            {
                return false;
            }
            page.sessions
                .retain(|link| link.session.as_str() != session);
            true
        });
    }
}

/// A target one of the run's sessions attached, as its event names it.
struct Attach {
    /// The page the attaching session shows.
    target: String,
    /// The attached target's own session.
    link: Link,
    /// Chromium holds it at its start until `Runtime.runIfWaitingForDebugger`.
    held: bool,
    iframe: bool,
}

/// Neo: reads the attach and detach events of the run's sessions until the run removes its
/// scripts (`stop`), on its own task, which neither the run's calls nor its end wait for. It never
/// waits on CDP: a detach cancels its session's token at once, and each attach is answered on a
/// task of its own (`InitScripts::attached`).
async fn watch(
    scripts: Weak<InitScripts>,
    cdp: CdpClient,
    mut events: broadcast::Receiver<CdpEvent>,
) {
    let Some(stop) = scripts.upgrade().map(|scripts| scripts.stop.clone()) else {
        return;
    };
    loop {
        let event = tokio::select! {
            biased;
            () = stop.cancelled() => return,
            event = events.recv() => event,
        };
        let event = match event {
            Ok(event) => event,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let attach = match event.method.as_str() {
            "Target.attachedToTarget" => true,
            "Target.detachedFromTarget" => false,
            _ => continue,
        };
        let (Some(scripts), Some(session)) =
            (scripts.upgrade(), event.params["sessionId"].as_str())
        else {
            continue;
        };
        if attach {
            let Some(target) = event
                .session_id
                .as_ref()
                .and_then(|parent| scripts.watched_target(parent.as_str()))
            else {
                continue;
            };
            // A dedicated worker is workers.rs's: it enables the worker's runtime before resuming
            // it, as Playwright's `_initializeWorker` does (slice W, DECIDED).
            if event.params["targetInfo"]["type"].as_str() == Some("worker") {
                continue;
            }
            let session = SessionId::from(session);
            let attach = Attach {
                link: Link {
                    gone: scripts.watched(&session, &target),
                    session,
                },
                target,
                held: event.params["waitingForDebugger"]
                    .as_bool()
                    .unwrap_or(false),
                iframe: event.params["targetInfo"]["type"].as_str() == Some("iframe"),
            };
            tokio::spawn(scripts.attached(cdp.clone(), attach));
        } else if let Some(watched) = scripts.unwatched(session) {
            watched.gone.cancel();
            let session = session.to_string();
            tokio::spawn(async move { scripts.detached(&watched.target, &session).await });
        }
    }
}

impl InitScripts {
    /// The page `session` shows, when it is one of the run's.
    fn watched_target(&self, session: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        sessions.get(session).map(|watched| watched.target.clone())
    }

    /// `session`, detached: no longer one of the run's.
    fn unwatched(&self, session: &str) -> Option<Watched> {
        self.sessions.lock().ok()?.remove(session)
    }
}

/// `request`'s answer, or `None` once `gone` (its session detached) or on a CDP error.
async fn until_gone<T>(
    gone: &CancellationToken,
    request: impl Future<Output = Result<T, CdpError>>,
) -> Option<T> {
    tokio::select! {
        () = gone.cancelled() => None,
        reply = request => reply.ok(),
    }
}

/// The agent's source wrapped as Playwright's `InitScript` wraps it, so its top-level
/// declarations stay its own.
/// Playwright: `server/page.ts` `InitScript.source`: `readonly source: string;`
fn wrap(source: &str) -> String {
    format!("(() => {{\n      {source}\n    }})();")
}

/// The page's target id, which is its main frame's id.
fn target_of(page: &PageSession) -> String {
    page.frames.main().id.clone()
}

/// Playwright: `server/chromium/crPage.ts` `FrameSession._evaluateOnNewDocument`: `this._initScriptIds.set(initScript, identifier);`
/// The script's identifier on `session`, `None` when the session is gone.
async fn add_script(
    cdp: &CdpClient,
    link: &Link,
    source: &str,
    run_immediately: bool,
) -> Option<String> {
    let reply = until_gone(
        &link.gone,
        cdp.send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source, "runImmediately": run_immediately }),
            Some(&link.session),
        ),
    )
    .await?;
    reply["identifier"].as_str().map(str::to_string)
}

/// Playwright: `server/chromium/crPage.ts` `FrameSession._removeEvaluatesOnNewDocument`: `this._client.send('Page.removeScriptToEvaluateOnNewDocument', { identifier }).catch(() => {})`
async fn remove_script(cdp: &CdpClient, link: &Link, identifier: &str) {
    let _removed = until_gone(
        &link.gone,
        cdp.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
            Some(&link.session),
        ),
    )
    .await;
}

/// Neo: Chromium attaches each out-of-process iframe of the session's frame as it starts, held
/// until its scripts are in (probe C1); off again when the run removes its scripts, which
/// detaches those iframes' sessions.
/// Turns the link's iframe auto-attach on or off, through the session's one setting
/// (`auto_attach.rs`), which also holds what `workers.rs` wants of it.
async fn set_auto_attach(cdp: &CdpClient, link: &Link, on: bool) {
    let _set = until_gone(&link.gone, async {
        if on {
            auto_attach::want(cdp, &link.session, AttachKind::Iframe).await
        } else {
            auto_attach::release(cdp, &link.session, AttachKind::Iframe).await
        }
    })
    .await;
}

/// The registry's lock, `None` once a panic poisoned it: the run then counts as ended, so
/// nothing new is installed or turned on.
fn lock(registry: &SyncMutex<Registry>) -> Option<MutexGuard<'_, Registry>> {
    registry.lock().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_wrapped_as_playwright_wraps_it() {
        assert_eq!(
            wrap("const a = 1"),
            "(() => {\n      const a = 1\n    })();"
        );
    }

    #[test]
    fn add_init_script_decodes_its_source_and_refuses_other_params() {
        assert_eq!(
            source_of(&PAGE_ADD_INIT_SCRIPT_META, Some(json!({"source": "x"}))).as_deref(),
            Ok("x")
        );
        assert!(source_of(&PAGE_ADD_INIT_SCRIPT_META, Some(json!({}))).is_err());
        assert!(
            source_of(
                &CONTEXT_ADD_INIT_SCRIPT_META,
                Some(json!({"source": "x", "worldName": "y"}))
            )
            .is_err()
        );
    }

    #[test]
    fn a_page_script_reaches_its_page_and_a_context_script_every_page() {
        let page = Script {
            guid: "a".into(),
            source: String::new(),
            page: Some("T1".into()),
        };
        let context = Script {
            guid: "b".into(),
            source: String::new(),
            page: None,
        };
        assert!(page.reaches("T1") && !page.reaches("T2"));
        assert!(context.reaches("T1") && context.reaches("T2"));
    }
}
