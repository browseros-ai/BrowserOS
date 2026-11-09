//! Neo: a run's hooks. A run may carry a hook file (`RunRequest::hooks`): customer JavaScript
//! that registers handlers with `neo.on(event, handler)`. Neo runs it in a sandbox of its own, a
//! second QuickJS realm per run next to the agent's, created only when the run has hooks (hooks
//! design D2). The agent's script can neither see nor reach it.
//!
//! ```text
//!  Runtime::run
//!    ├─ HookSandbox::start            the actor task (`hooks/realm.rs`) on the run's tracker
//!    ├─ before_run: load, beforeRun   race the run's deadline; block or fail → `blocked`
//!    ├─ quickjs::run                  the agent's script, as without hooks
//!    ├─ after_run: afterRun           always, full budget: keep, replace (ok runs only), notes
//!    ├─ close                         the actor ends and drops the realm
//!    └─ Dispatcher::finish            records: a hook's calls are `origin: hook`
//! ```
//!
//! The hook realm's own browser calls go through a dispatcher of its own
//! (`Dispatcher::for_hooks`), captured by its bridge (`quickjs_bridge::install_hooks`): whose call
//! it is depends on which realm sent it, never on the message (hooks design D7).
//!
//! Every handler failure fails closed (hooks design D8): a hook that throws, rejects, returns a
//! decision its event does not allow, or runs past its budget blocks the run before its script
//! starts, or turns its result into an error after. The model-visible error never carries the
//! hook's own error text, which may hold a secret; that goes to the host-only hook trace.
//!
//! [`HookSandbox::wants`], [`HookSandbox::before_action`] and [`HookSandbox::after_action`] are
//! the action hooks' API for the call path (hooks feature 3); the run reaches its sandbox from
//! the call path through `RunScope::hooks`.

use std::borrow::Cow;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::{CancellationToken, DropGuard};
use tokio_util::task::TaskTracker;

use crate::deadline::{Deadline, Stop};
use crate::method_router::Dispatcher;
use crate::quickjs;
use crate::quickjs_bridge::{Getters, Logs};
use crate::run::{CallLocation, RunBinary, RunError, RunHooks, RunOutcome, RunStatus};

mod decision;
mod realm;

use decision::Decided;
use realm::{Actor, Bridge, Decision, Invoked, Job, Loaded, Registration, Setup};

/// Neo: the model-visible error of a run whose hooks failed before its script started. The
/// failure itself goes to the hook trace: a hook's error text may hold a secret.
const BEFORE_RUN_FAILED: &str = "the run's hooks failed before its script started";

/// Neo: the model-visible error of a run whose `afterRun` hook failed, whose result is therefore
/// withheld: a filter that failed must not let the unfiltered result through.
const AFTER_RUN_FAILED: &str = "the run's afterRun hook failed, so its result is withheld";

/// The four hook events (hooks design D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HookEvent {
    BeforeRun,
    AfterRun,
    BeforeAction,
    AfterAction,
}

impl HookEvent {
    /// The event's name, as `neo.on` and `invoke` spell it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::BeforeRun => "beforeRun",
            Self::AfterRun => "afterRun",
            Self::BeforeAction => "beforeAction",
            Self::AfterAction => "afterAction",
        }
    }
}

/// An action event, before or after the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "hooks feature 3 fires the action hooks from the call path"
    )
)]
pub(crate) enum ActionHook {
    Before,
    After,
}

/// What an action hook's handlers receive about the action, as the payload `invoke` takes. The
/// SDK builds real Playwright objects from its guids (hooks design D5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ActionEvent {
    /// The action's kind (`fill`, `click`, …), `None` until hooks feature 1 names it.
    pub(crate) kind: Option<&'static str>,
    /// The action's group (`type`, `pointer`, …), `None` until hooks feature 1 names it.
    pub(crate) group: Option<&'static str>,
    pub(crate) method: String,
    /// The object the call named.
    pub(crate) guid: String,
    /// The call's params as the script sent them.
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
    /// The pinned element's handle guid (hooks feature 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) element: Option<String>,
}

/// The locator an element action targets: its frame and selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LocatorRef {
    pub(crate) frame: String,
    pub(crate) selector: String,
}

/// How an action ended, for `afterAction`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "hooks feature 3 fires the action hooks from the call path"
    )
)]
pub(crate) enum ActionEnd {
    Result(Value),
    Error(RunError),
}

/// What `beforeAction`'s handlers decided.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ActionDecision {
    Allow,
    Block(String),
    /// New arguments, an object, which the call re-checks like the script's own.
    ReplaceArgs(Value),
}

/// What an after event's handlers decided.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AfterDecision {
    /// What replaces the result, when a handler replaced it.
    pub(crate) replace: Option<Replaced>,
    /// Every handler's note, in order.
    pub(crate) notes: Vec<String>,
}

/// A result a hook put in place of the run's or the action's own.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Replaced {
    /// The replacement as JSON; `None` for `{ replace: undefined }`, which JSON cannot hold and
    /// which replaces too (`'replace' in r` decides).
    pub(crate) value: Option<Value>,
    /// Its binaries, lifted out of it as the agent's result's are: each one's place holds
    /// `{type, bytes}` and its bytes are here (`RunOutcome::binaries`).
    pub(crate) binaries: Vec<RunBinary>,
}

/// Why an invocation decided nothing usable. Every one fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookFailure {
    /// A handler threw or rejected, or loading failed; the detail is for the hook trace only.
    Threw(String),
    /// The invocation ran past its budget.
    Timeout,
    /// The decision was unreadable, or not one its event allows.
    BadDecision(String),
    /// The realm cannot answer: the hooks did not load, the run ended, or nobody waits.
    Realm(String),
}

/// A run's hook sandbox: a handle on the actor that owns the hook realm. Cheap to clone; every
/// clone sends to the same actor, which takes invocations one at a time in the order they were
/// sent.
#[derive(Clone)]
pub(crate) struct HookSandbox {
    jobs: mpsc::UnboundedSender<Job>,
    /// Set once loading succeeds, never changed after.
    registrations: Arc<OnceLock<Vec<Registration>>>,
    /// The host-only hook trace: the realm's `ctx.trace` and `console` lines, and Rust's notes
    /// on why a hook failed.
    trace: Logs,
    /// Cancelled when the run is done with its hooks, or its future is dropped: the actor ends
    /// and drops the realm.
    close: CancellationToken,
    /// The run page, as `beforeRun` and `afterRun` name it.
    page_guid: String,
}

/// Everything the run's hook realm is built from, besides the hook file itself.
pub(crate) struct RealmParts {
    pub(crate) page_guid: String,
    pub(crate) frame_guid: String,
    pub(crate) url: String,
    pub(crate) getters: Getters,
    /// The run's hook dispatcher (`Dispatcher::for_hooks`), whose run scope gives the sandbox
    /// the run's tracker and the slot the call path finds it in (`RunScope::hooks`).
    pub(crate) dispatcher: Dispatcher,
    /// Counts realms created on the run's `Runtime`.
    pub(crate) realms: Arc<AtomicU64>,
}

/// Whether the run's script starts, after loading the hooks and running `beforeRun`.
#[derive(Debug)]
pub(crate) enum Gate {
    Start,
    /// The run ends here, before its script: blocked, timed out or cancelled.
    End(Box<RunOutcome>),
}

impl HookSandbox {
    /// Starts the actor of a run's hook realm, on the run's tracker, for `hooks`, and puts the
    /// sandbox where the run's call path finds it (`RunScope::hooks`). Nothing runs until
    /// [`HookSandbox::before_run`] loads the hook file.
    pub(crate) fn start(hooks: RunHooks, parts: RealmParts) -> Self {
        let RealmParts {
            page_guid,
            frame_guid,
            url,
            getters,
            dispatcher,
            realms,
        } = parts;
        let run = dispatcher.run_scope();
        let (tasks, slot) = (run.tasks.clone(), Arc::clone(&run.hooks));
        let setup = Setup {
            code: hooks.code,
            meta: hooks.meta,
            page_guid,
            frame_guid,
            url,
            bridge: Bridge::Run {
                getters,
                dispatcher,
            },
            sdk: Cow::Borrowed(quickjs::SDK_BUNDLE),
            realms,
        };
        let sandbox = Self::spawn(setup, hooks.budget, &tasks);
        let _first = slot.set(sandbox.clone());
        sandbox
    }

    fn spawn(setup: Setup, budget: Duration, tasks: &TaskTracker) -> Self {
        let (jobs, receiver) = mpsc::unbounded_channel();
        let trace = Logs::default();
        let close = CancellationToken::new();
        let page_guid = setup.page_guid.clone();
        let actor = Actor::new(setup, budget, trace.clone(), close.clone());
        tasks.spawn(actor.run(receiver));
        Self {
            jobs,
            registrations: Arc::default(),
            trace,
            close,
            page_guid,
        }
    }

    /// Ends the actor, which drops the realm: the run is done with its hooks. Hook calls still
    /// running end with the run's other calls (`Dispatcher::finish`).
    pub(crate) fn close(&self) {
        self.close.cancel();
    }

    /// A guard that closes the sandbox when dropped, for a run whose future is dropped before
    /// it closes the sandbox itself: otherwise the realm, holding the run's hook dispatcher,
    /// would keep the run's sessions and tasks alive.
    pub(crate) fn close_on_drop(&self) -> DropGuard {
        self.close.clone().drop_guard()
    }

    /// The hook trace so far, taken: what the run's outcome carries as `hook_trace`.
    pub(crate) fn take_trace(&self) -> Vec<String> {
        self.trace.take()
    }

    /// The run's end for its hooks: `afterRun` ([`HookSandbox::after_run`]), then the sandbox
    /// closes, and the outcome carries the hook trace. Before `Dispatcher::finish`: the actor is a
    /// task of the run's tracker, which `finish` waits for.
    pub(crate) async fn end(&self, outcome: RunOutcome) -> RunOutcome {
        let outcome = self.after_run(outcome).await;
        self.close();
        outcome.with_hook_trace(self.take_trace())
    }

    /// Loads the hook file and runs `beforeRun`, both racing the run's `deadline`. The script
    /// starts only when every `beforeRun` handler allowed it.
    pub(crate) async fn before_run(&self, script: &str, deadline: &Deadline) -> Gate {
        let (reply, loaded) = oneshot::channel();
        let job = Job::Load {
            race: deadline.clone(),
            reply,
        };
        let registrations = match self.send(job, loaded).await {
            Some(Loaded::Ready(registrations)) => registrations,
            Some(Loaded::Stopped(stop)) => {
                return Gate::stopped(stop, deadline);
            }
            Some(Loaded::Failed(detail)) => return self.blocked(&format!("loading: {detail}")),
            None => return self.blocked("loading: the hook realm is gone"),
        };
        let _first = self.registrations.set(registrations);
        if !self.registered(HookEvent::BeforeRun) {
            return Gate::Start;
        }
        let payload = decision::before_run(script, &self.page_guid);
        match self
            .invoke(HookEvent::BeforeRun, payload, Some(deadline.clone()))
            .await
        {
            Invoked::Decided(decided) => match decision::parse(HookEvent::BeforeRun, &decided.text)
            {
                Ok(Decided::Allow) => Gate::Start,
                Ok(Decided::Block(reason)) => Gate::End(Box::new(RunOutcome::blocked(
                    RunError::new("Error", reason),
                ))),
                Ok(other) => self.blocked(&format!("beforeRun decided {other:?}")),
                Err(failure) => self.blocked(&format!("beforeRun: {failure:?}")),
            },
            Invoked::Failed(failure) => self.blocked(&format!("beforeRun: {failure:?}")),
            Invoked::Stopped(stop) => Gate::stopped(stop, deadline),
        }
    }

    /// Runs `afterRun` on how the run ended, unless the hooks never loaded or blocked the run:
    /// always otherwise, even after a timeout or cancel, with its full budget (its browser calls
    /// then fail on the stopped deadline). A replacement applies only to an `ok` run; notes go
    /// to the outcome. A failure turns the outcome into an error without the result.
    pub(crate) async fn after_run(&self, outcome: RunOutcome) -> RunOutcome {
        if outcome.status == RunStatus::Blocked || !self.registered(HookEvent::AfterRun) {
            return outcome;
        }
        let payload = decision::after_run(&outcome, &self.page_guid);
        let decided = match self.invoke(HookEvent::AfterRun, payload, None).await {
            Invoked::Decided(decided) => checked(HookEvent::AfterRun, decided),
            Invoked::Failed(failure) => Err(failure),
            Invoked::Stopped(_) => Err(HookFailure::Realm("the run stopped".to_string())),
        };
        let (replace, notes) = match decided {
            Ok(Decided::After(AfterDecision { replace, notes })) => (replace, notes),
            Ok(other) => {
                return self.withheld(outcome, &format!("afterRun decided {other:?}"));
            }
            Err(failure) => return self.withheld(outcome, &format!("afterRun: {failure:?}")),
        };
        let outcome = match replace {
            None => outcome,
            // The script's binaries belong to the value it returned, so they go with it.
            Some(Replaced { value, binaries }) if outcome.status == RunStatus::Ok => RunOutcome {
                value,
                binaries,
                ..outcome
            },
            Some(_) => {
                return self.withheld(outcome, "afterRun replaced the result of a run that failed");
            }
        };
        outcome.with_notes(notes)
    }

    /// Whether any `event` action registration matches an action of `kind` and `group`, read
    /// from the registrations fixed at load, without entering the realm: the call path skips the
    /// realm for every other action.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "hooks feature 3 fires the action hooks from the call path"
        )
    )]
    pub(crate) fn wants(&self, event: ActionHook, kind: Option<&str>, group: Option<&str>) -> bool {
        let event = match event {
            ActionHook::Before => HookEvent::BeforeAction,
            ActionHook::After => HookEvent::AfterAction,
        };
        self.registrations.get().is_some_and(|registrations| {
            registrations.iter().any(|registration| {
                registration.event == event && registration.matches(kind, group)
            })
        })
    }

    /// `beforeAction` for `event`, queued behind earlier invocations, within the budget. The
    /// caller races the run's deadline and the call's abort: dropping the future abandons the
    /// invocation, whose `ctx.signal` then fires.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "hooks feature 3 fires the action hooks from the call path"
        )
    )]
    pub(crate) async fn before_action(
        &self,
        event: ActionEvent,
    ) -> Result<ActionDecision, HookFailure> {
        let payload = decision::action(&event, None);
        match self.decide(HookEvent::BeforeAction, payload).await? {
            Decided::Allow => Ok(ActionDecision::Allow),
            Decided::Block(reason) => Ok(ActionDecision::Block(reason)),
            Decided::ReplaceArgs(args) => Ok(ActionDecision::ReplaceArgs(args)),
            Decided::After(decision) => Err(HookFailure::BadDecision(format!(
                "beforeAction decided {decision:?}"
            ))),
        }
    }

    /// `afterAction` for `event`, which `ended` so, as [`HookSandbox::before_action`].
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "hooks feature 3 fires the action hooks from the call path"
        )
    )]
    pub(crate) async fn after_action(
        &self,
        event: ActionEvent,
        ended: ActionEnd,
    ) -> Result<AfterDecision, HookFailure> {
        let payload = decision::action(&event, Some(&ended));
        match self.decide(HookEvent::AfterAction, payload).await? {
            Decided::After(decision) => Ok(decision),
            other => Err(HookFailure::BadDecision(format!(
                "afterAction decided {other:?}"
            ))),
        }
    }

    /// An action invocation's checked decision.
    async fn decide(&self, event: HookEvent, payload: String) -> Result<Decided, HookFailure> {
        match self.invoke(event, payload, None).await {
            Invoked::Decided(decided) => checked(event, decided),
            Invoked::Failed(failure) => Err(failure),
            Invoked::Stopped(_) => Err(HookFailure::Realm("the run stopped".to_string())),
        }
    }

    /// Queues one invocation and waits for it.
    async fn invoke(&self, event: HookEvent, payload: String, race: Option<Deadline>) -> Invoked {
        let (reply, invoked) = oneshot::channel();
        let job = Job::Invoke {
            event,
            payload,
            race,
            reply,
        };
        self.send(job, invoked).await.unwrap_or_else(|| {
            Invoked::Failed(HookFailure::Realm("the hook realm is gone".to_string()))
        })
    }

    /// Sends `job` to the actor and waits for its answer on `answer`; `None` when the actor is
    /// gone (the sandbox closed).
    async fn send<T>(&self, job: Job, answer: oneshot::Receiver<T>) -> Option<T> {
        self.jobs.send(job).ok()?;
        answer.await.ok()
    }

    /// Whether any handler is registered for `event`. A run event without one is skipped: no
    /// handler allows and keeps.
    fn registered(&self, event: HookEvent) -> bool {
        self.registrations.get().is_some_and(|registrations| {
            registrations
                .iter()
                .any(|registration| registration.event == event)
        })
    }

    /// The run ends blocked before its script, after the hooks failed: `detail` goes to the
    /// trace, the outcome names no detail.
    fn blocked(&self, detail: &str) -> Gate {
        self.trace.push(format!("neo: {detail}"));
        Gate::End(Box::new(RunOutcome::blocked(RunError::new(
            "Error",
            BEFORE_RUN_FAILED.to_string(),
        ))))
    }

    /// `afterRun` failed: the outcome becomes an error without the script's result.
    fn withheld(&self, outcome: RunOutcome, detail: &str) -> RunOutcome {
        self.trace.push(format!("neo: {detail}"));
        RunOutcome {
            status: RunStatus::Error,
            value: None,
            error: Some(RunError::new("Error", AFTER_RUN_FAILED.to_string())),
            binaries: Vec::new(),
            ..outcome
        }
    }
}

impl Gate {
    /// The run's deadline passed, or its host cancelled it, before the script started.
    fn stopped(stop: Stop, deadline: &Deadline) -> Self {
        Self::End(Box::new(RunOutcome::stopped(
            stop,
            deadline.timeout(),
            Vec::new(),
        )))
    }
}

/// `decided` checked against `event`, a replacement's binaries lifted out of its value.
fn checked(event: HookEvent, decided: Decision) -> Result<Decided, HookFailure> {
    let Decision {
        text,
        marker,
        shipped,
    } = decided;
    let mut decided = decision::parse(event, &text)?;
    if let Decided::After(AfterDecision {
        replace: Some(Replaced {
            value: Some(value),
            binaries,
        }),
        ..
    }) = &mut decided
    {
        *binaries = quickjs::lift_binaries(value, &marker, shipped);
    }
    Ok(decided)
}

#[cfg(test)]
mod tests;
