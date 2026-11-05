//! Main-world scripts behind sdk-privacy's sweep (design D14), for `page.evaluate` and
//! `page.waitForFunction`. One of the three files in core's `tests/page_js_guard.rs` ALLOWED
//! list: the only code that sends script to a page's main world.
//!
//! ```text
//!  EvaluateGuard::sweep_before   sealed `sweep` in the main frame and every same-process frame
//!  EvaluateGuard::run            name the main-world document, then
//!                                OpScope::send_effect ──▶ Sent, then
//!                                Runtime.callFunctionOn on that document ──▶ Unmasked
//!  EvaluateGuard::finish         sweep again; mask the result, or the thrown message, with both
//!  drop                          PlaywrightHooks::values_swept with every value either sweep found
//! ```
//!
//! A main-world script reads the page's own, unsealed getters, so whatever it returns may hold a
//! password the page shows. The sweeps hand Rust the raw value of every sensitive field the
//! script could reach: the main frame's, and those of the frames in the same renderer, which are
//! the documents a main-world script can touch. What the script produced is an [`Unmasked`],
//! whose only way out of the op is [`EvaluateGuard::finish`]. Results are always masked; there
//! is no refuse mode (build-design graft 3).
//!
//! The script runs on the main-world document node, never on an execution context id: context
//! ids are per-renderer counters, and after a cross-process navigation a cached one can name a
//! context of another world (measured, see `probe_call`). The sealed world names its document by
//! handle, and `DOM.resolveNode` without a context id resolves that node in its main world.

use std::mem;
use std::num::FpCategory;

use browseros_cdp::CdpError;
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{CoreError, FrameId, PageId, ProtocolSession};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use crate::op_scope::{OpScope, Sent};
use crate::page::OpPage;
use crate::page::document_watch::{FrameTreeReply, decode};
use crate::page::world_call::{unexpected, with_world};
use crate::probe::{self, ProbeFailure, ProbeReply, ProbeRequest};
use crate::resolve::failed;
use crate::world::{Endpoint, Place, SealedWorld, SessionKind};
use crate::{ErrorCode, Playwright, PlaywrightError};

/// The function every main-world script runs in: upstream's `normalizeEvaluationExpression`
/// (a method shorthand becomes a function) and `UtilityScript.evaluate` (a function's source is
/// called with the argument; an expression's value is the result). `eval` and `new Function` work
/// here even under a page CSP that forbids them, because Chromium allows code generation inside
/// a call from the inspector (measured on BrowserOS with `script-src 'self'`).
const MAIN_WORLD_FUNCTION: &str = r"function (expression, isFunction, arg) {
  expression = expression.trim();
  if (isFunction) {
    try {
      new Function('(' + expression + ')');
    } catch (_) {
      expression = expression.startsWith('async ') ? 'async function ' + expression.substring(6) : 'function ' + expression;
      try {
        new Function('(' + expression + ')');
      } catch (_) {
        throw new Error('Passed function is not well-serializable!');
      }
    }
  }
  if (/^(async)?\s*function(\s|\()/.test(expression)) expression = '(' + expression + ')';
  let result = globalThis.eval(expression);
  if (isFunction === true || (isFunction === undefined && typeof result === 'function')) result = result(arg);
  return result;
}";

/// Names the document from the sealed world, by handle, so `DOM.describeNode` can name its node.
const SEALED_DOCUMENT: &str = "function () { return document; }";

/// A main-world script as the prelude sends it: upstream's `evaluateExpression` parameters, the
/// argument as JSON. v1 passes JSON only, so `Date`, `RegExp`, `Map` and `undefined` inside an
/// argument are listed deviations. No `Debug`: the argument may be a password.
pub(crate) struct Script {
    /// `String(pageFunction)`: a function's source, or an expression.
    expression: String,
    /// `typeof pageFunction === 'function'`; `None` detects it in the page, as upstream does.
    is_function: Option<bool>,
    /// The argument; `None` is `undefined`.
    arg: Option<Value>,
}

impl Script {
    pub(crate) fn new(expression: String, is_function: Option<bool>, arg: Option<Value>) -> Self {
        Self {
            expression,
            is_function,
            arg,
        }
    }

    /// The three `CallArgument`s of [`MAIN_WORLD_FUNCTION`]; `{}` is `undefined`.
    fn arguments(&self) -> Value {
        let given = |value: Option<Value>| {
            value.map_or_else(|| json!({}), |value| json!({ "value": value }))
        };
        json!([
            { "value": self.expression },
            given(self.is_function.map(Value::Bool)),
            given(self.arg.clone()),
        ])
    }
}

/// What one run of the script produced, before any masking. Its only way out of the op is
/// [`EvaluateGuard::finish`]: it has no `Serialize` and no `Debug`.
pub(crate) enum Unmasked {
    /// The script's completion value.
    Returned(Completion),
    /// What it threw, as upstream prints it, without the page's stack frames.
    Threw(String),
}

/// A completion value as `returnByValue` reports it.
pub(crate) enum Completion {
    Undefined,
    /// JSON. A DOM node or a `Date` comes back as `{}`.
    Json(Value),
    /// `NaN`, `Infinity`, `-Infinity`, `-0` or a `BigInt` (`10n`), which JSON cannot hold.
    Unserializable(String),
}

impl Unmasked {
    /// Whether `waitForFunction` stops at this run: a truthy value ends it, and so does a throw.
    pub(crate) fn ends_wait(&self) -> bool {
        match self {
            Self::Threw(_) => true,
            Self::Returned(Completion::Undefined) => false,
            Self::Returned(Completion::Json(value)) => truthy(value),
            Self::Returned(Completion::Unserializable(text)) => {
                !matches!(text.as_str(), "NaN" | "-0" | "0n")
            }
        }
    }
}

/// JavaScript's truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number
            .as_f64()
            .is_some_and(|number| number.classify() != FpCategory::Zero),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// How one run of the script ended.
pub(crate) enum Ran {
    /// It returned or threw.
    Finished(Unmasked),
    /// The document was gone before the script started, so nothing ran.
    NotRun,
    /// The document was replaced while the script ran, by a navigation it started perhaps.
    Destroyed,
}

/// One op's main-world scripts on one page. Built by the first sweep; runs the script as often
/// as the op needs (once for `evaluate`, once per poll for `waitForFunction`); lets a result out
/// only through [`EvaluateGuard::finish`], after the second sweep.
///
/// Dropping it, on every exit and on cancellation, reports the values both sweeps found through
/// `PlaywrightHooks::values_swept` once a script was sent, so the host registers them as
/// secrets even when no result left the op.
pub(crate) struct EvaluateGuard<'op> {
    pw: &'op Playwright,
    page: OpPage<'op>,
    /// Every value a sweep found so far.
    swept: FieldValues,
    /// The main-world document by handle, in the op's object group. Dropped when a run finds it
    /// gone, so the next run resolves the new document's.
    document: Option<String>,
    /// A script was sent.
    sent: bool,
}

impl<'op> EvaluateGuard<'op> {
    /// Admits `page` and sweeps it before any script runs.
    pub(crate) async fn sweep_before(
        pw: &'op Playwright,
        scope: &'op OpScope<'_>,
        page: &PageId,
    ) -> Result<Self, PlaywrightError> {
        let page = OpPage::admit(pw, scope, page).await?;
        let mut guard = Self {
            pw,
            page,
            swept: FieldValues::default(),
            document: None,
            sent: false,
        };
        guard.sweep(scope).await?;
        Ok(guard)
    }

    /// Runs `script` once in the main frame's main world. It names the main-world document
    /// first (three CDP calls on a new document), and only then re-runs the host's policies and
    /// marks the op sent, with no await before the script goes out (STYLE P6). Minted before the
    /// document, a takeover during those calls would go unseen, and a cancel there would leave
    /// the op marked sent although no script ran.
    pub(crate) async fn run(
        &mut self,
        scope: &OpScope<'_>,
        script: &Script,
    ) -> Result<Ran, PlaywrightError> {
        let main = self.page.main_endpoint();
        let document = if let Some(document) = &self.document {
            document.clone()
        } else {
            let named = with_world(&self.page, &main, &Place::Main, |world| {
                main_world_document(scope, world)
            })
            .await
            .map_err(|failure| failed(scope, failure))?;
            self.document = Some(named.clone());
            named
        };
        let group = scope.object_group(&main.session).to_owned();
        let sent = scope.send_effect()?;
        self.sent = true;
        let called = call_main_world(&main.session, &document, &group, script, &sent).await;
        let ran = ran(called).map_err(|error| scope.core_error(&error))?;
        if !matches!(ran, Ran::Finished(_)) {
            self.document = None;
        }
        Ok(ran)
    }

    /// Sweeps again, then lets `unmasked` out with every swept value masked in it: a returned
    /// value as the op's result, a thrown message as its `page_error`.
    pub(crate) async fn finish(
        mut self,
        scope: &OpScope<'_>,
        unmasked: Unmasked,
    ) -> Result<Value, PlaywrightError> {
        self.sweep(scope).await?;
        match unmasked {
            Unmasked::Returned(completion) => Ok(mask_completion(&self.swept, completion)),
            Unmasked::Threw(message) => {
                Err(scope.fail(ErrorCode::PageError, &mask_text(&self.swept, message)))
            }
        }
    }

    /// Sweeps the main frame and every same-process frame, adding what it finds.
    async fn sweep(&mut self, scope: &OpScope<'_>) -> Result<(), PlaywrightError> {
        let main = self.page.main_endpoint();
        let mut found = with_world(&self.page, &main, &Place::Main, sweep_world)
            .await
            .map_err(|failure| failed(scope, failure))?;
        let frames = same_process_frames(&main)
            .await
            .map_err(|error| scope.core_error(&error))?;
        for frame in frames {
            let endpoint = Endpoint {
                session: main.session.clone(),
                frame,
                kind: SessionKind::Page,
            };
            match with_world(&self.page, &endpoint, &Place::Descendant, sweep_world).await {
                Ok(values) => found = found.union(values),
                // The frame went away after the tree was read, so no script can reach it now.
                Err(ProbeFailure::Gone) => {}
                // Fail closed: an unswept frame could hand the script a value no mask knows.
                Err(failure) => return Err(failed(scope, failure)),
            }
        }
        self.swept = mem::take(&mut self.swept).union(found);
        Ok(())
    }
}

impl Drop for EvaluateGuard<'_> {
    fn drop(&mut self) {
        if self.sent && !self.swept.is_empty() {
            self.pw
                .hooks()
                .values_swept(self.page.id().clone(), &self.swept);
        }
    }
}

/// One world's sweep.
async fn sweep_world(world: SealedWorld) -> Result<FieldValues, ProbeFailure> {
    match probe::call(&world, &ProbeRequest::Sweep).await? {
        ProbeReply::Sweep { values } => Ok(values),
        other => Err(unexpected("sweep", other)),
    }
}

/// The frames of the page's renderer besides the main frame: what `Page.getFrameTree` lists on
/// the page's own session, which leaves out every out-of-process frame.
async fn same_process_frames(main: &Endpoint) -> Result<Vec<FrameId>, CoreError> {
    let tree: FrameTreeReply = decode(
        main.session
            .send_value("Page.getFrameTree", json!({}))
            .await?,
    )?;
    Ok(tree
        .frame_tree
        .into_frames()
        .filter(|frame| frame.id != main.frame.as_str())
        .map(|frame| FrameId(frame.id))
        .collect())
}

/// A remote object by handle.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Handle {
    #[serde(default)]
    object_id: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    result: Handle,
}

#[derive(Deserialize)]
struct Described {
    node: DescribedNode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescribedNode {
    backend_node_id: i64,
}

#[derive(Deserialize)]
struct Resolved {
    object: Handle,
}

/// The main-world document of `world`'s frame, by handle in the op's object group.
async fn main_world_document(
    scope: &OpScope<'_>,
    world: SealedWorld,
) -> Result<String, ProbeFailure> {
    let session = world.session();
    let group = scope.object_group(session).to_owned();
    let named: Named = session
        .send(
            "Runtime.callFunctionOn",
            json!({ "objectId": world.global(), "functionDeclaration": SEALED_DOCUMENT,
                    "objectGroup": group }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    let sealed = named
        .result
        .object_id
        .ok_or_else(|| ProbeFailure::Broken("the sealed world returned no document".to_owned()))?;
    let described: Described = session
        .send("DOM.describeNode", json!({ "objectId": sealed }))
        .await
        .map_err(ProbeFailure::from_core)?;
    // No `executionContextId`: the node resolves in its own document's main world.
    let resolved: Resolved = session
        .send(
            "DOM.resolveNode",
            json!({ "backendNodeId": described.node.backend_node_id, "objectGroup": group }),
        )
        .await
        .map_err(ProbeFailure::from_core)?;
    resolved
        .object
        .object_id
        .ok_or_else(|| ProbeFailure::Broken("the main world returned no document".to_owned()))
}

/// Sends `script` to the main world. Never logs its params or result: either may hold a
/// password (STYLE P4).
async fn call_main_world(
    session: &ProtocolSession,
    document: &str,
    group: &str,
    script: &Script,
    _sent: &Sent<'_>,
) -> Result<Value, CoreError> {
    session
        .send_value(
            "Runtime.callFunctionOn",
            json!({
                "objectId": document,
                "functionDeclaration": MAIN_WORLD_FUNCTION,
                "arguments": script.arguments(),
                "returnByValue": true,
                "awaitPromise": true,
                "objectGroup": group,
            }),
        )
        .await
}

/// `Runtime.callFunctionOn`'s result.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Called {
    result: RemoteObject,
    #[serde(default)]
    exception_details: Option<ExceptionDetails>,
}

/// A by-value result. `null` arrives as a `value` of `null`, `undefined` as no value at all.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteObject {
    r#type: String,
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
    #[serde(default)]
    unserializable_value: Option<String>,
}

#[derive(Deserialize)]
struct ExceptionDetails {
    text: String,
    #[serde(default)]
    exception: Option<Thrown>,
}

/// What the page threw, as a remote object.
#[derive(Deserialize)]
struct Thrown {
    #[serde(default)]
    description: Option<String>,
    #[serde(default, deserialize_with = "present")]
    value: Option<Value>,
}

impl RemoteObject {
    fn completion(self) -> Completion {
        if let Some(text) = self.unserializable_value {
            Completion::Unserializable(text)
        } else if self.r#type == "undefined" {
            Completion::Undefined
        } else {
            Completion::Json(self.value.unwrap_or(Value::Null))
        }
    }
}

impl ExceptionDetails {
    /// Upstream's `getExceptionMessage` without the page's stack frames: an error's
    /// description, a thrown primitive as text, else CDP's summary.
    fn message(self) -> String {
        let text = match self.exception {
            Some(Thrown {
                description: Some(description),
                ..
            }) => description,
            Some(Thrown {
                value: Some(Value::String(text)),
                ..
            }) => text,
            Some(Thrown {
                value: Some(value), ..
            }) => value.to_string(),
            _ => self.text,
        };
        text.lines()
            .filter(|line| !line.starts_with("    at "))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// How the script's call ended, from CDP's reply.
fn ran(called: Result<Value, CoreError>) -> Result<Ran, CoreError> {
    let reply = match called {
        Ok(reply) => reply,
        Err(error) => {
            let outcome = match &error {
                CoreError::Cdp(CdpError::Protocol { message, .. }) => protocol_outcome(message),
                _ => None,
            };
            return outcome.ok_or(error);
        }
    };
    let called: Called = decode(reply)?;
    Ok(Ran::Finished(match called.exception_details {
        Some(exception) => Unmasked::Threw(exception.message()),
        None => Unmasked::Returned(called.result.completion()),
    }))
}

/// The CDP errors that are an outcome of the script rather than a failure, as upstream's
/// `rewriteError` reads them.
fn protocol_outcome(message: &str) -> Option<Ran> {
    const NOT_FOUND: [&str; 3] = [
        "Cannot find context with specified id",
        "Could not find object with given id",
        "does not belong to the document",
    ];
    if message.contains("Execution context was destroyed") {
        Some(Ran::Destroyed)
    } else if NOT_FOUND.iter().any(|gone| message.contains(gone)) {
        Some(Ran::NotRun)
    } else if message.contains("Object reference chain is too long")
        || message.contains("CBOR: stack limit exceeded")
    {
        Some(Ran::Finished(Unmasked::Threw(
            "Cannot serialize result: object reference chain is too long.".to_owned(),
        )))
    } else if message.contains("Object couldn't be returned by value") {
        Some(Ran::Finished(Unmasked::Returned(Completion::Undefined)))
    } else {
        None
    }
}

/// `completion` as the op's JSON result, with every swept value masked in it. JSON has no
/// `undefined`, `NaN`, `Infinity` or `BigInt`; each becomes `null` (and `-0` becomes `0`), as
/// `JSON.stringify` writes them in an array: a listed deviation from upstream's serializer.
fn mask_completion(swept: &FieldValues, completion: Completion) -> Value {
    match completion {
        Completion::Json(mut value) => {
            swept.mask_json(&mut value);
            mask_numbers(swept, &mut value);
            value
        }
        Completion::Unserializable(text) if text == "-0" => json!(0),
        Completion::Undefined | Completion::Unserializable(_) => Value::Null,
    }
}

/// `mask_json` masks strings only, but a script can return a code as a number
/// (`Number(otp.value)`). A number whose digits hold a swept value becomes the masked string.
fn mask_numbers(swept: &FieldValues, value: &mut Value) {
    match value {
        Value::Number(number) => {
            let mut digits = Value::String(number.to_string());
            if swept.mask_json(&mut digits) {
                *value = digits;
            }
        }
        Value::Array(items) => {
            for item in items {
                mask_numbers(swept, item);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                mask_numbers(swept, field);
            }
        }
        Value::Null | Value::Bool(_) | Value::String(_) => {}
    }
}

/// Page text with every swept value masked, as `mask_json` masks a string: a long value
/// wherever it occurs (with its newlines, unlike `mask_str`), a short one only as the whole text.
pub(crate) fn mask_text(swept: &FieldValues, text: String) -> String {
    let mut leaf = Value::String(text);
    swept.mask_json(&mut leaf);
    match leaf {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// Decodes a field that is present, `null` included, as `Some`: absent stays `None` through
/// `#[serde(default)]`. A plain `Option<Value>` would read an explicit `null` as absent.
pub(crate) fn present<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use browseros_cdp::CdpError;
    use browseros_core::CoreError;
    use browseros_core::sensitive_fields::FieldValues;
    use serde_json::{Value, json};

    use super::{Completion, Ran, Unmasked, mask_completion, mask_text, ran};

    type TestResult = Result<(), Box<dyn Error>>;

    fn swept() -> FieldValues {
        FieldValues::from_sweep(&json!(["hunter2-secret", "482913", "123"]))
    }

    fn finished(reply: Value) -> Result<Unmasked, Box<dyn Error>> {
        match ran(Ok(reply))? {
            Ran::Finished(unmasked) => Ok(unmasked),
            Ran::NotRun | Ran::Destroyed => Err("a reply is a finished run".into()),
        }
    }

    fn result_of(unmasked: Unmasked) -> Result<Value, Box<dyn Error>> {
        let Unmasked::Returned(completion) = unmasked else {
            return Err("the script returned".into());
        };
        Ok(mask_completion(&swept(), completion))
    }

    #[test]
    fn tests_that_a_result_is_json_and_what_json_cannot_hold_is_null() -> TestResult {
        let null =
            finished(json!({ "result": { "type": "object", "subtype": "null", "value": null } }))?;
        assert_eq!(result_of(null)?, json!(null));
        let undefined = finished(json!({ "result": { "type": "undefined" } }))?;
        assert_eq!(result_of(undefined)?, json!(null), "JSON has no undefined");
        let nan =
            finished(json!({ "result": { "type": "number", "unserializableValue": "NaN" } }))?;
        assert_eq!(result_of(nan)?, json!(null));
        let zero =
            finished(json!({ "result": { "type": "number", "unserializableValue": "-0" } }))?;
        assert_eq!(result_of(zero)?, json!(0));
        Ok(())
    }

    #[test]
    fn tests_that_a_swept_value_is_masked_in_strings_keys_and_numbers() -> TestResult {
        let echoed = finished(json!({ "result": { "type": "object", "value": {
            "typed": "you typed hunter2-secret",
            "hunter2-secret": true,
            "code": 482_913,
            "cvc": "123",
            "price": 1234,
            "plain": "untouched",
        } } }))?;
        assert_eq!(
            result_of(echoed)?,
            json!({
                "typed": "you typed [private]",
                "[private]": true,
                "code": "[private]",
                "cvc": "[private]",
                "price": 1234,
                "plain": "untouched",
            }),
            "a code returned as a number is as secret as the string; other numbers stay"
        );
        let bigint =
            finished(json!({ "result": { "type": "bigint", "unserializableValue": "482913n" } }))?;
        assert_eq!(
            result_of(bigint)?,
            json!(null),
            "a BigInt, a code perhaps, never leaves"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_thrown_message_is_upstreams_without_the_stack_and_masked() -> TestResult {
        let error = finished(json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": { "text": "Uncaught", "exception": {
                "description": "TypeError: hunter2-secret is not a function\n    at eval (eval at <anonymous> (:17:24), <anonymous>:1:7)\n    at <anonymous>:18:73" } },
        }))?;
        let Unmasked::Threw(message) = error else {
            return Err("the script threw".into());
        };
        assert_eq!(
            mask_text(&swept(), message),
            "TypeError: [private] is not a function"
        );
        let primitive = finished(json!({
            "result": { "type": "string" },
            "exceptionDetails": { "text": "Uncaught", "exception": { "type": "string", "value": "plain" } },
        }))?;
        assert!(
            matches!(&primitive, Unmasked::Threw(message) if message == "plain"),
            "a thrown string prints as itself"
        );
        Ok(())
    }

    #[test]
    fn tests_that_a_gone_document_is_an_outcome_and_other_errors_fail() -> TestResult {
        let protocol = |message: &str| {
            Err(CoreError::Cdp(CdpError::Protocol {
                code: -32000,
                message: message.to_owned(),
            }))
        };
        assert!(matches!(
            ran(protocol("Cannot find context with specified id"))?,
            Ran::NotRun
        ));
        assert!(matches!(
            ran(protocol("Execution context was destroyed."))?,
            Ran::Destroyed
        ));
        assert!(matches!(
            ran(protocol("Object couldn't be returned by value"))?,
            Ran::Finished(Unmasked::Returned(Completion::Undefined))
        ));
        assert!(matches!(
            ran(protocol("Object reference chain is too long"))?,
            Ran::Finished(Unmasked::Threw(message)) if message.starts_with("Cannot serialize result")
        ));
        assert!(
            ran(protocol("Internal error")).is_err(),
            "an unknown CDP error is the op's failure"
        );
        Ok(())
    }

    #[test]
    fn tests_that_waiting_ends_on_a_truthy_value_or_a_throw() {
        let returned = |value: Value| Unmasked::Returned(Completion::Json(value)).ends_wait();
        assert!(!returned(json!(null)) && !returned(json!(0)) && !returned(json!("")));
        assert!(!returned(json!(false)));
        assert!(returned(json!({})) && returned(json!([])) && returned(json!("0")));
        assert!(returned(json!(0.5)));
        assert!(!Unmasked::Returned(Completion::Undefined).ends_wait());
        assert!(!Unmasked::Returned(Completion::Unserializable("NaN".to_owned())).ends_wait());
        assert!(Unmasked::Returned(Completion::Unserializable("Infinity".to_owned())).ends_wait());
        assert!(
            Unmasked::Threw("Error: x".to_owned()).ends_wait(),
            "a throw fails the wait"
        );
    }
}
