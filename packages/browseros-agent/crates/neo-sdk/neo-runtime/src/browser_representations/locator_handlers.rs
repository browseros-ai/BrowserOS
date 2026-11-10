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
use std::future::{self, Future};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use neo_playwright_protocol::serve::FrameWaitForSelectorParamsState as SelectorState;
use tokio::sync::watch;

use crate::browser_representations::page::{FrameTab, PageSession};
use crate::chromium_bridge::frames::FrameScope;
use crate::playwright_methods::evaluate::CreatedHandle;
use crate::playwright_methods::states::{self, StateOptions};
use crate::playwright_methods::tab::{SelectorFacts, Tab};
use crate::playwright_methods::waits::{self, WaitForSelectorOptions};
use crate::progress::{CallError, PlaywrightError, Progress};

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
    /// A `resolveLocatorHandlerNoReply` call holds the handler's completion, admitted outside
    /// the dispatcher's in-flight cap ([`begin_resolving`]).
    claimed: bool,
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
                    claimed: false,
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

    /// Neo: claims running handler `uid`'s completion ([`begin_resolving`]): `false` unless the
    /// realm was told to run it, has not resolved it yet, and no other call holds the claim.
    fn claim(&self, uid: i64) -> bool {
        self.update(|registry| {
            registry.handlers.get_mut(&uid).is_some_and(|handler| {
                let claim = handler.resolved.is_some() && !handler.claimed;
                handler.claimed |= claim;
                claim
            })
        })
        .unwrap_or(false)
    }

    /// Neo: gives handler `uid`'s completion claim back.
    fn unclaim(&self, uid: i64) {
        let _unclaimed = self.update(|registry| {
            if let Some(handler) = registry.handlers.get_mut(&uid) {
                handler.claimed = false;
            }
        });
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

/// Neo: a `resolveLocatorHandlerNoReply` call's claim on the running handler it completes, held
/// until the call replies or its task ends ([`begin_resolving`]).
pub(crate) struct Resolving {
    page: Arc<PageSession>,
    uid: i64,
}

/// Neo: claims the completion of `page`'s handler `uid`, for the dispatcher to admit outside its
/// in-flight cap, as it admits a wait's release (`Cursors::begin_ending`): completing a handler
/// frees the action waiting on it and adds no work. `None` unless the handler is running (the
/// realm was told to run it and has not resolved it) and no other call holds the claim, so such
/// calls never outnumber the handlers running.
pub(crate) fn begin_resolving(page: &Arc<PageSession>, uid: i64) -> Option<Resolving> {
    page.locator_handlers.claim(uid).then(|| Resolving {
        page: Arc::clone(page),
        uid,
    })
}

/// Neo: gives the claim back.
impl Drop for Resolving {
    fn drop(&mut self) {
        self.page.locator_handlers.unclaim(self.uid);
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
        let visible = states::is_visible_internal(
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
            // Playwright: `server/page.ts` `Page._performLocatorHandlersCheckpoint`: `await this.mainFrame().waitForSelector(progress, { selector: handler.selector }, false, { state: 'hidden' });`
            let hidden = WaitForSelectorOptions {
                selector: handler.selector.clone(),
                strict: false,
                state: SelectorState::Hidden,
                return_element: false,
                pre_checks: false,
            };
            let _hidden = wait_hidden(&main, &hidden, progress).await?;
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

/// Neo: the wait for a handled selector to hide. Boxed behind a named type: the wait's attempts
/// reach this checkpoint through `WaitTab` (with no pre-checks at run time), so an unboxed
/// future would contain itself.
fn wait_hidden<'a>(
    main: &'a FrameTab,
    hidden: &'a WaitForSelectorOptions,
    progress: &'a Progress,
) -> Pin<Box<dyn Future<Output = Result<Option<CreatedHandle>, CallError>> + Send + 'a>> {
    Box::pin(waits::wait_for_selector_element(main, hidden, progress))
}

/// Neo: the handler's resolve. A handler removed while its callback runs is never resolved, as
/// Playwright's removed handler is not found by its resolve: the action waits on until its
/// deadline. Removal is not the callback's completion.
///
/// Playwright: `server/page.ts` `Page.resolveLocatorHandler`: `handler.resolved?.resolve();`
async fn resolved(mut waiting: watch::Receiver<bool>) {
    loop {
        if *waiting.borrow_and_update() {
            return;
        }
        if waiting.changed().await.is_err() {
            // The handler went: resolved only if its last word was the resolve.
            if *waiting.borrow() {
                return;
            }
            future::pending::<()>().await;
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

    #[test]
    fn tests_that_only_a_running_handler_lends_its_completion_one_claim() {
        let handlers = LocatorHandlers::default();
        let uid = handlers.register("#a".to_string(), true).expect("register");
        assert!(
            !handlers.claim(uid),
            "a handler not running has nothing to complete"
        );
        let (sender, _receiver) = watch::channel(false);
        handlers
            .update(|registry| {
                registry.handlers.get_mut(&uid).expect("uid").resolved = Some(sender);
            })
            .expect("set");
        assert!(
            handlers.claim(uid),
            "a running handler's completion is claimed"
        );
        assert!(!handlers.claim(uid), "one claim at a time");
        handlers.unclaim(uid);
        assert!(handlers.claim(uid), "a claim given back can be taken again");
        handlers.resolve(uid, false).expect("resolve");
        handlers.unclaim(uid);
        assert!(!handlers.claim(uid), "a resolved handler is not running");
        assert!(!handlers.claim(uid + 1), "an unknown handler");
    }
}
