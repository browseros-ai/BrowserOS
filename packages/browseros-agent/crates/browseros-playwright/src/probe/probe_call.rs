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
//!
//! A call may carry a [`ProbeMask`], the values of every sensitive field of its frame's site,
//! as `CALL_FUNCTION`'s third argument. The probe masks page text with it before Playwright's
//! engine cuts or quotes that text for a preview or a locator hint; a call without one whose
//! reply would hold such text answers `needsMask` (`resolve::Resolver` asks again with it).

use std::error::Error;
use std::fmt;

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use browseros_core::page_js::is_stale_context;
use browseros_core::sensitive_fields::FieldValues;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::op_scope::OpScope;
use crate::probe::probe_reply::ProbeReply;
use crate::probe::probe_request::ProbeRequest;
use crate::world::{Pin, SealedWorld};

/// The function declaration every request is sent with. It must equal `CALL_FUNCTION` in
/// protocol.ts (a test checks it against probe-protocol.json). It runs on the world's global,
/// with the request, an element handle's node or null as the call's root, and the call's mask
/// when it has one; a world without a booted probe answers `uninstalled`, which Rust treats
/// like a stale context.
pub(crate) const CALL_FUNCTION: &str = "function (request, root, mask) { const probe = globalThis.__neoProbe; return probe ? probe.call(request, root ?? this, mask) : { kind: \"uninstalled\" }; }";

/// The values of one frame's site that a call hands the probe as protocol.ts's `Mask`: those
/// long enough to be masked wherever they occur (`FieldValues::registrable`). A short value is
/// masked only where a whole string equals it, which no cut can make of it, so it stays out.
/// They go only to a world of that site, whose own page script could read those fields anyway,
/// and only into its sealed world. It borrows the values, keeping no copy of its own, never
/// prints and has no `Serialize` (STYLE P4): [`call_arguments`] is the one place that encodes
/// them.
#[derive(Clone, Copy)]
pub(crate) struct ProbeMask<'a>(&'a FieldValues);

impl<'a> ProbeMask<'a> {
    /// The mask of `values`.
    pub(crate) fn of(values: &'a FieldValues) -> Self {
        Self(values)
    }
}

impl fmt::Debug for ProbeMask<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[field values]")
    }
}

/// Why a probe call or an install produced no reply.
#[derive(Debug)]
pub(crate) enum ProbeFailure {
    /// The world is gone: its document was replaced (a stale context), its session detached,
    /// its frame is no longer there, or it holds no booted probe (`uninstalled`). The caller
    /// decides what that means: re-install and retry once on resolution, `element_detached`
    /// at once after a ref jump.
    Gone,
    /// CDP failed for any other reason: the transport, a closed target.
    Cdp(CoreError),
    /// The page asset and the crate disagree: a reply this build cannot read, a probe that
    /// threw, a stale asset.
    Broken(String),
}

impl ProbeFailure {
    /// Core's failure as a probe failure. A stale context, a lost session and a frame that is
    /// no longer in the page all mean the world is gone.
    pub(crate) fn from_core(error: CoreError) -> Self {
        let frame_gone = matches!(
            &error,
            CoreError::Cdp(CdpError::Protocol { message, .. })
                if message.contains("No frame for given id found")
        );
        if frame_gone
            || is_stale_context(&error)
            || matches!(error, CoreError::Cdp(CdpError::SessionGone))
        {
            Self::Gone
        } else {
            Self::Cdp(error)
        }
    }
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gone => f.write_str("the sealed world's document or session is gone"),
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
    send(world, request, None, None).await
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
    send(world, request, Some(root), None).await
}

/// [`call`], or [`call_on`] with `root`, carrying `mask`: what a request whose reply previews
/// page text is sent with, so that it never answers `needsMask`.
pub(crate) async fn call_masked(
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
    mask: ProbeMask<'_>,
) -> Result<ProbeReply, ProbeFailure> {
    send(world, request, root, Some(mask)).await
}

/// One `Runtime.callFunctionOn` of [`CALL_FUNCTION`] on the world's global.
async fn send(
    world: &SealedWorld,
    request: &ProbeRequest,
    root: Option<&str>,
    mask: Option<ProbeMask<'_>>,
) -> Result<ProbeReply, ProbeFailure> {
    let params = json!({
        "objectId": world.global(),
        "functionDeclaration": CALL_FUNCTION,
        "arguments": call_arguments(request, root, mask)?,
        "returnByValue": true,
        "awaitPromise": true,
    });
    let called = world
        .session()
        .send::<_, Value>("Runtime.callFunctionOn", params)
        .await
        .map_err(|error| match ProbeFailure::from_core(error) {
            ProbeFailure::Cdp(CoreError::Cdp(CdpError::Protocol { message, .. }))
                if root.is_some() && message.contains("same JavaScript world") =>
            {
                ProbeFailure::Gone
            }
            failure => failure,
        })?;
    reply_of(called)
}

/// [`CALL_FUNCTION`]'s `arguments` for one call: the request, the root (null when only a mask
/// follows), and the mask's values long enough to be masked wherever they occur, encoded here
/// by hand because nothing else may encode them.
fn call_arguments(
    request: &ProbeRequest,
    root: Option<&str>,
    mask: Option<ProbeMask<'_>>,
) -> Result<Vec<Value>, ProbeFailure> {
    let request = serde_json::to_value(request).map_err(|error| {
        ProbeFailure::Broken(format!("a probe request did not encode: {error}"))
    })?;
    let mut arguments = vec![json!({ "value": request })];
    match (root, mask) {
        (Some(root), _) => arguments.push(json!({ "objectId": root })),
        (None, Some(_)) => arguments.push(json!({ "value": null })),
        (None, None) => {}
    }
    if let Some(ProbeMask(values)) = mask {
        let values = values
            .registrable()
            .map(|value| Value::String(value.to_owned()))
            .collect();
        arguments.push(json!({ "value": Value::Array(values) }));
    }
    Ok(arguments)
}

/// The reply in a by-value `Runtime.callFunctionOn` result; `uninstalled` is `Gone`. The
/// install decodes its `described` with it too. A read's text stays raw in its [`PageText`]
/// here: only the op knows the read frame's site, whose values mask it (`page::SiteValues`).
///
/// [`PageText`]: crate::probe::PageText
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
        Ok(ProbeReply::Uninstalled) => Err(ProbeFailure::Gone),
        Ok(reply) => Ok(reply),
        Err(error) => Err(ProbeFailure::Broken(format!(
            "the probe answered a reply this build cannot read ({error}); rebuild \
             packages/browseros-playwright-js"
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
        _ if is_uninstalled(result.preview.as_ref()) => Err(ProbeFailure::Gone),
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
    use std::marker::PhantomData;

    use browseros_cdp::CdpError;
    use browseros_core::CoreError;
    use browseros_core::sensitive_fields::FieldValues;
    use serde::Serialize;
    use serde_json::{Value, json};

    use super::{CALL_FUNCTION, ProbeFailure, ProbeMask, call_arguments, reply_of};
    use crate::probe::probe_reply::ProbeReply;
    use crate::probe::probe_request::ProbeRequest;

    const PROTOCOL: &str = include_str!("../../tests/data/probe-protocol.json");

    /// `<Serializes<T>>::YES` is the inherent `true` when `T: Serialize`, and the trait's
    /// `false` otherwise.
    struct Serializes<T>(PhantomData<T>);

    trait NotSerialize {
        const YES: bool = false;
    }

    impl<T> NotSerialize for Serializes<T> {}

    impl<T: Serialize> Serializes<T> {
        const YES: bool = true;
    }

    // A mask is raw field values, which never print, serialize or log (STYLE P4): the build
    // fails once it can be serialized, and the check sees a type that can.
    const _: () = assert!(!<Serializes<ProbeMask<'static>>>::YES);
    const _: () = assert!(<Serializes<ProbeRequest>>::YES);

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
    fn tests_that_a_mask_holds_only_long_values_and_never_prints() -> Result<(), Box<dyn Error>> {
        let values = FieldValues::from_sweep(&json!(["hunter2", "987"]));
        let mask = ProbeMask::of(&values);
        let arguments = call_arguments(&ProbeRequest::Sweep, None, Some(mask))?;
        assert_eq!(
            arguments,
            [
                json!({ "value": { "op": "sweep" } }),
                json!({ "value": null }),
                json!({ "value": ["hunter2"] }),
            ],
            "the mask is CALL_FUNCTION's third argument, and a short value is masked only \
             whole, which no cut makes of it"
        );
        assert!(
            !format!("{mask:?}").contains("hunter2"),
            "a mask is secrets (STYLE P4)"
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
        for gone in [
            protocol_error("Cannot find context with specified id"),
            protocol_error("No frame for given id found"),
            CoreError::Cdp(CdpError::SessionGone),
        ] {
            assert!(
                matches!(ProbeFailure::from_core(gone), ProbeFailure::Gone),
                "the world must be re-installed or the ref fail, never retried as is"
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
        assert!(matches!(uninstalled, Err(ProbeFailure::Gone)));
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

    #[test]
    fn tests_that_every_string_a_read_returns_is_masked_once_with_every_value()
    -> Result<(), Box<dyn Error>> {
        let read = reply_of(json!({ "result": { "type": "object", "value": {
            "kind": "read", "sensitive": null, "sweep": ["s3cret&<pw>"],
            "value": { "pin": 4, "preview": "<p>s3cret&amp;&lt;pw&gt; abcdefgh</p>", "visible": true },
        } } }))?;
        let ProbeReply::Read { value, sweep, .. } = read else {
            return Err("a read is a read".into());
        };
        let site = sweep.union(FieldValues::from_sweep(&json!(["abcd", "abcdefgh"])));
        assert_eq!(
            value.masked(&site),
            json!({ "pin": 4, "preview": "<p>[private] [private]</p>", "visible": true }),
            "markup's escaped form too, a longer value of another frame whole, and a number \
             stays a number"
        );
        let all = reply_of(json!({ "result": { "type": "object", "value": {
            "kind": "values", "count": 3, "sweep": ["s3cret&<pw>", "123"],
            "values": ["before s3cret&<pw> after", "123", "1234"],
        } } }))?;
        let ProbeReply::Values { values, sweep, .. } = all else {
            return Err("a readAll is values".into());
        };
        let masked: Vec<Value> = values
            .into_iter()
            .map(|value| value.masked(&sweep))
            .collect();
        assert_eq!(
            masked,
            [
                json!("before [private] after"),
                json!("[private]"),
                json!("1234")
            ],
            "a short value only where it is the whole string"
        );
        Ok(())
    }
}
