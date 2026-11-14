//! Sends probe requests into an isolated world and reads the replies. One of the three files in
//! core's `tests/page_js_guard.rs` ALLOWED list (with `world/isolated_world.rs` and
//! `page/evaluate_guard.rs`). It never logs params or results: a fill's text travels here
//! (STYLE P4).
//!
//! Every call names the world by its global object's remote object id, never by
//! `executionContextId`. Context ids are small per-renderer counters, so after a cross-process
//! navigation a cached id names some context of the new process: measured on BrowserOS, the id
//! of an isolated world reached the main world of a `srcdoc` iframe on the next site, where a
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
use crate::page::OpPage;
use crate::private_entry::PrivateStep;
use crate::probe::probe_reply::ProbeReply;
use crate::probe::probe_request::{ProbeCall, ProbePolicy, ProbeRequest};
use crate::world::{BackendNodeId, IsolatedWorld, Pin};

/// The function declaration every request is sent with. It must equal `CALL_FUNCTION` in
/// protocol.ts (a test checks it against probe-protocol.json). It runs on the world's global,
/// with the call envelope, an element handle's node or null as the call's root, and any further
/// nodes the request reads ([`call_with_nodes`]); a world without a booted probe answers
/// `uninstalled`, which Rust treats like a stale context.
pub(crate) const CALL_FUNCTION: &str = "function (call, root, ...nodes) { const probe = globalThis.__neoProbe; return probe ? probe.call(call, root ?? this, nodes) : { kind: \"uninstalled\" }; }";

impl ProbePolicy {
    /// Drop cannot borrow an ended operation or inspect its admitted page. Cleanup retains
    /// the captured authority and conservatively permits adopted-field discovery instead.
    pub(crate) fn for_cleanup(scope: &OpScope<'_>) -> Self {
        Self {
            conformance: scope.conformance(),
            may_have_other_frame: true,
        }
    }
}

/// Compute topology at the send seam for the exact world, after any retry or frame hop.
/// Conformance must not even inspect the frame metadata used only for private output.
fn policy(scope: &OpScope<'_>, page: &OpPage<'_>, world: &IsolatedWorld) -> ProbePolicy {
    with_frame_context(scope.conformance(), || {
        page.may_have_other_frame(scope, world)
    })
}

fn with_frame_context(conformance: bool, hint: impl FnOnce() -> bool) -> ProbePolicy {
    ProbePolicy {
        conformance,
        may_have_other_frame: conformance || hint(),
    }
}

/// The only requests allowed after an operation's scope has ended. A token-specific verdict
/// stops its own interceptor during navigation; it cannot stop a later operation's arm. The one
/// other request sent without an op is the camera's [`picture_question`].
pub(crate) enum Control {
    Disarm,
    Verdict { token: u32 },
    Forget { pin: Pin },
    ScreenshotMaskCleanup,
}

/// Best-effort detached cleanup has no general request overload. Its captured policy cannot
/// authorize a fresh read or action once the operation's admission and deadline are gone. The
/// camera's [`picture_question`] is the other sender outside an op, fixed to one request.
pub(crate) async fn cleanup(
    policy: ProbePolicy,
    world: &IsolatedWorld,
    control: Control,
) -> Result<ProbeReply, ProbeFailure> {
    let request = match control {
        Control::Disarm => ProbeRequest::Disarm,
        Control::Verdict { token } => ProbeRequest::Verdict { token },
        Control::Forget { pin } => ProbeRequest::Private {
            pin,
            step: PrivateStep::Forget,
        },
        Control::ScreenshotMaskCleanup => ProbeRequest::ScreenshotMaskCleanup,
    };
    send(world, &request, policy, Handed::NONE, Gesture::None).await
}

/// Neo's camera's one question, asked outside any op (protected-fill ruling R45): only
/// `privateShown`, which changes nothing in the page and reads no value out of it. It installs
/// no mask and reads neither policy field; the policy is only the envelope every call carries,
/// so it sends a fixed one. With [`cleanup`], the second sender that runs without an op.
///
/// `walk` is the camera's walk of closed shadow roots on demand (ruling R65,
/// `picture_check.rs`): closed roots resolved into `world` from the hosts [`private_hosts`]
/// handed over, which the world keeps for this question alone and never opens to its engine.
pub(crate) async fn picture_question(
    world: &IsolatedWorld,
    walk: Option<&[String]>,
) -> Result<ProbeReply, ProbeFailure> {
    let policy = ProbePolicy {
        conformance: false,
        may_have_other_frame: true,
    };
    let request = ProbeRequest::PrivateShown {
        walk: walk.is_some(),
    };
    let handed = Handed {
        root: None,
        nodes: walk.unwrap_or_default(),
    };
    send(world, &request, policy, handed, Gesture::None).await
}

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
            // Chromium can keep the document's context while retiring its remote handles,
            // for example across history restoration. Reinstall the probe before a read
            // retries; the missing receiver guarantees this call ran nothing.
            || message.contains("Could not find object with given id")
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
            Self::Gone(_) => f.write_str("the isolated world's document or session is gone"),
            Self::Cdp(error) => error.fmt(f),
            Self::Broken(why) => f.write_str(why),
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
    deep_serialized_value: Option<DeepNodes>,
}

/// Chromium's deep serialization names DOM nodes by backend id without walking their
/// children. Only the batch pin reader asks for it; ordinary calls omit this field.
#[derive(Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
enum DeepNodes {
    Array(Vec<DeepNode>),
    Null,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
enum DeepNode {
    Node {
        #[serde(rename = "backendNodeId")]
        backend_node_id: BackendNodeId,
    },
    Null,
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
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    request: &ProbeRequest,
) -> Result<ProbeReply, ProbeFailure> {
    send(
        world,
        request,
        policy(scope, page, world),
        Handed::NONE,
        Gesture::None,
    )
    .await
}

/// Raw installation tests intentionally own worlds without admitting a page or keeping an
/// operation alive. Their sender still speaks the production envelope with normal policy.
#[cfg(test)]
pub(crate) async fn call_for_tests(
    world: &IsolatedWorld,
    request: &ProbeRequest,
) -> Result<ProbeReply, ProbeFailure> {
    let policy = ProbePolicy {
        conformance: false,
        may_have_other_frame: true,
    };
    send(world, request, policy, Handed::NONE, Gesture::None).await
}

/// [`call`] with a user gesture, as upstream sends its own utility-world calls
/// (`evaluateWithArguments`, `userGesture: true`): `setContent`'s write, whose HTML's scripts
/// run inside the call and so with the frame's transient activation, as they do upstream.
pub(crate) async fn call_with_gesture(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    request: &ProbeRequest,
) -> Result<ProbeReply, ProbeFailure> {
    send(
        world,
        request,
        policy(scope, page, world),
        Handed::NONE,
        Gesture::User,
    )
    .await
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
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    request: &ProbeRequest,
    root: &str,
) -> Result<ProbeReply, ProbeFailure> {
    let handed = Handed {
        root: Some(root),
        nodes: &[],
    };
    send(
        world,
        request,
        policy(scope, page, world),
        handed,
        Gesture::None,
    )
    .await
}

/// One request with several nodes (remote objects in this world, from `DOM.resolveNode` with
/// its context id), which the probe reads in their order after an empty root: a snapshot's
/// clickable targets. With no nodes it is [`call`]. As with [`call_on`], an id a later document
/// took over fails the call as `Gone`, and never reaches a page's own world.
pub(crate) async fn call_with_nodes(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    request: &ProbeRequest,
    nodes: &[String],
) -> Result<ProbeReply, ProbeFailure> {
    let handed = Handed { root: None, nodes };
    send(
        world,
        request,
        policy(scope, page, world),
        handed,
        Gesture::None,
    )
    .await
}

/// The remote objects a call hands the probe after its request.
#[derive(Clone, Copy)]
struct Handed<'a> {
    /// The call's root: an element handle's node, read as `ProbeTarget::Root`.
    root: Option<&'a str>,
    /// Nodes a request reads in order, after the root.
    nodes: &'a [String],
}

impl Handed<'_> {
    const NONE: Self = Self {
        root: None,
        nodes: &[],
    };

    /// Whether the call carries any object of the world, which V8 checks against the world
    /// the call runs in.
    fn any(self) -> bool {
        self.root.is_some() || !self.nodes.is_empty()
    }
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
    world: &IsolatedWorld,
    request: &ProbeRequest,
    policy: ProbePolicy,
    handed: Handed<'_>,
    gesture: Gesture,
) -> Result<ProbeReply, ProbeFailure> {
    let mut params = json!({
        "objectId": world.global(),
        "functionDeclaration": CALL_FUNCTION,
        "arguments": call_arguments(request, policy, handed)?,
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
                if handed.any() && message.contains("same JavaScript world") =>
            {
                ProbeFailure::Gone(Ran::No)
            }
            failure => failure,
        })?;
    reply_of(called)
}

/// [`CALL_FUNCTION`]'s `arguments` for one call: the envelope, then the root when there is one
/// (`null` when only nodes follow), then the nodes.
fn call_arguments(
    request: &ProbeRequest,
    policy: ProbePolicy,
    handed: Handed<'_>,
) -> Result<Vec<Value>, ProbeFailure> {
    let call = serde_json::to_value(ProbeCall { request, policy }).map_err(|error| {
        ProbeFailure::Broken(format!("a probe request did not encode: {error}"))
    })?;
    let mut arguments = vec![json!({ "value": call })];
    if handed.any() {
        arguments.push(match handed.root {
            Some(root) => json!({ "objectId": root }),
            None => json!({ "value": null }),
        });
    }
    arguments.extend(handed.nodes.iter().map(|node| json!({ "objectId": node })));
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
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pin: Pin,
) -> Result<Option<String>, ProbeFailure> {
    let request = ProbeRequest::Pinned { pin };
    let params = json!({
        "objectId": world.global(),
        "functionDeclaration": CALL_FUNCTION,
        "arguments": call_arguments(&request, policy(scope, page, world), Handed::NONE)?,
        "returnByValue": false,
        // Chromium truncates property previews by UTF-16 code unit and can emit lone
        // surrogates for emoji attributes, which Rust's JSON decoder cannot read. The node
        // subtype is sufficient here; inspect only a non-node reply for `uninstalled` below.
        "generatePreview": false,
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
        _ if is_uninstalled(world, result.object_id.as_deref()).await? => {
            Err(ProbeFailure::Gone(Ran::No))
        }
        _ => Err(ProbeFailure::Broken(
            "the probe answered `pinned` with neither an element nor null".to_owned(),
        )),
    }
}

/// Transfers a pinned query's nodes to Rust in one isolated-world call. Deep serialization
/// carries backend ids directly, avoiding a `pinned` call and `DOM.describeNode` per match.
/// The result's remote array lives in the op's object group, keeping every node alive until
/// the main-world gateway has adopted it; no selector or Neo callback runs in the page world.
pub(crate) async fn pinned_nodes(
    scope: &OpScope<'_>,
    page: &OpPage<'_>,
    world: &IsolatedWorld,
    pins: Vec<Pin>,
) -> Result<Option<Vec<BackendNodeId>>, ProbeFailure> {
    let count = pins.len();
    let requests: Vec<ProbeRequest> = pins
        .into_iter()
        .map(|pin| ProbeRequest::Pinned { pin })
        .collect();
    // Use the same request gateway as single pins, including its uninstalled marker. A
    // batch runs synchronously so a re-key cannot mix pins from two probe lifetimes.
    let function = format!(
        "function (calls) {{ const call = ({CALL_FUNCTION}); \
         const nodes = calls.map(envelope => call.call(this, envelope)); \
         return nodes.some(node => node && !(node instanceof Node) && \
                           node.kind === 'uninstalled') ? null : nodes; }}"
    );
    let policy = policy(scope, page, world);
    let calls: Vec<ProbeCall<'_>> = requests
        .iter()
        .map(|request| ProbeCall { request, policy })
        .collect();
    let called: Called = world
        .session()
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": world.global(),
                "functionDeclaration": function,
                "arguments": [{ "value": calls }],
                "objectGroup": scope.object_group(world.session()),
                "serializationOptions": {
                    "serialization": "deep",
                    "maxDepth": 2,
                    "additionalParameters": { "maxNodeDepth": 0, "includeShadowTree": "none" }
                }
            }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    if let Some(exception) = called.exception_details {
        return Err(ProbeFailure::Broken(format!(
            "the probe threw: {}",
            exception.message()
        )));
    }
    match called.result.deep_serialized_value {
        Some(DeepNodes::Array(nodes)) if nodes.len() == count => Ok(nodes
            .into_iter()
            .map(|node| match node {
                DeepNode::Node { backend_node_id } => Some(backend_node_id),
                DeepNode::Null => None,
            })
            .collect()),
        Some(DeepNodes::Null) => Err(ProbeFailure::Gone(Ran::No)),
        Some(DeepNodes::Array(_)) | None => Err(ProbeFailure::Broken(
            "the probe returned an incomplete batch of pinned nodes".to_owned(),
        )),
    }
}

/// The hosts the camera's question has waiting (`privateHosts`, ruling R65), outside any op as
/// that question is: elements a trusted input event or the focus pointed at, which take no input
/// themselves and may carry a closed shadow root the world does not know. Handed over by deep
/// serialization, as [`pinned_nodes`] reads backend ids, so Rust takes no pin and no handle of a
/// host: the call's remote array lives in `group`, the walk's own object group, until the walk
/// ends. The page keeps the count of what it handed over: a host the focus pointed at once, one an
/// input event named once a reading until its root is kept, five readings at most.
/// No page script runs: the request's handler only reads the world's own list.
pub(crate) async fn private_hosts(
    world: &IsolatedWorld,
    group: &str,
    limit: u32,
) -> Result<Vec<BackendNodeId>, ProbeFailure> {
    let request = ProbeRequest::PrivateHosts { limit };
    let call = ProbeCall {
        request: &request,
        policy: ProbePolicy {
            conformance: false,
            may_have_other_frame: true,
        },
    };
    // A reply that is no list (`uninstalled`, or an error reply) comes back as null.
    let function = format!(
        "function (call) {{ const hosts = ({CALL_FUNCTION}).call(this, call); \
         return Array.isArray(hosts) ? hosts : null; }}"
    );
    let called: Called = world
        .session()
        .send(
            "Runtime.callFunctionOn",
            json!({
                "objectId": world.global(),
                "functionDeclaration": function,
                "arguments": [{ "value": call }],
                "objectGroup": group,
                "serializationOptions": {
                    "serialization": "deep",
                    "maxDepth": 2,
                    "additionalParameters": { "maxNodeDepth": 0, "includeShadowTree": "none" }
                }
            }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    if let Some(exception) = called.exception_details {
        return Err(ProbeFailure::Broken(format!(
            "the probe threw: {}",
            exception.message()
        )));
    }
    match called.result.deep_serialized_value {
        Some(DeepNodes::Array(nodes)) => Ok(nodes
            .into_iter()
            .filter_map(|node| match node {
                DeepNode::Node { backend_node_id } => Some(backend_node_id),
                DeepNode::Null => None,
            })
            .collect()),
        Some(DeepNodes::Null) => Err(ProbeFailure::Gone(Ran::No)),
        None => Err(ProbeFailure::Broken(
            "the probe answered `privateHosts` with no list".to_owned(),
        )),
    }
}

/// Whether a by-handle result is the `{ kind: "uninstalled" }` of [`CALL_FUNCTION`].
async fn is_uninstalled(world: &IsolatedWorld, object: Option<&str>) -> Result<bool, ProbeFailure> {
    let Some(object) = object else {
        return Ok(false);
    };
    // This receiver came from CALL_FUNCTION in the isolated world. Reading only the marker
    // avoids previews of page-owned properties, and a stale receiver still fails as Gone.
    let called: Called = world.session().send(
        "Runtime.callFunctionOn",
        json!({ "objectId": object, "functionDeclaration": "function () { return this.kind === 'uninstalled'; }",
                "returnByValue": true }),
    ).await.map_err(ProbeFailure::from_core)?;
    if let Some(exception) = called.exception_details {
        return Err(ProbeFailure::Broken(format!(
            "the probe threw: {}",
            exception.message()
        )));
    }
    Ok(called.result.value == Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::slice;

    use browseros_cdp::CdpError;
    use browseros_core::CoreError;
    use serde_json::{Value, json};

    use super::{
        CALL_FUNCTION, Handed, ProbeFailure, Ran, call_arguments, reply_of, with_frame_context,
    };
    use crate::probe::probe_reply::ProbeReply;
    use crate::probe::probe_request::{ProbePolicy, ProbeRequest};

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
    fn tests_that_a_call_sends_its_policy_envelope_then_its_root() -> Result<(), Box<dyn Error>> {
        let request = ProbeRequest::Describe;
        let policy = ProbePolicy {
            conformance: false,
            may_have_other_frame: true,
        };
        let envelope = json!({ "value": {
            "request": { "op": "describe" },
            "policy": { "conformance": false, "mayHaveOtherFrame": true },
        } });
        assert_eq!(
            call_arguments(
                &request,
                policy,
                Handed {
                    root: None,
                    nodes: &[]
                }
            )?,
            call_arguments(&request, policy, Handed::NONE)?,
            "a call with no nodes is the plain call"
        );
        assert_eq!(
            call_arguments(&request, policy, Handed::NONE)?.as_slice(),
            slice::from_ref(&envelope),
            "a call on the world's global sends the envelope alone"
        );
        let rooted = Handed {
            root: Some("node-1"),
            nodes: &[],
        };
        assert_eq!(
            call_arguments(&request, policy, rooted)?,
            [envelope.clone(), json!({ "objectId": "node-1" }),],
            "an element handle's node follows as the call's root"
        );
        let nodes = ["node-2".to_owned(), "node-3".to_owned()];
        let several = Handed {
            root: None,
            nodes: &nodes,
        };
        assert_eq!(
            call_arguments(&request, policy, several)?,
            [
                envelope,
                json!({ "value": null }),
                json!({ "objectId": "node-2" }),
                json!({ "objectId": "node-3" }),
            ],
            "nodes follow an empty root, in order, so the probe reads them positionally"
        );
        Ok(())
    }

    #[test]
    fn conformance_skips_even_the_lazy_frame_metadata_read() {
        let read = Cell::new(false);
        let policy = with_frame_context(true, || {
            read.set(true);
            false
        });
        assert!(policy.conformance);
        assert!(
            !read.get(),
            "upstream diagnostics do not inspect private topology"
        );
        let policy = with_frame_context(false, || {
            read.set(true);
            false
        });
        assert!(read.get());
        assert!(!policy.conformance);
        assert!(!policy.may_have_other_frame);
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
            (
                protocol_error("Could not find object with given id"),
                Ran::No,
            ),
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
