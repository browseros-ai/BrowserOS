//! The browser-context ops and their registration metadata.

use std::future::Future;

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::pages::{NewPageOptions, PageInfo};
use browseros_core::{CoreError, TargetId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use ts_rs::TS;

use crate::listed_enum::listed_enum;
use crate::op_input::OpArgs;
use crate::op_scope::OpScope;
use crate::op_spec::{Capability, OpSpec, OpTarget};
use crate::page::{PageHandle, init_script};
use crate::wire::{self, Describe, OpWire, Route, Run, options};
use crate::{Playwright, PlaywrightError};

listed_enum! {
    /// The one browser context a run sees: its tabs, opening a new one, and the init scripts and
    /// bindings it gives every tab the run writes to, with the disposable each registration
    /// returns.
    pub(crate) enum ContextOp {
        Pages,
        NewPage,
        AddInitScript,
        ExposeBinding,
        Dispose,
    }
}

impl ContextOp {
    /// Shapes and runtime dispatch come from the same route rows.
    pub(crate) fn wire(self) -> Option<OpWire> {
        route(self, Describe)
    }

    /// This op's registration metadata.
    pub(crate) const fn spec(self) -> OpSpec {
        match self {
            Self::Pages => OpSpec::read(
                "context.pages",
                OpTarget::None,
                "every tab this run's view admits",
            ),
            Self::NewPage => {
                OpSpec::write("context.newPage", OpTarget::None, "open a background tab")
            }
            Self::AddInitScript => OpSpec::write(
                "context.addInitScript",
                OpTarget::None,
                "run a script in the main world of every new document of each tab the run \
                 writes to, until the run ends",
            )
            .using(&[Capability::MainWorld, Capability::HostFiles])
            .policed(),
            Self::ExposeBinding => OpSpec::write(
                "context.exposeBinding",
                OpTarget::None,
                "let every tab the run writes to call a function of the script, until the run ends",
            )
            .using(&[Capability::MainWorld])
            .policed(),
            Self::Dispose => OpSpec::write(
                "disposable.dispose",
                OpTarget::None,
                "remove what an addInitScript or exposeBinding registered",
            ),
        }
    }
}

/// `context.pages` and `context.newPage` take no arguments beyond the call header.
#[derive(Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ContextNoArgs")]
struct NoArgs {}
options!(NoArgs);

/// `context.pages()`: every tab the run's view admits, the owner's and other sessions' tabs
/// included (D8, the owner's decision: no ownership filter).
#[derive(Debug, Serialize, TS)]
#[ts(rename = "ContextPages")]
struct Pages {
    pages: Vec<PageEntry>,
}

/// One tab, with what the prelude caches for its synchronous `url()`.
#[derive(Debug, Serialize, TS)]
#[ts(rename = "ContextPageEntry")]
struct PageEntry {
    page: PageHandle,
    url: String,
}

/// Context registration/disposal stays with the page init-script family until PR 12 moves it.
/// Execution and description share these rows; the policy above remains unchanged.
fn route<'a, R: Route<'a>>(op: ContextOp, r: R) -> R::Out {
    match op {
        ContextOp::Pages => r.op(
            wire::neo_only("Neo lists live tabs instead of upstream's client-side page objects"),
            pages,
        ),
        ContextOp::NewPage => r.op(wire::command("BrowserContext.newPage"), new_page),
        op @ (ContextOp::AddInitScript | ContextOp::ExposeBinding | ContextOp::Dispose) => {
            r.untyped(move |pw, scope, args| run_untyped(pw, scope, op, args))
        }
    }
}

/// Runs one context op.
pub(crate) async fn invoke(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ContextOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    route(op, Run { pw, scope, args }).await
}

async fn run_untyped(
    pw: &Playwright,
    scope: &OpScope<'_>,
    op: ContextOp,
    args: OpArgs,
) -> Result<Value, PlaywrightError> {
    match op {
        ContextOp::AddInitScript => init_script::add_to_context(scope, args.decode(scope)?).await,
        ContextOp::ExposeBinding => init_script::expose_on_context(pw, scope, args.decode(scope)?),
        ContextOp::Dispose => init_script::dispose(scope, args.decode(scope)?).await,
        ContextOp::Pages | ContextOp::NewPage => Err(scope.not_built()),
    }
}

async fn pages(
    _pw: &Playwright,
    scope: &OpScope<'_>,
    _input: NoArgs,
) -> Result<Pages, PlaywrightError> {
    let listing = scope
        .view()
        .list_pages()
        .await
        .map_err(|error| scope.core_error(&error))?;
    let pages = listing
        .pages
        .into_iter()
        .map(|page| PageEntry {
            page: PageHandle::new(page.tab_id),
            url: page.url,
        })
        .collect();
    Ok(Pages { pages })
}

async fn new_page(
    pw: &Playwright,
    scope: &OpScope<'_>,
    _input: NoArgs,
) -> Result<PageEntry, PlaywrightError> {
    // The list of tabs open at the instance's start must be read before this tab
    // exists, or a newPage right after `Playwright::new` lands in that list and its
    // refs print from the random base (`f250e3`) instead of bare (`e3`). Measured by
    // conformance Runner N, whose first op in a new instance is this one.
    pw.pages().note_open_tabs(pw.browser()).await;
    let _sent = scope.send_effect()?;
    // Core opens it in the background and keeps focus emulation on, as for every tab it
    // opens for the agent.
    let opened = scope
        .view()
        .new_page(
            "about:blank",
            NewPageOptions {
                background: Some(true),
                ..NewPageOptions::default()
            },
        )
        .await
        .map_err(|error| scope.core_error(&error))?;
    ready(pw, scope, &opened).await;
    Ok(PageEntry {
        page: PageHandle::new(opened.tab_id),
        url: opened.url,
    })
}

/// Readies the tab `newPage` just opened, as upstream's `newPage` does before it returns:
/// core attaches the tab's session (admission), and the tab's renderer answers
/// `Page.setLifecycleEventsEnabled`, the first command a navigation sends. Upstream's
/// `BrowserContext.newPage` awaits `FrameSession._initialize` (crPage.ts), which sends that
/// command among its first, with no timeout. Neo left both to the tab's first op, so they ran
/// inside that op's own timeout; core's attach even runs outside `OpScope::within`, so a slow
/// one ate that timeout whole. A new background tab's renderer runs at background priority
/// (measured on macOS: PRI 4, where a foreground tab's runs at 47) and on a loaded machine it
/// answered those first commands up to 75 s late, so a first `goto` timed out before it
/// navigated. Done here, the wait is `newPage`'s own.
///
/// The CDP client's own request timeout (60 s a command) is no answer either: that renderer
/// answers later (4 of those attaches hit it), so a step the client gave up on is sent again
/// ([`again_on_timeout`]). `Page.setLifecycleEventsEnabled` is safe to send again. Admission is
/// core's attach, which starts over from `Target.attachToTarget`, and that is not: each one
/// makes a session. But an attach that timed out leaves whatever session it made attached, as
/// core's attach detaches nothing when a step fails and records the session only once its
/// enables are answered, so the tab's next admission attaches anew in any case, the first op's
/// when this wait gives up. A second attach here makes no session that would not have been
/// made; detaching the stale one is core's to do.
///
/// Like any op without a Playwright timeout, it ends with the run: the host drops the op at the
/// run's deadline or on a stop, which drops these calls (their reply slots and core's locks go
/// with them), as it drops every other op. It ends as well once the tab closes or its renderer
/// crashes, which no answer may follow. Nothing here fails `newPage`, which returns the tab it
/// opened, as it did before: after any other failure the tab's first op admits it and enables
/// lifecycle events again, inside its own timeout, and reports what it meets there, a closed or
/// crashed tab included.
async fn ready(pw: &Playwright, scope: &OpScope<'_>, opened: &PageInfo) {
    // Taken before anything is sent, so a close or crash while the tab answers is not missed.
    let mut events = pw.browser().cdp_events();
    let answered = async {
        let admitted = again_on_timeout(|| scope.view().page_session(opened.page_id.clone()));
        let Some(admitted) = admitted.await else {
            return;
        };
        let session = pw.browser().protocol_session(admitted.session_id);
        let enable =
            || session.send_value("Page.setLifecycleEventsEnabled", json!({ "enabled": true }));
        let _answered = again_on_timeout(enable).await;
    };
    let ended = async {
        loop {
            match events.recv().await {
                Ok(event) if ends(&event, &opened.target_id) => return,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            }
        }
    };
    tokio::select! {
        () = answered => {}
        () = ended => {}
    }
}

/// `send`'s answer, sent again each time the CDP client gave up waiting for it
/// (`CdpError::Timeout`, which comes only after the client's request timeout, so this sends at
/// most once a timeout); `None` once it fails otherwise.
async fn again_on_timeout<T, F, Fut>(mut send: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, CoreError>>,
{
    loop {
        match send().await {
            Ok(answer) => return Some(answer),
            Err(CoreError::Cdp(CdpError::Timeout { .. })) => {}
            Err(_) => return None,
        }
    }
}

/// Whether `event` says tab `target` closed or its renderer crashed: its target destroyed or
/// crashed (the run's target watcher turns target discovery on), or the session core attached
/// to it detached.
fn ends(event: &CdpEvent, target: &TargetId) -> bool {
    matches!(
        event.method.as_str(),
        "Target.targetDestroyed" | "Target.targetCrashed" | "Target.detachedFromTarget"
    ) && event.params.get("targetId").and_then(Value::as_str) == Some(target.as_str())
}

#[cfg(test)]
mod tests;
