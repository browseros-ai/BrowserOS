//! Neo: a run's init scripts (`page.addInitScript`, `browserContext.addInitScript`): Playwright's
//! `Page.addScriptToEvaluateOnNewDocument` on every session that shows a page the run touched,
//! the tab's own and each out-of-process iframe's. A script belongs to one context, the run's own
//! or one `browser.newContext` created, and reaches only that context's pages (a page's script,
//! only its page), as each Playwright context keeps its own. Chromium attaches an iframe of another
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
use std::future::Future;
use std::hash::{BuildHasher, Hasher};
use std::mem;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex, MutexGuard, OnceLock, Weak};

use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId};
use futures_util::future;
use neo_runtime_spec::{MethodGroup, MethodMeta};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::{Mutex, oneshot};
use tokio::time::{self, Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::bindings::Bindings;
use crate::debugger::DEFAULT_CONTEXT;
use crate::page::PageSession;
use crate::progress::{PlaywrightError, RunScope};

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
    /// `BrowserContext.addInitScript` on `context`, the run's own or one it created.
    Context { context: String, source: String },
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
    /// The context each live script belongs to, by `Disposable` guid, for the dispatcher's
    /// synchronous choice of the debugger that sees its `dispose`.
    owners: SyncMutex<HashMap<String, String>>,
    /// Every session that shows a page the run touched, by id: the page's target and the token
    /// its detach cancels. The event reader keeps it without waiting, so a plain lock.
    sessions: SyncMutex<HashMap<String, Watched>>,
    /// Stops the event reader, at run end.
    stop: CancellationToken,
    /// Cancelled as run end starts: a setup still waiting on a reply stops waiting, while the
    /// request's own task keeps the reply and undoes it.
    ending: CancellationToken,
    /// The run's bindings once it exposed one: each session this file shows a page on then gets
    /// the CDP binding before the page runs (`bindings.rs`).
    bindings: OnceLock<Weak<Bindings>>,
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
    /// Each request that installs a script or turns auto-attach on, by number, from before it is
    /// sent until it settles: what the release waits for, so none it began outlives the release.
    pending: HashMap<u64, Pending>,
    next_pending: u64,
}

/// A request still out: its task registers its effect or undoes it, then ends.
struct Pending {
    link: Link,
    /// Closed when the request's task ends.
    settled: oneshot::Receiver<()>,
    /// `Page.addScriptToEvaluateOnNewDocument`, rather than `Target.setAutoAttach`.
    installs: bool,
}

impl Registry {
    /// Registers a request about to go out on `link`, unless the run's scripts are released:
    /// the sender its task keeps until it settles.
    fn begin(&mut self, link: &Link, installs: bool) -> Option<(u64, oneshot::Sender<()>)> {
        if self.closed {
            return None;
        }
        let (done, settled) = oneshot::channel();
        let id = self.next_pending;
        self.next_pending += 1;
        self.pending.insert(
            id,
            Pending {
                link: link.clone(),
                settled,
                installs,
            },
        );
        Some((id, done))
    }
}

/// Neo: how long the release waits for a request still out before it stops the scripts of a
/// session whose installation has not answered (`Page.disable`): Chromium answers in
/// milliseconds, so one silent this long is taken as stuck. Within the release's own budget
/// (`Tabs::release_run_scoped`'s `PAGE_END_WAIT`).
const PENDING_WAIT: Duration = Duration::from_millis(500);

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
    /// The context the script belongs to: the page's for a page's script.
    context: String,
    rank: Rank,
}

impl Script {
    /// Whether the script runs in `target`, a page of `context`.
    fn reaches(&self, target: &str, context: &str) -> bool {
        self.context == context && self.page.as_deref().is_none_or(|page| page == target)
    }
}

/// Where a script runs in a session that starts with every script in: Playwright's order. A
/// session that is already showing a page gets each script after the ones it has, in call order.
/// Playwright: `server/page.ts` `Page.allInitScripts`: `return [...bindings, ...this.browserContext.initScripts, ...this.initScripts];`
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Rank {
    /// Playwright's `BindingsController` (`bindings.rs`).
    Controller,
    /// A context's binding, then a page's (`bindings.rs`).
    ContextBinding,
    PageBinding,
    /// `browserContext.addInitScript`, then `page.addInitScript`.
    Context,
    Page,
}

/// The scripts a session that starts showing `target`, a page of `context`, gets, in [`Rank`]
/// order, each rank's in the order the run added them.
fn for_new_session<'a>(scripts: &'a [Script], target: &str, context: &str) -> Vec<&'a Script> {
    let mut reaching: Vec<&Script> = scripts
        .iter()
        .filter(|script| script.reaches(target, context))
        .collect();
    reaching.sort_by_key(|script| script.rank);
    reaching
}

/// A page the run touched, with every session that shows it: the tab's own first, then each
/// out-of-process iframe Chromium attached for it.
struct ShownPage {
    target: String,
    /// The context the page belongs to.
    context: String,
    sessions: Vec<Link>,
    /// What the page does with each out-of-process iframe session attached for it.
    hook: SessionHook,
}

/// Neo: what the page an out-of-process iframe session belongs to does with it before it runs,
/// given the session and the frame's target id: the page's own producers on it, its requests
/// and their routes (`page.rs` `PageSession::init_child_hook`), late-bound so this file names
/// none of them, as the bindings' are (`Bindings::prepare_child`).
pub(crate) type SessionHook =
    Arc<dyn Fn(CdpClient, Link, String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A session that shows a page the run touched, with the token its detach cancels.
#[derive(Clone)]
pub(crate) struct Link {
    pub(crate) session: SessionId,
    pub(crate) gone: CancellationToken,
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
            owners: SyncMutex::default(),
            sessions: SyncMutex::default(),
            stop: CancellationToken::new(),
            ending: CancellationToken::new(),
            bindings: OnceLock::new(),
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
    /// `page.addInitScript` (with `page`) or `browserContext.addInitScript` (without) in
    /// `context`, the page's for a page's script: the new script's `Disposable` guid. A page's
    /// script reaches that page, a context's every page of that context the run touches, each
    /// from its next document on, as Playwright's do.
    ///
    /// # Errors
    ///
    /// `TargetClosedError` after the run ended or when the page's own session cannot take it.
    pub(crate) async fn add(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: Option<&Arc<PageSession>>,
        context: &str,
        source: &str,
    ) -> Result<String, PlaywrightError> {
        let rank = if page.is_some() {
            Rank::Page
        } else {
            Rank::Context
        };
        self.add_ranked(cdp, page, context, source, rank).await
    }

    /// [`Self::add`] for a script of `rank`, as `bindings.rs` adds its own.
    ///
    /// # Errors
    ///
    /// As [`Self::add`].
    pub(crate) async fn add_ranked(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: Option<&Arc<PageSession>>,
        context: &str,
        source: &str,
        rank: Rank,
    ) -> Result<String, PlaywrightError> {
        if self.closed() {
            return Err(PlaywrightError::target_closed());
        }
        if let Some(page) = page {
            self.show(cdp, page, context, true, false).await?;
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
            context: context.to_string(),
            rank,
        };
        let mut state = self.state.lock().await;
        self.any.store(true, Ordering::Relaxed);
        if let Ok(mut owners) = self.owners.lock() {
            owners.insert(guid.clone(), context.to_string());
        }
        for shown_page in &state.pages {
            if !script.reaches(&shown_page.target, &shown_page.context) {
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

    /// The context the live script `guid` belongs to, whose debugger sees its `dispose`.
    pub(crate) fn owner_context(&self, guid: &str) -> Option<String> {
        self.owners.lock().ok()?.get(guid).cloned()
    }

    /// Shows the run's scripts on `page`, of `context`, once a call touches it: its session gets
    /// every script that reaches it, and Chromium attaches its out-of-process iframes as they
    /// start. Nothing happens while the run has no script; every failure leaves the call as it
    /// was.
    pub(crate) async fn ensure(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: &Arc<PageSession>,
        context: &str,
    ) {
        let _shown = self.show(cdp, page, context, false, false).await;
    }

    /// [`Self::ensure`] for a page the run just opened: its first document already exists, so
    /// each script runs there too, as Playwright's `FrameSession` of a new page runs them
    /// (`runImmediately`).
    pub(crate) async fn show_new(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: &Arc<PageSession>,
        context: &str,
    ) {
        let _shown = self.show(cdp, page, context, false, true).await;
    }

    async fn show(
        self: &Arc<Self>,
        cdp: &CdpClient,
        page: &Arc<PageSession>,
        context: &str,
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
        for script in for_new_session(&state.scripts, &target, context) {
            self.install(cdp, &link, &script.guid, &script.source, run_immediately)
                .await;
        }
        self.attach_iframes(cdp, &link).await;
        state.pages.push(ShownPage {
            target,
            context: context.to_string(),
            sessions: vec![link],
            hook: page.init_child_hook(),
        });
        drop(state);
        // Bindings are the run's own context's (`bindings.rs`): a created context's page never
        // gets the CDP binding.
        if context == DEFAULT_CONTEXT
            && let Some(bindings) = self.bindings()
        {
            bindings.prepare_tab(cdp, page).await;
        }
        Ok(())
    }

    /// From now on every session this file shows a page on gets `bindings`' CDP binding.
    pub(crate) fn attach_bindings(&self, bindings: &Arc<Bindings>) {
        let _set = self.bindings.set(Arc::downgrade(bindings));
    }

    fn bindings(&self) -> Option<Arc<Bindings>> {
        self.bindings.get().and_then(Weak::upgrade)
    }

    /// The out-of-process iframe sessions this file holds for `target`'s page, tab excluded.
    pub(crate) async fn child_links(&self, target: &str) -> Vec<Link> {
        let state = self.state.lock().await;
        state
            .pages
            .iter()
            .find(|page| page.target == target)
            .map(|page| page.sessions.iter().skip(1).cloned().collect())
            .unwrap_or_default()
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
        // Registered before it is sent, so a release that comes while it is out waits for it.
        let Some((id, done)) =
            lock(&self.registry).and_then(|mut registry| registry.begin(link, true))
        else {
            return false;
        };
        let (registry, cdp, link, guid, source) = (
            Arc::clone(&self.registry),
            cdp.clone(),
            link.clone(),
            guid.to_string(),
            source.to_string(),
        );
        let task = tokio::spawn(async move {
            // Kept until the task ends: the release's wait for it ends with it.
            let _settled = done;
            let Some(identifier) = add_script(&cdp, &link, &source, run_immediately).await else {
                if let Some(mut registry) = lock(&registry) {
                    registry.pending.remove(&id);
                }
                return false;
            };
            let late = match lock(&registry) {
                Some(mut registry) => {
                    registry.pending.remove(&id);
                    if registry.closed {
                        true
                    } else {
                        registry.installed.push(Installed {
                            link: link.clone(),
                            guid,
                            identifier: identifier.clone(),
                        });
                        false
                    }
                }
                None => true,
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
        let Some((id, done)) = lock(&self.registry).and_then(|mut registry| {
            let begun = registry.begin(link, false)?;
            registry.attaching.push(link.clone());
            Some(begun)
        }) else {
            return;
        };
        let (registry, cdp, link) = (Arc::clone(&self.registry), cdp.clone(), link.clone());
        let task = tokio::spawn(async move {
            let _settled = done;
            // A release since it was registered turned auto-attach off already.
            if lock(&registry).is_none_or(|registry| registry.closed) {
                return;
            }
            set_auto_attach(&cdp, &link, true).await;
            let closed = lock(&registry).is_none_or(|mut registry| {
                registry.pending.remove(&id);
                registry.closed
            });
            if closed {
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
        if let Ok(mut owners) = self.owners.lock() {
            owners.remove(guid);
        }
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
        let (installed, attaching, pending) = match lock(&self.registry) {
            Some(mut registry) => {
                registry.closed = true;
                (
                    mem::take(&mut registry.installed),
                    mem::take(&mut registry.attaching),
                    mem::take(&mut registry.pending),
                )
            }
            None => (Vec::new(), Vec::new(), HashMap::new()),
        };
        if let Some(cdp) = self.cdp.get().cloned() {
            // On a task of its own, so a run end cut short still removes every script.
            let removal = tokio::spawn(async move {
                // Send every removal before waiting: one held acknowledgement must not leave
                // later scripts installed, including those that recreate exposed bindings.
                future::join_all(installed.iter().rev().map(|installed| async {
                    remove_script(&cdp, &installed.link, &installed.identifier).await;
                }))
                .await;
                // The iframes' sessions first: turning auto-attach off on the tab's detaches them.
                for link in attaching.iter().rev() {
                    set_auto_attach(&cdp, link, false).await;
                }
                settle(&cdp, pending).await;
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
            frame,
            link,
            held,
            iframe,
        } = attach;
        let mut shown_here = false;
        let mut hook = None;
        // Bindings are the run's own context's (`bindings.rs`): a created context's iframe gets
        // none.
        let mut binds_here = false;
        if iframe && !self.closed() {
            let mut state = self.state.lock().await;
            let State { scripts, pages } = &mut *state;
            if let Some(page) = pages.iter_mut().find(|page| page.target == target) {
                let context = page.context.clone();
                let _enabled = until_gone(
                    &link.gone,
                    cdp.send("Page.enable", json!({}), Some(&link.session)),
                )
                .await;
                for script in for_new_session(scripts, &target, &context) {
                    self.install(&cdp, &link, &script.guid, &script.source, held)
                        .await;
                }
                if !link.gone.is_cancelled() {
                    page.sessions.push(link.clone());
                    shown_here = true;
                    hook = Some(Arc::clone(&page.hook));
                    binds_here = context == DEFAULT_CONTEXT;
                }
            }
        }
        // Its CDP binding too, before it runs, once the run exposed one (checked after the push,
        // so a binding exposed meanwhile reaches this session one way or the other).
        if binds_here && let Some(bindings) = self.bindings() {
            bindings.prepare_child(&cdp, &link, &target).await;
        }
        // Then the page's requests and routes, so its first request is the tab's to see and to
        // pause, as Playwright's `FrameSession._initialize` turns on a frame session's network
        // before it runs it.
        if let Some(hook) = hook {
            tokio::select! {
                () = link.gone.cancelled() => {}
                () = hook(cdp.clone(), link.clone(), frame) => {}
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

/// Releases the run's init scripts, as a closed context and the run's end let go of what the
/// run holds (`Tabs::release_run_scoped`, awaited before the close is acknowledged and at run
/// end): every script the run added leaves every session that shows it, the out-of-process
/// iframes' included, and the registry closes first, so an attach still pending installs nothing
/// and a reply that comes late undoes itself ([`InitScripts::close`]). No document loaded after
/// it returns runs one of the run's scripts; a second call finds nothing left. It waits on the
/// browser's replies, so its caller bounds it.
///
/// Playwright: `server/browserContext.ts` `BrowserContext.close`: `const disposition = await progress.race(this.doClose(options.reason));`
pub(crate) async fn release_all(run: &RunScope) {
    run.init_scripts.close().await;
}

/// A target one of the run's sessions attached, as its event names it.
struct Attach {
    /// The page the attaching session shows.
    target: String,
    /// The attached target's id: an out-of-process iframe's is its frame's.
    frame: String,
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
            let session = SessionId::from(session);
            let attach = Attach {
                link: Link {
                    gone: scripts.watched(&session, &target),
                    session,
                },
                frame: event.params["targetInfo"]["targetId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
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
pub(crate) async fn until_gone<T>(
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
pub(crate) fn target_of(page: &PageSession) -> String {
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
async fn set_auto_attach(cdp: &CdpClient, link: &Link, on: bool) {
    let _set = until_gone(
        &link.gone,
        cdp.send(
            "Target.setAutoAttach",
            json!({
                "autoAttach": on,
                "waitForDebuggerOnStart": on,
                "flatten": true,
                "filter": [{ "type": "iframe" }],
            }),
            Some(&link.session),
        ),
    )
    .await;
}

/// Waits for the requests still out when the release closed the registry: each undoes its own
/// effect once its reply comes ([`InitScripts::install`], [`InitScripts::attach_iframes`]). A
/// session whose installation has not answered within [`PENDING_WAIT`] already runs that script
/// on its next document, and only the reply names it, so its new-document scripts stop at once
/// ([`suspend_scripts`]); the script itself still goes when the reply comes.
async fn settle(cdp: &CdpClient, pending: HashMap<u64, Pending>) {
    let deadline = Instant::now() + PENDING_WAIT;
    let mut suspended: Vec<SessionId> = Vec::new();
    for (_, request) in pending {
        if time::timeout_at(deadline, request.settled).await.is_ok() || !request.installs {
            continue;
        }
        if !suspended.contains(&request.link.session) {
            suspend_scripts(cdp, &request.link).await;
            suspended.push(request.link.session.clone());
        }
    }
}

/// Neo: `Page.disable` on `link`'s session, which stops every script that session added from
/// running on its next documents (measured on BrowserOS neo 155.0.8318.26: `Page.enable` would
/// bring them back, and nothing enables the page of a released run again). The session's
/// isolated worlds keep working.
async fn suspend_scripts(cdp: &CdpClient, link: &Link) {
    let _disabled = until_gone(
        &link.gone,
        cdp.send("Page.disable", json!({}), Some(&link.session)),
    )
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
    fn a_page_script_reaches_its_page_and_a_context_script_every_page_of_its_context() {
        let page = script("a", Some("T1"), Rank::Page);
        let context = script("b", None, Rank::Context);
        let created = Script {
            context: "C1".into(),
            ..script("c", None, Rank::Context)
        };
        assert!(page.reaches("T1", "default") && !page.reaches("T2", "default"));
        assert!(context.reaches("T1", "default") && context.reaches("T2", "default"));
        assert!(
            !context.reaches("T3", "C1"),
            "the run's own context's script stays in it"
        );
        assert!(created.reaches("T3", "C1") && !created.reaches("T1", "default"));
    }

    fn script(guid: &str, page: Option<&str>, rank: Rank) -> Script {
        Script {
            guid: guid.into(),
            source: String::new(),
            page: page.map(Into::into),
            context: "default".into(),
            rank,
        }
    }

    #[test]
    fn a_new_session_gets_bindings_first_then_context_then_page_scripts_each_in_call_order() {
        let scripts = [
            script("page-1", Some("T1"), Rank::Page),
            script("context-1", None, Rank::Context),
            script("controller", None, Rank::Controller),
            script("page-binding", Some("T1"), Rank::PageBinding),
            script("other-page", Some("T2"), Rank::Page),
            script("context-binding", None, Rank::ContextBinding),
            script("page-2", Some("T1"), Rank::Page),
            script("context-2", None, Rank::Context),
        ];
        let order: Vec<&str> = for_new_session(&scripts, "T1", "default")
            .into_iter()
            .map(|script| script.guid.as_str())
            .collect();
        assert_eq!(
            order,
            [
                "controller",
                "context-binding",
                "page-binding",
                "context-1",
                "context-2",
                "page-1",
                "page-2"
            ]
        );
    }
}
