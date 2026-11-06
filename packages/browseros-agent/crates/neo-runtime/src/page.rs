//! The run's tab as the server side of a browser call sees it (Playwright's `server/page.ts`
//! with `chromium/crPage.ts` and `crInput.ts`, cut to what click needs): the run's CDP session,
//! the domains a browser call needs, one watcher task that answers dialogs and tracks
//! navigations, the mouse writer, and the `click::Tab` implementation over all of it and the
//! utility world (`world.rs`).
//!
//! This file is mechanism; `click.rs` is policy. Nothing here writes a call-log line or decides
//! a retry, and nothing in `click.rs` names a CDP type.
//!
//! Lifetime: [`PageSession`] owns the run's [`Session`], whose `Drop` detaches, so the session
//! detaches when the last `Arc<PageSession>` drops. Every task that writes on the session holds
//! one: the dispatcher's call tasks, the mouse writers, the interceptor stops and the
//! object-group releases, all spawned on the run's `TaskTracker`. So no release can find its
//! session gone, and no end-of-run step has to order a drain before a detach; this also covers
//! a caller that drops the run's future. The watcher holds no `Arc` (it would keep the page
//! alive forever): it holds a CDP client, the session id and a token cancelled when the page
//! drops. A task stuck past the run (a renderer frozen by a dialog the watcher missed) keeps the
//! session attached until its own CDP request times out (browseros-cdp's 60 s), then lets go.
//!
//! Domains, enabled by [`PageSession::prepare`] on the run's first browser call only, so a
//! getter-only run keeps its single live CDP call per getter:
//! - `Page.enable`: `javascriptDialogOpening` (an `alert()` in an `onclick` freezes the renderer,
//!   and the click's mouse acknowledgement never returns until it is answered), the navigation
//!   events behind the pre-check and the post-click barrier, and `Page.enable` itself as that
//!   barrier's flush (`crPage.ts` `inputActionEpilogue`).
//! - `Emulation.setFocusEmulationEnabled {enabled: true}` (untyped: browseros-cdp's surface has
//!   no Emulation domain), as Playwright sends it. It makes a background tab paint, so
//!   `requestAnimationFrame` runs and `checkElementStates`' `stable` resolves (probe P3: 0
//!   frames before, 30 per 500 ms after). The page also sees itself visible and focused while
//!   attached. A failure is logged and ignored: a foreground tab paints anyway.
//! - Not `Runtime.enable` (worlds are found by name, `world.rs`) and not `DOM.enable` (quads,
//!   box models and scrolls by `objectId` need none, probe P2).

use std::future::Future;
use std::iter;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use browseros_cdp::input::DispatchMouseEventParams;
use browseros_cdp::{CdpClient, CdpError, CdpEvent, SessionId, dom, target};
use futures_util::future;
use neo_runtime_spec::injected::{
    ElementState, ElementStatesOutcome, HitTargetInterceptor, HitTargetOutcome, HitTargetSetup,
    InjectedCheckElementStatesParams, InjectedDescribeSelectorParams,
    InjectedGetElementBorderWidthParams, InjectedResolveParams, InjectedScrollIntoViewParams,
    InjectedSetupHitTargetInterceptorParams, InjectedStopHitTargetInterceptorParams,
    InjectedViewportSizeParams, Point, ScrollAlignment,
};
use neo_runtime_spec::{Element, Handle};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{OnceCell, broadcast, oneshot, watch};
use tokio::time;
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;
use tracing::{debug, warn};

use crate::click::{
    Arming, Geometry, MouseButton, MouseClick, NotReady, PageError, PressReport, Resolved, Scroll,
    SelectorFacts, StatesCheck, Tab, Verdict,
};
use crate::progress::InputTicket;
use crate::run::PageId;
use crate::world::{ObjectGroup, World, classify};

/// Detaching is best effort. It runs in the background after a run, so this bound only keeps
/// a hung browser from piling up detach tasks.
const DETACH_TIMEOUT: Duration = Duration::from_secs(1);

/// The run's CDP session on its tab, which owns the attach: dropping it detaches. The
/// `Runtime`'s connection outlives every run, so a session nobody detaches stays attached
/// until the browser exits. Tying the detach to the value covers every way a run ends after
/// the browser answered the attach: the script's end, a failed or stopped setup, an answer
/// that arrives after the run stopped, and a caller that drops the run's future.
pub(crate) struct Session {
    pub(crate) cdp: CdpClient,
    pub(crate) id: SessionId,
    /// browseros-cdp's socket incarnation, read just before the attach is sent: the
    /// incarnation `id` belongs to. Mouse input is written with `send_at_epoch(epoch, …)`, so
    /// a press is never sent on a reconnected socket under a session id that does not exist
    /// there.
    epoch: u64,
    /// Drop is synchronous, so the detach and every cleanup task run on this runtime.
    tokio: TokioHandle,
}

impl Session {
    pub(crate) async fn attach(
        cdp: CdpClient,
        page: PageId,
        tokio: TokioHandle,
    ) -> Result<Self, CdpError> {
        let epoch = cdp.epoch();
        let attached: target::AttachToTargetResult = cdp
            .send_typed(
                "Target.attachToTarget",
                target::AttachToTargetParams {
                    target_id: page.as_str().to_string(),
                    flatten: Some(true),
                },
                None,
            )
            .await?;
        Ok(Self {
            cdp,
            id: SessionId::from(attached.session_id),
            epoch,
            tokio,
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let cdp = self.cdp.clone();
        let session = self.id.clone();
        self.tokio.spawn(async move {
            let detach = cdp.send_typed::<_, Value>(
                "Target.detachFromTarget",
                target::DetachFromTargetParams {
                    session_id: Some(session.as_str().to_string()),
                    target_id: None,
                },
                None,
            );
            match time::timeout(DETACH_TIMEOUT, detach).await {
                Ok(Ok(_)) => {}
                // Expected once the tab has closed: the session went with it.
                Ok(Err(error)) => debug!(%session, %error, "detaching a run's session failed"),
                Err(_) => debug!(%session, "detaching a run's session timed out"),
            }
        });
    }
}

/// The run's attached tab. One per run, shared (`Arc`) by the dispatcher and every task that
/// writes on its session.
pub(crate) struct PageSession {
    session: Session,
    /// The main frame's id, which is the target id.
    main_frame: String,
    /// The run's tracker, for the writer, interceptor-stop and group-release tasks, so
    /// `Dispatcher::finish` can wait for them before reading final deliveries.
    tasks: TaskTracker,
    /// The domains and the watcher, set up by the run's first browser call.
    prepared: OnceCell<Watcher>,
    /// Cancelled when the tab closes or the session detaches; every call's `Progress` races
    /// it, so a call on a closed tab ends with Playwright's `TargetClosedError`.
    closed: CancellationToken,
    /// The main frame's utility world, where the glue runs.
    world: World,
    /// Names this session's object groups apart.
    next_group: AtomicU64,
}

/// The watcher's published state, and the guard that stops it when the page drops.
struct Watcher {
    state: watch::Receiver<PageState>,
    _stop: DropGuard,
}

/// What the run's session has reported since `prepare`. Counters only grow, so a caller takes
/// a [`PageMark`] and compares later.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageState {
    /// Main-frame `Page.frameRequestedNavigation` with `disposition: currentTab`, what
    /// Playwright's `SignalBarrier` waits on.
    pub(crate) navigations_requested: u64,
    /// The URL of the newest requested navigation that has not committed (`frameNavigated`),
    /// finished within the document (`navigatedWithinDocument`) or stopped
    /// (`frameStoppedLoading`): Playwright's `pendingDocument`.
    pub(crate) pending_url: Option<String>,
    /// Main-frame commits, cross-document or within it.
    pub(crate) commits: u64,
    /// The URL of the newest main-frame commit.
    pub(crate) committed_url: Option<String>,
    /// `Page.javascriptDialogOpening` events seen; each was answered.
    pub(crate) dialogs_opened: u64,
}

/// A snapshot of the counters in [`PageState`], taken when an interceptor is armed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PageMark {
    navigations_requested: u64,
    dialogs_opened: u64,
}

/// How the watcher answers a dialog: Playwright's default with no `page.on('dialog')` listener
/// (`client/browserContext.ts`): accept `beforeunload`, dismiss everything else. This applies
/// to the user's real tab while a clicking run is attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogAnswer {
    Accept,
    Dismiss,
}

/// What one event did to the page, for the watcher.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) changed: bool,
    pub(crate) dialog: Option<DialogAnswer>,
    pub(crate) closed: bool,
}

impl PageSession {
    pub(crate) fn new(session: Session, page: &PageId, tasks: TaskTracker) -> Self {
        let world = World::new(
            session.cdp.clone(),
            session.id.clone(),
            page.as_str().to_string(),
        );
        Self {
            session,
            main_frame: page.as_str().to_string(),
            tasks,
            prepared: OnceCell::new(),
            closed: CancellationToken::new(),
            world,
            next_group: AtomicU64::new(1),
        }
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session.id
    }

    pub(crate) fn closed(&self) -> CancellationToken {
        self.closed.clone()
    }

    /// Enables the domains above and starts the watcher, once per run; concurrent first calls
    /// wait for the same setup. Subscribes `cdp.events()` before `Page.enable`, so no event the
    /// enable triggers (a dialog already open is reported then) is missed. Never
    /// `events_targeted`, which is exclusive per method across the whole shared client.
    ///
    /// # Errors
    ///
    /// The CDP error of `Page.enable`.
    async fn prepare(&self) -> Result<(), CdpError> {
        self.prepared
            .get_or_try_init(|| async {
                let events = self.session.cdp.events();
                self.send("Page.enable", json!({})).await?;
                let (state, receiver) = watch::channel(PageState::default());
                // Guarded before the spawn: if this setup is dropped before it finishes (the
                // call stopped), the guard stops the watcher it already started.
                let stop = CancellationToken::new();
                let stop_guard = stop.clone().drop_guard();
                self.session.tokio.spawn(watch_events(
                    events,
                    self.session.cdp.clone(),
                    self.session.id.clone(),
                    self.main_frame.clone(),
                    state,
                    self.closed.clone(),
                    stop,
                ));
                let focus = self
                    .send(
                        "Emulation.setFocusEmulationEnabled",
                        json!({ "enabled": true }),
                    )
                    .await;
                if let Err(error) = focus {
                    debug!(%error, "focus emulation refused; a background tab may not paint");
                }
                Ok(Watcher {
                    state: receiver,
                    _stop: stop_guard,
                })
            })
            .await
            .map(|_| ())
    }

    async fn send(&self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.session
            .cdp
            .send(method, params, Some(&self.session.id))
            .await
    }

    /// Runs a cleanup or writer task on the run's tracker. Spawned on the session's runtime
    /// handle because it may be called from a `Drop`.
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks.spawn_on(task, &self.session.tokio);
    }

    fn state(&self) -> Option<watch::Receiver<PageState>> {
        self.prepared.get().map(|watcher| watcher.state.clone())
    }

    /// Resolves when no navigation is pending, with the URL it committed to, if any.
    async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
        let Some(mut state) = self.state() else {
            return Ok(None);
        };
        let commits = state.borrow().commits;
        let settled = state
            .wait_for(|state| state.pending_url.is_none())
            .await
            .map_err(|_| PageError::Closed)?;
        Ok(if settled.commits > commits {
            settled.committed_url.clone()
        } else {
            None
        })
    }

    fn mark(&self) -> PageMark {
        self.state().map_or_else(PageMark::default, |state| {
            let state = state.borrow();
            PageMark {
                navigations_requested: state.navigations_requested,
                dialogs_opened: state.dialogs_opened,
            }
        })
    }
}

/// A fresh object group of `page`'s session, released when the guard drops.
fn new_group(page: &Arc<PageSession>, purpose: &str) -> GroupGuard {
    let index = page.next_group.fetch_add(1, Ordering::Relaxed);
    GroupGuard {
        page: Arc::clone(page),
        group: ObjectGroup(format!("neo-{purpose}-{index}")),
    }
}

// ---------------------------------------------------------------------------------------------
// The watcher.
// ---------------------------------------------------------------------------------------------

/// The watcher task: one `cdp.events()` receiver for the run, filtered to this session (and to
/// `Target.detachedFromTarget` naming it). Answers dialogs with `Page.handleJavaScriptDialog`
/// on its own task, so no click await can block the answer; applies every event to `state`;
/// cancels `closed` when the session goes. On `RecvError::Lagged` it warns, forgets any pending
/// navigation (whose end it may have missed, and which would otherwise hold every later click
/// until its timeout), and sends one `Page.handleJavaScriptDialog {accept: false}` whose "No
/// dialog is showing" error is ignored, so a dialog event lost to the 4096-slot ring cannot
/// hang the run.
async fn watch_events(
    mut events: broadcast::Receiver<CdpEvent>,
    cdp: CdpClient,
    session: SessionId,
    main_frame: String,
    state: watch::Sender<PageState>,
    closed: CancellationToken,
    stop: CancellationToken,
) {
    loop {
        let event = tokio::select! {
            () = stop.cancelled() => return,
            event = events.recv() => event,
        };
        match event {
            Ok(event) => {
                let mut applied = Applied::default();
                state.send_if_modified(|state| {
                    applied = apply(state, &event, &session, &main_frame);
                    applied.changed
                });
                if let Some(answer) = applied.dialog {
                    answer_dialog(&cdp, &session, answer);
                }
                if applied.closed {
                    closed.cancel();
                    return;
                }
            }
            Err(RecvError::Lagged(skipped)) => {
                warn!(%session, skipped, "the run's event watcher lagged; answering any dialog");
                state.send_if_modified(|state| state.pending_url.take().is_some());
                answer_dialog(&cdp, &session, DialogAnswer::Dismiss);
            }
            Err(RecvError::Closed) => {
                closed.cancel();
                return;
            }
        }
    }
}

/// Sends a dialog answer without waiting for it: the renderer is blocked until it lands, and
/// the watcher must keep reading events meanwhile.
fn answer_dialog(cdp: &CdpClient, session: &SessionId, answer: DialogAnswer) {
    let cdp = cdp.clone();
    let session = session.clone();
    tokio::spawn(async move {
        let accept = answer == DialogAnswer::Accept;
        let answered = cdp
            .send(
                "Page.handleJavaScriptDialog",
                json!({ "accept": accept }),
                Some(&session),
            )
            .await;
        if let Err(error) = answered {
            debug!(%session, %error, "answering a dialog failed");
        }
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameRequestedNavigation {
    frame_id: String,
    url: String,
    disposition: String,
}

#[derive(Deserialize)]
struct FrameNavigated {
    frame: NavigatedFrame,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NavigatedFrame {
    id: String,
    url: String,
    #[serde(default)]
    url_fragment: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameEvent {
    frame_id: String,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct DialogOpening {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachedFromTarget {
    #[serde(default)]
    session_id: Option<String>,
}

/// Applies one event to the page state. Pure. Event shapes are local serde structs:
/// browseros-cdp's typed surface lacks most of them, and adding them would change a shared
/// crate that core also compiles.
pub(crate) fn apply(
    state: &mut PageState,
    event: &CdpEvent,
    session: &SessionId,
    main_frame: &str,
) -> Applied {
    let mut applied = Applied::default();
    if event.method == "Target.detachedFromTarget" {
        let detached = serde_json::from_value::<DetachedFromTarget>(event.params.clone());
        applied.closed =
            detached.is_ok_and(|detached| detached.session_id.as_deref() == Some(session.as_str()));
        return applied;
    }
    if event.session_id.as_ref() != Some(session) {
        return applied;
    }
    let params = event.params.clone();
    match event.method.as_str() {
        "Page.frameRequestedNavigation" => {
            if let Ok(requested) = serde_json::from_value::<FrameRequestedNavigation>(params)
                && requested.frame_id == main_frame
                && requested.disposition == "currentTab"
            {
                state.navigations_requested += 1;
                state.pending_url = Some(requested.url);
                applied.changed = true;
            }
        }
        "Page.frameNavigated" => {
            if let Ok(FrameNavigated { frame }) = serde_json::from_value(params)
                && frame.id == main_frame
            {
                state.commits += 1;
                let fragment = frame.url_fragment.as_deref().unwrap_or("");
                state.committed_url = Some(format!("{}{fragment}", frame.url));
                state.pending_url = None;
                applied.changed = true;
            }
        }
        "Page.navigatedWithinDocument" => {
            if let Ok(FrameEvent {
                frame_id,
                url: Some(url),
            }) = serde_json::from_value(params)
                && frame_id == main_frame
            {
                state.commits += 1;
                state.committed_url = Some(url);
                state.pending_url = None;
                applied.changed = true;
            }
        }
        "Page.frameStoppedLoading" => {
            if let Ok(FrameEvent { frame_id, .. }) = serde_json::from_value(params)
                && frame_id == main_frame
                && state.pending_url.is_some()
            {
                state.pending_url = None;
                applied.changed = true;
            }
        }
        "Page.javascriptDialogOpening" => {
            let beforeunload = serde_json::from_value::<DialogOpening>(params)
                .is_ok_and(|dialog| dialog.kind == "beforeunload");
            state.dialogs_opened += 1;
            applied.changed = true;
            applied.dialog = Some(if beforeunload {
                DialogAnswer::Accept
            } else {
                DialogAnswer::Dismiss
            });
        }
        "Inspector.detached" | "Inspector.targetCrashed" => applied.closed = true,
        _ => {}
    }
    applied
}

// ---------------------------------------------------------------------------------------------
// Handles that clean up after themselves.
// ---------------------------------------------------------------------------------------------

/// An object group this session holds. Dropping it releases the group
/// (`Runtime.releaseObjectGroup`) on a tracked task holding the page, fire and forget: a
/// renderer stuck on a dialog must not hold the dropping call. One release frees the result
/// object and every handle `getProperties` read out of it (probe P1).
pub(crate) struct GroupGuard {
    page: Arc<PageSession>,
    group: ObjectGroup,
}

impl GroupGuard {
    fn group(&self) -> &ObjectGroup {
        &self.group
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        let page = Arc::clone(&self.page);
        let ObjectGroup(group) = self.group.clone();
        self.page.spawn(async move {
            let released = page
                .send(
                    "Runtime.releaseObjectGroup",
                    json!({ "objectGroup": group }),
                )
                .await;
            if let Err(error) = released {
                debug!(%error, "releasing an object group failed");
            }
        });
    }
}

/// `click::Tab::Element` for the real tab: one element and the group of the resolution that
/// produced it. The handle dies with the group.
pub(crate) struct ResolvedElement {
    handle: Handle<Element>,
    _group: GroupGuard,
}

/// `click::Tab::Armed` for the real tab. The interceptor handle lives in its own object group,
/// separate from the element's, so its stop never races the element group's release: the
/// interceptor's JS closure keeps its own reference to the element.
pub(crate) struct ArmedInterceptor {
    handle: Handle<HitTargetInterceptor>,
    group: Option<GroupGuard>,
    page: Arc<PageSession>,
    /// The counters when it was armed, for the verdict's dialog and navigation race.
    mark: PageMark,
    /// Set once `stopHitTargetInterceptor` has answered, so `Drop` does not send it again.
    stopped: bool,
}

impl Drop for ArmedInterceptor {
    fn drop(&mut self) {
        let group = self.group.take();
        if self.stopped {
            return;
        }
        // Playwright's `finally` sends the same fire-and-forget stop. The stop goes before the
        // group release, in one task, so the handle is still alive when the stop names it.
        let page = Arc::clone(&self.page);
        let interceptor = self.handle.clone();
        self.page.spawn(async move {
            let stopped = page
                .world
                .call(
                    None,
                    &InjectedStopHitTargetInterceptorParams { interceptor },
                )
                .await;
            if let Err(error) = stopped {
                debug!(?error, "disarming a hit-target interceptor failed");
            }
            drop(group);
        });
    }
}

/// Navigation events of this session, from a receiver subscribed before a click's input.
pub(crate) struct NavigationWatch {
    events: broadcast::Receiver<CdpEvent>,
}

/// What a click's own navigations came to while draining a [`NavigationWatch`]: the same
/// [`apply`] the watcher runs, on a state of its own that starts at the input.
#[derive(Default)]
struct Barrier {
    state: PageState,
    /// Requested main-frame navigations not yet settled: Playwright's barrier retains once per
    /// request, and one navigation event releases every waiter at once.
    waiting: usize,
    /// One URL per waiter a commit released, as each Playwright waiter logs its own line.
    committed: Vec<String>,
}

impl Barrier {
    fn apply(&mut self, event: &CdpEvent, session: &SessionId, main_frame: &str) {
        let before = (self.state.navigations_requested, self.state.commits);
        let was_pending = self.state.pending_url.is_some();
        if !apply(&mut self.state, event, session, main_frame).changed {
            return;
        }
        if self.state.navigations_requested > before.0 {
            self.waiting += 1;
        } else if self.state.commits > before.1 {
            let url = self.state.committed_url.clone().unwrap_or_default();
            self.committed.extend(iter::repeat_n(url, self.waiting));
            self.waiting = 0;
        } else if was_pending && self.state.pending_url.is_none() {
            // `frameStoppedLoading`: a request that never commits (a 204, a download).
            self.waiting = 0;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The tab, for click.
// ---------------------------------------------------------------------------------------------

impl Tab for Arc<PageSession> {
    type Element = ResolvedElement;
    type Armed = ArmedInterceptor;
    type Navigations = NavigationWatch;

    async fn prepare(&self) -> Result<(), PageError> {
        PageSession::prepare(self)
            .await
            .map_err(|error| classify(&error))
    }

    async fn describe(&self, selector: &str) -> Result<SelectorFacts, PageError> {
        let described = self
            .world
            .call(
                None,
                &InjectedDescribeSelectorParams {
                    selector: selector.to_string(),
                },
            )
            .await?;
        Ok(SelectorFacts {
            locator: described.locator,
            invalid: described.invalid_selector,
        })
    }

    async fn resolve(
        &self,
        selector: &str,
        strict: bool,
    ) -> Result<Option<Resolved<ResolvedElement>>, PageError> {
        let group = new_group(self, "resolve");
        let resolved = self
            .world
            .call(
                Some(group.group()),
                &InjectedResolveParams {
                    selector: selector.to_string(),
                    strict,
                },
            )
            .await?;
        Ok(resolved.element.map(|handle| Resolved {
            element: ResolvedElement {
                handle,
                _group: group,
            },
            count: u32::try_from(resolved.count).unwrap_or(u32::MAX),
            preview: resolved.preview.unwrap_or_default(),
        }))
    }

    async fn check_states(
        &self,
        element: &ResolvedElement,
        states: &[ElementState],
    ) -> Result<StatesCheck, PageError> {
        let checked = self
            .world
            .call(
                None,
                &InjectedCheckElementStatesParams {
                    element: element.handle.clone(),
                    states: states.to_vec(),
                    frame_visible: true,
                },
            )
            .await?;
        Ok(match checked.outcome {
            None => StatesCheck::Ready,
            Some(ElementStatesOutcome::NotConnected(_)) => StatesCheck::NotConnected,
            Some(ElementStatesOutcome::Missing(missing)) => {
                StatesCheck::Missing(missing.missing_state)
            }
        })
    }

    /// `Protocol`: `DOM.scrollIntoViewIfNeeded {objectId, rect?}`; "Node does not have a layout
    /// object" is `NotVisible` and "Node is detached from document" `NotConnected`, the one
    /// place Playwright itself reads CDP error text (`crPage.ts`
    /// `_scrollRectIntoViewIfNeeded`). Other modes: the `scrollIntoView` glue with `block` and
    /// `inline` both the mode.
    async fn scroll(
        &self,
        element: &ResolvedElement,
        scroll: Scroll,
        position: Option<&Point>,
    ) -> Result<Option<NotReady>, PageError> {
        let alignment = match scroll {
            Scroll::Protocol => {
                let params = dom::ScrollIntoViewIfNeededParams {
                    object_id: Some(element.handle.id().object_id().to_string()),
                    rect: position.map(|position| dom::Rect {
                        x: position.x,
                        y: position.y,
                        width: 0.0,
                        height: 0.0,
                    }),
                    ..dom::ScrollIntoViewIfNeededParams::default()
                };
                let scrolled = self
                    .session
                    .cdp
                    .send_typed::<_, Value>(
                        "DOM.scrollIntoViewIfNeeded",
                        params,
                        Some(&self.session.id),
                    )
                    .await;
                return match scrolled {
                    Ok(_) => Ok(None),
                    Err(CdpError::Protocol { message, .. })
                        if message.contains("Node does not have a layout object") =>
                    {
                        Ok(Some(NotReady::NotVisible))
                    }
                    Err(CdpError::Protocol { message, .. })
                        if message.contains("Node is detached from document") =>
                    {
                        Ok(Some(NotReady::NotConnected))
                    }
                    Err(error) => Err(classify(&error)),
                };
            }
            Scroll::End => ScrollAlignment::End,
            Scroll::Center => ScrollAlignment::Center,
            Scroll::Start => ScrollAlignment::Start,
        };
        self.world
            .call(
                None,
                &InjectedScrollIntoViewParams {
                    element: element.handle.clone(),
                    block: alignment,
                    inline: alignment,
                },
            )
            .await?;
        Ok(None)
    }

    /// `DOM.getContentQuads {objectId}` together with the `viewportSize` glue (Playwright's
    /// `Promise.all`), or `DOM.getBoxModel {objectId}` with `getElementBorderWidth` for
    /// `position`. A failed quads, box or border read is `Geometry::NoQuads`, as Playwright's
    /// `_sendMayFail` and `.catch` make it, unless the session is gone.
    async fn geometry(
        &self,
        element: &ResolvedElement,
        position: Option<&Point>,
    ) -> Result<Geometry, PageError> {
        let object_id = Some(element.handle.id().object_id().to_string());
        let session = Some(&self.session.id);
        if position.is_some() {
            let model = self.session.cdp.send_typed::<_, dom::GetBoxModelResult>(
                "DOM.getBoxModel",
                dom::GetBoxModelParams {
                    object_id,
                    ..dom::GetBoxModelParams::default()
                },
                session,
            );
            let border_params = InjectedGetElementBorderWidthParams {
                element: element.handle.clone(),
            };
            let border = self.world.call(None, &border_params);
            let (model, border) = tokio::join!(model, border);
            let model = match model {
                Ok(model) => model.model,
                Err(error) => return no_geometry(&classify(&error)),
            };
            let border = match border {
                Ok(border) => border,
                Err(error) => return no_geometry(&error),
            };
            return Ok(Geometry::Offset {
                border_box: quad(model.border)?,
                border: (border.left, border.top),
            });
        }
        let quads = self
            .session
            .cdp
            .send_typed::<_, dom::GetContentQuadsResult>(
                "DOM.getContentQuads",
                dom::GetContentQuadsParams {
                    object_id,
                    ..dom::GetContentQuadsParams::default()
                },
                session,
            );
        let viewport = self.world.call(None, &InjectedViewportSizeParams {});
        let (quads, viewport) = tokio::join!(quads, viewport);
        let viewport = viewport?;
        let quads = match quads {
            Ok(quads) => quads.quads,
            Err(error) => return no_geometry(&classify(&error)),
        };
        Ok(Geometry::Quads {
            quads: quads.into_iter().map(quad).collect::<Result<_, _>>()?,
            viewport: (pixels(viewport.width), pixels(viewport.height)),
        })
    }

    /// `setupHitTargetInterceptor` into a fresh group: `error:notconnected` is `NotConnected`,
    /// any other description `Intercepted`, the handle `Armed` with the current counters.
    async fn arm(
        &self,
        element: &ResolvedElement,
        hit_point: Option<Point>,
        block_all: bool,
    ) -> Result<Arming<ArmedInterceptor>, PageError> {
        let mark = self.mark();
        let group = new_group(self, "interceptor");
        let armed = self
            .world
            .call(
                Some(group.group()),
                &InjectedSetupHitTargetInterceptorParams {
                    element: element.handle.clone(),
                    hit_point,
                    block_all_events: block_all,
                },
            )
            .await?;
        Ok(match armed.outcome {
            HitTargetSetup::Description(description) if description == "error:notconnected" => {
                Arming::NotConnected
            }
            HitTargetSetup::Description(description) => Arming::Intercepted(description),
            HitTargetSetup::Armed(handle) => Arming::Armed(ArmedInterceptor {
                handle,
                group: Some(group),
                page: Arc::clone(self),
                mark,
                stopped: false,
            }),
        })
    }

    /// Spawns [`write_mouse`] on the run's tracker now, with an `Arc` of this page (so of the
    /// session) and the ticket, and returns a future that awaits its report while holding a
    /// drop guard on the writer's `cut_short` token: dropping the future (the call stopped)
    /// cuts any remaining `delay` short, and the writer still sends the release and marks the
    /// ticket.
    fn press(
        &self,
        point: Point,
        click: MouseClick,
        ticket: InputTicket,
    ) -> impl Future<Output = PressReport> + Send {
        let cut_short = CancellationToken::new();
        let (report, receiver) = oneshot::channel();
        let writer = write_mouse(Arc::clone(self), point, click, ticket, cut_short.clone());
        self.spawn(async move {
            // Nobody may be waiting any more; the ticket already holds the outcome.
            let _waiting = report.send(writer.await);
        });
        let cut_short = cut_short.drop_guard();
        async move {
            let report = receiver.await.unwrap_or_else(|_| PressReport::Failed {
                closed: true,
                message: "the mouse writer stopped before reporting".to_string(),
            });
            drop(cut_short);
            report
        }
    }

    /// `stopHitTargetInterceptor`, raced against `PageState` moving past the mark taken when it
    /// was armed (a dialog opened or a navigation requested): Playwright's
    /// `raceAgainstEvaluationStallingEvents`, which gives up on evaluating while either is in
    /// the way. If the race is lost, the stop still goes out in the background
    /// (`ArmedInterceptor`'s `Drop`), as Playwright's `finally` sends it.
    async fn verdict(&self, mut armed: ArmedInterceptor) -> Verdict {
        let mark = armed.mark;
        let mut state = self.state();
        let stalled = async {
            match state.as_mut() {
                Some(state) => {
                    // A closed watcher (the tab went) reads as stalled too.
                    let _moved: bool = state
                        .wait_for(|state| {
                            state.dialogs_opened > mark.dialogs_opened
                                || state.navigations_requested > mark.navigations_requested
                        })
                        .await
                        .is_ok();
                }
                None => future::pending::<()>().await,
            }
        };
        let params = InjectedStopHitTargetInterceptorParams {
            interceptor: armed.handle.clone(),
        };
        let stop = self.world.call(None, &params);
        tokio::select! {
            biased;
            () = stalled => Verdict::Done,
            stopped = stop => match stopped {
                Ok(stopped) => {
                    armed.stopped = true;
                    match stopped.outcome {
                        HitTargetOutcome::Done(_) => Verdict::Done,
                        HitTargetOutcome::Blocked(blocked) => {
                            Verdict::Blocked(blocked.hit_target_description)
                        }
                    }
                }
                Err(_) => Verdict::Done,
            },
        }
    }

    fn pending_navigation(&self) -> Option<String> {
        self.state()
            .and_then(|state| state.borrow().pending_url.clone())
    }

    async fn navigation_settled(&self) -> Result<Option<String>, PageError> {
        PageSession::navigation_settled(self).await
    }

    fn watch_navigations(&self) -> NavigationWatch {
        NavigationWatch {
            events: self.session.cdp.events(),
        }
    }

    /// Sends `Page.enable` again as a flush: Chromium replies after every
    /// `frameRequestedNavigation` the input caused (`crPage.ts` `inputActionEpilogue`).
    /// browseros-cdp's single reader broadcasts events and routes replies in socket order
    /// (`client.rs` `handle_text_message`), so once the reply is back every such event is
    /// already queued in `watched`, which was subscribed before the input: draining it finds
    /// exactly the navigations the click requested. Each is then waited out until it commits,
    /// finishes in the document, or stops. On `Lagged`, it falls back to the watcher's
    /// `pending_url`. The caller races this against the call's deadline.
    async fn settle_after_input(
        &self,
        mut watched: NavigationWatch,
    ) -> Result<Vec<String>, PageError> {
        if let Err(error) = self.send("Page.enable", json!({})).await
            && classify(&error) == PageError::Closed
        {
            return Err(PageError::Closed);
        }
        let mut barrier = Barrier::default();
        let session = &self.session.id;
        loop {
            match watched.events.try_recv() {
                Ok(event) => barrier.apply(&event, session, &self.main_frame),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(_)) => return settled_after_lag(self).await,
                Err(TryRecvError::Closed) => return Err(PageError::Closed),
            }
        }
        while barrier.waiting > 0 {
            match watched.events.recv().await {
                Ok(event) => barrier.apply(&event, session, &self.main_frame),
                Err(RecvError::Lagged(_)) => return settled_after_lag(self).await,
                Err(RecvError::Closed) => return Err(PageError::Closed),
            }
        }
        Ok(barrier.committed)
    }
}

/// For a barrier whose own receiver lagged: waits for the watcher to see no pending navigation.
async fn settled_after_lag(page: &PageSession) -> Result<Vec<String>, PageError> {
    Ok(page.navigation_settled().await?.into_iter().collect())
}

/// A geometry read that failed: `NoQuads` (Playwright reads it as not visible), unless the
/// session is gone.
fn no_geometry(error: &PageError) -> Result<Geometry, PageError> {
    match error {
        PageError::Closed => Err(PageError::Closed),
        _ => Ok(Geometry::NoQuads),
    }
}

/// A CDP `DOM.Quad`, which is always eight numbers.
fn quad(points: Vec<f64>) -> Result<[f64; 8], PageError> {
    <[f64; 8]>::try_from(points)
        .map_err(|points| PageError::Contract(format!("a DOM quad of {} numbers", points.len())))
}

/// A viewport dimension in CSS pixels.
fn pixels(value: i64) -> f64 {
    i32::try_from(value).map_or(f64::MAX, f64::from)
}

// ---------------------------------------------------------------------------------------------
// The mouse writer.
// ---------------------------------------------------------------------------------------------

/// The writer task. It owns every mouse event of one attempt and the ticket that records them,
/// and holds the page (so the session) until the last event is acknowledged or failed:
/// cancellation can stop a caller's wait, never split a press from its release.
///
/// Without `delay`: every event from [`mouse_events`] is sent at once with `send_at_epoch` and
/// awaited together (`join_all` polls them in order, and browseros-cdp's FIFO send lock writes
/// them in that order: Playwright's pipelined move, press, release). With `delay`: move, then
/// per click press, sleep `delay` (ended early by `cut_short`), release, and the same sleep
/// between clicks; after `cut_short` no further click starts. A release is sent after every
/// press that may have been written, whatever happened in between.
///
/// Report: every written event acknowledged → `ticket.acked()`; every event refused with
/// `ConnectionEpochChanged` (raised before writing) → `ticket.unsent()`; otherwise the ticket is
/// dropped (`Unknown`) and the first error is reported.
async fn write_mouse(
    page: Arc<PageSession>,
    point: Point,
    click: MouseClick,
    ticket: InputTicket,
    cut_short: CancellationToken,
) -> PressReport {
    let events = mouse_events(&point, &click);
    let outcomes = match click.delay {
        None => future::join_all(events.iter().map(|event| send_input(&page, event))).await,
        Some(delay) => write_with_delay(&page, events, delay, &cut_short).await,
    };
    let first_error = outcomes.iter().find_map(|outcome| outcome.as_ref().err());
    match first_error {
        None => PressReport::Acked(ticket.acked()),
        Some(_)
            if outcomes
                .iter()
                .all(|outcome| matches!(outcome, Err(CdpError::ConnectionEpochChanged { .. }))) =>
        {
            ticket.unsent();
            PressReport::NothingWritten
        }
        Some(error) => {
            drop(ticket);
            PressReport::Failed {
                closed: classify(error) == PageError::Closed,
                message: error.to_string(),
            }
        }
    }
}

/// Playwright's sequential path (`input.ts` `Mouse.click` with `delay`).
async fn write_with_delay(
    page: &PageSession,
    events: Vec<DispatchMouseEventParams>,
    delay: Duration,
    cut_short: &CancellationToken,
) -> Vec<Result<(), CdpError>> {
    let mut outcomes = Vec::with_capacity(events.len());
    let mut events = events.into_iter();
    if let Some(moved) = events.next() {
        outcomes.push(send_input(page, &moved).await);
    }
    while let Some(pressed) = events.next() {
        if cut_short.is_cancelled() || outcomes.iter().any(Result::is_err) {
            break;
        }
        let written = send_input(page, &pressed).await;
        let refused = matches!(written, Err(CdpError::ConnectionEpochChanged { .. }));
        outcomes.push(written);
        if refused {
            break;
        }
        pause(delay, cut_short).await;
        if let Some(released) = events.next() {
            outcomes.push(send_input(page, &released).await);
        }
        if !events.as_slice().is_empty() {
            pause(delay, cut_short).await;
        }
    }
    outcomes
}

/// A `delay` sleep that the call stopping ends early.
async fn pause(delay: Duration, cut_short: &CancellationToken) {
    tokio::select! {
        () = time::sleep(delay) => {}
        () = cut_short.cancelled() => {}
    }
}

/// One mouse event on the session's own socket incarnation.
async fn send_input(page: &PageSession, event: &DispatchMouseEventParams) -> Result<(), CdpError> {
    let params = serde_json::to_value(event)?;
    page.session
        .cdp
        .send_at_epoch(
            page.session.epoch,
            "Input.dispatchMouseEvent",
            params,
            Some(&page.session.id),
        )
        .await
        .map(|_| ())
}

/// Playwright's exact events for a click (`input.ts` `Mouse.click`, `crInput.ts`):
///
/// | type          | button   | buttons | clickCount | force |
/// |---------------|----------|---------|------------|-------|
/// | mouseMoved    | `none`   | 0       | —          | 0     |
/// | mousePressed  | button   | mask    | 1..=n      | 0.5   |
/// | mouseReleased | button   | 0       | 1..=n      | —     |
///
/// all with `x`, `y` and `modifiers: 0`; one press and release pair per click count. Pure.
pub(crate) fn mouse_events(point: &Point, click: &MouseClick) -> Vec<DispatchMouseEventParams> {
    let (name, mask) = match click.button {
        MouseButton::Left => ("left", 1),
        MouseButton::Right => ("right", 2),
        MouseButton::Middle => ("middle", 4),
    };
    let event = |kind: &str, button: &str, buttons: i64| DispatchMouseEventParams {
        r#type: kind.to_string(),
        x: point.x,
        y: point.y,
        modifiers: Some(0),
        button: Some(button.to_string()),
        buttons: Some(buttons),
        ..DispatchMouseEventParams::default()
    };
    let mut events = vec![DispatchMouseEventParams {
        force: Some(0.0),
        ..event("mouseMoved", "none", 0)
    }];
    for count in 1..=click.click_count {
        events.push(DispatchMouseEventParams {
            click_count: Some(i64::from(count)),
            force: Some(0.5),
            ..event("mousePressed", name, mask)
        });
        events.push(DispatchMouseEventParams {
            click_count: Some(i64::from(count)),
            ..event("mouseReleased", name, 0)
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "S1";
    const MAIN: &str = "MAIN";

    fn event(method: &str, session: Option<&str>, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: session.map(SessionId::from),
        }
    }

    fn applied(state: &mut PageState, event: &CdpEvent) -> Applied {
        apply(state, event, &SessionId::from(SESSION), MAIN)
    }

    /// The `ok` golden's three recorded `Input.dispatchMouseEvent` params, verbatim.
    #[test]
    fn mouse_events_are_the_ok_goldens() {
        let click = MouseClick {
            button: MouseButton::Left,
            click_count: 1,
            delay: None,
        };
        let sent: Vec<Value> = mouse_events(&Point { x: 36.75, y: 18.5 }, &click)
            .iter()
            .map(|event| serde_json::to_value(event).expect("serializes"))
            .collect();
        assert_eq!(
            sent,
            [
                json!({ "type": "mouseMoved", "button": "none", "buttons": 0, "x": 36.75,
                        "y": 18.5, "modifiers": 0, "force": 0.0 }),
                json!({ "type": "mousePressed", "button": "left", "buttons": 1, "x": 36.75,
                        "y": 18.5, "modifiers": 0, "clickCount": 1, "force": 0.5 }),
                json!({ "type": "mouseReleased", "button": "left", "buttons": 0, "x": 36.75,
                        "y": 18.5, "modifiers": 0, "clickCount": 1 }),
            ]
        );
    }

    /// An event's type, button, buttons mask and click count.
    type EventSummary = (String, Option<String>, Option<i64>, Option<i64>);

    #[test]
    fn every_click_count_presses_and_releases_with_its_button_mask() {
        for (button, name, mask) in [
            (MouseButton::Right, "right", 2),
            (MouseButton::Middle, "middle", 4),
        ] {
            let click = MouseClick {
                button,
                click_count: 3,
                delay: None,
            };
            let events = mouse_events(&Point { x: 1.0, y: 2.0 }, &click);
            let summary: Vec<EventSummary> = events
                .into_iter()
                .map(|event| (event.r#type, event.button, event.buttons, event.click_count))
                .collect();
            let pair = |count| {
                [
                    (
                        "mousePressed".to_string(),
                        Some(name.to_string()),
                        Some(mask),
                        Some(count),
                    ),
                    (
                        "mouseReleased".to_string(),
                        Some(name.to_string()),
                        Some(0),
                        Some(count),
                    ),
                ]
            };
            let mut expected = vec![(
                "mouseMoved".to_string(),
                Some("none".to_string()),
                Some(0),
                None,
            )];
            for count in 1..=3 {
                expected.extend(pair(count));
            }
            assert_eq!(summary, expected, "{name}");
        }
    }

    #[test]
    fn requested_main_frame_navigations_in_this_tab_are_pending_until_they_commit() {
        let mut state = PageState::default();
        let requested = |frame: &str, disposition: &str| {
            event(
                "Page.frameRequestedNavigation",
                Some(SESSION),
                json!({ "frameId": frame, "reason": "anchorClick", "url": "https://a.test/next",
                        "disposition": disposition }),
            )
        };
        assert!(!applied(&mut state, &requested("CHILD", "currentTab")).changed);
        assert!(!applied(&mut state, &requested(MAIN, "newTab")).changed);
        let other_session = event(
            "Page.frameRequestedNavigation",
            Some("S2"),
            json!({ "frameId": MAIN, "url": "x", "disposition": "currentTab" }),
        );
        assert!(!applied(&mut state, &other_session).changed);
        assert!(applied(&mut state, &requested(MAIN, "currentTab")).changed);
        assert_eq!(state.pending_url.as_deref(), Some("https://a.test/next"));
        assert_eq!(state.navigations_requested, 1);

        let navigated = event(
            "Page.frameNavigated",
            Some(SESSION),
            json!({ "frame": { "id": MAIN, "loaderId": "L", "url": "https://a.test/next",
                               "urlFragment": "#top", "securityOrigin": "", "mimeType": "" },
                    "type": "Navigation" }),
        );
        assert!(applied(&mut state, &navigated).changed);
        assert_eq!(
            (
                state.pending_url.as_deref(),
                state.committed_url.as_deref(),
                state.commits
            ),
            (None, Some("https://a.test/next#top"), 1)
        );
    }

    /// The `ok` golden's own events after its click: a fragment navigation raises no request,
    /// only a commit within the document.
    #[test]
    fn a_fragment_navigation_commits_within_the_document() {
        let mut state = PageState::default();
        for (method, params) in [
            (
                "Page.frameScheduledNavigation",
                json!({ "frameId": MAIN, "delay": 0,
                "reason": "scriptInitiated", "url": "http://127.0.0.1/ok.html#done" }),
            ),
            ("Page.frameStartedLoading", json!({ "frameId": MAIN })),
            (
                "Page.navigatedWithinDocument",
                json!({ "frameId": MAIN,
                "url": "http://127.0.0.1/ok.html#done", "navigationType": "fragment" }),
            ),
            ("Page.frameStoppedLoading", json!({ "frameId": MAIN })),
            (
                "Page.frameClearedScheduledNavigation",
                json!({ "frameId": MAIN }),
            ),
        ] {
            applied(&mut state, &event(method, Some(SESSION), params));
        }
        assert_eq!(state.navigations_requested, 0);
        assert_eq!(state.commits, 1);
        assert_eq!(
            state.committed_url.as_deref(),
            Some("http://127.0.0.1/ok.html#done")
        );
    }

    #[test]
    fn a_stopped_load_ends_a_pending_navigation() {
        let mut state = PageState {
            pending_url: Some("https://a.test/204".to_string()),
            ..PageState::default()
        };
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({ "frameId": MAIN }),
        );
        assert!(applied(&mut state, &stopped).changed);
        assert_eq!(state.pending_url, None);
        assert_eq!(state.commits, 0);
    }

    #[test]
    fn dialogs_are_answered_as_playwright_does_without_a_listener() {
        let mut state = PageState::default();
        let opening = |kind: &str| {
            event(
                "Page.javascriptDialogOpening",
                Some(SESSION),
                json!({ "url": "", "message": "x", "type": kind, "hasBrowserHandler": false }),
            )
        };
        assert_eq!(
            applied(&mut state, &opening("alert")).dialog,
            Some(DialogAnswer::Dismiss)
        );
        assert_eq!(
            applied(&mut state, &opening("beforeunload")).dialog,
            Some(DialogAnswer::Accept)
        );
        assert_eq!(state.dialogs_opened, 2);
    }

    #[test]
    fn the_session_closes_on_its_own_detach_only() {
        let mut state = PageState::default();
        let detached = |session: &str| {
            event(
                "Target.detachedFromTarget",
                None,
                json!({ "sessionId": session, "targetId": MAIN }),
            )
        };
        assert!(!applied(&mut state, &detached("S2")).closed);
        assert!(applied(&mut state, &detached(SESSION)).closed);
        let crashed = event("Inspector.targetCrashed", Some(SESSION), json!({}));
        assert!(applied(&mut state, &crashed).closed);
    }

    #[test]
    fn the_barrier_releases_every_waiter_on_one_commit() {
        let session = SessionId::from(SESSION);
        let mut barrier = Barrier::default();
        let requested = event(
            "Page.frameRequestedNavigation",
            Some(SESSION),
            json!({ "frameId": MAIN, "url": "https://a.test/b", "disposition": "currentTab" }),
        );
        barrier.apply(&requested, &session, MAIN);
        barrier.apply(&requested, &session, MAIN);
        assert_eq!(barrier.waiting, 2);
        let navigated = event(
            "Page.frameNavigated",
            Some(SESSION),
            json!({ "frame": { "id": MAIN, "url": "https://a.test/b" } }),
        );
        barrier.apply(&navigated, &session, MAIN);
        assert_eq!(barrier.waiting, 0);
        assert_eq!(barrier.committed, ["https://a.test/b", "https://a.test/b"]);

        barrier.apply(&requested, &session, MAIN);
        let stopped = event(
            "Page.frameStoppedLoading",
            Some(SESSION),
            json!({ "frameId": MAIN }),
        );
        barrier.apply(&stopped, &session, MAIN);
        assert_eq!(
            barrier.waiting, 0,
            "a request that stopped without committing"
        );
    }
}
