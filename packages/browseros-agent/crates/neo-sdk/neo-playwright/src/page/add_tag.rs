//! Script and stylesheet insertion: host files become content, the owning frame's main world
//! creates the tag, and CDP's CSP log races its load. Returned handles name that document and
//! backend node; temporary remote objects belong to the operation and are freed on cancellation.

use std::io::ErrorKind;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError, error::TryRecvError};
use tokio::{fs, time};

use crate::op_scope::OpScope;
use crate::op_spec::Capability;
use crate::page::frame_tree::{self, FrameTree};
use crate::page::{OpPage, evaluate_guard, world_call};
use crate::target::PageRef;
use crate::{ErrorCode, Playwright, PlaywrightError, op_output, resolve};

/// Which upstream insertion algorithm and sourceURL syntax to use.
#[derive(Clone, Copy)]
pub(crate) enum TagKind {
    Script,
    Style,
}

/// Paths are host-side inputs, as in Playwright's client; URLs are resolved by the document.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AddTagInput {
    page: PageRef,
    frame: Option<FrameId>,
    url: Option<String>,
    path: Option<String>,
    content: Option<String>,
    r#type: Option<String>,
}

/// Only renderer inputs cross the browser boundary; host paths are consumed before this point.
#[derive(Serialize)]
struct TagParams {
    kind: &'static str,
    url: Option<String>,
    content: Option<String>,
    r#type: String,
}

#[derive(Deserialize)]
struct Described {
    node: Node,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    backend_node_id: i64,
}

/// Inserts once, without retrying a script that might already have executed. The run's door
/// owns cancellation; dropping this future releases its CSP subscription and CDP object group.
pub(crate) async fn insert(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: AddTagInput,
    kind: TagKind,
) -> Result<Value, PlaywrightError> {
    scope.require(Capability::MainWorld)?;
    let AddTagInput {
        page: page_ref,
        frame,
        url,
        path,
        mut content,
        r#type,
    } = input;
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        scope.require(Capability::HostFiles)?;
        content = Some(file_content(scope, &path, kind).await?);
    }
    if url.as_ref().is_none_or(String::is_empty) && content.as_ref().is_none_or(String::is_empty) {
        return Err(scope.fail(
            ErrorCode::InvalidInput,
            "Provide an object with a `url`, `path` or `content` property",
        ));
    }
    let page = OpPage::admit_handle(pw, scope, &page_ref).await?;
    let world = world_call::frame_world(&page, scope, frame.as_ref()).await?;
    let frame = evaluate_guard::frame_endpoint(&page, world.session(), world.frame().clone());
    // Resolving the document is read-only and may retry a cached world after navigation.
    // Once insertion is sent below, a script may have run and must never be sent twice.
    let (world, document) =
        world_call::with_world(&page, &frame.endpoint, &frame.place, |world| async move {
            let document = evaluate_guard::main_world_document(scope, world.clone()).await?;
            Ok((world, document))
        })
        .await
        .map_err(|failure| resolve::failed(scope, failure))?;
    let session = world.session();
    // The broadcast, not a targeted channel: that would take `Log.entryAdded` from the event
    // watcher, which passes the entries on as console messages in wire order.
    let mut events = pw.browser().cdp_events();
    session
        .send_value("Log.enable", json!({}))
        .await
        .map_err(|error| scope.core_error(&error))?;
    // Log.enable replays retained entries. They belong to earlier actions, so only messages
    // arriving after this barrier participate in this insertion's upstream-style CSP race.
    while queued(&mut events).is_some() {}
    let params = TagParams {
        kind: match kind {
            TagKind::Script => "script",
            TagKind::Style => "style",
        },
        url,
        content,
        r#type: r#type.unwrap_or_default(),
    };
    let sent = scope.send_effect()?;
    let insert = evaluate_guard::insert_tag(scope, &world, &document, json!(params), &sent);
    let failure = async {
        loop {
            match events.recv().await {
                Ok(event) => {
                    if let Some(message) = csp_message(&event, world.session_id()) {
                        return scope.fail(ErrorCode::PageError, &message);
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
        scope.fail(
            ErrorCode::TargetClosed,
            "Target page, context or browser has been closed",
        )
    };
    let node = tokio::select! {
        biased;
        error = failure => return Err(error),
        node = insert => node,
    };
    // The gateway's final round trip flushes asynchronous inline CSP reports, as upstream
    // Chromium does. Prefer their useful browser wording over a generic load-event failure.
    while let Some(event) = queued(&mut events) {
        if let Some(message) = csp_message(&event, world.session_id()) {
            return Err(scope.fail(ErrorCode::PageError, &message));
        }
    }
    let object = node?;
    let described: Described = session
        .send("DOM.describeNode", json!({ "objectId": object }))
        .await
        .map_err(|error| scope.core_error(&error))?;
    let element = json!({ "$neo": "element", "page": page_ref, "seq": world.seq(),
        "node": described.node.backend_node_id, "display": format!("<{}>", params.kind) });
    tagged(pw, scope, &page_ref, element).await
}

/// The page's error console receives CSP reports through CDP's Log domain, including inline
/// styles whose load promise never settles when blocked. Other targets and log levels do not
/// participate, matching Playwright's page-scoped error-console listener.
fn csp_message(event: &CdpEvent, session: Option<&browseros_core::SessionId>) -> Option<String> {
    #[derive(Deserialize)]
    struct Entry {
        level: String,
        text: String,
    }
    #[derive(Deserialize)]
    struct Logged {
        entry: Entry,
    }
    if event.method != "Log.entryAdded" || event.session_id.as_ref() != session {
        return None;
    }
    let logged: Logged = serde_json::from_value(event.params.clone()).ok()?;
    let Entry { level, text } = logged.entry;
    (level == "error"
        && (text.contains("Content-Security-Policy") || text.contains("Content Security Policy")))
    .then_some(text)
}

/// The next event already received, past any the receiver lagged over; `None` when none is.
fn queued(events: &mut broadcast::Receiver<CdpEvent>) -> Option<CdpEvent> {
    loop {
        match events.try_recv() {
            Ok(event) => return Some(event),
            Err(TryRecvError::Lagged(_)) => {}
            Err(TryRecvError::Empty | TryRecvError::Closed) => return None,
        }
    }
}

/// Playwright reads UTF-8 with replacement and strips newlines only from the sourceURL.
async fn file_content(
    scope: &OpScope<'_>,
    path: &str,
    kind: TagKind,
) -> Result<String, PlaywrightError> {
    let bytes = fs::read(path).await.map_err(|error| {
        let (code, message) = match error.kind() {
            ErrorKind::NotFound => ("ENOENT", "no such file or directory".to_owned()),
            ErrorKind::PermissionDenied => ("EACCES", "permission denied".to_owned()),
            ErrorKind::IsADirectory => ("EISDIR", "illegal operation on a directory".to_owned()),
            _ => ("UNKNOWN", error.to_string()),
        };
        scope.fail(
            ErrorCode::InvalidInput,
            &format!("{code}: {message}, open '{path}'"),
        )
    })?;
    let source = String::from_utf8_lossy(&bytes);
    let source_path = path.replace('\n', "");
    Ok(match kind {
        TagKind::Script => format!("{source}\n//# sourceURL={source_path}"),
        TagKind::Style => format!("{source}/*# sourceURL={source_path}*/"),
    })
}

/// Injected scripts can create frames. As for evaluate, report a bounded best-effort snapshot
/// with the handle so the prelude's synchronous frame accessors observe that completed write.
async fn tagged(
    pw: &Playwright,
    scope: &OpScope<'_>,
    page: &PageRef,
    element: Value,
) -> Result<Value, PlaywrightError> {
    #[derive(Serialize)]
    struct Tagged {
        element: Value,
        #[serde(flatten)]
        frames: Option<FrameTree>,
    }
    let frames = time::timeout(
        Duration::from_secs(2),
        frame_tree::snapshot(pw, scope, page.tab()),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    op_output::to_output(scope, &Tagged { element, frames })
}
