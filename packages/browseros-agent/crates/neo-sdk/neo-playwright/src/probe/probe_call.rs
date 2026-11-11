//! Sends probe requests into a sealed world and reads the replies. One of the three files in
//! core's `tests/page_js_guard.rs` ALLOWED list (with `world/sealed_world.rs` and
//! `page/evaluate_guard.rs`). It never logs params or results: a fill's text travels here
//! (STYLE P4).
//!
//! Every call names the world by its global object's remote object id, never by
//! `executionContextId`. Context ids are small per-renderer counters, so after a cross-process
//! navigation a cached id names some context of the new process: measured on BrowserOS, the id
//! of a sealed world reached the main world of a `srcdoc` iframe on the next site, where a
//! hostile page could have defined `__neoProbe` and read the request. An object id carries its
//! renderer isolate's id, and the same call on a stale handle fails with "Cannot find context
//! with specified id", which [`ProbeFailure::from_core`] reads as gone.

use std::error::Error;
use std::fmt;

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use browseros_core::page_js::is_stale_context;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::probe::probe_reply::ProbeReply;
use crate::probe::probe_request::ProbeRequest;
use crate::world::{Pin, SealedWorld};

/// The function declaration every request is sent with. It must equal `CALL_FUNCTION` in
/// protocol.ts (a test checks it against probe-protocol.json). It runs on the world's global,
/// with the request and an element handle's node or null as the call's root; a world without
/// a booted probe answers `uninstalled`, which Rust treats like a stale context.
pub(crate) const CALL_FUNCTION: &str = "function (request, root) { const probe = globalThis.__neoProbe; return probe ? probe.call(request, root ?? this) : { kind: \"uninstalled\" }; }";

/// Why a probe call or an install produced no reply.
#[derive(Debug)]
pub(crate) enum ProbeFailure {
    /// The world is gone: its document was replaced (a stale context), its session detached,
    /// its frame is no longer there, or it holds no booted probe (`uninstalled`); [`Ran`] says
    /// whether the call may have run before it went. The caller decides what that means:
    /// re-install and retry once on resolution, `element_detached` at once after a ref jump,
    /// and never a second write after one that may have run.
    Gone(Ran),
    /// CDP failed for any other reason: the transport, a closed target.
    Cdp(CoreError),
    /// The page asset and the crate disagree: a reply this build cannot read, a probe that
    /// threw, a stale asset.
    Broken(String),
    /// An install's scan for hidden shadow roots missed the frame's budget (`FRAME_BUDGET`).
    /// A world whose closed roots the seal may not see is not sealed, so the op is refused.
    Late,
}

/// Whether a call whose world went away ran there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ran {
    /// It did not: the world was gone when the call reached it.
    No,
    /// It may have: the call was still pending when its document went away, and Chromium cut it
    /// off ([`CUT_OFF`]). A read is safe to send again in the new document, and upstream retries
    /// reads and element actions after the same errors; a document write is not, because the
    /// first may have run the HTML's inline scripts.
    Maybe,
}

/// How Chromium answers a call its document's navigation cut off while it was pending. The
/// first answers a call into a document that left for another (measured: a `waitForSelector`
/// poll during a cross-site `goto`, and main-world calls under a same-site or a cross-site
/// `location.href`), the second one whose context was destroyed while it awaited. Upstream
/// turns both into "Execution context was destroyed, most likely because of a navigation"
/// (`crExecutionContext.rewriteError`, which ends it with a period, or the frame's own word
/// for a context it saw go, which does not). Core's `is_stale_context` also takes the second
/// for a stale context, so these are read first.
const CUT_OFF: [&str; 2] = [NAVIGATED_OR_CLOSED, "Execution context was destroyed"];

/// How Chromium fails a call into a document that left for another while the call was pending,
/// same-site or cross-site (measured on BrowserOS 155, spike S-cut-off), before it sends any
/// event of the context going.
pub(crate) const NAVIGATED_OR_CLOSED: &str = "Inspected target navigated or closed";

impl Ran {
    /// Whether `error` says a call's world went away, and if so whether the call may have run
    /// there. A stale context, a lost session and a frame that is no longer in the page mean
    /// the world was gone before the call ran; a call cut off by its document's navigation
    /// ([`CUT_OFF`]) went while it was pending. `None` for any other error. Probe calls and the
    /// evaluate guard's main-world calls read their errors with it alike.
    pub(crate) fn when_gone(error: &CoreError) -> Option<Self> {
        let message = match error {
            CoreError::Cdp(CdpError::Protocol { message, .. }) => message.as_str(),
            _ => "",
        };
        if CUT_OFF.iter().any(|cut| message.contains(cut)) {
            Some(Self::Maybe)
        } else if message.contains("No frame for given id found")
            || is_stale_context(error)
            || matches!(error, CoreError::Cdp(CdpError::SessionGone))
        {
            Some(Self::No)
        } else {
            None
        }
    }
}

impl ProbeFailure {
    /// Core's failure as a probe failure: `Gone` when the world went away
    /// ([`Ran::when_gone`]), else the CDP failure itself.
    pub(crate) fn from_core(error: CoreError) -> Self {
        match Ran::when_gone(&error) {
            Some(ran) => Self::Gone(ran),
            None => Self::Cdp(error),
        }
    }
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone(_) => f.write_str("the sealed world's document or session is gone"),
            Self::Cdp(error) => error.fmt(f),
            Self::Broken(why) => f.write_str(why),
            Self::Late => f.write_str("the frame did not answer Neo's privacy scan in time"),
        }
    }
}

impl Error for ProbeFailure {}

/// `Runtime.callFunctionOn`'s result.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Called {
    result: RemoteObject,
    #[serde(default)]
    exception_details: Option<ExceptionDetails>,
}

/// The parts of a `Runtime.RemoteObject` the probe calls read.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    #[serde(default)]
    value: Value,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    object_id: Option<String>,
    #[serde(default)]
    preview: Option<ObjectPreview>,
}

#[derive(Deserialize)]
struct ObjectPreview {
    #[serde(default)]
    properties: Vec<PreviewProperty>,
}

#[derive(Deserialize)]
struct PreviewProperty {
    name: String,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    #[serde(default)]
    exception: Option<ExceptionObject>,
}

#[derive(Deserialize)]
struct ExceptionObject {
    #[serde(default)]
    description: Option<String>,
}

impl ExceptionDetails {
    /// The thrown error's message line, or CDP's summary.
    fn message(&self) -> String {
        self.exception
            .as_ref()
            .and_then(|exception| exception.description.as_deref())
            .and_then(|description| description.lines().next())
            .unwrap_or(&self.text)
            .to_owned()
    }
}

/// One request, answered by value.
pub(crate) async fn call(
    world: &SealedWorld,
    request: &ProbeRequest,
) -> Result<ProbeReply, ProbeFailure> {
    send(world, request, None, Gesture::None).await
}

/// [`call`] with a user gesture, as upstream sends its own utility-world calls
/// (`evaluateWithArguments`, `userGesture: true`): `setContent`'s write, whose HTML's scripts
/// run inside the call and so with the frame's transient activation, as they do upstream.
pub(crate) async fn call_with_gesture(
    world: &SealedWorld,
    request: &ProbeRequest,
) -> Result<ProbeReply, ProbeFailure> {
    send(world, request, None, Gesture::User).await
}

/// One request about an element handle's node (`root`, a remote object in this world), which
/// the probe reads as `ProbeTarget::Root`.
///
/// The call still goes to the world's global, with the node as the second argument, never to
/// the node itself. `root` comes from `DOM.resolveNode` with the world's context id, which a
/// later document may have taken over; V8 refuses an argument from another world than the
/// call's target ("Argument should belong to the same JavaScript world as target object"),
/// and that refusal is `Gone`. So a stale id can fail the call, but can never hand a request
/// to a page's own world, as calling on the node's object would.
pub(crate) async fn call_on(
    world: &SealedWorld,
    request: &ProbeRequest,
    root: &str,
) -> Result<ProbeReply, ProbeFailure> {
    send(world, request, Some(root), Gesture::None).await
}

/// Whether a probe call carries a user gesture.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gesture {
    None,
    /// `userGesture: true`: the call activates the frame, as a click would.
    User,
}

/// One `Runtime.callFunctionOn` of [`CALL_FUNCTION`] on the world's global.
async fn send(
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
    gesture: Gesture,
) -> Result<ProbeReply, ProbeFailure> {
    let mut params = json!({
        "objectId": world.global(),
        "functionDeclaration": CALL_FUNCTION,
        "arguments": call_arguments(request, root)?,
        "returnByValue": true,
        "awaitPromise": true,
    });
    if gesture == Gesture::User
        && let Some(params) = params.as_object_mut()
    {
        params.insert("userGesture".to_owned(), Value::Bool(true));
    }
    let called = world
        .session()
        .send::<_, Value>("Runtime.callFunctionOn", params)
        .await
        .map_err(|error| match ProbeFailure::from_core(error) {
            ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { message, .. }))
                if root.is_some() && message.contains("same JavaScript world") =>
            {
                ProbeFailure::Gone(Ran::No)
            }
            failure => failure,
        })?;
    reply_of(called)
}

/// [`CALL_FUNCTION`]'s `arguments` for one call: the request, then the root when there is one.
fn call_arguments(request: &ProbeRequest, root: Option<&str>) -> Result<Vec<Value>, ProbeFailure> {
    let request = serde_json::to_value(request).map_err(|error| {
        ProbeFailure::Broken(format!("a probe request did not encode: {error}"))
    })?;
    let mut arguments = vec![json!({ "value": request })];
    if let Some(root) = root {
        arguments.push(json!({ "objectId": root }));
    }
    Ok(arguments)
}

/// The reply in a by-value `Runtime.callFunctionOn` result; `uninstalled` is `Gone`. The
/// install decodes its `described` with it too.
pub(crate) fn reply_of(called: Value) -> Result<ProbeReply, ProbeFailure> {
    let called: Called = serde_json::from_value(called)
        .map_err(|error| ProbeFailure::Broken(format!("CDP returned no call result: {error}")))?;
    if let Some(exception) = called.exception_details {
        return Err(ProbeFailure::Broken(format!(
            "the probe threw: {}",
            exception.message()
        )));
    }
    match serde_json::from_value(called.result.value) {
        Ok(ProbeReply::Uninstalled) => Err(ProbeFailure::Gone(Ran::No)),
        Ok(reply) => Ok(reply),
        Err(error) => Err(ProbeFailure::Broken(format!(
            "the probe answered a reply this build cannot read ({error}); rebuild \
             crates/neo-sdk/neo-playwright-js"
        ))),
    }
}

/// The element `pin` names, as a remote object in this op's object group (released when the
/// op ends), for `DOM.describeNode` and other by-node CDP calls. `None` once the pin is gone.
pub(crate) async fn pinned_object(
    scope: &OpScope<'_>,
    world: &SealedWorld,
    pin: Pin,
) -> Result<Option<String>, ProbeFailure> {
    let request = serde_json::to_value(ProbeRequest::Pinned { pin }).map_err(|error| {
        ProbeFailure::Broken(format!("a probe request did not encode: {error}"))
    })?;
    let params = json!({
        "objectId": world.global(),
        "functionDeclaration": CALL_FUNCTION,
        "arguments": [{ "value": request }],
        "returnByValue": false,
        // By handle the reply's fields cannot be read, so a preview tells `uninstalled` (a
        // plain object) apart from the element.
        "generatePreview": true,
        "objectGroup": scope.object_group(world.session()),
    });
    let called = world
        .session()
        .send::<_, Value>("Runtime.callFunctionOn", params)
        .await
        .map_err(ProbeFailure::from_core)?;
    let called: Called = serde_json::from_value(called)
        .map_err(|error| ProbeFailure::Broken(format!("CDP returned no call result: {error}")))?;
    if let Some(exception) = called.exception_details {
        return Err(ProbeFailure::Broken(format!(
            "the probe threw: {}",
            exception.message()
        )));
    }
    let result = called.result;
    match result.subtype.as_deref() {
        Some("node") => Ok(result.object_id),
        Some("null") => Ok(None),
        _ if is_uninstalled(result.preview.as_ref()) => Err(ProbeFailure::Gone(Ran::No)),
        _ => Err(ProbeFailure::Broken(
            "the probe answered `pinned` with neither an element nor null".to_owned(),
        )),
    }
}

/// Whether a by-handle result is the `{ kind: "uninstalled" }` of [`CALL_FUNCTION`].
fn is_uninstalled(preview: Option<&ObjectPreview>) -> bool {
    preview.is_some_and(|preview| {
        preview.properties.iter().any(|property| {
            property.name == "kind" && property.value.as_deref() == Some("uninstalled")
        })
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_cdp::CdpError;
    use browseros_core::CoreError;
    use serde_json::{Value, json};

    use super::{CALL_FUNCTION, ProbeFailure, Ran, call_arguments, reply_of};
    use crate::probe::probe_reply::ProbeReply;
    use crate::probe::probe_request::ProbeRequest;

    const PROTOCOL: &str = include_str!("../../tests/data/probe-protocol.json");

    #[test]
    fn tests_that_the_call_function_is_the_one_the_probe_documents() -> Result<(), Box<dyn Error>> {
        let protocol: Value = serde_json::from_str(PROTOCOL)?;
        assert_eq!(
            protocol["callFunction"].as_str(),
            Some(CALL_FUNCTION),
            "Rust sends protocol.ts's CALL_FUNCTION, byte for byte"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_call_sends_the_request_then_its_root() -> Result<(), Box<dyn Error>> {
        let request = ProbeRequest::Focused;
        assert_eq!(
            call_arguments(&request, None)?,
            [json!({ "value": { "op": "focused" } })],
            "a call on the world's global sends the request alone"
        );
        assert_eq!(
            call_arguments(&request, Some("node-1"))?,
            [
                json!({ "value": { "op": "focused" } }),
                json!({ "objectId": "node-1" }),
            ],
            "an element handle's node follows as the call's root"
        );
        Ok(())
    }

    fn protocol_error(message: &str) -> CoreError {
        CoreError::Cdp(CdpError::Protocol {
            code: -32000,
            message: message.to_owned(),
        })
    }

    #[test]
    fn tests_that_a_replaced_document_or_a_lost_session_is_gone() {
        for (gone, ran) in [
            (
                protocol_error("Cannot find context with specified id"),
                Ran::No,
            ),
            (protocol_error("No frame for given id found"), Ran::No),
            (CoreError::Cdp(CdpError::SessionGone), Ran::No),
            (
                protocol_error("Inspected target navigated or closed"),
                Ran::Maybe,
            ),
            (
                protocol_error("Execution context was destroyed."),
                Ran::Maybe,
            ),
        ] {
            assert!(
                matches!(ProbeFailure::from_core(gone), ProbeFailure::Gone(found) if found == ran),
                "the world must be re-installed or the ref fail, never retried as is, and a \
                 call cut off while pending may have run ({ran:?})"
            );
        }
        let ProbeFailure::Cdp(lost) =
            ProbeFailure::from_core(CoreError::Cdp(CdpError::ConnectionLost))
        else {
            panic!("a lost connection is not a gone world");
        };
        assert!(
            matches!(lost, CoreError::Cdp(CdpError::ConnectionLost)),
            "core's error is kept for the op's own code"
        );
    }

    #[test]
    fn tests_that_replies_decode_and_uninstalled_is_gone() -> Result<(), Box<dyn Error>> {
        let reply =
            reply_of(json!({ "result": { "type": "object", "value": { "kind": "none" } } }));
        assert!(matches!(reply, Ok(ProbeReply::None { stale_ref: None })));
        let uninstalled =
            reply_of(json!({ "result": { "type": "object", "value": { "kind": "uninstalled" } } }));
        assert!(matches!(uninstalled, Err(ProbeFailure::Gone(_))));
        let thrown = reply_of(json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": { "text": "Uncaught", "exception": { "description": "TypeError: x is not a function\n    at y" } },
        }));
        let Err(ProbeFailure::Broken(message)) = thrown else {
            return Err("a probe that throws is a broken asset".into());
        };
        assert_eq!(message, "the probe threw: TypeError: x is not a function");
        let unreadable =
            reply_of(json!({ "result": { "type": "object", "value": { "kind": "teleported" } } }));
        assert!(matches!(unreadable, Err(ProbeFailure::Broken(_))));
        Ok(())
    }
}
