//! Init scripts: `page.addInitScript`, `context.addInitScript` and the disposable each returns.
//! A script runs in the page's main world in every new document of its tab, before the page's
//! own scripts, through CDP's `Page.addScriptToEvaluateOnNewDocument`, as upstream's `crPage`
//! registers one. It is the second gateway of `Capability::MainWorld` (after the evaluate guard),
//! and, for the `{ path }` form, of `Capability::HostFiles`.
//!
//! ```text
//!  page.addInitScript     require(MainWorld, HostFiles) ─ read a path ─ admit the page
//!                         ─ send_effect (the door's rules on the tab) ─ add on its session
//!  context.addInitScript  require(...) ─ read a path ─ send_effect ─ kept in the run's registry
//!     └▶ every write op later admitted on a tab without it: recheck that op's tab, then add it
//!        there (OpScope::admit), before the op's own effect, so the op's navigation runs it
//!  disposable.dispose     every registration of that id removed, and awaited
//!  the run ends           RunState drops the registry; each registration's guard removes it
//! ```
//!
//! **Lifetime: the run.** A Neo run is a whole Playwright script, and the tab outlives it, so
//! a script registered by one run must not run in documents another run's script loads later.
//! Every registration lives in the run's [`RunState`](crate::RunState) and is removed when the
//! run ends, however it ends (returned, failed, cancelled, timed out): a registration's guard
//! removes it on drop. Within the run it stays registered across navigations, so a later `goto`
//! in the same run gets it, as a model writing a Playwright script expects.
//!
//! **The context's tabs.** Neo's context is the owner's whole profile, so a context script does
//! not go to every tab at once: it goes to each tab the run writes to, at that write op's
//! admission, through that op's own recheck of its tab. Every navigation a run drives is a write
//! (`goto`, `reload`, a click, a script), so each gets the script; a popup's first document,
//! which loads before the run acts on the popup, does not, where Playwright's would.
//!
//! **Sessions.** A registration belongs to the session it was added on: the tab's page session
//! covers the main frame and its same-process frames, across cross-process navigations of the
//! main frame. An out-of-process frame has a session of its own, which Neo does not add the
//! script to, so a cross-site iframe runs without it. A tab whose page session changed (core
//! attached again) gets the context's scripts again on the new one.

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use browseros_core::{BrowserSession, ProtocolSession, SessionId, TabId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::OnceCell;

use crate::action::input_files;
use crate::op_output::to_output;
use crate::op_scope::{AdmittedPage, OpScope, Sent};
use crate::op_spec::Capability;
use crate::target::PageRef;
use crate::{Playwright, PlaywrightError};

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
    /// The context's scripts, in registration order.
    context: Vec<ContextScript>,
    /// Every script added to a tab, the page's own and the context's.
    added: Vec<Added>,
    /// One single-flight cell per context script and tab session: two write ops admitted on one
    /// tab at once share one add, the second awaiting the first's, so the tab runs the script
    /// once a document. A failed add leaves its cell empty, and the next write tries again.
    adding: HashMap<Adding, Arc<OnceCell<()>>>,
}

/// A context script on one tab's page session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Adding {
    id: u64,
    tab: TabId,
    session: SessionId,
}

/// One `context.addInitScript`, which each tab the run writes to gets once.
#[derive(Debug)]
struct ContextScript {
    id: u64,
    source: Arc<str>,
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
}

impl Registration {
    /// Removes the script now. A session that is gone (the tab closed) has nothing to remove.
    async fn remove(mut self) {
        if let Some(session) = self.session.take() {
            remove(&session, &self.identifier).await;
        }
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
        drop(runtime.spawn(async move { remove(&session, &identifier).await }));
    }
}

async fn remove(session: &ProtocolSession, identifier: &str) {
    let _removed = session
        .send::<_, Value>(
            "Page.removeScriptToEvaluateOnNewDocument",
            json!({ "identifier": identifier }),
        )
        .await;
}

/// The script's source as the prelude built it (upstream's client `evaluationScript`), or the
/// path of a file to read it from. No `Debug`: the source may hold a value the agent was given.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) enum InitSource {
    /// A function call with its argument, a string, or `{ content }`.
    Source(String),
    /// `{ path }`, read as upstream's client reads it: as UTF-8, relative to Neo's working
    /// directory, with `//# sourceURL=<path>` appended.
    Path(String),
}

/// `page.addInitScript`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageScriptInput {
    page: PageRef,
    script: InitSource,
}

/// `context.addInitScript`'s input.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextScriptInput {
    script: InitSource,
}

/// `disposable.dispose`'s input: the id an `addInitScript` returned.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DisposeInput {
    id: u64,
}

/// What `addInitScript` returns: the id its disposable disposes.
#[derive(Debug, Serialize)]
struct Registered {
    id: u64,
}

/// `page.addInitScript`: the script added to the page's session now, after the door's rules on
/// the page. Every later document of the tab runs it until the run ends or it is disposed.
pub(crate) async fn add_to_page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: PageScriptInput,
) -> Result<Value, PlaywrightError> {
    let source = gate(scope, input.script).await?;
    let admitted = scope.admit(pw.browser(), input.page.tab()).await?;
    let session = pw
        .browser()
        .protocol_session(admitted.session.session_id.clone());
    let sent = scope.send_effect()?;
    let registration = add(&session, &source, &sent)
        .await
        .map_err(|error| scope.core_error(&error))?;
    let scripts = &scope.run_state().init_scripts;
    let mut registry = scripts.lock();
    let id = registry.next_id();
    registry.added.push(Added { id, registration });
    drop(registry);
    to_output(scope, &Registered { id })
}

/// `context.addInitScript`: the script kept for every tab the run writes to from now on. Marked
/// sent, since it changes what the run's later writes do; no tab is touched here.
pub(crate) async fn add_to_context(
    scope: &OpScope<'_>,
    input: ContextScriptInput,
) -> Result<Value, PlaywrightError> {
    let source = gate(scope, input.script).await?;
    let _sent = scope.send_effect()?;
    let scripts = &scope.run_state().init_scripts;
    let mut registry = scripts.lock();
    let id = registry.next_id();
    registry.context.push(ContextScript {
        id,
        source: source.into(),
    });
    drop(registry);
    to_output(scope, &Registered { id })
}

/// `disposable.dispose`: removes every registration of `id`, page or context, and awaits each
/// removal, as upstream's `removeInitScripts` does. An id already disposed, or one the run never
/// handed out, disposes nothing, as a second upstream `dispose()` does.
pub(crate) async fn dispose(
    scope: &OpScope<'_>,
    input: DisposeInput,
) -> Result<Value, PlaywrightError> {
    let _sent = scope.send_effect()?;
    let removed = {
        let mut registry = scope.run_state().init_scripts.lock();
        registry.context.retain(|script| script.id != input.id);
        registry.adding.retain(|adding, _| adding.id != input.id);
        let (removed, kept) = mem::take(&mut registry.added)
            .into_iter()
            .partition(|added| added.id == input.id);
        registry.added = kept;
        removed
    };
    for added in removed {
        added.registration.remove().await;
    }
    Ok(json!({}))
}

impl InitScripts {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds every context script `admitted`'s session lacks, for a write op admitted on that tab.
    /// The op's own tab rules run first ([`OpScope::recheck`]), so a tab the door refuses gets
    /// nothing, and the op fails with the refusal it would meet at its own send. A disposal that
    /// lands while a script is being added removes it again.
    pub(crate) async fn add_context_scripts(
        &self,
        scope: &OpScope<'_>,
        browser: &BrowserSession,
        admitted: &AdmittedPage,
    ) -> Result<(), PlaywrightError> {
        let missing = self.missing(admitted);
        if missing.is_empty() {
            return Ok(());
        }
        scope.recheck()?;
        let session = browser.protocol_session(admitted.session.session_id.clone());
        for (adding, source) in missing {
            let cell = Arc::clone(self.lock().adding.entry(adding.clone()).or_default());
            cell.get_or_try_init(|| async {
                let registration = add_unsent(&session, &source)
                    .await
                    .map_err(|error| scope.core_error(&error))?;
                let mut registry = self.lock();
                if registry.context.iter().any(|kept| kept.id == adding.id) {
                    registry.added.push(Added {
                        id: adding.id,
                        registration,
                    });
                } else {
                    // Disposed while it was being added: its guard removes it again.
                    drop(registry);
                    drop(registration);
                }
                Ok::<(), PlaywrightError>(())
            })
            .await?;
        }
        Ok(())
    }

    /// The context's scripts whose add to `admitted`'s tab session has not completed, with their
    /// sources, copied out so no lock is held while they are added.
    fn missing(&self, admitted: &AdmittedPage) -> Vec<(Adding, Arc<str>)> {
        let registry = self.lock();
        registry
            .context
            .iter()
            .map(|script| {
                let adding = Adding {
                    id: script.id,
                    tab: admitted.tab,
                    session: admitted.session.session_id.clone(),
                };
                (adding, Arc::clone(&script.source))
            })
            .filter(|(adding, _)| {
                !registry
                    .adding
                    .get(adding)
                    .is_some_and(|cell| cell.initialized())
            })
            .collect()
    }
}

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
) -> Result<Registration, browseros_core::CoreError> {
    add_unsent(session, source).await
}

/// [`add`] for a context script, which the admitting op's own recheck covers and which does not
/// mark that op sent: it changes only documents a later effect loads.
async fn add_unsent(
    session: &ProtocolSession,
    source: &str,
) -> Result<Registration, browseros_core::CoreError> {
    #[derive(Deserialize)]
    struct AddedScript {
        identifier: String,
    }
    let AddedScript { identifier } = session
        .send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": source }),
        )
        .await?;
    Ok(Registration {
        session: Some(session.cancellation_endpoint()),
        identifier,
    })
}

#[cfg(test)]
mod tests;
