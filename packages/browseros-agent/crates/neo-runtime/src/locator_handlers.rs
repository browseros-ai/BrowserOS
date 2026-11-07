//! Neo: `Page.addLocatorHandler` (Playwright's `server/page.ts` locator handlers): each tab's
//! registered handlers, the checkpoint every action attempt runs before it takes the tab's
//! input turn (`performActionPreChecks`, through click's `Tab::locator_handlers_checkpoint`),
//! and the three page commands. The handler itself runs in the realm: the checkpoint emits
//! `locatorHandlerTriggered` on the page's guid and waits for the realm's
//! `resolveLocatorHandlerNoReply`.
//!
//! ```text
//!  Page.registerLocatorHandler {selector, noWaitAfter?}  → {uid}, the tab's next
//!  an action attempt, unless a handler runs or waits:    each handler in registration order:
//!    visible (strict, main frame) → it waits; emit locatorHandlerTriggered {uid}
//!    a waiting one: wait for its resolve, then (unless noWaitAfter) its selector hidden
//!  Page.resolveLocatorHandlerNoReply {uid, remove?}      → the wait ends; `remove` drops it
//!  Page.unregisterLocatorHandler {uid}                   → the handler goes
//! ```

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use neo_runtime_spec::serve::{
    FrameWaitForSelectorParams, FrameWaitForSelectorParamsState as SelectorState,
};
use tokio::sync::watch;

use crate::click::{SelectorFacts, Tab};
use crate::frames::FrameScope;
use crate::page::FrameTab;
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::states::{self, StateOptions};
use crate::waits::{self, WaitForSelectorOptions};

/// Neo: one tab's handlers, in registration order, and how many checkpoints are running a
/// handler right now.
#[derive(Default)]
pub(crate) struct LocatorHandlers {
    registry: Mutex<Registry>,
    running: AtomicUsize,
}

#[derive(Default)]
struct Registry {
    last_uid: i64,
    handlers: BTreeMap<i64, Handler>,
}

/// One handler: Playwright's `{ selector, noWaitAfter, resolved }`. `resolved` is set while the
/// realm runs the handler; its receiver sees `true` once the realm resolves it.
struct Handler {
    selector: String,
    no_wait_after: bool,
    resolved: Option<watch::Sender<bool>>,
}

impl LocatorHandlers {
    /// Serves `Page.registerLocatorHandler`.
    ///
    /// Playwright: `server/page.ts` `Page.registerLocatorHandler`: `const uid = ++this._lastLocatorHandlerUid;`
    pub(crate) fn register(&self, selector: String, no_wait_after: bool) -> Result<i64, CallError> {
        self.update(|registry| {
            registry.last_uid += 1;
            let uid = registry.last_uid;
            registry.handlers.insert(
                uid,
                Handler {
                    selector,
                    no_wait_after,
                    resolved: None,
                },
            );
            uid
        })
    }

    /// Serves `Page.resolveLocatorHandlerNoReply`: the waiting checkpoint goes on.
    ///
    /// Playwright: `server/page.ts` `Page.resolveLocatorHandler`: `handler.resolved?.resolve();`
    pub(crate) fn resolve(&self, uid: i64, remove: bool) -> Result<(), CallError> {
        self.update(|registry| {
            let resolved = if remove {
                registry
                    .handlers
                    .remove(&uid)
                    .and_then(|handler| handler.resolved)
            } else {
                registry
                    .handlers
                    .get_mut(&uid)
                    .and_then(|handler| handler.resolved.take())
            };
            if let Some(resolved) = resolved {
                resolved.send_replace(true);
            }
        })
    }

    /// Serves `Page.unregisterLocatorHandler`.
    ///
    /// Playwright: `server/page.ts` `Page.unregisterLocatorHandler`: `this._locatorHandlers.delete(uid);`
    pub(crate) fn unregister(&self, uid: i64) -> Result<(), CallError> {
        self.update(|registry| {
            registry.handlers.remove(&uid);
        })
    }

    fn update<R>(&self, change: impl FnOnce(&mut Registry) -> R) -> Result<R, CallError> {
        let mut registry = self.registry.lock().map_err(|_| {
            CallError::from(PlaywrightError::error(
                "neo-runtime: the tab's locator handlers cannot be read",
            ))
        })?;
        Ok(change(&mut registry))
    }
}

/// Neo: a checkpoint that runs a handler, counted while it does.
struct Running<'a>(&'a AtomicUsize);

impl<'a> Running<'a> {
    fn enter(running: &'a AtomicUsize) -> Self {
        running.fetch_add(1, Ordering::SeqCst);
        Self(running)
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Neo: what the checkpoint reads of one handler.
struct Due {
    uid: i64,
    selector: String,
    no_wait_after: bool,
    waiting: Option<watch::Receiver<bool>>,
}

/// The checkpoint between `performActionPreChecks`' two navigation checks: each handler whose
/// selector is visible in the main frame runs in the realm, and the action waits for it (and,
/// unless `noWaitAfter`, for the selector to hide). An action that a handler itself runs skips
/// it, as does every action while any handler waits.
///
/// Playwright: `server/page.ts` `Page._performLocatorHandlersCheckpoint`: `if (this._locatorHandlerRunningCounter || [...this._locatorHandlers.values()].some(handler => !!handler.resolved))`
pub(crate) async fn checkpoint(tab: &FrameTab, progress: &Progress) -> Result<(), CallError> {
    let handlers = &tab.page.locator_handlers;
    let due = handlers.update(|registry| {
        if registry
            .handlers
            .values()
            .any(|handler| handler.resolved.is_some())
        {
            return Vec::new();
        }
        registry
            .handlers
            .iter()
            .map(|(uid, handler)| Due {
                uid: *uid,
                selector: handler.selector.clone(),
                no_wait_after: handler.no_wait_after,
                waiting: None,
            })
            .collect()
    })?;
    if handlers.running.load(Ordering::SeqCst) > 0 || due.is_empty() {
        return Ok(());
    }
    let main = FrameTab {
        page: Arc::clone(&tab.page),
        scope: FrameScope::Main,
    };
    for mut handler in due {
        let visible = states::is_visible(
            &main,
            &StateOptions::new(handler.selector.clone(), Some(true)),
            progress,
        )
        .await?;
        if visible {
            handler.waiting = trigger(&main, handler.uid)?;
        }
        let Some(waiting) = handler.waiting else {
            continue;
        };
        let _running = Running::enter(&handlers.running);
        let locator = name(&main, &handler.selector, progress).await;
        progress.log(format!(
            "  found {locator}, intercepting action to run the handler"
        ));
        progress.race(resolved(waiting)).await?;
        if handler.no_wait_after {
            progress.log("  locator handler has finished");
        } else {
            progress.log(format!(
                "  locator handler has finished, waiting for {locator} to be hidden"
            ));
            let hidden = WaitForSelectorOptions::from_params(FrameWaitForSelectorParams {
                selector: handler.selector.clone(),
                strict: None,
                state: Some(SelectorState::Hidden),
                omit_return_value: Some(true),
            });
            let _hidden = waits::wait_for_selector_element(&main, &hidden, progress).await?;
        }
        progress.log("  interception handler has finished, continuing");
    }
    Ok(())
}

/// Neo: marks handler `uid` as running and emits `locatorHandlerTriggered` for it, in that
/// order, so a realm that resolves at once finds it running. Nothing waits when no listener
/// would run it (the realm subscribes while it holds a handler) or the handler is gone.
fn trigger(main: &FrameTab, uid: i64) -> Result<Option<watch::Receiver<bool>>, CallError> {
    let handlers = &main.page.locator_handlers;
    let waiting = handlers.update(|registry| {
        let handler = registry.handlers.get_mut(&uid)?;
        let (sender, receiver) = watch::channel(false);
        handler.resolved = Some(sender);
        Some(receiver)
    })?;
    if waiting.is_some() && !main.page.events.locator_handler_triggered(uid) {
        handlers.update(|registry| {
            if let Some(handler) = registry.handlers.get_mut(&uid) {
                handler.resolved = None;
            }
        })?;
        return Ok(None);
    }
    Ok(waiting)
}

/// Neo: the handler's resolve. A handler removed while it runs ends the wait too.
async fn resolved(mut waiting: watch::Receiver<bool>) {
    while !*waiting.borrow() {
        if waiting.changed().await.is_err() {
            return;
        }
    }
}

/// Neo: the handler's locator as Playwright's call log names it, or its selector when the page
/// cannot say.
async fn name(main: &FrameTab, selector: &str, progress: &Progress) -> String {
    match progress.race(Tab::describe(main, selector)).await {
        Ok(Ok(SelectorFacts { locator, .. })) => locator,
        _ => selector.to_string(),
    }
}

#[cfg(test)]
mod tests {
    //! The registry; the checkpoint is covered by the real-browser tests.

    use super::*;

    #[test]
    fn tests_that_handlers_register_in_order_and_resolve_or_go() {
        let handlers = LocatorHandlers::default();
        let one = handlers.register("#a".to_string(), false).expect("one");
        let two = handlers.register("#b".to_string(), true).expect("two");
        assert_eq!((one, two), (1, 2));
        let (sender, mut receiver) = watch::channel(false);
        handlers
            .update(|registry| {
                registry.handlers.get_mut(&one).expect("one").resolved = Some(sender);
            })
            .expect("set");
        handlers.resolve(one, false).expect("resolve");
        assert!(*receiver.borrow_and_update());
        handlers.resolve(two, true).expect("remove");
        handlers.unregister(one).expect("unregister");
        assert!(
            handlers
                .update(|registry| registry.handlers.is_empty())
                .expect("read")
        );
    }
}
