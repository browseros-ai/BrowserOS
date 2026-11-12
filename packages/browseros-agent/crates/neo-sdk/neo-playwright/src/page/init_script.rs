//! Init scripts and bindings: `page.addInitScript`, `context.addInitScript`, `exposeBinding` and
//! `exposeFunction` on a page and on the context, and the disposable each returns. An init script
//! runs in the page's main world in every new document of its tab, before the page's own scripts,
//! through CDP's `Page.addScriptToEvaluateOnNewDocument`, as upstream's `crPage` registers one. A
//! binding is upstream's too: the CDP binding `__playwright__binding__`, the vendored
//! `BindingsController` (`assets/bindings-controller.js`) and one stub per binding, each an init
//! script that also runs at once in the documents already loaded. This is a gateway of
//! `Capability::MainWorld` (beside the evaluate guard), and, for the `{ path }` form, of
//! `Capability::HostFiles`.
//!
//! ```text
//!  page.addInitScript     require(MainWorld, HostFiles) ─ read a path ─ admit the page
//!                         ─ send_effect (the door's rules on the tab) ─ add on its session
//!  page.exposeBinding     require(MainWorld) ─ admit ─ send_effect ─ plumb the tab session once:
//!                         listen for its binding calls, Runtime.addBinding, the controller
//!                         ─ add the stub, run at once in the loaded documents
//!  context.addInitScript, context.exposeBinding   send_effect ─ kept in the run's registry
//!     └▶ every write op later admitted on a tab without it: recheck that op's tab, then add it
//!        there (OpScope::admit), before the op's own effect, so the op's navigation runs it
//!  a page calls a binding ─ Runtime.bindingCalled ─ the watcher records it for the run
//!     ─ the prelude's pump runs the agent's function ─ bindingCall.resolve answers it in the
//!       calling context (evaluate_guard::resolve_binding)
//!  disposable.dispose     every registration of that id removed, and awaited; a binding's stub
//!                         also leaves the loaded documents
//!  the run ends           the host awaits RunState::remove_init_scripts before it answers;
//!                         dropping the state then removes whatever that missed (guards)
//! ```
//!
//! **Lifetime: the run.** A Neo run is a whole Playwright script, and the tab outlives it, so
//! a script registered by one run must not run in documents another run's script loads later,
//! and a binding whose run (the realm that answers it) ended must not stay callable. Every
//! registration lives in the run's [`RunState`](crate::RunState) and is removed when the run ends,
//! however it ends (returned, failed, cancelled, timed out): the host awaits the removals before
//! it answers, and a registration's guard removes it on drop in any case. Within the run it stays
//! registered across navigations, so a later `goto` in the same run gets it, as a model writing a
//! Playwright script expects. The CDP binding itself stays on the session: it only posts calls,
//! which the watcher records only while a run exposes a binding there, and another run on the tab
//! may be using it.
//!
//! **The context's tabs.** Neo's context is the owner's whole profile, so a context script or
//! binding does not go to every tab at once: it goes to each tab the run writes to, at that write
//! op's admission, through that op's own recheck of its tab. Every navigation a run drives is a
//! write (`goto`, `reload`, a click, a script), so each gets it; a popup's first document, which
//! loads before the run acts on the popup, does not, where Playwright's would.
//!
//! **Sessions.** A registration belongs to the session it was added on: the tab's page session
//! covers the main frame and its same-process frames, across cross-process navigations of the
//! main frame. An out-of-process frame has a session of its own, which Neo does not add to, so a
//! cross-site iframe runs without the scripts and bindings. A tab whose page session changed
//! (core attached again) gets the context's registrations again on the new one.

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{BrowserSession, CoreError, ProtocolSession, SessionId, TabId, TargetId};
use futures_util::future;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;
use ts_rs::TS;

use crate::action::input_files;
use crate::events::event_hub::now_ms;
use crate::events::{BINDING_NAME, EventHub, PageEventKind, Subscription};
use crate::op_scope::{AdmittedPage, OpScope, Sent};
use crate::op_spec::Capability;
use crate::target::PageRef;
use crate::wire::{Nothing, options};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// The page side of every binding: upstream's `BindingsController`, which installs itself at
/// `globalThis.__playwright__binding__controller__` unless a run already did. Built by
/// crates/neo-sdk/neo-playwright-js/scripts/build.ts from the vendored source.
const BINDINGS_CONTROLLER_JS: &str = include_str!("../../assets/bindings-controller.js");

/// upstream's `kBindingsControllerProperty`.
const CONTROLLER: &str = "__playwright__binding__controller__";

/// The disposable id of a tab session's binding plumbing (the controller script): never handed
/// out, so only the run's end removes it.
const PLUMBING: u64 = 0;

/// The run's init scripts. Lives in [`RunState`](crate::RunState), so it ends with the run.
#[derive(Debug, Default)]
pub(crate) struct InitScripts {
    /// A std mutex, held for a list update and never across an await.
    registry: Mutex<Registry>,
}

#[derive(Debug, Default)]
struct Registry {
    /// The last disposable id handed out; ids start at 1 and are never reused in a run.
    last_id: u64,
    /// The context's scripts and bindings, in registration order.
    context: Vec<ContextScript>,
    /// Every script added to a tab: the page's own, the context's, binding stubs and plumbing.
    added: Vec<Added>,
    /// One single-flight cell per context registration and tab session: two write ops admitted
    /// on one tab at once share one add, the second awaiting the first's, so the tab runs the
    /// script once a document. A failed add leaves its cell empty, and the next write tries again.
    adding: HashMap<Adding, Arc<OnceCell<()>>>,
    /// One single-flight cell per tab session that has the bindings' plumbing for this run.
    plumbed: HashMap<(TabId, SessionId), Arc<OnceCell<()>>>,
    /// The event hub binding calls are recorded in, kept by the first `exposeBinding`, for a
    /// context binding a later write op adds, whose admission has no `Playwright` at hand.
    hub: Option<Arc<EventHub>>,
    /// The binding names each disposable id holds on each tab ([`EventHub::claim_binding`]),
    /// released when it is disposed so its calls stop reaching the run.
    claims: HashMap<u64, Vec<(TargetId, String)>>,
}

/// A context script on one tab's page session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Adding {
    id: u64,
    tab: TabId,
    session: SessionId,
}

/// One `context.addInitScript` or `context.exposeBinding`, which each tab the run writes to gets
/// once.
#[derive(Debug)]
struct ContextScript {
    id: u64,
    reach: Reach,
}

/// What a context registration adds to a tab.
#[derive(Clone, Debug)]
enum Reach {
    /// An init script's source.
    Script(Arc<str>),
    /// A binding's stub, after the tab session's plumbing.
    Binding(Arc<BindingName>),
}

/// A binding as `exposeBinding` names it.
#[derive(Debug)]
struct BindingName {
    name: String,
    /// upstream's `noGlobal`: the binding exists for the controller only, not as `window[name]`,
    /// as for the functions `addInitScript` passes with `exposeFunctions`.
    no_global: bool,
}

impl BindingName {
    /// upstream's `PageBinding` init script: the controller adds the binding, and unless
    /// `no_global`, `window[name]`.
    fn stub(&self) -> String {
        format!(
            "(() => {{\n  globalThis['{CONTROLLER}'].addBinding({}, {});\n}})();",
            json!(self.name),
            self.no_global
        )
    }

    /// upstream's `cleanupScript`, run in the documents loaded when the binding goes. The
    /// controller is there in every document the stub ran in; one loaded before the run's
    /// plumbing has neither, hence `?.`.
    fn cleanup(&self) -> String {
        format!(
            "{{ globalThis['{CONTROLLER}']?.removeBinding({}); }};",
            json!(self.name)
        )
    }
}

/// One script added to one tab's page session.
#[derive(Debug)]
struct Added {
    /// The disposable id: the page script's own, or the context script's.
    id: u64,
    registration: Registration,
}

/// One `Page.addScriptToEvaluateOnNewDocument` on one session. Dropping it removes the script
/// from the session in a spawned task, so a run that ends any way takes its scripts with it, as
/// `ObjectGroup` releases an op's objects.
#[derive(Debug)]
struct Registration {
    /// `None` once removed. The cancellation endpoint keeps the session it was added on, even
    /// across a reconnect, as cleanup must (`ProtocolSession::cancellation_endpoint`).
    session: Option<ProtocolSession>,
    identifier: String,
    /// What the documents already loaded run once the script goes: a binding's
    /// `removeBinding`, so the page can no longer call into a run that let it go.
    cleanup: Option<String>,
}

impl Registration {
    /// Removes the script now, and runs its cleanup. A session that is gone (the tab closed) has
    /// nothing to remove. The session is let go only once both answered, so a removal cut short
    /// by the op's drop is sent again by `Drop`; a second removal of one identifier, or a second
    /// `removeBinding`, changes nothing.
    async fn remove(self) {
        self.remove_reporting(|| {}).await;
    }

    /// [`Registration::remove`] driven by `scope`'s op, which reports the main world right
    /// before the cleanup goes out to the loaded documents, so a removal that stalls first
    /// reports nothing. The run's end ([`InitScripts::remove_all`]) and a dropped registration
    /// remove with no op to report to, so their cleanup goes unflagged.
    async fn remove_by(self, scope: &OpScope<'_>) {
        self.remove_reporting(|| scope.sending_main_world()).await;
    }

    /// The session is cleared only after the removal: a future dropped mid-removal leaves it
    /// for `Drop` to retry.
    async fn remove_reporting(mut self, before_cleanup: impl FnOnce()) {
        if let Some(session) = &self.session {
            remove(
                session,
                &self.identifier,
                self.cleanup.as_deref(),
                before_cleanup,
            )
            .await;
        }
        self.session = None;
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            return;
        };
        let identifier = mem::take(&mut self.identifier);
        let cleanup = self.cleanup.take();
        drop(runtime.spawn(async move {
            remove(&session, &identifier, cleanup.as_deref(), || {}).await;
        }));
    }
}

/// `before_cleanup` runs right before a binding's cleanup is sent into the main world, and not
/// at all for a script with no cleanup.
async fn remove(
    session: &ProtocolSession,
    identifier: &str,
    cleanup: Option<&str>,
    before_cleanup: impl FnOnce(),
) {
    let _removed = session
        .send::<_, Value>(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
        )
        .await;
    if let Some(cleanup) = cleanup {
        before_cleanup();
        run_in_loaded_documents(session, cleanup).await;
    }
}

/// Runs `source` once in the main world of every document the session has loaded now, and in no
/// later one: added with `runImmediately`, then removed. Upstream evaluates a binding's cleanup
/// in every frame (`safeNonStallingEvaluateInAllFrames`); this reaches the same documents without
/// a `Runtime.evaluate`, and, like upstream's, ignores a document that fails it.
async fn run_in_loaded_documents(session: &ProtocolSession, source: &str) {
    #[derive(Deserialize)]
    struct AddedScript {
        identifier: String,
    }
    let added: Result<AddedScript, CoreError> = session
        .send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source, "runImmediately": true }),
        )
        .await;
    if let Ok(AddedScript { identifier }) = added {
        let _removed = session
            .send::<_, Value>(
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({ "identifier": identifier }),
            )
            .await;
    }
}

/// The script's source as the prelude built it (upstream's client `evaluationScript`), or the
/// path of a file to read it from. No `Debug`: the source may hold a value the agent was given.
pub(crate) enum InitSource {
    /// A function call with its argument, a string, or `{ content }`.
    Source(String),
    /// `{ path }`, read as upstream's client reads it: as UTF-8, relative to Neo's working
    /// directory, with `//# sourceURL=<path>` appended.
    Path(String),
}

impl InitSource {
    /// The one of `source` and `path` an input carries: upstream's `source`, or the path Rust
    /// reads it from, since the prelude has no filesystem. Exactly one travels.
    fn of(
        scope: &OpScope<'_>,
        source: Option<String>,
        path: Option<String>,
    ) -> Result<Self, PlaywrightError> {
        match (source, path) {
            (Some(source), None) => Ok(Self::Source(source)),
            (None, Some(path)) => Ok(Self::Path(path)),
            (Some(_), Some(_)) => Err(scope.fail(
                ErrorCode::InvalidInput,
                "source and path: an init script is one or the other",
            )),
            (None, None) => Err(scope.fail(ErrorCode::InvalidInput, "missing field `source`")),
        }
    }
}

/// `page.addInitScript`'s input: upstream's `source`, or Neo's `path` in its place.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageScriptInput {
    page: PageRef,
    #[serde(default)]
    #[ts(optional)]
    source: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    path: Option<String>,
}
options!(PageScriptInput);

/// `context.addInitScript`'s input: upstream's `source`, or Neo's `path` in its place.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextScriptInput {
    #[serde(default)]
    #[ts(optional)]
    source: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    path: Option<String>,
}
options!(ContextScriptInput);

/// `disposable.dispose`'s input: the id an `addInitScript` or `exposeBinding` returned, which
/// stands for upstream's `Disposable` channel.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisposeInput {
    id: u64,
}
options!(DisposeInput);

/// `page.exposeBinding`'s input: `exposeFunction` sends it too.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct PageBindingInput {
    page: PageRef,
    name: String,
    /// The prelude sends it only when set, so it prints as optional.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    no_global: bool,
}
options!(PageBindingInput);

/// `context.exposeBinding`'s input.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ContextBindingInput {
    name: String,
    /// The prelude sends it only when set, so it prints as optional.
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    no_global: bool,
}
options!(ContextBindingInput);

/// What `addInitScript` and `exposeBinding` return: the id their disposable disposes, standing
/// for upstream's `Disposable` channel, upstream's one result field unwrapped.
#[derive(Debug, Serialize, TS)]
pub(crate) struct Registered(u64);

/// The page and context route rows' reason for [`Registered`], where upstream answers its
/// disposable channel.
pub(crate) const REGISTRATION: &str = "the registration's id, standing for upstream's Disposable channel; disposable.dispose takes it";

/// `page.addInitScript`: the script added to the page's session now, after the door's rules on
/// the page. Every later document of the tab runs it until the run ends or it is disposed.
pub(crate) async fn add_to_page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageScriptInput,
) -> Result<Registered, PlaywrightError> {
    let script = InitSource::of(scope, input.source, input.path)?;
    let source = gate(scope, script).await?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let sent = scope.send_effect()?;
    let registration = add(&session, &source, &sent)
        .await
        .map_err(|error| scope.core_error(&error))?;
    let id = scope.run_state().init_scripts.keep(registration);
    Ok(Registered(id))
}

/// `page.exposeBinding`: the binding's stub added to the page's session, after the door's rules
/// on the page and the session's plumbing, and run at once in the documents it has loaded, as
/// upstream's `Page.exposeBinding` adds its init script and evaluates it in every frame.
pub(crate) async fn expose_on_page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageBindingInput,
) -> Result<Registered, PlaywrightError> {
    scope.require(Capability::MainWorld)?;
    let scripts = &scope.run_state().init_scripts;
    let hub = scripts.keep_hub(pw);
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let sent = scope.send_effect()?;
    let binding = BindingName {
        name: input.name,
        no_global: input.no_global,
    };
    let registration = expose(scope, &hub, admitted, &session, &binding, &sent).await?;
    let id = scripts.keep(registration);
    scripts
        .lock()
        .claims
        .entry(id)
        .or_default()
        .push((admitted.session.target_id.clone(), binding.name));
    Ok(Registered(id))
}

/// `context.exposeBinding`: the binding kept for every tab the run writes to from now on, as
/// `context.addInitScript` keeps a script. Marked sent; no tab is touched here.
pub(crate) fn expose_on_context(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: ContextBindingInput,
) -> Result<Registered, PlaywrightError> {
    scope.require(Capability::MainWorld)?;
    let scripts = &scope.run_state().init_scripts;
    let _hub = scripts.keep_hub(pw);
    let _sent = scope.send_effect()?;
    let binding = BindingName {
        name: input.name,
        no_global: input.no_global,
    };
    let id = scripts.keep_for_context(Reach::Binding(Arc::new(binding)));
    Ok(Registered(id))
}

/// `context.addInitScript`: the script kept for every tab the run writes to from now on. Marked
/// sent, since it changes what the run's later writes do; no tab is touched here.
pub(crate) async fn add_to_context(
    scope: &OpScope<'_>,
    input: ContextScriptInput,
) -> Result<Registered, PlaywrightError> {
    let script = InitSource::of(scope, input.source, input.path)?;
    let source = gate(scope, script).await?;
    let _sent = scope.send_effect()?;
    let id = scope
        .run_state()
        .init_scripts
        .keep_for_context(Reach::Script(source.into()));
    Ok(Registered(id))
}

/// `disposable.dispose`: removes every registration of `id`, page or context, and awaits each
/// removal, as upstream's `removeInitScripts` does, so no document loaded after it resolves runs
/// the script. That includes a context script a concurrent write is adding right now: dispose
/// waits for that add, which finds its script gone and removes it again before it finishes. An id
/// already disposed, or one the run never handed out, disposes nothing, as a second upstream
/// `dispose()` does.
pub(crate) async fn dispose(
    scope: &OpScope<'_>,
    input: DisposeInput,
) -> Result<Nothing, PlaywrightError> {
    let _sent = scope.send_effect()?;
    let run = scope.run_state();
    let (removed, in_flight) = {
        let mut registry = run.init_scripts.lock();
        if let (Some(hub), Some(claims)) = (registry.hub.clone(), registry.claims.remove(&input.id))
        {
            for (target, name) in claims {
                hub.release_binding(&target, &name, run);
                if !registry
                    .claims
                    .values()
                    .flatten()
                    .any(|(held, _)| *held == target)
                {
                    run.events.release_binding_runtime(&target);
                }
            }
        }
        registry.context.retain(|script| script.id != input.id);
        let in_flight: Vec<Arc<OnceCell<()>>> = registry
            .adding
            .iter()
            .filter(|(adding, _)| adding.id == input.id)
            .map(|(_, cell)| Arc::clone(cell))
            .collect();
        registry.adding.retain(|adding, _| adding.id != input.id);
        let (removed, kept): (Vec<Added>, Vec<Added>) = mem::take(&mut registry.added)
            .into_iter()
            .partition(|added| added.id == input.id);
        registry.added = kept;
        (removed, in_flight)
    };
    for added in removed {
        added.registration.remove_by(scope).await;
    }
    for cell in in_flight {
        // Waits out an add in progress; with none, this closure runs and initializes nothing.
        let _settled = cell.get_or_try_init(|| async { Err(()) }).await;
    }
    Ok(())
}

impl InitScripts {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Files a registration of the page's own under a new disposable id.
    fn keep(&self, registration: Registration) -> u64 {
        let mut registry = self.lock();
        let id = registry.next_id();
        registry.added.push(Added { id, registration });
        id
    }

    /// Keeps a context registration under a new disposable id.
    fn keep_for_context(&self, reach: Reach) -> u64 {
        let mut registry = self.lock();
        let id = registry.next_id();
        registry.context.push(ContextScript { id, reach });
        id
    }

    /// The event hub binding calls go to, kept for the context's later adds.
    fn keep_hub(&self, pw: &Playwright) -> Arc<EventHub> {
        Arc::clone(
            self.lock()
                .hub
                .get_or_insert_with(|| Arc::clone(pw.event_hub())),
        )
    }

    /// The bindings' plumbing on `admitted`'s tab session, once a run, in the order a binding
    /// call needs it: the run listens for the tab's binding calls before any can be made, the CDP
    /// binding goes into the session's main worlds (only those: `executionContextName: ""`, so
    /// Neo's isolated world never holds it), then the controller, in every document from now on and
    /// at once in the loaded ones. Concurrent first uses share one plumbing.
    async fn plumb(
        &self,
        scope: &OpScope<'_>,
        hub: &EventHub,
        admitted: &AdmittedPage,
        session: &ProtocolSession,
    ) -> Result<(), PlaywrightError> {
        let key = (admitted.tab, admitted.session.session_id.clone());
        let cell = Arc::clone(self.lock().plumbed.entry(key).or_default());
        cell.get_or_try_init(|| async {
            // Each write below rechecks the tab first: an owner who took it while this op
            // waited (for another add's plumbing, or for Chromium) gets nothing more.
            scope.recheck()?;
            let run = scope.run_state();
            hub.bind(run);
            let since = now_ms();
            run.events.listen(
                Subscription::Page {
                    tab: admitted.tab,
                    target: admitted.session.target_id.clone(),
                    event: PageEventKind::Binding,
                },
                since,
                hub.position_at(since),
            );
            hub.wake();
            let core = |error| scope.core_error(&error);
            scope.sending_main_world();
            session
                .send_value(
                    "Runtime.addBinding",
                    json!({ "name": BINDING_NAME, "executionContextName": "" }),
                )
                .await
                .map_err(core)?;
            scope.recheck()?;
            scope.sending_main_world();
            let controller = add_script(session, BINDINGS_CONTROLLER_JS, RunNow::Yes, None)
                .await
                .map_err(core)?;
            self.lock().added.push(Added {
                id: PLUMBING,
                registration: controller,
            });
            Ok::<(), PlaywrightError>(())
        })
        .await?;
        Ok(())
    }

    /// Forgets every script and removes each registration, all at once, awaiting them all:
    /// [`RunState::remove_init_scripts`](crate::RunState::remove_init_scripts), at the run's end.
    pub(crate) async fn remove_all(&self) {
        let added = {
            let mut registry = self.lock();
            registry.context.clear();
            registry.adding.clear();
            registry.plumbed.clear();
            // The run is ending: its claims go with its state, unread calls with its queue.
            registry.claims.clear();
            mem::take(&mut registry.added)
        };
        // No op drives the run's end, so a binding's cleanup here runs in the main world with
        // no op's record to flag (`Registration::remove_by`).
        future::join_all(added.into_iter().map(|added| added.registration.remove())).await;
    }

    /// Adds every context script `admitted`'s session lacks, for a write op admitted on that tab.
    /// The op's own tab rules run right before each add ([`OpScope::recheck`]), so a tab the door
    /// refuses, even one the owner takes between two adds, gets nothing more, and the op fails
    /// with the refusal it would meet at its own send. The op is not marked sent: the add is
    /// `context.addInitScript`'s effect, which that op marked, and it happens once per tab
    /// session, so the admitting op stays safe to retry when its own effect never went out.
    /// It is reported main-world, though, at each add that goes out: the scripts and bindings it
    /// adds run in the tab's main world, which the admitting op's spec does not declare. A
    /// binding whose name another run holds on the tab adds nothing and reports nothing.
    pub(crate) async fn add_context_scripts(
        &self,
        scope: &OpScope<'_>,
        browser: &BrowserSession,
        admitted: &AdmittedPage,
    ) -> Result<(), PlaywrightError> {
        let missing = self.missing(admitted);
        self.add_missing(scope, browser, admitted, missing).await
    }

    /// [`InitScripts::add_context_scripts`]' adds, from a snapshot [`InitScripts::missing`] took.
    async fn add_missing(
        &self,
        scope: &OpScope<'_>,
        browser: &BrowserSession,
        admitted: &AdmittedPage,
        missing: Vec<Missing>,
    ) -> Result<(), PlaywrightError> {
        if missing.is_empty() {
            return Ok(());
        }
        let session = browser.protocol_session(admitted.session.session_id.clone());
        for (adding, reach, cell) in missing {
            cell.get_or_try_init(|| async {
                // A dispose since the snapshot took this script's cell and waited for nothing,
                // so it must not go out now.
                if !self.keeps(adding.id) {
                    return Ok(());
                }
                scope.recheck()?;
                let registration = match &reach {
                    Reach::Script(source) => {
                        scope.sending_main_world();
                        add_script(&session, source, RunNow::No, None)
                            .await
                            .map_err(|error| scope.core_error(&error))?
                    }
                    Reach::Binding(binding) => {
                        let hub = self.lock().hub.clone().ok_or_else(|| {
                            scope.fail(
                                ErrorCode::Internal,
                                "a context binding was kept without its event hub",
                            )
                        })?;
                        match add_binding(self, scope, &hub, admitted, &session, binding).await? {
                            Some(registration) => registration,
                            // Another run's binding of that name stays on this tab.
                            None => return Ok(()),
                        }
                    }
                };
                // Kept or handed back under one lock, so a dispose sees either the registration
                // or this add still in flight, never neither.
                let disposed = {
                    let mut registry = self.lock();
                    if registry.context.iter().any(|script| script.id == adding.id) {
                        registry.added.push(Added {
                            id: adding.id,
                            registration,
                        });
                        if let Reach::Binding(binding) = &reach {
                            registry
                                .claims
                                .entry(adding.id)
                                .or_default()
                                .push((admitted.session.target_id.clone(), binding.name.clone()));
                        }
                        None
                    } else {
                        Some(registration)
                    }
                };
                if let Some(registration) = disposed {
                    // Disposed while it was being added. Removed before the cell settles, which
                    // is what `dispose` waits for, so dispose resolves only once it is gone.
                    registration.remove_by(scope).await;
                }
                Ok::<(), PlaywrightError>(())
            })
            .await?;
        }
        Ok(())
    }

    /// Whether the context still keeps the registration `id`.
    fn keeps(&self, id: u64) -> bool {
        self.lock().context.iter().any(|script| script.id == id)
    }

    /// The context's registrations whose add to `admitted`'s tab session has not completed, with
    /// their single-flight cells, copied out so no lock is held while they are added. The
    /// cells are made here, under the lock that reads the scripts, so a dispose from now on finds
    /// each one and waits out its add, which then sees the script gone.
    fn missing(&self, admitted: &AdmittedPage) -> Vec<Missing> {
        let mut registry = self.lock();
        let Registry {
            context, adding, ..
        } = &mut *registry;
        context
            .iter()
            .filter_map(|script| {
                let key = Adding {
                    id: script.id,
                    tab: admitted.tab,
                    session: admitted.session.session_id.clone(),
                };
                let cell = Arc::clone(adding.entry(key.clone()).or_default());
                (!cell.initialized()).then(|| (key, script.reach.clone(), cell))
            })
            .collect()
    }
}

/// A context registration one tab session lacks: where it goes, what it adds, and its add's cell.
type Missing = (Adding, Reach, Arc<OnceCell<()>>);

impl Registry {
    fn next_id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }
}

/// The gateway checks, then the script's source: an op that does not declare both capabilities
/// is refused with nothing read or sent, and a path is read now, as upstream's client reads it
/// before any channel call.
async fn gate(scope: &OpScope<'_>, script: InitSource) -> Result<String, PlaywrightError> {
    scope.require(Capability::MainWorld)?;
    scope.require(Capability::HostFiles)?;
    match script {
        InitSource::Source(source) => Ok(source),
        InitSource::Path(path) => input_files::read_script_file(&path).await,
    }
}

/// `Page.addScriptToEvaluateOnNewDocument` in the main world, as upstream registers an init
/// script: no `worldName`, and no `runImmediately`, so the document already loaded does not run
/// it. Takes `&Sent`: it changes every later document of the page.
async fn add(
    session: &ProtocolSession,
    source: &str,
    _sent: &Sent<'_>,
) -> Result<Registration, CoreError> {
    add_script(session, source, RunNow::No, None).await
}

/// A binding's stub on a tab session whose plumbing it brings in first. Takes `&Sent`. A name
/// another run on the tab holds fails as upstream's duplicate does, with nothing added.
async fn expose(
    scope: &OpScope<'_>,
    hub: &EventHub,
    admitted: &AdmittedPage,
    session: &ProtocolSession,
    binding: &BindingName,
    _sent: &Sent<'_>,
) -> Result<Registration, PlaywrightError> {
    add_binding(
        &scope.run_state().init_scripts,
        scope,
        hub,
        admitted,
        session,
        binding,
    )
    .await?
    .ok_or_else(|| {
        scope.fail(
            ErrorCode::InvalidInput,
            &format!(
                "Function \"{}\" has been already registered by another run on this page",
                binding.name
            ),
        )
    })
}

/// Holds `Runtime` on the tab's session while the run has a binding there: binding calls, and
/// the main-world contexts that name their frame, are `Runtime` events, and core attaches pages
/// with it off. The dispose of the run's last binding on the tab releases it ([`dispose`]); the
/// next add leases again.
async fn hold_binding_runtime(
    scope: &OpScope<'_>,
    admitted: &AdmittedPage,
    session: &ProtocolSession,
) -> Result<(), PlaywrightError> {
    let run = scope.run_state();
    let calls = Subscription::Page {
        tab: admitted.tab,
        target: admitted.session.target_id.clone(),
        event: PageEventKind::Binding,
    };
    if run.events.holds_runtime(&calls) {
        return Ok(());
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "epoch milliseconds stay far below 2^53"
    )]
    let floor = now_ms() as f64;
    let lease = scope
        .view()
        .signals()
        .runtime_leases()
        .acquire(session, floor)
        .await
        .map_err(|error| scope.core_error(&error))?;
    run.events.hold_runtime(&calls, vec![lease.into()]);
    Ok(())
}

/// [`expose`] without the proof, for a context binding, which the admitting op's recheck covers.
/// `None` when another live run holds the name on this tab ([`EventHub::claim_binding`]): the
/// page keeps that run's binding, and this one adds nothing there.
async fn add_binding(
    scripts: &InitScripts,
    scope: &OpScope<'_>,
    hub: &EventHub,
    admitted: &AdmittedPage,
    session: &ProtocolSession,
    binding: &BindingName,
) -> Result<Option<Registration>, PlaywrightError> {
    let target = &admitted.session.target_id;
    if !hub.claim_binding(target, &binding.name, scope.run_state()) {
        return Ok(None);
    }
    scripts.plumb(scope, hub, admitted, session).await?;
    hold_binding_runtime(scope, admitted, session).await?;
    scope.recheck()?;
    scope.sending_main_world();
    add_script(
        session,
        &binding.stub(),
        RunNow::Yes,
        Some(binding.cleanup()),
    )
    .await
    .map(Some)
    .map_err(|error| scope.core_error(&error))
}

/// Whether a script also runs at once in the documents the session has loaded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunNow {
    /// An init script: new documents only, as upstream's `addInitScript`.
    No,
    /// The bindings' scripts, which upstream also evaluates in every frame when it adds them.
    Yes,
}

/// Registers `source` for new documents, in a task of its own, and returns the registration
/// that removes it (and runs `cleanup`, when given, in the documents loaded then).
///
/// The command goes out from a task of its own, which the op awaits. The host drops an op's
/// future at the run's stop or deadline, and that can land after the command was sent and before
/// Chromium answered: awaited inline, the answer would be lost with the future and the script
/// would stay registered past the run. The task always gets the answer, and when the op is gone
/// its output, the [`Registration`], is dropped as the task ends, which removes the script.
async fn add_script(
    session: &ProtocolSession,
    source: &str,
    run_now: RunNow,
    cleanup: Option<String>,
) -> Result<Registration, CoreError> {
    #[derive(Deserialize)]
    struct AddedScript {
        identifier: String,
    }
    let session = session.cancellation_endpoint();
    let params = if run_now == RunNow::Yes {
        json!({ "source": source, "runImmediately": true })
    } else {
        json!({ "source": source })
    };
    let added = tokio::spawn(async move {
        let AddedScript { identifier } = session
            .send("Page.addScriptToEvaluateOnNewDocument", params)
            .await?;
        Ok::<_, CoreError>(Registration {
            session: Some(session),
            identifier,
            cleanup,
        })
    });
    added.await.unwrap_or_else(|join| {
        Err(CoreError::Message(format!(
            "the task that registered an init script ended without an answer: {join}"
        )))
    })
}

#[cfg(test)]
mod tests;
