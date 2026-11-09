//! Neo: the action hooks of actions that target no element (hooks feature 3). Before such a
//! served call runs, the run's hooks may allow it, block it or swap its arguments; after it ran,
//! they may keep its result, swap it, or leave the model a note. The gate sits in
//! `Dispatcher::call`, the one door every browser call takes, so a script that calls
//! `__neo_runtime.call` itself is hooked like one that uses the SDK.
//!
//! ```text
//!  Dispatcher::call  (sync, in call order)
//!    ├─ HookPlan::new        element action (F4's) or no hook wants it → None, nothing else
//!    ├─ ordered turns        beforeAction wants it: taken only once it is decided (below)
//!    └─ HookPlan::admitted   its place in the run's CallOrder, unless the debugger holds it
//!  the call's task
//!    ├─ released()           the debugger's hold, as without hooks
//!    ├─ CallHooks::before    its turn → beforeAction → allow · block · swap args; then its
//!    │                       ordered turns; then the next call's turn. Races the run's
//!    │                       deadline and end, the call's abort and the context's close,
//!    │                       never the call's own timeout, which starts after (`Progress::new`)
//!    ├─ serve_call           a call that stops before its handler starts never ran: no
//!    │                       afterAction (`ReplyHooks::never_ran`)
//!    └─ finish_reply → ReplyHooks::finish   afterAction → keep · replace · note (races the
//!                            run); then no log for a replaced or withheld result, and no
//!                            own error text for a call run with a hook's arguments
//! ```
//!
//! Every failure fails closed (hooks design D8): a beforeAction that throws, times out or decides
//! nothing usable blocks the call, an afterAction that does fails it after it ran. The script
//! gets a generic error; the failure's own text, which may hold a secret, goes to the host-only
//! trace of the run's [`HookLedger`].
//!
//! The hooks themselves (feature 2's hook sandbox) are reached through [`ActionHooks`]. Until that
//! sandbox merges here, the types it publishes are mirrored below under the same names and
//! fields, each marked as a mirror; phase 2 replaces them with `crate::hooks`'s and implements
//! [`ActionHooks`] for its `HookSandbox`.

use std::collections::BTreeSet;
use std::future::Future;
use std::mem;
use std::pin::pin;
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;

use crate::actions::{ResultShapeError, ServedMethod};
use crate::progress::{CallEntry, PlaywrightError, RunScope};
use crate::quickjs_bridge::Logs;
use crate::run::{AfterHookResult, BeforeHookResult, CallLocation, RunBinary, RunError};

#[cfg(test)]
mod tests;

/// Neo: what a script's blocked call says first; the hook's reason follows it.
const BLOCKED_BY_HOOK: &str = "blocked by hook";

/// Neo: the reason a call gives when its beforeAction hook failed. Generic on purpose: the
/// failure's own text may hold a secret and goes to the host-only trace instead.
const BEFORE_ACTION_FAILED: &str = "its beforeAction hook failed";

/// Neo: the error of a call whose afterAction hook failed: the action ran (its delivery says
/// what it sent), and its result is withheld rather than let through unfiltered.
const AFTER_ACTION_FAILED: &str = "a hook failed after the action ran; its result is withheld";

/// Neo: the error text of a call that ran with a hook's arguments and failed. Its own text may
/// quote those arguments (`Unknown permission: <value>`, `… at <url>`), so the script and the
/// call's record get this under the error's own name, and the host-only trace the real text.
const REPLACED_ARGS_FAILED: &str =
    "the action failed with arguments a hook replaced; its message is withheld";

/// The run's action hooks, as the call path calls them. Dyn-compatible, so a run holds them as
/// `Arc<dyn ActionHooks>` in [`RunScope::action_hooks`], set once they are ready.
///
/// Phase 2: feature 2's `HookSandbox` implements it, each method forwarding to its own.
pub(crate) trait ActionHooks: Send + Sync + 'static {
    /// Whether a registration wants `event` for an action of `kind` and `group`. It never enters
    /// the hook realm: a call no hook wants costs only this.
    fn wants(&self, event: ActionHook, kind: Option<&str>, group: Option<&str>) -> bool;

    /// `beforeAction` for `event`. The invocation starts when the future is first polled. The
    /// caller races the run's deadline and the call's abort: dropping the future abandons it.
    fn before_action(
        &self,
        event: ActionEvent,
    ) -> BoxFuture<'static, Result<ActionDecision, HookFailure>>;

    /// `afterAction` for `event`, which ended as `ended`, as [`ActionHooks::before_action`].
    fn after_action(
        &self,
        event: ActionEvent,
        ended: ActionEnd,
    ) -> BoxFuture<'static, Result<AfterDecision, HookFailure>>;
}

/// An action event, before or after the action.
///
/// Mirror of `crate::hooks::ActionHook` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionHook {
    Before,
    After,
}

/// What an action hook's handlers receive about the action. The hook realm builds real
/// Playwright objects from its guids (hooks design D5).
///
/// Mirror of `crate::hooks::ActionEvent` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ActionEvent {
    /// The action's kind (`goto`, `keyboard`, …), feature 1's label.
    pub(crate) kind: Option<&'static str>,
    /// The action's group (`navigate`, `type`, …), feature 1's label.
    pub(crate) group: Option<&'static str>,
    pub(crate) method: String,
    /// The object the call named.
    pub(crate) guid: String,
    /// The params the call runs with: the script's, or the hook's once it replaced them.
    pub(crate) args: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) location: Option<CallLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) frame: Option<String>,
    pub(crate) context: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) locator: Option<LocatorRef>,
    /// The pinned element's handle guid (hooks feature 4); always `None` here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) element: Option<String>,
}

/// The locator a call names: its frame and selector.
///
/// Mirror of `crate::hooks::LocatorRef` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LocatorRef {
    pub(crate) frame: String,
    pub(crate) selector: String,
}

/// How an action ended, for `afterAction`.
///
/// Mirror of `crate::hooks::ActionEnd` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ActionEnd {
    /// The call's reply `result`; `null` for a command that answers none.
    Result(Value),
    Error(RunError),
}

/// What `beforeAction`'s handlers decided.
///
/// Mirror of `crate::hooks::ActionDecision` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "phase 2: the hook sandbox decides; until then only the tests' fake does"
    )
)]
pub(crate) enum ActionDecision {
    Allow,
    Block(String),
    /// New arguments, which the call re-checks like the script's own.
    ReplaceArgs(Value),
}

/// What `afterAction`'s handlers decided.
///
/// Mirror of `crate::hooks::AfterDecision` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AfterDecision {
    /// What replaces the result, when a handler replaced it.
    pub(crate) replace: Option<Replaced>,
    /// Every handler's note, in order.
    pub(crate) notes: Vec<String>,
}

/// A result a hook put in place of the action's own.
///
/// Mirror of `crate::hooks::Replaced` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Replaced {
    /// The replacement as JSON; `None` for `{ replace: undefined }`.
    pub(crate) value: Option<Value>,
    /// Its binaries, lifted out of it; an action's reply cannot carry any.
    pub(crate) binaries: Vec<RunBinary>,
}

/// Why an invocation decided nothing usable. Every one fails closed.
///
/// Mirror of `crate::hooks::HookFailure` (hooks feature 2); replaced by it when that merges.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "phase 2: the hook sandbox fails; until then only the tests' fake does"
    )
)]
pub(crate) enum HookFailure {
    /// A handler threw or rejected; the detail is for the hook trace only.
    Threw(String),
    /// The invocation ran past its budget.
    Timeout,
    /// The decision was unreadable, or not one its event allows.
    BadDecision(String),
    /// The realm cannot answer: the hooks did not load, the run ended, or nobody waits.
    Realm(String),
}

impl HookFailure {
    /// The failure as the host-only trace words it.
    fn detail(&self) -> String {
        match self {
            Self::Threw(detail) => format!("threw: {detail}"),
            Self::Timeout => "ran past its budget".to_string(),
            Self::BadDecision(detail) => format!("decided nothing usable: {detail}"),
            Self::Realm(detail) => format!("could not run: {detail}"),
        }
    }
}

/// What a run's action hooks left: notes for the model (the hooks design keeps them out of
/// every method's return value) and trace lines for the host only, among them why a hook
/// failed, whose text may hold a secret. Each is capped as a run's `console` lines are.
///
/// Phase 2: the run moves them into feature 2's `RunOutcome::notes` and `hook_trace`.
#[derive(Default)]
pub(crate) struct HookLedger {
    notes: Logs,
    trace: Logs,
}

impl HookLedger {
    /// Adds one of a hook's notes for the model.
    pub(crate) fn note(&self, note: String) {
        self.notes.push(note);
    }

    /// Adds one line to the host-only trace.
    pub(crate) fn trace(&self, line: String) {
        self.trace.push(line);
    }

    /// The notes so far, in order, and forgets them.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "phase 2 moves the notes into the run's outcome (feature 2's `notes`)"
        )
    )]
    pub(crate) fn take_notes(&self) -> Vec<String> {
        self.notes.take()
    }

    /// The trace so far, in order, and forgets it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "phase 2 moves the trace into the run's outcome (feature 2's `hook_trace`)"
        )
    )]
    pub(crate) fn take_trace(&self) -> Vec<String> {
        self.trace.take()
    }
}

/// The run's `beforeAction` invocations, one at a time in call order (hooks design:
/// invocations queue in call order). A call takes its [`Place`] synchronously as it arrives in
/// `Dispatcher::call`, so the order is the script's however the call tasks interleave; its task
/// waits until every earlier place still live is gone, invokes the hook, and gives its place up
/// once it has the decision and its ordered turns.
///
/// A call the debugger holds as it arrives takes its place only once released: the debugger is
/// resumed by a later call of the same script, which must not wait behind the held one.
#[derive(Default)]
pub(crate) struct CallOrder {
    places: Mutex<Places>,
    changed: Notify,
}

#[derive(Default)]
struct Places {
    next: u64,
    live: BTreeSet<u64>,
}

/// One call's place in its run's [`CallOrder`]. Dropping it, however the call ends (decided,
/// aborted, stopped by the run), lets the next place go.
pub(crate) struct Place {
    order: Arc<CallOrder>,
    ticket: u64,
}

impl CallOrder {
    /// The next place, taken now.
    pub(crate) fn place(self: &Arc<Self>) -> Place {
        let ticket = self.places.lock().map_or(0, |mut places| {
            let ticket = places.next;
            places.next += 1;
            places.live.insert(ticket);
            ticket
        });
        Place {
            order: Arc::clone(self),
            ticket,
        }
    }

    /// Whether no place before `ticket` is live (a poisoned lock orders nothing).
    fn first(&self, ticket: u64) -> bool {
        self.places.lock().map_or(true, |places| {
            places.live.first().is_none_or(|first| *first >= ticket)
        })
    }
}

impl Place {
    /// Waits until every earlier place is gone. Cancel-safe: a dropped wait keeps the place.
    async fn turn(&self) {
        loop {
            // Registered before the check, so a release between the two is not missed.
            let mut changed = pin!(self.order.changed.notified());
            changed.as_mut().enable();
            if self.order.first(self.ticket) {
                return;
            }
            changed.await;
        }
    }
}

/// Neo: a call leaves the order however it ends, and wakes the places behind it.
impl Drop for Place {
    fn drop(&mut self) {
        if let Ok(mut places) = self.order.places.lock() {
            places.live.remove(&self.ticket);
        }
        self.order.changed.notify_waiters();
    }
}

/// How the call path routes a call held at its gate once its hook decides: a hook's replacement
/// arguments exactly as the script's own params would be (or why they are refused), and what an
/// allowed call settles in call order (`Regate` in `method_router.rs`). It lives in the call's
/// [`CallHooks`] beside its place, so a call that waits for `beforeAction` cannot run without it.
pub(crate) trait CallRoute {
    /// The call, routed.
    type Call;

    /// The hook's `args`, routed as the script's params were and admitted as they would be; or
    /// why the call refuses them, for the host-only trace (it may quote them).
    fn reroute(&self, args: Value) -> Result<Self::Call, String>;

    /// What the allowed `call` settles in call order: its write or storage turn, a new test id
    /// attribute.
    fn settle(&self, call: Self::Call) -> Self::Call;
}

/// Which of a call's action hooks run, decided as it arrives.
pub(crate) struct HookPlan {
    hooks: Arc<dyn ActionHooks>,
    served: ServedMethod,
    before: bool,
    after: bool,
}

impl HookPlan {
    /// The hooks that see a call of `served`, or `None`: for an element action, whose hooks
    /// fire once its element is found (feature 4), and for an action no hook wants. Decided by
    /// the command alone, never by a field of the message, which the script may forge.
    pub(crate) fn new(hooks: &Arc<dyn ActionHooks>, served: ServedMethod) -> Option<Self> {
        let action = served.action();
        if action.element_action() {
            return None;
        }
        let kind = Some(action.kind().as_str());
        let group = Some(action.group().as_str());
        let before = hooks.wants(ActionHook::Before, kind, group);
        let after = hooks.wants(ActionHook::After, kind, group);
        (before || after).then(|| Self {
            hooks: Arc::clone(hooks),
            served,
            before,
            after,
        })
    }

    /// The command the hooks see.
    pub(crate) fn served(&self) -> ServedMethod {
        self.served
    }

    /// Whether `beforeAction` holds the call at its gate. Then whatever the call settles in call
    /// order (its write or storage turn, a new test id attribute) waits for the decision: it
    /// must not happen for a blocked call, and a turn held through the hook's wait would keep
    /// the hook's own `setContent` waiting behind the call it decides.
    pub(crate) fn gates(&self) -> bool {
        self.before
    }

    /// The call's hooks once it is admitted, with the event they see; a `frame_call`'s event
    /// names the locator its args carry. A gated call keeps `route` (asked for only then) for
    /// its gate, and takes its place in `order` now, or, when the debugger `held` it, once it
    /// is released.
    pub(crate) fn admitted<G: CallRoute>(
        self,
        mut event: ActionEvent,
        call_id: u64,
        frame_call: bool,
        order: &Arc<CallOrder>,
        held: bool,
        route: impl FnOnce() -> G,
    ) -> CallHooks<G> {
        if frame_call {
            event.locator = locator_of(&event.guid, &event.args);
        }
        let before = if !self.before {
            Before::Unwanted
        } else if held {
            Before::Held(Arc::clone(order), route())
        } else {
            Before::Placed(order.place(), route())
        };
        CallHooks {
            hooks: self.hooks,
            served: self.served,
            event,
            call_id,
            frame_call,
            before,
            after: self.after,
            args_replaced: false,
        }
    }
}

/// A call's `beforeAction`, from its arrival to its decision, with the call path's route for it.
enum Before<G> {
    /// No `beforeAction` wants the call, or it has decided.
    Unwanted,
    /// Its place in the call order, taken as it arrived.
    Placed(Place, G),
    /// The debugger held it as it arrived: it takes its place in this order once released.
    Held(Arc<CallOrder>, G),
}

/// What a gated call does next.
#[derive(Debug, PartialEq)]
pub(crate) enum Gated<R> {
    /// Runs `R`: the script's call, or the call with the hook's arguments.
    Run(R),
    /// Replies with this error and never runs: blocked, aborted, or its context closed.
    Refused(PlaywrightError),
    /// The run ended or its deadline passed: no reply, as for a call the debugger held.
    Stopped,
}

/// One call's action hooks, from its admission until it starts running ([`CallHooks::ran`]).
pub(crate) struct CallHooks<G> {
    hooks: Arc<dyn ActionHooks>,
    served: ServedMethod,
    /// What the hooks see; its `args` become the hook's once it replaces them.
    event: ActionEvent,
    call_id: u64,
    /// Whether the call names a frame, whose args may carry a selector ([`locator_of`]).
    frame_call: bool,
    before: Before<G>,
    after: bool,
    /// Set once `beforeAction` replaced the call's arguments ([`ReplyHooks::finish`]).
    args_replaced: bool,
}

impl<G: CallRoute> CallHooks<G> {
    /// The call's gate: its turn in the call order, then `beforeAction`'s decision, applied with
    /// the call path's route. `call` is the call as the script made it; for a call no
    /// `beforeAction` wants, it runs as it is, having settled as it arrived.
    ///
    /// The whole wait races the run's deadline and end (no reply), the call's abort
    /// (`AbortError`) and the context's close (`TargetClosedError`), as the debugger's hold
    /// does; the call's own timeout starts only after it. Nothing another call waits on is held
    /// meanwhile but the call's place: its ordered turns come after the decision, and the tab's
    /// input lock only inside the handler.
    pub(crate) async fn before(
        &mut self,
        run: &RunScope,
        entry: &CallEntry,
        call: G::Call,
    ) -> Gated<G::Call> {
        let (place, route) = match mem::replace(&mut self.before, Before::Unwanted) {
            Before::Unwanted => return Gated::Run(call),
            Before::Placed(place, route) => (place, route),
            Before::Held(order, route) => (order.place(), route),
        };
        if let Err(halt) = gate_race(run, entry, place.turn()).await {
            return halt;
        }
        // Dropping this future (any branch of the race but its own) abandons the invocation.
        let invoked = self.hooks.before_action(self.event.clone());
        let decided = match gate_race(run, entry, invoked).await {
            Ok(decided) => decided,
            Err(halt) => return halt,
        };
        let gated = self.decided(run, entry, decided, call, &route);
        // Only now may the next call's beforeAction start: this call's ordered turns are taken.
        drop(place);
        gated
    }

    /// What `decided` means for the call; records it on the entry.
    fn decided(
        &mut self,
        run: &RunScope,
        entry: &CallEntry,
        decided: Result<ActionDecision, HookFailure>,
        call: G::Call,
        route: &G,
    ) -> Gated<G::Call> {
        match decided {
            Ok(ActionDecision::Allow) => {
                entry.hook_before(BeforeHookResult::Allowed);
                Gated::Run(route.settle(call))
            }
            Ok(ActionDecision::Block(reason)) => {
                entry.hook_before(BeforeHookResult::Blocked);
                Gated::Refused(PlaywrightError::error(format!(
                    "{BLOCKED_BY_HOOK}: {reason}"
                )))
            }
            Ok(ActionDecision::ReplaceArgs(args)) => {
                // Before anything runs: no line naming the hook's arguments is ever written.
                entry.hide_log();
                match route.reroute(args.clone()) {
                    Ok(rerouted) => {
                        entry.hook_before(BeforeHookResult::ArgsReplaced);
                        self.args_replaced = true;
                        if self.frame_call {
                            self.event.locator = locator_of(&self.event.guid, &args);
                        }
                        self.event.args = args;
                        Gated::Run(route.settle(rerouted))
                    }
                    Err(detail) => {
                        entry.hook_before(BeforeHookResult::Failed);
                        self.trace(
                            run,
                            "beforeAction",
                            &format!("replaced arguments: {detail}"),
                        );
                        Gated::Refused(before_failed())
                    }
                }
            }
            Err(failure) => {
                entry.hook_before(BeforeHookResult::Failed);
                self.trace(run, "beforeAction", &failure.detail());
                Gated::Refused(before_failed())
            }
        }
    }

    /// One host-only trace line about this call's `event` hook.
    fn trace(&self, run: &RunScope, event: &str, detail: &str) {
        run.hook_ledger
            .trace(trace_line(event, self.call_id, &self.event.method, detail));
    }
}

impl<G> CallHooks<G> {
    /// What the call's hooks do to its reply, once it passed its gate and runs: its
    /// `afterAction` when one wants it, and the redaction of a call that runs with a hook's
    /// arguments. `None` when neither applies.
    pub(crate) fn ran(self, run: &RunScope) -> Option<ReplyHooks> {
        let Self {
            hooks,
            served,
            event,
            call_id,
            after,
            args_replaced,
            ..
        } = self;
        if !after && !args_replaced {
            return None;
        }
        let method = event.method.clone();
        Some(ReplyHooks {
            after: after.then(|| AfterHook {
                hooks,
                served,
                event,
            }),
            args_replaced,
            call_id,
            method,
            run: run.clone(),
        })
    }
}

/// What a call's hooks do to its reply ([`CallHooks::ran`]), in `finish_reply` before the call's
/// entry ends.
pub(crate) struct ReplyHooks {
    after: Option<AfterHook>,
    /// The call runs with a hook's arguments: an error it replies is redacted.
    args_replaced: bool,
    call_id: u64,
    method: String,
    run: RunScope,
}

impl ReplyHooks {
    /// The call stopped before its handler started (aborted, its page or frame not ready, its
    /// session gone): it never ran, so no `afterAction` sees it. Its error text is still
    /// redacted if it was to run with a hook's arguments.
    pub(crate) fn never_ran(&mut self) {
        self.after = None;
    }

    /// The reply the call sends instead of `outcome`, and what its entry keeps. `afterAction`
    /// first: keep, replace, or fail the call closed. A call whose result a hook replaced or
    /// withheld keeps no call log, which quotes the page (a read logs `locator resolved to
    /// <preview>`), in its reply or its record. A call that ran with a hook's arguments replies
    /// none of its own error text, which may quote them (`Unknown permission: <value>`,
    /// `… at <url>`): it keeps the error's name, and the text goes to the host-only trace. `None`
    /// when the run stopped during `afterAction`: no reply, as for any call the run ended.
    pub(crate) async fn finish(
        self,
        entry: &CallEntry,
        outcome: Result<Option<Value>, PlaywrightError>,
    ) -> Option<Result<Option<Value>, PlaywrightError>> {
        let Self {
            after,
            args_replaced,
            call_id,
            method,
            run,
        } = self;
        let (outcome, decided) = match after {
            Some(after) => {
                let (outcome, decided) =
                    after.apply(&run, entry, call_id, &method, outcome).await?;
                (outcome, Some(decided))
            }
            None => (outcome, None),
        };
        if matches!(
            decided,
            Some(AfterHookResult::Replaced | AfterHookResult::Failed)
        ) {
            entry.hide_log();
        }
        match outcome {
            // A failed afterAction's error is Neo's own already.
            Err(error) if args_replaced && decided != Some(AfterHookResult::Failed) => {
                run.hook_ledger.trace(format!(
                    "neo: call {call_id} ({method}) failed with replaced arguments: {}: {}",
                    error.name.as_str(),
                    error.message
                ));
                Some(Err(PlaywrightError {
                    name: error.name,
                    message: REPLACED_ARGS_FAILED.to_string(),
                }))
            }
            outcome => Some(outcome),
        }
    }
}

/// A call's `afterAction`, ready once the call ran.
struct AfterHook {
    hooks: Arc<dyn ActionHooks>,
    served: ServedMethod,
    event: ActionEvent,
}

impl AfterHook {
    /// `afterAction` on the call's `outcome`, which it may keep or replace; records the decision
    /// on the entry and the notes in the run's ledger. Races the run's deadline and end only:
    /// the outcome is decided, so an abort no longer counts. `None` when the run stopped first.
    /// A failure fails the call closed, its delivery left as the action recorded it.
    async fn apply(
        self,
        run: &RunScope,
        entry: &CallEntry,
        call_id: u64,
        method: &str,
        outcome: Result<Option<Value>, PlaywrightError>,
    ) -> Option<(Result<Option<Value>, PlaywrightError>, AfterHookResult)> {
        let Self {
            hooks,
            served,
            event,
        } = self;
        let ended = match &outcome {
            Ok(result) => ActionEnd::Result(result.clone().unwrap_or(Value::Null)),
            Err(error) => {
                ActionEnd::Error(RunError::new(error.name.as_str(), error.message.clone()))
            }
        };
        let decided = tokio::select! {
            biased;
            _ = run.deadline.reached() => return None,
            () = run.ended.cancelled() => return None,
            decided = hooks.after_action(event, ended) => decided,
        };
        let applied = match decided {
            Ok(AfterDecision {
                replace: None,
                notes,
            }) => Ok((AfterHookResult::Kept, outcome, notes)),
            Ok(AfterDecision {
                replace: Some(replaced),
                notes,
            }) => replacement(served, outcome, replaced)
                .map(|result| (AfterHookResult::Replaced, Ok(result), notes)),
            Err(failure) => Err(failure.detail()),
        };
        match applied {
            Ok((decided, outcome, notes)) => {
                entry.hook_after(decided);
                for note in notes {
                    run.hook_ledger.note(note);
                }
                Some((outcome, decided))
            }
            Err(detail) => {
                entry.hook_after(AfterHookResult::Failed);
                run.hook_ledger
                    .trace(trace_line("afterAction", call_id, method, &detail));
                Some((
                    Err(PlaywrightError::error(AFTER_ACTION_FAILED)),
                    AfterHookResult::Failed,
                ))
            }
        }
    }
}

/// The reply `result` a hook's `replaced` becomes, or why it cannot: only a call that succeeded
/// takes a replacement, never with binaries, and only one with its command's result shape
/// (`ServedMethod::replacement_result`). `{ replace: undefined }` is checked as `null`.
fn replacement(
    served: ServedMethod,
    outcome: Result<Option<Value>, PlaywrightError>,
    replaced: Replaced,
) -> Result<Option<Value>, String> {
    let Ok(actual) = outcome else {
        return Err("it replaced the result of a call that failed".to_string());
    };
    if !replaced.binaries.is_empty() {
        return Err("its replacement holds binaries, which no action's result carries".to_string());
    }
    served
        .replacement_result(replaced.value.unwrap_or(Value::Null), actual.as_ref())
        .map_err(|ResultShapeError(detail)| {
            format!("its replacement has the wrong shape: {detail}")
        })
}

/// The gate's race: `work` against the run's deadline and end, then the call's abort, then the
/// context's close, in that precedence (`Progress::race`'s, without the call's own timeout).
async fn gate_race<T, R>(
    run: &RunScope,
    entry: &CallEntry,
    work: impl Future<Output = T>,
) -> Result<T, Gated<R>> {
    tokio::select! {
        biased;
        _ = run.deadline.reached() => Err(Gated::Stopped),
        () = run.ended.cancelled() => Err(Gated::Stopped),
        error = entry.aborted() => Err(Gated::Refused(error)),
        () = run.context_closed.cancelled() => {
            Err(Gated::Refused(PlaywrightError::target_closed()))
        }
        value = work => Ok(value),
    }
}

/// The error of a call whose `beforeAction` failed; the failure itself is in the trace.
fn before_failed() -> PlaywrightError {
    PlaywrightError::error(format!("{BLOCKED_BY_HOOK}: {BEFORE_ACTION_FAILED}"))
}

/// One host-only trace line: which hook failed on which call, and how.
fn trace_line(event: &str, call_id: u64, method: &str, detail: &str) -> String {
    format!("neo: {event} failed for call {call_id} ({method}): {detail}")
}

/// The locator a frame call's `args` name: their string `selector` in `frame`.
fn locator_of(frame: &str, args: &Value) -> Option<LocatorRef> {
    let selector = args.get("selector")?.as_str()?;
    Some(LocatorRef {
        frame: frame.to_string(),
        selector: selector.to_string(),
    })
}
