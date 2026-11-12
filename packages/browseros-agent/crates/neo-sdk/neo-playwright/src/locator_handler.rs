//! `page.addLocatorHandler`: upstream's locator-handler checkpoint (server/page.ts
//! `performActionPreChecks` at v1.64.1), and the run's registry it reads.
//!
//! ```text
//!  page.addLocatorHandler     register { tab, locator, noWaitAfter } ─▶ uid
//!  an action's retry          checkpoint(page):
//!    (Subject::locate, inner    nothing waits on this tab, no handler of it is running
//!     retries, waitFor polls,   for each handler: its locator visible (strict)?
//!     expect polls)               ─ trigger: queue uid, wake the pull ─ "found …, intercepting"
//!                                 ─ wait until the script resolves it
//!                                 ─ noWaitAfter, or wait for the locator to be hidden
//!  page.locatorHandlerTriggered  the prelude's background pull ─▶ { uid } │ { uid: null }
//!  page.resolveLocatorHandler    the script ran it: the paused action goes on
//! ```
//!
//! The handler is the script's function, so the action cannot run it: it pauses and the run's
//! realm, which hears of the trigger only through its pull (nothing calls into the runtime from
//! Rust), runs it with ops of its own. Those ops reach the checkpoint too, and pass it at once
//! while a handler of their tab runs, which is what keeps a handler's own clicks from
//! deadlocking on it. The checkpoint itself has no clock: each caller races it with its op's
//! timeout, as upstream races it with the action's progress, and reports the timeout as its own.
//! Registrations live in the run's [`crate::RunState`] and end with the run, with the realm that
//! holds their functions.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use browseros_core::TabId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Notify, watch};
use tokio::time;
use ts_rs::TS;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, OpTarget, TARGET};
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read};
use crate::resolve::{self, Landed, Resolver};
use crate::target::{LocatorTarget, Strictness, Target};
use crate::wire::{self, Describe, Nothing, OpWire, Route, Run, options};
use crate::{ErrorCode, Playwright, PlaywrightError, read};

/// How often a checkpoint waiting on a handler looks whether its page closed.
const CLOSE_POLL: Duration = Duration::from_millis(250);

listed_enum! {
    /// The ops behind `page.addLocatorHandler` and `removeLocatorHandler`, and the pull the
    /// prelude runs handlers from (`locator-handlers.ts`).
    pub(crate) enum LocatorHandlerOp {
        Add,
        Remove,
        Triggered,
        Resolve,
    }
}

impl LocatorHandlerOp {
    /// Describes the input and result bound to the runtime handler by its route row.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

    /// This op's registration metadata. None of them touches the page: they read and write
    /// the run's registry, and the checkpoint's own reads run inside the action that pauses.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Add => OpSpec::read(
                "page.addLocatorHandler",
                TARGET,
                "register a handler: an action that finds its locator visible waits while it runs",
            ),
            Self::Remove => OpSpec::read(
                "page.removeLocatorHandler",
                OpTarget::None,
                "unregister one locator handler",
            ),
            Self::Triggered => OpSpec::read(
                "page.locatorHandlerTriggered",
                OpTarget::None,
                "the prelude's pull: a long poll for the next locator handler an action paused for",
            )
            .background(),
            Self::Resolve => OpSpec::read(
                "page.resolveLocatorHandler",
                OpTarget::None,
                "the script ran a locator handler: the action it paused goes on",
            ),
        }
    }
}

/// The registry protocol has one route for execution and contract generation; the checkpoint
/// and its run-scoped lifetime remain independent of this wire description.
fn route<'a, R: Route<'a>>(op: LocatorHandlerOp, r: R) -> R::Out {
    match op {
        LocatorHandlerOp::Add => r.op(wire::command("Page.registerLocatorHandler"), add),
        LocatorHandlerOp::Remove => r.op(wire::command("Page.unregisterLocatorHandler"), remove),
        LocatorHandlerOp::Triggered => r.op(
            wire::neo_only(
                "Neo background pull replacing upstream locatorHandlerTriggered events.",
            ),
            triggered,
        ),
        LocatorHandlerOp::Resolve => {
            r.op(wire::command("Page.resolveLocatorHandlerNoReply"), resolve)
        }
    }
}

/// `page.addLocatorHandler(locator, handler, { noWaitAfter })`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(rename = "LocatorHandlerAddInput")]
struct AddInput {
    target: Target,
    // The prelude omits false; keep serde's defaulted bool so explicit null still fails.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    no_wait_after: bool,
}
options!(AddInput);

/// One registration, by the uid `page.addLocatorHandler` answered.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorHandlerRemoveInput")]
struct RemoveInput {
    uid: u64,
}
options!(RemoveInput);

/// The run's background pull carries no payload.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorHandlerTriggeredInput")]
struct TriggeredInput {}
options!(TriggeredInput);

/// `remove` once the handler's `times` ran out, as upstream's `resolveLocatorHandlerNoReply`.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "LocatorHandlerResolveInput")]
struct ResolveInput {
    uid: u64,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    remove: bool,
}
options!(ResolveInput);

/// The registration id retained by the prelude for subsequent pulls and cleanup.
#[derive(Debug, Serialize, TS)]
struct LocatorHandlerAdded {
    uid: u64,
}

/// A trigger to run, or an explicit null when the run has no handler left.
#[derive(Debug, Serialize, TS)]
struct LocatorHandlerTriggered {
    uid: Option<u64>,
}

/// Runs one registry op through the same row that describes its generated wire contract.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: LocatorHandlerOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

async fn add(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    input: AddInput,
) -> Result<LocatorHandlerAdded, PlaywrightError> {
    let Target::Locator(target) = input.target else {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "target: a locator handler takes a locator, not an element handle",
        ));
    };
    let uid = scope
        .run_state()
        .locator_handlers
        .register(target, input.no_wait_after);
    Ok(LocatorHandlerAdded { uid })
}

async fn remove(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    input: RemoveInput,
) -> Result<Nothing, PlaywrightError> {
    scope.run_state().locator_handlers.unregister(input.uid);
    Ok(())
}

async fn triggered(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    _input: TriggeredInput,
) -> Result<LocatorHandlerTriggered, PlaywrightError> {
    let uid = scope.run_state().locator_handlers.next_trigger().await;
    Ok(LocatorHandlerTriggered { uid })
}

async fn resolve(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    input: ResolveInput,
) -> Result<Nothing, PlaywrightError> {
    scope
        .run_state()
        .locator_handlers
        .resolve(input.uid, input.remove);
    Ok(())
}

/// Whether an action runs the checkpoint before its attempts, as upstream decides per action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prechecks {
    /// Before every pass and retry, on a locator or a handle: the actions upstream runs through
    /// `_retryAction` (pointer actions, fill, selectOption, selectText, scrollIntoViewIfNeeded).
    Run,
    /// Before a locator's passes only (`_retryWithProgressIfNotConnected`): the handle forms of
    /// focus, blur, press, type and setInputFiles have no retry loop, and run no handler.
    LocatorPasses,
    /// Never: under `force`, for `dispatchEvent`, and for `dragTo`'s drop half, which must not
    /// move the mouse between press and release.
    Skip,
}

impl Prechecks {
    /// [`Prechecks::Skip`] when `skip`.
    pub(crate) const fn unless(skip: bool) -> Self {
        if skip { Self::Skip } else { Self::Run }
    }
}

/// Upstream's `_performLocatorHandlersCheckpoint` for an action on `page`: each handler of the
/// tab whose locator is visible runs, in registration order, before the action goes on. It
/// passes at once while any of the tab's handlers runs, or another checkpoint of the tab waits,
/// so a handler's own actions never wait on it. Unbounded: the caller races it with its op's
/// timeout, and a handler still running when that fires keeps running.
pub(crate) async fn checkpoint(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
) -> Result<(), PlaywrightError> {
    let handlers = &scope.run_state().locator_handlers;
    let tab = page.tab();
    let Some(ready) = handlers.ready_on(tab) else {
        return Ok(());
    };
    for handler in ready {
        if !handlers.is_running(handler.uid) && !visible(scope, page, &handler.target).await? {
            continue;
        }
        // Gone: the script removed it while the check ran.
        let Some(mut resolved) = handlers.trigger(handler.uid) else {
            continue;
        };
        let _waiting = handlers.wait_on(tab);
        let display = &handler.target.display;
        scope.log(format!(
            "  found {display}, intercepting action to run the handler"
        ));
        let handled = async {
            // Ends when the sender goes: the script resolved or removed the handler, or the
            // run ended. Nothing is ever sent on it.
            let _gone = resolved.changed().await;
            if handler.no_wait_after {
                scope.log("  locator handler has finished".to_owned());
            } else {
                scope.log(format!(
                    "  locator handler has finished, waiting for {display} to be hidden"
                ));
                // Boxed: the wait's polls are the ones `locator.waitFor` runs, which
                // themselves run this checkpoint first.
                Box::pin(read::wait_hidden(scope, page, &handler.target)).await?;
            }
            Ok::<(), PlaywrightError>(())
        };
        // Upstream races both waits with the page's scope, so a page closed meanwhile fails
        // the action at once.
        tokio::select! {
            biased;
            handled = handled => handled?,
            closed = closed(scope, page) => return Err(closed),
        }
        // A closed tab reads as "nothing there", so the hidden wait passes at once; a handler
        // that closed its own page, or a close the poll had not seen yet, would leave the
        // action looking for its element in a tab that is gone.
        if !still_open(scope, page).await? {
            return Err(closed_error(scope));
        }
        scope.log("  interception handler has finished, continuing".to_owned());
    }
    Ok(())
}

/// The error once `page`'s tab closed under the action, while a handler of it runs. Polled:
/// core's tab registry offers no close signal an op can wait on, and a lookup is a map read.
async fn closed(scope: &OpScope<'_>, page: &OpPage<'_>) -> PlaywrightError {
    loop {
        match still_open(scope, page).await {
            Ok(true) => time::sleep(CLOSE_POLL).await,
            Ok(false) => return closed_error(scope),
            Err(error) => return error,
        }
    }
}

/// Upstream's error for a call on a page that closed.
fn closed_error(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(
        ErrorCode::TargetClosed,
        "Target page, context or browser has been closed",
    )
}

/// Whether `page`'s tab is still open, by core's tab registry, which `page.close` updates before
/// it answers; a miss is listed once, as admission does.
async fn still_open(scope: &OpScope<'_>, page: &OpPage<'_>) -> Result<bool, PlaywrightError> {
    let pages = &page.browser().pages;
    if pages.get_info_by_tab(page.tab()).await.is_some() {
        return Ok(true);
    }
    scope
        .view()
        .resolve_tab_ids(&[page.tab()])
        .await
        .map_err(|error| scope.core_error(&error))?;
    Ok(pages.get_info_by_tab(page.tab()).await.is_some())
}

/// Upstream's `isVisibleInternal` with `strict: true`: a second match fails the action, as an
/// error thrown in the page, and anything that only means "not there now" reads as hidden.
async fn visible(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    target: &LocatorTarget,
) -> Result<bool, PlaywrightError> {
    let resolver = Resolver::new(page, scope, target, Strictness::Strict);
    let landed = resolver
        .attempt_at(|last, frame_visible| ProbeRequest::Read {
            target: ProbeTarget::Chunk(last),
            strictness: Strictness::Strict,
            frame_visible,
            read: Read::State {
                state: QueryState::Visible,
            },
        })
        .await;
    match landed {
        Ok(Some(Landed {
            reply: ProbeReply::Read { value, .. },
            ..
        })) => Ok(value.into_value().as_bool() == Some(true)),
        Ok(
            None
            | Some(Landed {
                reply: ProbeReply::None { .. } | ProbeReply::Detached,
                ..
            }),
        ) => Ok(false),
        Ok(Some(Landed { reply, .. })) => Err(resolve::refused(scope, reply)),
        Err(error) if error.error_code() == Some(ErrorCode::ElementDetached) => Ok(false),
        Err(error) => Err(error),
    }
}

/// One run's locator handlers, behind `RunState`. A std mutex, held for a map update and never
/// across an await.
#[derive(Debug, Default)]
pub(crate) struct LocatorHandlers {
    inner: Mutex<Inner>,
    /// Wakes the pull: a trigger was queued, or the run's last handler went.
    changed: Notify,
}

#[derive(Debug, Default)]
struct Inner {
    last_uid: u64,
    /// In uid order, which is registration order, as upstream's `Map` iterates.
    handlers: BTreeMap<u64, Handler>,
    /// Checkpoints waiting on a handler, per tab: upstream's `_locatorHandlerRunningCounter`.
    waiting: HashMap<TabId, usize>,
    /// Triggers the pull has not answered yet.
    triggered: VecDeque<u64>,
}

#[derive(Debug)]
struct Handler {
    tab: TabId,
    target: Arc<LocatorTarget>,
    no_wait_after: bool,
    /// Set from its trigger until the script resolves it: upstream's `resolved`. Checkpoints
    /// wait for the sender to drop, which also happens when the handler is removed or the run
    /// ends, so none waits on a handler nobody will resolve.
    running: Option<watch::Sender<()>>,
}

/// A handler a checkpoint may run.
pub(crate) struct Ready {
    uid: u64,
    target: Arc<LocatorTarget>,
    no_wait_after: bool,
}

/// One checkpoint waiting on a handler of `tab`, until dropped (its op's timeout included).
/// While it lives, the tab's other checkpoints pass at once: dropped early, a handler's own
/// actions could run the handler again.
#[must_use = "the tab's checkpoints pass only while this guard lives"]
pub(crate) struct Waiting<'a> {
    handlers: &'a LocatorHandlers,
    tab: TabId,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut inner = self.handlers.lock();
        if let Some(count) = inner.waiting.get_mut(&self.tab) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.waiting.remove(&self.tab);
            }
        }
    }
}

impl LocatorHandlers {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers `target`'s handler on its page's tab; the uid names it to the script.
    fn register(&self, target: LocatorTarget, no_wait_after: bool) -> u64 {
        let mut inner = self.lock();
        inner.last_uid += 1;
        let uid = inner.last_uid;
        inner.handlers.insert(
            uid,
            Handler {
                tab: target.page().tab(),
                target: Arc::new(target),
                no_wait_after,
                running: None,
            },
        );
        uid
    }

    /// Forgets `uid`. A checkpoint waiting on it goes on (upstream's waits until its timeout).
    fn unregister(&self, uid: u64) {
        let mut inner = self.lock();
        inner.handlers.remove(&uid);
        inner.triggered.retain(|queued| *queued != uid);
        self.wake_if_empty(&inner);
    }

    /// The script ran `uid`: every checkpoint waiting on it goes on. `remove` forgets it.
    fn resolve(&self, uid: u64, remove: bool) {
        let mut inner = self.lock();
        if remove {
            inner.handlers.remove(&uid);
        } else if let Some(handler) = inner.handlers.get_mut(&uid) {
            handler.running = None;
        }
        self.wake_if_empty(&inner);
    }

    /// The next trigger, or `None` once the run has no handler left. Waits for either.
    async fn next_trigger(&self) -> Option<u64> {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            {
                let mut inner = self.lock();
                if let Some(uid) = inner.triggered.pop_front() {
                    return Some(uid);
                }
                if inner.handlers.is_empty() {
                    return None;
                }
            }
            changed.await;
        }
    }

    /// The tab's handlers, unless one of them runs or a checkpoint of the tab waits on one.
    fn ready_on(&self, tab: TabId) -> Option<Vec<Ready>> {
        let inner = self.lock();
        let mut ready = Vec::new();
        for (uid, handler) in &inner.handlers {
            if handler.tab != tab {
                continue;
            }
            if handler.running.is_some() {
                return None;
            }
            ready.push(Ready {
                uid: *uid,
                target: Arc::clone(&handler.target),
                no_wait_after: handler.no_wait_after,
            });
        }
        let waiting = inner.waiting.get(&tab).copied().unwrap_or_default();
        (waiting == 0 && !ready.is_empty()).then_some(ready)
    }

    /// Whether another checkpoint triggered `uid` since [`LocatorHandlers::ready_on`].
    fn is_running(&self, uid: u64) -> bool {
        self.lock()
            .handlers
            .get(&uid)
            .is_some_and(|handler| handler.running.is_some())
    }

    /// Marks `uid` running and queues it for the pull, unless it already runs; either way a
    /// receiver that sees its end. `None` when the script removed it.
    fn trigger(&self, uid: u64) -> Option<watch::Receiver<()>> {
        let mut inner = self.lock();
        let handler = inner.handlers.get_mut(&uid)?;
        if let Some(running) = &handler.running {
            return Some(running.subscribe());
        }
        let (running, resolved) = watch::channel(());
        handler.running = Some(running);
        inner.triggered.push_back(uid);
        drop(inner);
        self.changed.notify_waiters();
        Some(resolved)
    }

    /// Counts one checkpoint of `tab` waiting on a handler, until the guard drops.
    fn wait_on(&self, tab: TabId) -> Waiting<'_> {
        *self.lock().waiting.entry(tab).or_default() += 1;
        Waiting {
            handlers: self,
            tab,
        }
    }

    fn wake_if_empty(&self, inner: &Inner) {
        if inner.handlers.is_empty() {
            self.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests;
