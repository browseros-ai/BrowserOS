//! Neo: the beforeAction hook of an action that targets an element (hooks feature 4). The hook
//! fires once Neo has found the element, with that element pinned, so the hook's check and the
//! action hit the same node (hooks design D6). A retry that finds another node asks again.
//!
//! ```text
//!  loop A (`click.rs`), or `attached` (`element_ops.rs`)
//!    resolve(selector, strict) → element
//!    checkpoint (this module), only for a call whose run's hooks want beforeAction
//!      the node pinned before?   same node → its decision stands, no new question
//!                                another node → that pin is released, this one pinned
//!      pin: a handle of the hook realm (`Tab::pin`), which the agent can never reach
//!      beforeAction(event + locator + element)    raced by the call's `Progress`: its own
//!                                timeout, its abort, the run's end, the tab closing
//!      allow → act with the script's options
//!      block / failure → the call fails, nothing sent
//!      replace args → routed as the script's, same command, same element target → act with
//!                     them on this element; the call's log is hidden and its error redacted
//!    act(element, options)   the input lock and the delivery ticket are taken only in here
//! ```
//!
//! Nothing here holds the tab's input lock or a delivery ticket while the hook decides: the
//! checkpoint runs before the action's first `Tab::input`, so a hook's own input, or another
//! call's, goes meanwhile. Pins never touch the page's snapshot refs: pinning adopts the node
//! into the main world and reads its backend id, nothing else.
//!
//! A gate exists only for a call whose run's hooks want its beforeAction (`HookPlan`); every
//! other call's checkpoint is one `Option` check, and mints no pin.

use std::any::Any;
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use super::{ActionHooks, before_failed, failure_detail, trace_line};
use crate::hooks::{ActionDecision, ActionEvent, ElementRef, HookFailure};
use crate::playwright_methods::click::{Resolved, Stop, Tab};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::run::{BeforeHookResult, RunEnd};

/// What a hook's replacement arguments become, type-erased: the dispatcher's route of the
/// element action (`method_router.rs`), which only that action's handler reads, by type.
pub(crate) type ElementOptions = Arc<dyn Any + Send + Sync>;

/// A DOM node's identity across resolutions: the same node gives an equal key, another node a
/// different one. Read in the page (a node's frame and backend node id), never from a preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeKey {
    frame: String,
    backend: i64,
}

impl NodeKey {
    pub(crate) fn new(frame: impl Into<String>, backend: i64) -> Self {
        Self {
            frame: frame.into(),
            backend,
        }
    }
}

/// An element pinned for the run's hooks: the handle they receive, the node it holds, and what
/// releases it. Dropping it releases the handle, so a hook can never reach a pin after its call.
pub(crate) struct PinnedElement {
    reference: ElementRef,
    key: NodeKey,
    _release: Box<dyn Any + Send + Sync>,
}

impl PinnedElement {
    /// A pin of the node `key` names, handed to hooks as `reference`; `release` frees it as it
    /// drops.
    pub(crate) fn new(
        reference: ElementRef,
        key: NodeKey,
        release: Box<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            reference,
            key,
            _release: release,
        }
    }
}

/// How the call path routes an element action's replacement arguments (`Regate` in
/// `method_router.rs`): as the script's own params, on the same command, aimed at the same
/// element; and what they become for the action's handler.
pub(crate) trait ElementRoute: Send + Sync + 'static {
    /// The hook's `args`, routed and admitted as the script's were; or why the call refuses
    /// them, for the host-only trace (it may quote them).
    fn reroute_element(&self, args: Value) -> Result<Rerouted, String>;
}

/// A replacement the call takes: its options, and what it holds against the run's in-flight
/// caps until the call ends.
pub(crate) struct Rerouted {
    pub(crate) options: ElementOptions,
    pub(crate) charge: Option<Box<dyn Any + Send + Sync>>,
}

/// One element action's beforeAction, from its first resolved element to the call's end.
/// Shared by the call's `Progress`, where its handler reaches it, and its reply hooks, which
/// read what it decided.
pub(crate) struct ElementGate {
    hooks: Arc<dyn ActionHooks>,
    /// What the hooks see of the call as the script made it; each question adds its element.
    event: ActionEvent,
    call_id: u64,
    route: Box<dyn ElementRoute>,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    /// The element the last question was about, and its answer.
    pinned: Option<Pinned>,
    /// Set once any attempt was to run with a hook's arguments, and never cleared: the call's
    /// error is redacted even if a later element is allowed with the script's.
    replaced: bool,
    /// Set when the call ended at a question: blocked, a failed hook, or stopped (its timeout,
    /// its abort) while the hook decided. It never acted there, so no afterAction sees it.
    refused: bool,
    /// The in-flight charge of the replacement in use.
    charge: Option<Box<dyn Any + Send + Sync>>,
}

struct Pinned {
    element: PinnedElement,
    decided: Decided,
}

#[derive(Clone)]
enum Decided {
    Allow,
    Replaced {
        args: Value,
        options: ElementOptions,
    },
}

impl Decided {
    fn options(&self) -> Option<ElementOptions> {
        match self {
            Self::Allow => None,
            Self::Replaced { options, .. } => Some(Arc::clone(options)),
        }
    }
}

impl ElementGate {
    /// The gate of call `call_id`, whose hooks see `event` (its locator set for a frame call)
    /// and whose replacement arguments go through `route`.
    pub(crate) fn new(
        hooks: Arc<dyn ActionHooks>,
        event: ActionEvent,
        call_id: u64,
        route: Box<dyn ElementRoute>,
    ) -> Self {
        Self {
            hooks,
            event,
            call_id,
            route,
            state: Mutex::default(),
        }
    }

    /// Whether an attempt of the call was to run with a hook's arguments: its error is redacted.
    pub(crate) fn replaced(&self) -> bool {
        self.lock().is_some_and(|state| state.replaced)
    }

    /// Whether the call ended at a question about its element, before it acted on it.
    pub(crate) fn refused(&self) -> bool {
        self.lock().is_some_and(|state| state.refused)
    }

    /// The element the call acted on last, and the arguments it acted with when a hook replaced
    /// them, for its afterAction.
    pub(crate) fn acted(&self) -> (Option<ElementRef>, Option<Value>) {
        let Some(state) = self.lock() else {
            return (None, None);
        };
        let Some(pinned) = state.pinned.as_ref() else {
            return (None, None);
        };
        let args = match &pinned.decided {
            Decided::Allow => None,
            Decided::Replaced { args, .. } => Some(args.clone()),
        };
        (Some(pinned.element.reference.clone()), args)
    }

    /// The replacement's in-flight charge, taken to be given back before the call replies.
    pub(crate) fn take_charge(&self) -> Option<Box<dyn Any + Send + Sync>> {
        self.lock().and_then(|mut state| state.charge.take())
    }

    /// A poisoned lock reads as a gate that decided nothing.
    fn lock(&self) -> Option<MutexGuard<'_, GateState>> {
        self.state.lock().ok()
    }

    /// The checkpoint for `resolved`: the replacement options the action runs with on it, or
    /// `None` for its script's own. The node pinned before keeps its decision; another node is
    /// pinned and asked about. Every wait races the call's `Progress`.
    async fn checkpoint<T: Tab>(
        &self,
        tab: &T,
        resolved: &Resolved<T::Element>,
        progress: &Progress,
    ) -> Result<Option<ElementOptions>, Stop> {
        let pinned = self.lock().and_then(|state| {
            state
                .pinned
                .as_ref()
                .map(|pinned| pinned.element.key.clone())
        });
        if let Some(pinned) = pinned {
            let key = progress.race(tab.node_key(&resolved.element)).await??;
            if key == pinned {
                return Ok(self.lock().and_then(|state| {
                    state
                        .pinned
                        .as_ref()
                        .and_then(|pinned| pinned.decided.options())
                }));
            }
            // Another node: its predecessor's pin goes before this one is minted.
            let released = self.lock().and_then(|mut state| state.pinned.take());
            drop(released);
        }
        let hook_handles = &progress.run_scope().hook_handles;
        let element = progress
            .race(tab.pin(&resolved.element, &resolved.preview, hook_handles))
            .await??;
        let mut event = self.event.clone();
        event.element = Some(element.reference.clone());
        // Dropping this future (any of the race's stops) abandons the invocation. A call that
        // stops while its hook decides never acts on this element, so, as a call stopped at its
        // gate, no afterAction sees it.
        let decided = match progress.race(self.hooks.before_action(event)).await {
            Ok(decided) => decided,
            Err(stop) => {
                self.refuse();
                return Err(stop.into());
            }
        };
        self.decided(progress, element, decided)
    }

    /// What `decided` means for the call, on `element`; records it on the call's entry.
    fn decided(
        &self,
        progress: &Progress,
        element: PinnedElement,
        decided: Result<ActionDecision, HookFailure>,
    ) -> Result<Option<ElementOptions>, Stop> {
        let (decided, options) = match decided {
            Ok(ActionDecision::Allow) => {
                progress.record_hook_before(BeforeHookResult::Allowed);
                (Decided::Allow, None)
            }
            Ok(ActionDecision::Block(reason)) => {
                progress.record_hook_before(BeforeHookResult::Blocked);
                self.refuse();
                return Err(PlaywrightError::error(format!(
                    "{}: {reason}",
                    super::BLOCKED_BY_HOOK
                ))
                .into());
            }
            Ok(ActionDecision::ReplaceArgs(args)) => {
                // Before anything acts: no line written from here on names the hook's arguments.
                progress.hide_log();
                match self.route.reroute_element(args.clone()) {
                    Ok(Rerouted { options, charge }) => {
                        progress.record_hook_before(BeforeHookResult::ArgsReplaced);
                        if let Some(mut state) = self.lock() {
                            state.replaced = true;
                            // The replacement in use is the one charged; an earlier one goes.
                            drop(mem::replace(&mut state.charge, charge));
                        }
                        let decided = Decided::Replaced {
                            args,
                            options: Arc::clone(&options),
                        };
                        (decided, Some(options))
                    }
                    Err(detail) => {
                        progress.record_hook_before(BeforeHookResult::Failed);
                        self.trace(&format!("replaced arguments: {detail}"));
                        self.refuse();
                        return Err(before_failed().into());
                    }
                }
            }
            // The run is done with its hooks: the call stops as one the run ended, with no
            // reply, as a call meeting the closed hooks at its gate does.
            Err(_) if self.hooks.ended() => {
                return Err(CallError::RunEnded(RunEnd::Settled).into());
            }
            Err(failure) => {
                progress.record_hook_before(BeforeHookResult::Failed);
                self.trace(&failure_detail(&failure));
                self.refuse();
                return Err(before_failed().into());
            }
        };
        if let Some(mut state) = self.lock() {
            state.pinned = Some(Pinned { element, decided });
        }
        Ok(options)
    }

    /// The call ends at its decision: it never acts.
    fn refuse(&self) {
        if let Some(mut state) = self.lock() {
            state.refused = true;
        }
    }

    /// The decision's options are not the type the action's handler reads: a route and a
    /// handler that disagree. Fails the call closed.
    fn misfit(&self, progress: &Progress) -> Stop {
        progress.record_hook_before(BeforeHookResult::Failed);
        self.trace("replaced arguments: they do not fit the action's handler");
        self.refuse();
        before_failed().into()
    }

    /// One host-only trace line about this call's beforeAction.
    fn trace(&self, detail: &str) {
        self.hooks.add_failure(trace_line(
            "beforeAction",
            self.call_id,
            &self.event.method,
            detail,
        ));
    }
}

/// The element checkpoint of loop A and of `attached`: for a call with an element gate, the
/// options its handler acts with on `resolved` (hooks feature 4), `None` for the script's own;
/// for any other call, `None` at once. `O` is the handler's options type, the type its route
/// carries.
///
/// # Errors
///
/// The call's stop: blocked or a failed hook (an `Error`, nothing sent), the call's timeout,
/// abort or the run's end while the hook decides, or a page step's error (`Stop::Retry` when
/// the element went away, so loop A resolves again).
pub(crate) async fn checkpoint<T: Tab, O: Send + Sync + 'static>(
    tab: &T,
    resolved: &Resolved<T::Element>,
    progress: &Progress,
) -> Result<Option<Arc<O>>, Stop> {
    let Some(gate) = progress.element_gate() else {
        return Ok(None);
    };
    let Some(options) = gate.checkpoint(tab, resolved, progress).await? else {
        return Ok(None);
    };
    options
        .downcast::<O>()
        .map(Some)
        .map_err(|_| gate.misfit(progress))
}
