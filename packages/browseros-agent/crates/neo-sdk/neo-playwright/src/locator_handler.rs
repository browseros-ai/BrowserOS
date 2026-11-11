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

use browseros_core::TabId;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{OpSpec, OpTarget, TARGET};
use crate::page::OpPage;
use crate::probe::{ProbeReply, ProbeRequest, ProbeTarget, QueryState, Read};
use crate::read::wait_hidden;
use crate::resolve::{self, Landed, Resolver};
use crate::target::{LocatorTarget, Strictness, Target};
use crate::{ErrorCode, PlaywrightError};

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

/// `page.addLocatorHandler(locator, handler, { noWaitAfter })`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AddInput {
    target: Target,
    #[serde(default)]
    no_wait_after: bool,
}

/// One registration, by the uid `page.addLocatorHandler` answered.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveInput {
    uid: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggeredInput {}

/// `remove` once the handler's `times` ran out, as upstream's `resolveLocatorHandlerNoReply`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveInput {
    uid: u64,
    #[serde(default)]
    remove: bool,
}

/// Runs one locator-handler op.
pub(crate) async fn invoke(
    scope: &OpScope<'_>,
    op: LocatorHandlerOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    let handlers = &scope.run_state().locator_handlers;
    match op {
        LocatorHandlerOp::Add => {
            let AddInput {
                target,
                no_wait_after,
            } = args.decode(scope)?;
            let Target::Locator(target) = target else {
                return Err(scope.fail(
                    ErrorCode::InvalidInput,
                    "target: a locator handler takes a locator, not an element handle",
                ));
            };
            let uid = handlers.register(target, no_wait_after);
            Ok(json!({ "uid": uid }))
        }
        LocatorHandlerOp::Remove => {
            let RemoveInput { uid } = args.decode(scope)?;
            handlers.unregister(uid);
            Ok(json!({}))
        }
        LocatorHandlerOp::Triggered => {
            let TriggeredInput {} = args.decode(scope)?;
            Ok(json!({ "uid": handlers.next_trigger().await }))
        }
        LocatorHandlerOp::Resolve => {
            let ResolveInput { uid, remove } = args.decode(scope)?;
            handlers.resolve(uid, remove);
            Ok(json!({}))
        }
    }
}

/// Whether an action runs the checkpoint before its attempts. Upstream skips it under `force`,
/// and for `dragTo`'s drop half, which must not move the mouse between press and release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prechecks {
    Run,
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
        // Ends when the sender goes: the script resolved or removed the handler, or the run
        // ended. Nothing is ever sent on it.
        let _gone = resolved.changed().await;
        // A handler may close its own page. Upstream's page scope then fails the action; here
        // the action would go on looking for its element in a tab that is gone.
        if !still_open(scope, page).await? {
            return Err(scope.fail(
                ErrorCode::TargetClosed,
                "Target page, context or browser has been closed",
            ));
        }
        if handler.no_wait_after {
            scope.log("  locator handler has finished".to_owned());
        } else {
            scope.log(format!(
                "  locator handler has finished, waiting for {display} to be hidden"
            ));
            // Boxed: the wait's polls are the ones `locator.waitFor` runs, which themselves
            // run this checkpoint first.
            Box::pin(wait_hidden(scope, page, &handler.target)).await?;
        }
        scope.log("  interception handler has finished, continuing".to_owned());
    }
    Ok(())
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
