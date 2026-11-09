//! Neo: the actor that owns one run's hook realm. One tokio task, started with the run's hooks,
//! takes the sandbox's jobs one at a time, in the order they were sent (FIFO): loading the hook
//! file first, then each invocation. Nothing else ever touches the realm, so its QuickJS runtime
//! needs no sharing, and two hooks never run at once.
//!
//! ```text
//!  HookSandbox::send(Job) ──mpsc──▶ actor task (spawned on the run's tracker)
//!                                     ├─ Load: new realm, bridge, globals, SDK, hook file, finishLoading
//!                                     └─ Invoke: arm the watch → enter the realm → invoke(id, event, payload)
//!                                          └─ select: closed │ run deadline │ caller gone │ budget │ decision
//!  ◀──oneshot── Loaded | Invoked
//! ```
//!
//! The realm runs only inside a job. Each job enters it with its own `async_with`, which drives
//! the realm's timers, pending browser-call replies and promise jobs only while the job waits, so
//! between jobs the realm is frozen: work a hook left behind (a timer, a call it never awaited)
//! resumes in a later job, under that job's budget. The values a job needs again (the SDK's
//! `invoke` and `abort`) stay inside the realm, under a global with a random name: rquickjs's
//! `Persistent` would keep them in Rust, but it is not `Send`, and this task must be. Host calls
//! are different: each job holds a cancellation guard, so its callbacks end with it even
//! while the realm is frozen, and their replies cannot resolve in a later job.
//!
//! Time has two guards, both armed per job ([`Watch`]): JS that spins is stopped by the realm's
//! interrupt handler, which QuickJS calls while JS runs, once the job's budget (or the run's
//! deadline, for a job that races it) has passed; JS that waits is stopped by the job's `select!`
//! at the same instant. A job that does not decide aborts its invocation, so the hook's
//! `ctx.signal` fires.
//!
//! Cleanup: the task ends when the sandbox closes (`HookSandbox::close`, or the drop guard the
//! run holds), dropping the realm and with it the bridge's hook dispatcher and getters. That
//! breaks the one reference cycle: realm → hook dispatcher → the run's dispatcher state →
//! `RunScope` → sandbox.

use std::borrow::Cow;
use std::future;
use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rquickjs::function::Async;
use rquickjs::{
    AsyncContext, AsyncRuntime, CatchResultExt, CaughtError, Ctx, Function, Object, Promise,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio::time;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::deadline::{Deadline, Stop};
use crate::hooks::host::Host;
use crate::hooks::{HookEvent, HookFailure};
use crate::method_router::Dispatcher;
use crate::quickjs;
use crate::quickjs_bridge::{self, Getters, Logs};

/// Neo: how long the listeners of a job's `ctx.signal` may run when the job ends without a
/// decision: the hook's budget is spent by then, and a listener that spins must not hold the
/// next job.
const ABORT_GRACE: Duration = Duration::from_millis(100);

/// Neo: a budget too large to add to the clock is no budget at all; tokio's and the run
/// deadline's own horizon (`deadline.rs`).
const FAR_FUTURE: Duration = Duration::from_hours(24 * 365 * 30);

/// Neo: keeps the SDK's `createHooks` result in the hook realm between jobs, under the global
/// `key` (a random name, not enumerable, not writable): `invoke` and the hook file's promise race
/// every uncaught-error promise of the realm, as the agent's script races them
/// (`quickjs::RACE_UNCAUGHT`), so a hook's listener or timer that throws fails the job it runs in.
/// `invoke` passes `ship` and `marker` on, so a `replace` value's binaries leave the realm as the
/// agent's result's do (`quickjs::to_json`).
const STASH: &str = r"(key, hooks, uncaught) => {
  const race = (promise) => Promise.race([promise, ...uncaught])
  Object.defineProperty(globalThis, key, {
    value: Object.freeze({
      race,
      invoke: (id, event, payload, ship, marker) =>
        race(hooks.invoke(id, event, payload, ship, marker)),
      abort: (id, reason) => hooks.abort(id, reason),
      finishLoading: () => hooks.finishLoading(),
    }),
  })
}";

/// Neo: the reason a job's `ctx.signal` aborts with when its budget ran out.
const BUDGET_SPENT: &str = "the hook's time budget ran out";
/// Neo: the reason when the run's deadline passed or its host cancelled it during the job.
const RUN_STOPPED: &str = "the run stopped";
/// Neo: the reason when nobody waits for the job's decision any more.
const ABANDONED: &str = "nobody waits for this hook any more";

/// Everything the realm is built from, handed over once, to the load job.
pub(crate) struct Setup {
    pub(crate) host: Host,
    /// The hook file: the body of an async function of `neo`.
    pub(crate) code: String,
    /// The run's `meta`, `ctx.meta` in every handler.
    pub(crate) meta: Value,
    pub(crate) page_guid: String,
    pub(crate) frame_guid: String,
    /// The run page's URL at the start.
    pub(crate) url: String,
    pub(crate) bridge: Bridge,
    /// The SDK bundle to evaluate: `quickjs::SDK_BUNDLE`, or a test's stand-in.
    pub(crate) sdk: Cow<'static, str>,
    /// Counts realms created on the run's `Runtime`.
    pub(crate) realms: Arc<AtomicU64>,
}

/// What the realm's `__neo_runtime` is bound to.
pub(crate) enum Bridge {
    /// The run's own: the hook realm's getters and the run's hook dispatcher
    /// (`quickjs_bridge::install_hooks`).
    Run {
        getters: Getters,
        dispatcher: Dispatcher,
    },
    /// Tests of the realm alone: a `__neo_runtime` with `log` (into the hook trace) and nothing
    /// else, so no browser is needed.
    #[cfg(test)]
    Bare,
}

/// One unit of the actor's work. Each carries the channel its result goes back on; a caller that
/// stops waiting drops its receiver, and the actor skips or abandons that job.
pub(crate) enum Job {
    Load {
        /// Loading also races the run's deadline and cancellation.
        race: Deadline,
        reply: oneshot::Sender<Loaded>,
    },
    Invoke {
        event: HookEvent,
        payload: String,
        /// The run's deadline when this job races it (`beforeRun`); `None` for one that runs its
        /// full budget whatever the run's state (`afterRun`) or whose caller races it.
        race: Option<Deadline>,
        reply: oneshot::Sender<Invoked>,
    },
}

/// How loading the hook file ended.
#[derive(Debug)]
pub(crate) enum Loaded {
    /// The hooks registered, in order.
    Ready(Vec<Registration>),
    /// The hooks cannot run: a syntax error, a throw or rejection, a bad registration, the
    /// budget spent, or the realm closed. The detail goes to the hook trace only.
    Failed(String),
    /// The run's deadline passed, or the host cancelled it, first.
    Stopped(Stop),
}

/// How one invocation ended.
#[derive(Debug)]
pub(crate) enum Invoked {
    /// `invoke`'s decision, not yet checked against the event (`decision.rs`).
    Decided(Decision),
    Failed(HookFailure),
    /// The run's deadline passed, or the host cancelled it, first.
    Stopped(Stop),
}

/// `invoke`'s decision JSON, and the binaries a `replace` value shipped while being exported:
/// each one's place in `value` carries `marker` and its index in `shipped`
/// (`quickjs::lift_binaries`).
#[derive(Debug)]
pub(crate) struct Decision {
    pub(crate) text: String,
    pub(crate) marker: String,
    pub(crate) shipped: Vec<Vec<u8>>,
}

/// One `neo.on(...)` registration, as `finishLoading` lists it. Fixed once loading ends, so the
/// call path can skip the realm for an action no registration matches.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub(crate) event: HookEvent,
    #[serde(default)]
    pub(crate) kinds: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) groups: Option<Vec<String>>,
}

impl Registration {
    /// Whether an action of `kind` and `group` reaches this registration: always without a
    /// filter, otherwise when its kind is in `kinds` or its group in `groups`.
    pub(crate) fn matches(&self, kind: Option<&str>, group: Option<&str>) -> bool {
        let listed = |names: Option<&Vec<String>>, name: Option<&str>| {
            names
                .zip(name)
                .is_some_and(|(names, name)| names.iter().any(|listed| listed == name))
        };
        (self.kinds.is_none() && self.groups.is_none())
            || listed(self.kinds.as_ref(), kind)
            || listed(self.groups.as_ref(), group)
    }
}

/// When JS in the hook realm must stop: shared by the actor, which arms it for each job, and the
/// realm's interrupt handler, which reads it while JS runs.
#[derive(Clone, Default)]
pub(crate) struct Watch(Arc<Mutex<Option<Limit>>>);

struct Limit {
    until: Instant,
    /// The run's deadline, for a job that races it.
    run: Option<Deadline>,
}

impl Watch {
    fn arm(&self, until: Instant, run: Option<Deadline>) {
        if let Ok(mut limit) = self.0.lock() {
            *limit = Some(Limit { until, run });
        }
    }

    /// Between jobs no JS may run at all.
    fn disarm(&self) {
        if let Ok(mut limit) = self.0.lock() {
            *limit = None;
        }
    }

    /// Whether running JS must stop now. A poisoned lock (a panic while arming) stops it.
    fn expired(&self) -> bool {
        let Ok(limit) = self.0.lock() else {
            return true;
        };
        limit.as_ref().is_none_or(|limit| {
            Instant::now() >= limit.until
                || limit.run.as_ref().is_some_and(|run| run.stop().is_some())
        })
    }
}

/// The hook realm once loaded. `context` is declared before `_runtime`, so it drops first.
struct Realm {
    /// The global [`STASH`] keeps the SDK's hooks under.
    key: String,
    context: AsyncContext,
    _runtime: AsyncRuntime,
}

/// The actor's state; owned by its task alone.
pub(crate) struct Actor {
    host: Host,
    setup: Option<Setup>,
    budget: Duration,
    /// The hook trace, which the realm's bridge writes.
    trace: Logs,
    /// Neo's own lines on why a hook failed.
    failures: Logs,
    watch: Watch,
    close: CancellationToken,
    realm: Option<Realm>,
    next_id: u32,
}

impl Actor {
    pub(crate) fn new(
        setup: Setup,
        budget: Duration,
        trace: Logs,
        failures: Logs,
        close: CancellationToken,
    ) -> Self {
        Self {
            host: setup.host.clone(),
            setup: Some(setup),
            budget,
            trace,
            failures,
            watch: Watch::default(),
            close,
            realm: None,
            next_id: 0,
        }
    }

    /// The actor task: one job at a time, in arrival order, until the sandbox closes. Returning
    /// drops the realm.
    pub(crate) async fn run(mut self, mut jobs: mpsc::UnboundedReceiver<Job>) {
        loop {
            let job = tokio::select! {
                biased;
                () = self.close.cancelled() => break,
                job = jobs.recv() => match job {
                    Some(job) => job,
                    None => break,
                },
            };
            match job {
                Job::Load { race, mut reply } => {
                    let loaded = self.load(&race, &mut reply).await;
                    let _sent = reply.send(loaded);
                }
                Job::Invoke {
                    event,
                    payload,
                    race,
                    mut reply,
                } => {
                    // A caller that stopped waiting before its turn costs nothing.
                    if reply.is_closed() {
                        continue;
                    }
                    let invoked = self
                        .invoke(event, &payload, race.as_ref(), &mut reply)
                        .await;
                    let _sent = reply.send(invoked);
                }
            }
        }
    }

    /// The load job: builds the realm and runs the hook file, within the budget and the run's
    /// deadline.
    async fn load(&mut self, race: &Deadline, reply: &mut oneshot::Sender<Loaded>) -> Loaded {
        let Some(setup) = self.setup.take() else {
            return Loaded::Failed("the hooks were loaded twice".to_string());
        };
        let until = self.until();
        let host_job = self.host.arm(until, &self.close);
        self.watch.arm(until, Some(race.clone()));
        let loading = build(
            setup,
            self.trace.clone(),
            self.watch.clone(),
            self.close.clone(),
        );
        let loaded = tokio::select! {
            biased;
            () = self.close.cancelled() => Loaded::Failed("the run ended".to_string()),
            stop = race.reached() => Loaded::Stopped(stop),
            () = reply.closed() => Loaded::Failed(ABANDONED.to_string()),
            () = time::sleep_until(until) => Loaded::Failed(BUDGET_SPENT.to_string()),
            built = loading => match built {
                Ok((realm, registrations)) => {
                    self.realm = Some(realm);
                    Loaded::Ready(registrations)
                }
                Err(detail) => match race.stop() {
                    Some(stop) => Loaded::Stopped(stop),
                    None if Instant::now() >= until => Loaded::Failed(BUDGET_SPENT.to_string()),
                    None => Loaded::Failed(detail),
                },
            },
        };
        drop(host_job);
        self.watch.disarm();
        loaded
    }

    /// One invocation of `event`'s handlers with `payload`, within the budget (and the run's
    /// deadline when `race` is set). Anything but a decision aborts the invocation's signal.
    async fn invoke(
        &mut self,
        event: HookEvent,
        payload: &str,
        race: Option<&Deadline>,
        reply: &mut oneshot::Sender<Invoked>,
    ) -> Invoked {
        self.next_id += 1;
        let id = self.next_id;
        let until = self.until();
        let Some(realm) = &self.realm else {
            return Invoked::Failed(HookFailure::Realm("the hooks did not load".to_string()));
        };
        let host_job = self.host.arm(until, &self.close);
        self.watch.arm(until, race.cloned());
        let key = realm.key.as_str();
        let shipped = quickjs::Shipped::default();
        let marker = quickjs::binary_marker();
        // The realm boundary: the SDK's `invoke` runs the handlers, and its promise is awaited
        // here, which is when the realm's timers, call replies and promise jobs run.
        let entered = realm.context.async_with(async |ctx| {
            let promise = quickjs::ship(&ctx, &shipped)
                .and_then(|ship| {
                    let invoke = stashed(&ctx, key, "invoke")?;
                    invoke.call::<_, Promise<'_>>((
                        id,
                        event.name(),
                        payload,
                        ship,
                        marker.as_str(),
                    ))
                })
                .catch(&ctx)
                .map_err(|error| describe(&ctx, error))?;
            promise
                .into_future::<String>()
                .await
                .catch(&ctx)
                .map_err(|error| describe(&ctx, error))
        });
        let invoked = tokio::select! {
            biased;
            () = self.close.cancelled() => {
                Invoked::Failed(HookFailure::Realm("the run ended".to_string()))
            }
            stop = reached(race) => Invoked::Stopped(stop),
            () = reply.closed() => Invoked::Failed(HookFailure::Realm(ABANDONED.to_string())),
            () = time::sleep_until(until) => Invoked::Failed(HookFailure::Timeout),
            entered = entered => match entered {
                Ok(_) if Instant::now() >= until => Invoked::Failed(HookFailure::Timeout),
                Ok(text) => match shipped.take() {
                    Some(shipped) => Invoked::Decided(Decision {
                        text,
                        marker: marker.clone(),
                        shipped,
                    }),
                    None => Invoked::Failed(HookFailure::Realm(
                        "a replaced value's binaries were lost".to_string(),
                    )),
                },
                Err(detail) => match race.and_then(Deadline::stop) {
                    Some(stop) => Invoked::Stopped(stop),
                    None if Instant::now() >= until => Invoked::Failed(HookFailure::Timeout),
                    None => Invoked::Failed(HookFailure::Threw(detail)),
                },
            },
        };
        drop(host_job);
        self.watch.disarm();
        let reason = match &invoked {
            Invoked::Decided(_) => None,
            Invoked::Stopped(_) => Some(RUN_STOPPED),
            Invoked::Failed(HookFailure::Timeout) => Some(BUDGET_SPENT),
            Invoked::Failed(_) => Some(ABANDONED),
        };
        if let Some(reason) = reason
            && !self.close.is_cancelled()
        {
            // The listeners run now, synchronously, on a short grace of their own.
            self.watch.arm(Instant::now() + ABORT_GRACE, None);
            let aborted = realm
                .context
                .with(|ctx| stashed(&ctx, key, "abort")?.call::<_, ()>((id, reason)))
                .await;
            self.watch.disarm();
            if let Err(error) = aborted {
                self.failures
                    .push(format!("neo: aborting {} failed: {error}", event.name()));
            }
        }
        invoked
    }

    fn until(&self) -> Instant {
        let now = Instant::now();
        now.checked_add(self.budget).unwrap_or(now + FAR_FUTURE)
    }
}

/// Resolves when the run must stop, for a job that races it; never otherwise.
async fn reached(race: Option<&Deadline>) -> Stop {
    match race {
        Some(deadline) => deadline.reached().await,
        None => future::pending().await,
    }
}

/// Builds the realm: limits and the interrupt handler, the bridge, the platform globals, the
/// SDK's `createHooks`, then the hook file run with `neo` until its promise settles, then the
/// registrations `finishLoading` lists. Counts the realm once QuickJS has made it.
async fn build(
    setup: Setup,
    trace: Logs,
    watch: Watch,
    close: CancellationToken,
) -> Result<(Realm, Vec<Registration>), String> {
    let (runtime, context) =
        quickjs::new_realm(Box::new(move || close.is_cancelled() || watch.expired()))
            .await
            .map_err(|error| error.message)?;
    setup.realms.fetch_add(1, Ordering::Relaxed);
    let key = format!("neo:hooks:{:016x}", RandomState::new().hash_one(0_u8));
    let registrations = context
        .async_with(async |ctx| {
            let loading = start(&ctx, setup, trace, &key)
                .catch(&ctx)
                .map_err(|error| describe(&ctx, error))?;
            loading
                .into_future::<()>()
                .await
                .catch(&ctx)
                .map_err(|error| describe(&ctx, error))?;
            stashed(&ctx, &key, "finishLoading")
                .and_then(|finish| finish.call::<_, String>(()))
                .catch(&ctx)
                .map_err(|error| describe(&ctx, error))
        })
        .await?;
    let registrations = serde_json::from_str(&registrations)
        .map_err(|error| format!("finishLoading listed {registrations}: {error}"))?;
    Ok((
        Realm {
            key,
            context,
            _runtime: runtime,
        },
        registrations,
    ))
}

/// Sets the realm up and starts the hook file: returns its promise, raced with the realm's
/// uncaught-error promises.
fn start<'js>(
    ctx: &Ctx<'js>,
    setup: Setup,
    trace: Logs,
    key: &str,
) -> rquickjs::Result<Promise<'js>> {
    match setup.bridge {
        Bridge::Run {
            getters,
            dispatcher,
        } => {
            quickjs_bridge::install_hooks(ctx, getters, trace, dispatcher, setup.host)?;
        }
        #[cfg(test)]
        Bridge::Bare => {
            let bridge = Object::new(ctx.clone())?;
            quickjs_bridge::install_host(&bridge, setup.host)?;
            bridge.set(
                "log",
                Function::new(ctx.clone(), move |line: String| trace.push(line))?,
            )?;
            ctx.globals().set("__neo_runtime", bridge)?;
        }
    }
    // The hook realm's timers are not cut at the run's deadline as the agent's are: `afterRun`
    // runs after it, with a budget of its own, and the realm only runs inside a job anyway.
    let sleep = Function::new(
        ctx.clone(),
        Async(|delay_ms: f64| async move {
            let delay = quickjs::delay_from_millis(delay_ms);
            match Instant::now().checked_add(delay) {
                Some(due) => time::sleep_until(due).await,
                None => future::pending().await,
            }
        }),
    )?;
    let [timer_uncaught, abort_uncaught] = quickjs::install_platform_globals(ctx, sleep)?;
    let sdk: Object<'js> = ctx.eval(setup.sdk.as_ref())?;
    let create_hooks: Function<'js> = sdk.get("createHooks")?;
    let init = quickjs::sdk_init(ctx, setup.page_guid, setup.frame_guid, setup.url)?;
    init.set("meta", ctx.json_parse(setup.meta.to_string())?)?;
    let hooks: Object<'js> = create_hooks.call((init,))?;
    quickjs::install_sdk_globals(ctx, &hooks)?;
    let neo: rquickjs::Value<'js> = hooks.get("neo")?;
    let sdk_uncaught: rquickjs::Value<'js> = hooks.get("uncaught")?;
    let stash: Function<'js> = ctx.eval(STASH)?;
    stash.call::<_, ()>((
        key,
        hooks,
        vec![sdk_uncaught, abort_uncaught, timer_uncaught],
    ))?;
    let body = quickjs::async_function(ctx, ("neo", setup.code))?;
    let loading: Promise<'js> = body.call((neo,))?;
    stashed(ctx, key, "race")?.call((loading,))
}

/// One function of the SDK's hooks the realm keeps under `key` ([`STASH`]).
fn stashed<'js>(ctx: &Ctx<'js>, key: &str, name: &str) -> rquickjs::Result<Function<'js>> {
    ctx.globals().get::<_, Object<'js>>(key)?.get(name)
}

/// A thrown value as the hook trace keeps it: `name: message`.
fn describe<'js>(ctx: &Ctx<'js>, error: CaughtError<'js>) -> String {
    let error = quickjs::script_error(ctx, error);
    format!("{}: {}", error.name, error.message)
}
