//! Script and stylesheet insertion: host files become content, the owning frame's main world
//! creates the tag, and CDP's CSP log races its load. Returned handles name that document and
//! backend node; temporary remote objects belong to the operation and are freed on cancellation.
//! `Log` is on for the insert only, under a lease it shares with console listeners
//! (`events/log_lease.rs`).

use std::io::ErrorKind;
use std::time::Duration;

use browseros_cdp::CdpEvent;
use browseros_core::FrameId;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::{fs, time};
use ts_rs::TS;

use crate::events::event_hub::now_ms;

use crate::world::FrameSeq;

use crate::op_scope::OpScope;
use crate::op_spec::Capability;
use crate::page::frame_tree::{self, FrameInfo, FrameTree};
use crate::page::{OpPage, evaluate_guard, world_call};
use crate::target::PageRef;
use crate::wire::options;
use crate::{ErrorCode, Playwright, PlaywrightError, resolve};

/// Which upstream insertion algorithm and sourceURL syntax to use.
#[derive(Clone, Copy)]
pub(crate) enum TagKind {
    Script,
    Style,
}

/// Paths are host-side inputs, as in Playwright's client; URLs are resolved by the document.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct AddTagInput {
    page: PageRef,
    frame: Option<FrameId>,
    url: Option<String>,
    path: Option<String>,
    content: Option<String>,
    r#type: Option<String>,
}

options!(AddTagInput);

/// Styles omit the script-only type field; the prelude already drops that script option.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
#[ts(optional_fields)]
pub(crate) struct AddStyleTagInput {
    page: PageRef,
    frame: Option<FrameId>,
    url: Option<String>,
    path: Option<String>,
    content: Option<String>,
}
options!(AddStyleTagInput);

/// The inserted node stays pinned to its document; frame facts refresh synchronous accessors.
#[derive(Serialize, TS)]
pub(crate) struct Tagged {
    element: TagElement,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    frames: Option<Vec<FrameInfo>>,
}

/// A returned node is pinned to the admitted page and the document that created it.
#[derive(Serialize, TS)]
struct TagElement {
    #[serde(rename = "$neo")]
    #[ts(type = "\"element\"")]
    brand: &'static str,
    display: String,
    node: i64,
    #[ts(as = "crate::page::PageHandle")]
    page: PageRef,
    seq: FrameSeq,
}

pub(crate) async fn script(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: AddTagInput,
) -> Result<Tagged, PlaywrightError> {
    Box::pin(insert(pw, scope, input, TagKind::Script)).await
}

pub(crate) async fn style(
    pw: &Playwright,
    scope: &OpScope<'_>,
    input: AddStyleTagInput,
) -> Result<Tagged, PlaywrightError> {
    let AddStyleTagInput {
        page,
        frame,
        url,
        path,
        content,
    } = input;
    Box::pin(insert(
        pw,
        scope,
        AddTagInput {
            page,
            frame,
            url,
            path,
            content,
            r#type: None,
        },
        TagKind::Style,
    ))
    .await
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
) -> Result<Tagged, PlaywrightError> {
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
            let document = evaluate_guard::main_world_document(scope, world.clone())
                .await?
                .object;
            Ok((world, document))
        })
        .await
        .map_err(|failure| resolve::failed(scope, failure))?;
    let session = world.session();
    // The broadcast, not a targeted channel: that would take `Log.entryAdded` off the broadcast,
    // where the event watcher passes entries on as console messages, for every tab meanwhile.
    let mut events = pw.browser().cdp_events();
    // Counted with console listeners' `Log`, so a listener's release cannot switch the CSP
    // reports off under this insert, and `Log` goes off again once the last holder is done.
    #[expect(
        clippy::cast_precision_loss,
        reason = "epoch milliseconds stay far below 2^53"
    )]
    let _log = pw
        .events()
        .log_leases()
        .acquire(session, now_ms() as f64)
        .await
        .map_err(|error| scope.core_error(&error))?;
    // Log.enable replays retained entries. They belong to earlier actions, so only messages
    // arriving after this barrier participate in this insertion's upstream-style CSP race.
    while let Ok(Some(_)) | Err(Lagged) = queued(&mut events) {}
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
                // A report may have gone with what the reader missed: say so rather than wait
                // on a load that a blocked tag never finishes.
                Err(RecvError::Lagged(_)) => return lagged(scope),
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
    received_report(scope, &mut events, world.session_id())?;
    let object = node?;
    let described: Described = session
        .send("DOM.describeNode", json!({ "objectId": object }))
        .await
        .map_err(|error| scope.core_error(&error))?;
    let element = TagElement {
        brand: "element",
        page: page_ref.clone(),
        seq: world.seq(),
        node: described.node.backend_node_id,
        display: format!("<{}>", params.kind),
    };
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

/// Fails with the CSP report `events` already received for `session`, if one is there.
fn received_report(
    scope: &OpScope<'_>,
    events: &mut broadcast::Receiver<CdpEvent>,
    session: Option<&browseros_core::SessionId>,
) -> Result<(), PlaywrightError> {
    loop {
        match queued(events) {
            Ok(Some(event)) => {
                if let Some(message) = csp_message(&event, session) {
                    return Err(scope.fail(ErrorCode::PageError, &message));
                }
            }
            Ok(None) => return Ok(()),
            Err(Lagged) => return Err(lagged(scope)),
        }
    }
}

/// The reader fell behind the shared broadcast and missed events.
struct Lagged;

/// The next event already received; `None` when none is.
fn queued(events: &mut broadcast::Receiver<CdpEvent>) -> Result<Option<CdpEvent>, Lagged> {
    match events.try_recv() {
        Ok(event) => Ok(Some(event)),
        Err(TryRecvError::Lagged(_)) => Err(Lagged),
        Err(TryRecvError::Empty | TryRecvError::Closed) => Ok(None),
    }
}

/// The insertion's CSP reports arrive on the shared broadcast, which drops what a reader that
/// fell behind has not read; a dropped report would leave a blocked tag waiting on a load it
/// never finishes, so the op fails plainly instead.
fn lagged(scope: &OpScope<'_>) -> PlaywrightError {
    scope.fail(
        ErrorCode::Internal,
        "Neo fell behind the browser's events while this tag's Content Security Policy reports \
         were due, so it cannot tell whether the page blocked the tag",
    )
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
    element: TagElement,
) -> Result<Tagged, PlaywrightError> {
    // ts-rs cannot flatten Option<FrameTree>. Spell out its sole serialized field in
    // the original position, retaining omission when the best-effort snapshot fails.
    let frames = time::timeout(
        Duration::from_secs(2),
        frame_tree::snapshot(pw, scope, page.tab()),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .flatten();
    Ok(Tagged {
        element,
        frames: frames.map(FrameTree::into_frames),
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::Serialize;
    use serde_json::json;

    use super::{TagElement, Tagged};
    use crate::Op;
    use crate::op_output::to_output;
    use crate::op_scope::OpScope;
    use crate::page::frame_tree::FrameTree;
    use crate::target::CallHeader;
    use crate::test_env::TestEnv;
    use crate::world::FrameSeq;

    #[tokio::test]
    async fn tag_reply_preserves_the_flattened_before_bytes() -> Result<(), Box<dyn Error>> {
        #[derive(Serialize)]
        struct Before {
            element: serde_json::Value,
            #[serde(flatten)]
            frames: Option<FrameTree>,
        }
        let env = TestEnv::new();
        let op = Op::from_name("page.addScriptTag").ok_or("tag op exists")?;
        let scope = OpScope::new(&env, op, CallHeader::default());
        for has_frames in [false, true] {
            let element = json!({"$neo":"element", "page":{"$neo":"page","tab":7},
                "seq":3,"node":9,"display":"<script>"});
            let before = Before {
                element,
                frames: has_frames.then(FrameTree::default),
            };
            let after = Tagged {
                element: TagElement {
                    brand: "element",
                    page: serde_json::from_value(json!({"$neo":"page","tab":7}))?,
                    seq: FrameSeq(3),
                    node: 9,
                    display: "<script>".to_owned(),
                },
                frames: has_frames.then(Vec::new),
            };
            // Run uses this encoder before the host records JSON. It sorts object keys, so
            // direct struct serialization would measure declaration order instead of wire bytes.
            let before = to_output(&scope, &before)?;
            let after = to_output(&scope, &after)?;
            assert_eq!(
                serde_json::to_string(&after)?,
                serde_json::to_string(&before)?
            );
            let expected = if has_frames {
                r#"{"element":{"$neo":"element","display":"<script>","node":9,"page":{"$neo":"page","tab":7},"seq":3},"frames":[]}"#
            } else {
                r#"{"element":{"$neo":"element","display":"<script>","node":9,"page":{"$neo":"page","tab":7},"seq":3}}"#
            };
            assert_eq!(serde_json::to_string(&after)?, expected);
        }
        Ok(())
    }
}
