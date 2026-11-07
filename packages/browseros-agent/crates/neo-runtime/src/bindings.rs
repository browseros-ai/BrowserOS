//! Neo: a run's exposed bindings (`page.exposeBinding`, `page.exposeFunction` and the
//! context's): Playwright's `PageBinding`, and the setup Playwright's Chromium `FrameSession`
//! does for it on each session.
//!
//! ```text
//!  expose(name)  the run's first: the controller's init script (bindings-controller.js) and
//!                this file's reader of the run's sessions
//!                the name's init script (bindings-call.js `add`); every page the run has a
//!                session on shows the run's scripts (init_scripts.rs); in each frame of their
//!                current documents: the controller, then the name (main world)
//!                each session that shows a page: Runtime.addBinding(__playwright__binding__)
//!  page          window[name](...args) → the controller → __playwright__binding__(payload)
//!  Chromium      Runtime.bindingCalled{payload, executionContextId} on that session
//!  here          the calling frame (contexts.rs) → `bindingCall` on the page, or on the
//!                context for its binding (EventHub), the call kept by its `bindingcall@` guid
//!  realm         BindingCall.call(callback) → resolve{result} | reject{error}
//!  here          bindings-call.js `deliver` in the calling frame's main world
//!  run end       after init_scripts.rs removed every script: bindings-call.js `end` in every
//!                frame (each call still waiting rejects; the names and the controller go)
//! ```
//!
//! A binding needs `Runtime.enable` on its session (probe C2). The tab's comes from net's
//! `PageSession::enable_runtime`. This file sends it only on the out-of-process iframe sessions
//! `init_scripts.rs` auto-attached, which no other reader takes events from, and only in a run
//! that exposed a binding: such an iframe needs the binding from its first script, before Neo
//! attaches a frame host of its own (DECIDED with med-pw-eval for C2).
//!
//! The page JavaScript guard allows this file, and only it, to send `Runtime.addBinding` and
//! `Runtime.removeBinding`; it runs no script itself.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use browseros_cdp::{CdpClient, CdpEvent, SessionId};
use neo_runtime_spec::{MethodGroup, MethodMeta};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::bridge;
use crate::contexts::ContextFrames;
use crate::evaluate::{self, Handles, SerializedArgument};
use crate::events::{CONTEXT_GUID, Emitted};
use crate::frames::FrameScope;
use crate::init_scripts::{self, InitScripts, Link, Rank, until_gone};
use crate::main_world::BuiltExpression;
use crate::page::{FrameTab, PageSession};
use crate::progress::{CallError, PlaywrightError, Progress};
use crate::run::PageId;
use crate::tabs::Tabs;

/// Playwright: `server/page.ts` `PageBinding.kBindingName`: `static kBindingName = '__playwright__binding__';`
pub(crate) const BINDING_NAME: &str = "__playwright__binding__";

/// Neo: the guid prefix of a binding call the realm answers by guid.
pub(crate) const BINDING_CALL_PREFIX: &str = "bindingcall@";

/// Neo: an exposed binding's `Disposable` guid, inside `init_scripts.rs`'s `disposable@` space,
/// which `dispatch.rs` routes here.
const DISPOSABLE: &str = "disposable@binding.";

/// Neo: the reason a call still waiting at run end rejects with in the page: Playwright's words
/// for a call whose page or context went away.
const RUN_ENDED: &str = "Target page, context or browser has been closed";

/// Neo: `Page.exposeBinding`'s trace metadata (`neo-runtime-spec/spec/playwright/page.yml`).
pub(crate) static PAGE_EXPOSE_BINDING_META: MethodMeta = MethodMeta {
    interface: "Page",
    method: "exposeBinding",
    title: Some("Expose binding"),
    subtitle: None,
    group: Some(MethodGroup::Configuration),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `BrowserContext.exposeBinding`'s trace metadata
/// (`neo-runtime-spec/spec/playwright/browserContext.yml`).
pub(crate) static CONTEXT_EXPOSE_BINDING_META: MethodMeta = MethodMeta {
    interface: "BrowserContext",
    method: "exposeBinding",
    title: Some("Expose binding"),
    subtitle: None,
    group: Some(MethodGroup::Configuration),
    render_params: &[],
    internal: false,
    flags: &[],
};
/// Neo: `BindingCall.resolve`'s trace metadata (`neo-runtime-spec/spec/playwright/playwright.yml`).
pub(crate) static BINDING_CALL_RESOLVE_META: MethodMeta = MethodMeta {
    interface: "BindingCall",
    method: "resolve",
    title: None,
    subtitle: None,
    group: None,
    render_params: &[],
    internal: true,
    flags: &[],
};
/// Neo: `BindingCall.reject`'s trace metadata (`neo-runtime-spec/spec/playwright/playwright.yml`).
pub(crate) static BINDING_CALL_REJECT_META: MethodMeta = MethodMeta {
    interface: "BindingCall",
    method: "reject",
    title: None,
    subtitle: None,
    group: None,
    render_params: &[],
    internal: true,
    flags: &[],
};

/// A binding command, routed by `dispatch.rs`.
#[derive(Debug)]
pub(crate) enum BindingRoute {
    /// `Page.exposeBinding`, on the call's page.
    Page { name: String, no_global: bool },
    /// `BrowserContext.exposeBinding`.
    Context { name: String, no_global: bool },
    /// `BindingCall.resolve` of the call with this guid.
    Resolve {
        guid: String,
        result: SerializedArgument,
    },
    /// `BindingCall.reject`.
    Reject { guid: String, error: Thrown },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExposeParams {
    name: String,
    #[serde(default)]
    no_global: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveParams {
    result: SerializedArgument,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RejectParams {
    error: Thrown,
}

/// What a callback threw, as the SDK sends it: an `Error`'s fields, or any other value in the
/// page's format (Playwright's client `serializeError`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Thrown {
    #[serde(default)]
    error: Option<ThrownError>,
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThrownError {
    message: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    stack: Option<String>,
}

impl Thrown {
    /// The page-format value the page's call rejects with: an `Error` of the same name,
    /// message and stack, or the thrown value itself.
    fn page_value(self) -> Value {
        match (self.error, self.value) {
            (Some(error), _) => json!({ "e": {
                "n": error.name.unwrap_or_else(|| "Error".to_string()),
                "m": error.message,
                "s": error.stack.unwrap_or_default(),
            }}),
            (None, Some(value)) => value,
            (None, None) => json!({ "v": "undefined" }),
        }
    }
}

/// `exposeBinding`'s params, on the context or a page.
///
/// # Errors
///
/// Playwright's plain `Error` for params that do not match.
pub(crate) fn decode_expose(
    context: bool,
    params: Option<Value>,
) -> (&'static MethodMeta, Result<BindingRoute, PlaywrightError>) {
    let meta = if context {
        &CONTEXT_EXPOSE_BINDING_META
    } else {
        &PAGE_EXPOSE_BINDING_META
    };
    let route = decode::<ExposeParams>(meta, params).map(|params| {
        let (name, no_global) = (params.name, params.no_global.unwrap_or(false));
        if context {
            BindingRoute::Context { name, no_global }
        } else {
            BindingRoute::Page { name, no_global }
        }
    });
    (meta, route)
}

/// A `BindingCall` command on `guid`, or `None` for a method it does not have.
pub(crate) fn decode_call(
    guid: &str,
    method: &str,
    params: Option<Value>,
) -> Option<(&'static MethodMeta, Result<BindingRoute, PlaywrightError>)> {
    let guid = guid.to_string();
    match method {
        "resolve" => Some((
            &BINDING_CALL_RESOLVE_META,
            decode::<ResolveParams>(&BINDING_CALL_RESOLVE_META, params).map(|params| {
                BindingRoute::Resolve {
                    guid,
                    result: params.result,
                }
            }),
        )),
        "reject" => Some((
            &BINDING_CALL_REJECT_META,
            decode::<RejectParams>(&BINDING_CALL_REJECT_META, params).map(|params| {
                BindingRoute::Reject {
                    guid,
                    error: params.error,
                }
            }),
        )),
        _ => None,
    }
}

fn decode<T: DeserializeOwned>(
    meta: &MethodMeta,
    params: Option<Value>,
) -> Result<T, PlaywrightError> {
    serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(|error| {
        PlaywrightError::error(format!("{}.{}: {error}", meta.interface, meta.method))
    })
}

/// Whether `guid` is an exposed binding's `Disposable`.
pub(crate) fn owns(guid: &str) -> bool {
    guid.starts_with(DISPOSABLE)
}

/// The run's exposed bindings and the sessions that carry the CDP binding.
pub(crate) struct Bindings {
    /// This run's random token in every guid, as `init_scripts.rs`'s carry one.
    run: String,
    next: AtomicU64,
    /// Serializes expose, dispose and run end, which each change the run's scripts and pages.
    work: Mutex<()>,
    state: Mutex<State>,
    /// The contexts of the iframe sessions this file enabled, which net's watcher does not read.
    contexts: ContextFrames,
    /// Stops the session reader, at run end.
    stop: CancellationToken,
}

#[derive(Default)]
struct State {
    /// Set at run end: nothing is exposed after the run removed its bindings.
    closed: bool,
    /// The controller's init script is in and the session reader runs.
    started: bool,
    exposed: Vec<Exposed>,
    /// Each session given the CDP binding, by id.
    sessions: HashMap<String, Prepared>,
    /// Each call the realm has not answered yet, by its `bindingcall@` guid.
    calls: HashMap<String, Call>,
}

struct Exposed {
    name: String,
    /// The page's target id for `page.exposeBinding`; `None` for the context's.
    page: Option<String>,
    disposable: String,
    /// Its `add` init script's guid (`init_scripts.rs`).
    script: String,
}

impl Exposed {
    fn reaches(&self, target: &str) -> bool {
        self.page.as_deref().is_none_or(|page| page == target)
    }
}

#[derive(Clone, Debug)]
struct Prepared {
    /// The page's target id.
    target: String,
    /// An iframe session of `init_scripts.rs`, not the tab's own.
    child: bool,
}

struct Call {
    name: String,
    seq: u64,
    target: String,
    frame: String,
}

/// A page's call, as the controller sends it.
/// Playwright: `injected/src/bindingsController.ts` `BindingsController.callBinding`: `const payload: BindingPayload = { name: bindingName, seq, serializedArgs };`
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    name: String,
    seq: u64,
    #[serde(default)]
    serialized_args: Value,
}

/// One `Runtime.bindingCalled` of a session of the run.
struct Incoming {
    session: String,
    prepared: Prepared,
    context: i64,
    payload: String,
}

/// The answer the realm gave a call.
pub(crate) enum Outcome {
    Result(SerializedArgument),
    Error(Thrown),
}

impl Default for Bindings {
    fn default() -> Self {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(NEXT_RUN.fetch_add(1, Ordering::Relaxed));
        Self {
            run: format!("{:016x}", hasher.finish()),
            next: AtomicU64::new(1),
            work: Mutex::default(),
            state: Mutex::default(),
            contexts: ContextFrames::default(),
            stop: CancellationToken::new(),
        }
    }
}

impl Drop for Bindings {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// Neo: the process-wide count that keeps two runs' tokens apart within one process.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

impl Bindings {
    /// Playwright's `exposeBinding` on `page` (or on the context, without): the binding's
    /// `Disposable` guid. Every page the run has a session on, and every page it touches later,
    /// gets the controller and the CDP binding; a page's binding reaches that page, a context's
    /// every page.
    ///
    /// # Errors
    ///
    /// Playwright's registration errors, and `TargetClosedError` after the run ended.
    pub(crate) async fn expose(
        self: &Arc<Self>,
        tabs: &Tabs,
        scripts: &Arc<InitScripts>,
        page: Option<&Arc<PageSession>>,
        name: &str,
        no_global: bool,
    ) -> Result<String, PlaywrightError> {
        let _work = self.work.lock().await;
        let target = page.map(|page| init_scripts::target_of(page));
        let first = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(PlaywrightError::target_closed());
            }
            refusal(&state.exposed, target.as_deref(), name)?;
            !state.started
        };
        let cdp = tabs.cdp();
        if first {
            scripts.attach_bindings(self);
            // Subscribed before any session gets the binding, so no call is missed.
            tokio::spawn(watch(Arc::downgrade(self), tabs.clone(), cdp.events()));
            scripts
                .add_ranked(cdp, None, &controller_source(), Rank::Controller)
                .await?;
            self.state.lock().await.started = true;
        }
        let rank = if page.is_some() {
            Rank::PageBinding
        } else {
            Rank::ContextBinding
        };
        let script = scripts
            .add_ranked(cdp, page, &add_source(name, no_global), rank)
            .await?;
        let disposable = format!(
            "{DISPOSABLE}{}.{}",
            self.run,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        {
            let mut state = self.state.lock().await;
            // The run ended while the scripts went in: its end removed them, and this name too.
            if state.closed {
                return Err(PlaywrightError::target_closed());
            }
            state.exposed.push(Exposed {
                name: name.to_string(),
                page: target.clone(),
                disposable: disposable.clone(),
                script,
            });
        }
        let add = add_arg(name, no_global);
        for (_, session) in tabs.attached() {
            let shown = init_scripts::target_of(&session);
            // Shows the run's scripts on a page touched before it had any (its tab gets the
            // binding there, `prepare_tab`), and gives its held iframes the binding.
            scripts.ensure(cdp, &session).await;
            self.prepare_tab(cdp, &session).await;
            for link in scripts.child_links(&shown).await {
                self.prepare_child(cdp, &link, &shown).await;
            }
            let mut calls = Vec::new();
            if first {
                calls.push((BuiltExpression::BindingsController, Vec::new()));
            }
            if target.as_deref().is_none_or(|target| target == shown) {
                calls.push((
                    BuiltExpression::BindingsCall,
                    vec![json!("add"), add.clone()],
                ));
            }
            in_all_frames(&session, &calls).await;
        }
        Ok(disposable)
    }

    /// Gives the tab's session of `page` the CDP binding, once: after net's runtime enable,
    /// which the binding needs.
    pub(crate) async fn prepare_tab(&self, cdp: &CdpClient, page: &Arc<PageSession>) {
        let target = init_scripts::target_of(page);
        if !self.mark(page.session_id().as_str(), &target, false).await {
            return;
        }
        if let Err(error) = page.enable_runtime().await {
            debug!(%error, "a page's runtime did not turn on for its bindings");
        }
        add_binding(cdp, page.session_id(), None).await;
    }

    /// Gives an iframe session `init_scripts.rs` holds for `target`'s page the CDP binding,
    /// once, before the iframe runs: `Runtime.enable` (which it needs) and `Runtime.addBinding`.
    pub(crate) async fn prepare_child(&self, cdp: &CdpClient, link: &Link, target: &str) {
        if !self.mark(link.session.as_str(), target, true).await {
            return;
        }
        let _enabled = until_gone(
            &link.gone,
            cdp.send("Runtime.enable", json!({}), Some(&link.session)),
        )
        .await;
        add_binding(cdp, &link.session, Some(&link.gone)).await;
    }

    /// Records `session` as one with the CDP binding: `false` when it already was, or the run
    /// is over.
    async fn mark(&self, session: &str, target: &str, child: bool) -> bool {
        let mut state = self.state.lock().await;
        if state.closed || state.sessions.contains_key(session) {
            return false;
        }
        state.sessions.insert(
            session.to_string(),
            Prepared {
                target: target.to_string(),
                child,
            },
        );
        true
    }

    async fn prepared(&self, session: &str) -> Option<Prepared> {
        self.state.lock().await.sessions.get(session).cloned()
    }

    async fn forget(&self, session: &str) {
        self.state.lock().await.sessions.remove(session);
        self.contexts.forget(session);
    }

    /// A page called a binding: the realm gets the call, or the page its error at once.
    async fn called(self: Arc<Self>, tabs: Tabs, incoming: Incoming) {
        let Incoming {
            session,
            prepared,
            context,
            payload,
        } = incoming;
        let Some(page) = page_of(&tabs, &prepared.target) else {
            return;
        };
        let frame = if prepared.child {
            self.contexts.frame_of(&session, context)
        } else {
            // net's watcher applies the tab's contexts; it has applied what came before once
            // the hub's barrier returns.
            tabs.events().barrier().await;
            page.events.contexts.frame_of(&session, context)
        }
        .unwrap_or_else(|| prepared.target.clone());
        let tab = frame_tab(&page, &prepared.target, &frame);
        let Ok(payload) = serde_json::from_str::<Payload>(&payload) else {
            debug!("a binding call's payload is not the controller's");
            return;
        };
        let binding = {
            let state = self.state.lock().await;
            state
                .exposed
                .iter()
                .find(|exposed| exposed.name == payload.name && exposed.reaches(&prepared.target))
                .map(|exposed| exposed.page.is_some())
        };
        let refused = match binding {
            // Playwright: `server/page.ts` `PageBinding.dispatch`: `" is not exposed`
            None => Some(format!("Function \"{}\" is not exposed", payload.name)),
            // Playwright: `server/page.ts` `PageBinding.dispatch`: `serializedArgs is not an array. This can happen when Array.prototype.toJSON is defined incorrectly`
            Some(_) if !payload.serialized_args.is_array() => Some(
                "serializedArgs is not an array. This can happen when Array.prototype.toJSON is defined incorrectly"
                    .to_string(),
            ),
            Some(_) => None,
        };
        if let Some(message) = refused {
            deliver_error(&tab, &payload.name, payload.seq, &message).await;
            return;
        }
        let guid = format!(
            "{BINDING_CALL_PREFIX}{}.{}",
            self.run,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let page_id = PageId::new(prepared.target.clone());
        let page_guid = bridge::page_guid(&page_id);
        let owner = if binding == Some(true) {
            page_guid.clone()
        } else {
            CONTEXT_GUID.to_string()
        };
        let params = json!({ "binding": {
            "guid": guid,
            "frame": { "guid": bridge::frame_guid_of(&page_id, &frame) },
            "page": { "guid": page_guid },
            "name": payload.name,
            "args": payload.serialized_args,
        }});
        {
            let mut state = self.state.lock().await;
            if state.closed {
                return;
            }
            state.calls.insert(
                guid.clone(),
                Call {
                    name: payload.name.clone(),
                    seq: payload.seq,
                    target: prepared.target.clone(),
                    frame: frame.clone(),
                },
            );
        }
        if tabs.events().emit(&owner, "bindingCall", &params) != Emitted::Queued {
            self.state.lock().await.calls.remove(&guid);
            deliver_error(&tab, &payload.name, payload.seq, RUN_ENDED).await;
        }
    }

    /// The realm's answer to the call `guid`: delivered to the page's waiting call in its frame.
    /// A call answered already, or one of an ended run, is done.
    ///
    /// # Errors
    ///
    /// None the realm sees: Playwright's client ignores its answer's outcome.
    pub(crate) async fn answer(
        &self,
        tabs: &Tabs,
        guid: &str,
        outcome: Outcome,
        handles: &Handles,
        progress: &Progress,
    ) -> Result<(), CallError> {
        let Some(call) = self.state.lock().await.calls.remove(guid) else {
            return Ok(());
        };
        let Some(page) = page_of(tabs, &call.target) else {
            return Ok(());
        };
        let tab = frame_tab(&page, &call.target, &call.frame);
        let (key, value, carried) = match outcome {
            Outcome::Result(result) => ("result", result.value, result.handles),
            Outcome::Error(thrown) => ("error", thrown.page_value(), Vec::new()),
        };
        let arg = SerializedArgument {
            value: page_object(
                0,
                vec![
                    ("name", json!(call.name)),
                    ("seq", json!(call.seq)),
                    (key, value),
                ],
            ),
            handles: carried,
        };
        // Playwright: `server/page.ts` `PageBinding.dispatch`: `.catch(e => debugLogger.log('error', e));`
        if let Err(error) = evaluate::call_built(
            &tab,
            BuiltExpression::BindingsCall,
            vec![json!("deliver")],
            &arg,
            handles,
            progress,
        )
        .await
        {
            debug!(?error, "a binding call's answer did not reach its page");
        }
        Ok(())
    }

    /// Playwright's `PageBinding.dispose`: the name's init script goes, and every frame of the
    /// pages it reached removes it.
    ///
    /// # Errors
    ///
    /// `TargetClosedError` for a binding that is gone, which Playwright's client takes as done.
    pub(crate) async fn dispose(
        &self,
        tabs: &Tabs,
        scripts: &InitScripts,
        guid: &str,
    ) -> Result<(), PlaywrightError> {
        let _work = self.work.lock().await;
        let exposed = {
            let mut state = self.state.lock().await;
            let index = state
                .exposed
                .iter()
                .position(|exposed| exposed.disposable == guid)
                .ok_or_else(PlaywrightError::target_closed)?;
            state.exposed.remove(index)
        };
        // Playwright: `server/page.ts` `Page.removeExposedBinding`: `await this.delegate.removeInitScripts([binding.initScript]);`
        let _removed = scripts.dispose(&exposed.script).await;
        let remove = page_object(1, vec![("name", json!(exposed.name))]);
        for (_, session) in tabs.attached() {
            if exposed.reaches(&init_scripts::target_of(&session)) {
                in_all_frames(
                    &session,
                    &[(
                        BuiltExpression::BindingsCall,
                        vec![json!("remove"), remove.clone()],
                    )],
                )
                .await;
            }
        }
        Ok(())
    }

    /// Ends the run's bindings, after `init_scripts.rs` removed every script: in every frame of
    /// the run's pages, each call still waiting on one rejects, each name goes, and the
    /// controller with them; the tabs stop reporting calls. It never waits on an expose still
    /// running, which finds the run ended at its next step.
    /// Deviation: `server/page.ts` `Page.removeExposedBinding`: `await this.delegate.removeInitScripts([binding.initScript]);` Playwright's bindings live as long as their page or context; Neo's live as long as the run.
    pub(crate) async fn close(&self, tabs: &Tabs) {
        let (exposed, sessions) = {
            let mut state = self.state.lock().await;
            state.closed = true;
            state.calls.clear();
            (
                mem::take(&mut state.exposed),
                mem::take(&mut state.sessions),
            )
        };
        for (_, session) in tabs.attached() {
            let target = init_scripts::target_of(&session);
            let names: Vec<Value> = exposed
                .iter()
                .filter(|exposed| exposed.reaches(&target))
                .map(|exposed| json!(exposed.name))
                .collect();
            if !names.is_empty() {
                let end = page_object(
                    1,
                    vec![
                        ("names", json!({ "a": names, "id": 2 })),
                        ("message", json!(RUN_ENDED)),
                    ],
                );
                in_all_frames(
                    &session,
                    &[(BuiltExpression::BindingsCall, vec![json!("end"), end])],
                )
                .await;
            }
        }
        // The iframe sessions went with `init_scripts.rs`'s auto-attach.
        let cdp = tabs.cdp();
        for (session, prepared) in sessions {
            if !prepared.child {
                let _removed = cdp
                    .send(
                        "Runtime.removeBinding",
                        json!({ "name": BINDING_NAME }),
                        Some(&SessionId::from(session.as_str())),
                    )
                    .await;
            }
        }
        self.stop.cancel();
    }
}

/// Playwright's registration checks, for a page's binding (`target`) or the context's.
fn refusal(exposed: &[Exposed], target: Option<&str>, name: &str) -> Result<(), PlaywrightError> {
    let named = |page: bool| {
        exposed.iter().any(|exposed| {
            exposed.name == name
                && match (page, target) {
                    (true, Some(target)) => exposed.page.as_deref() == Some(target),
                    (true, None) => exposed.page.is_some(),
                    (false, _) => exposed.page.is_none(),
                }
        })
    };
    let message = match target {
        // Playwright: `server/page.ts` `Page.exposeBinding`: `has been already registered in the browser context`
        Some(_) if named(true) => format!("Function \"{name}\" has been already registered"),
        Some(_) if named(false) => {
            format!("Function \"{name}\" has been already registered in the browser context")
        }
        // Playwright: `server/browserContext.ts` `BrowserContext.exposeBinding`: `has been already registered in one of the pages`
        None if named(false) => format!("Function \"{name}\" has been already registered"),
        None if named(true) => {
            format!("Function \"{name}\" has been already registered in one of the pages")
        }
        _ => return Ok(()),
    };
    Err(PlaywrightError::error(message))
}

/// The run's controller init script.
/// Playwright: `server/page.ts` `PageBinding.createInitScript`: `globalThis[property] = new (module.exports.BindingsController())(globalThis, '${PageBinding.kBindingName}');`
fn controller_source() -> String {
    format!("({})()", BuiltExpression::BindingsController.source())
}

/// A name's init script.
/// Playwright: `server/page.ts` `PageBinding.constructor`: `.addBinding(${JSON.stringify(name)}, ${!!noGlobal})`
fn add_source(name: &str, no_global: bool) -> String {
    format!(
        "({})(\"add\", {})",
        BuiltExpression::BindingsCall.source(),
        json!({ "name": name, "noGlobal": no_global })
    )
}

/// `add`'s argument in the page's value format.
fn add_arg(name: &str, no_global: bool) -> Value {
    page_object(
        1,
        vec![("name", json!(name)), ("noGlobal", json!(no_global))],
    )
}

/// Neo: an object in the page's value format (`isomorphic/utilityScriptSerializers.ts`), whose
/// strings, numbers and booleans are themselves; `id` keeps it apart from the objects inside.
fn page_object(id: u64, fields: Vec<(&str, Value)>) -> Value {
    let fields: Vec<Value> = fields
        .into_iter()
        .map(|(key, value)| json!({ "k": key, "v": value }))
        .collect();
    json!({ "o": fields, "id": id })
}

async fn add_binding(cdp: &CdpClient, session: &SessionId, gone: Option<&CancellationToken>) {
    // Playwright: `server/chromium/crPage.ts` `FrameSession.exposePlaywrightBinding`: `await this._client.send('Runtime.addBinding', { name: PageBinding.kBindingName });`
    let add = cdp.send(
        "Runtime.addBinding",
        json!({ "name": BINDING_NAME }),
        Some(session),
    );
    let added = match gone {
        Some(gone) => until_gone(gone, add).await.is_some(),
        None => add.await.is_ok(),
    };
    if !added {
        debug!(%session, "a session did not take the binding");
    }
}

/// The page's call `seq` of `name` rejects with `message`, at once.
async fn deliver_error(tab: &FrameTab, name: &str, seq: u64, message: &str) {
    let error = json!({ "e": { "n": "Error", "m": message, "s": "" } });
    let arg = page_object(
        0,
        vec![("name", json!(name)), ("seq", json!(seq)), ("error", error)],
    );
    if let Err(error) = evaluate::run_built(
        tab,
        BuiltExpression::BindingsCall,
        vec![json!("deliver"), arg],
    )
    .await
    {
        debug!(?error, "a binding call's error did not reach its page");
    }
}

/// Runs `calls` in every frame of `page`'s current document, each frame in tree order;
/// Playwright's `safeNonStallingEvaluateInAllFrames`: a frame that fails is skipped.
async fn in_all_frames(page: &Arc<PageSession>, calls: &[(BuiltExpression, Vec<Value>)]) {
    if calls.is_empty() {
        return;
    }
    let target = init_scripts::target_of(page);
    let Ok(tree) = page.frames.tree().await else {
        return;
    };
    for frame in tree {
        let tab = frame_tab(page, &target, &frame.id);
        for (expression, leading) in calls {
            if let Err(error) = evaluate::run_built(&tab, *expression, leading.clone()).await {
                debug!(?error, frame = %frame.id, "a frame skipped a bindings call");
            }
        }
    }
}

fn frame_tab(page: &Arc<PageSession>, target: &str, frame: &str) -> FrameTab {
    FrameTab {
        page: Arc::clone(page),
        scope: if frame == target {
            FrameScope::Main
        } else {
            FrameScope::Frame(frame.to_string())
        },
    }
}

fn page_of(tabs: &Tabs, target: &str) -> Option<Arc<PageSession>> {
    tabs.attached()
        .into_iter()
        .map(|(_, session)| session)
        .find(|session| init_scripts::target_of(session) == target)
}

/// Neo: reads the binding calls and the iframe sessions' contexts of the run until it ends, on
/// its own task; each call is answered on a task of its own.
async fn watch(bindings: Weak<Bindings>, tabs: Tabs, mut events: broadcast::Receiver<CdpEvent>) {
    let Some(stop) = bindings.upgrade().map(|bindings| bindings.stop.clone()) else {
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
        let Some(bindings) = bindings.upgrade() else {
            return;
        };
        if event.method == "Target.detachedFromTarget" {
            if let Some(session) = event.params["sessionId"].as_str() {
                bindings.forget(session).await;
            }
            continue;
        }
        let Some(session) = event.session_id.as_ref().map(SessionId::as_str) else {
            continue;
        };
        let Some(prepared) = bindings.prepared(session).await else {
            continue;
        };
        match event.method.as_str() {
            "Runtime.executionContextCreated"
            | "Runtime.executionContextDestroyed"
            | "Runtime.executionContextsCleared"
                if prepared.child =>
            {
                bindings.contexts.apply(&event);
            }
            "Runtime.bindingCalled" if event.params["name"].as_str() == Some(BINDING_NAME) => {
                let (Some(context), Some(payload)) = (
                    event.params["executionContextId"].as_i64(),
                    event.params["payload"].as_str(),
                ) else {
                    continue;
                };
                let incoming = Incoming {
                    session: session.to_string(),
                    prepared,
                    context,
                    payload: payload.to_string(),
                };
                tokio::spawn(Arc::clone(&bindings).called(tabs.clone(), incoming));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exposed(name: &str, page: Option<&str>) -> Exposed {
        Exposed {
            name: name.into(),
            page: page.map(Into::into),
            disposable: String::new(),
            script: String::new(),
        }
    }

    #[test]
    fn a_name_registers_once_per_page_and_never_beside_the_contexts() {
        let page = [exposed("woof", Some("T1"))];
        let context = [exposed("woof", None)];
        let refused = |list: &[Exposed], target: Option<&str>| {
            refusal(list, target, "woof")
                .err()
                .map(|error| error.message)
        };
        assert_eq!(
            refused(&page, Some("T1")).as_deref(),
            Some("Function \"woof\" has been already registered")
        );
        assert_eq!(refused(&page, Some("T2")), None, "another page may take it");
        assert_eq!(
            refused(&context, Some("T1")).as_deref(),
            Some("Function \"woof\" has been already registered in the browser context")
        );
        assert_eq!(
            refused(&context, None).as_deref(),
            Some("Function \"woof\" has been already registered")
        );
        assert_eq!(
            refused(&page, None).as_deref(),
            Some("Function \"woof\" has been already registered in one of the pages")
        );
        assert!(refusal(&page, Some("T1"), "bark").is_ok());
    }

    #[test]
    fn scripts_apply_the_built_functions_to_json_data() {
        let add = add_source("a\"b", true);
        assert!(
            add.starts_with('(') && add.ends_with(r#")("add", {"name":"a\"b","noGlobal":true})"#),
            "{add}"
        );
        assert!(controller_source().ends_with(")()"));
    }

    #[test]
    fn a_thrown_value_reaches_the_page_as_an_error_or_as_itself() {
        let error: Thrown = serde_json::from_value(
            json!({"error": {"message": "boom", "name": "TypeError", "stack": "at x"}}),
        )
        .expect("an error");
        assert_eq!(
            error.page_value(),
            json!({"e": {"n": "TypeError", "m": "boom", "s": "at x"}})
        );
        let null: Thrown =
            serde_json::from_value(json!({"value": {"v": "null"}})).expect("a value");
        assert_eq!(null.page_value(), json!({"v": "null"}));
    }

    #[test]
    fn binding_commands_decode_and_refuse_other_params() {
        let (meta, route) = decode_expose(false, Some(json!({"name": "woof"})));
        assert_eq!(meta.interface, "Page");
        assert!(
            matches!(route, Ok(BindingRoute::Page { ref name, no_global: false }) if name == "woof")
        );
        let (meta, route) = decode_expose(true, Some(json!({"name": "woof", "noGlobal": true})));
        assert_eq!(meta.interface, "BrowserContext");
        assert!(matches!(
            route,
            Ok(BindingRoute::Context {
                no_global: true,
                ..
            })
        ));
        assert!(
            decode_expose(false, Some(json!({"name": "woof", "x": 1})))
                .1
                .is_err()
        );
        let resolve = decode_call(
            "bindingcall@r.1",
            "resolve",
            Some(json!({"result": {"value": 1, "handles": []}})),
        );
        assert!(matches!(
            resolve,
            Some((_, Ok(BindingRoute::Resolve { .. })))
        ));
        assert!(decode_call("bindingcall@r.1", "dispose", None).is_none());
    }
}
