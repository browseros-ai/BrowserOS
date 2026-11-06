use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, CoreError, SessionId,
};
use browseros_trace::ErrorOwner;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{self, Instant};

use super::{
    ArgText, Called, EvaluateGuard, FRAME_BUDGET, MainWorldCall, Outcome, RemoteObject, Reply,
    Script, Subject, Unmasked, decode, masked, not_run, returned,
};
use crate::op_scope::OpScope;
use crate::page::page_value::MaskedValue;
use crate::probe::PROBE_PROTOCOL;
use crate::target::{CallHeader, PageRef};
use crate::test_env::{self, TestEnv};
use crate::test_support::RecordingHooks;
use crate::{ErrorCode, Op, Playwright};

type TestResult = Result<(), Box<dyn Error>>;

/// A script the prelude built, its argument `undefined`.
fn script(expression: &str) -> Script {
    Script::Run {
        expression: expression.to_owned(),
        is_function: true,
        arg: ArgText(r#"{"v":"undefined"}"#.to_owned()),
        handles: Vec::new(),
    }
}

/// The page brand a script would send for page `id` on tab `tab`.
fn page_ref(id: u64, tab: u64) -> Result<PageRef, serde_json::Error> {
    serde_json::from_value(json!({ "$neo": "page", "id": id, "tab": tab }))
}

fn reply(value: Value) -> Result<Reply, CoreError> {
    decode::<Reply>(value)
}

fn object(value: Value) -> Result<RemoteObject, CoreError> {
    decode::<RemoteObject>(value)
}

#[test]
fn tests_that_an_evaluate_answer_is_the_pages_text_or_undefined() -> TestResult {
    let text = reply(json!({ "result": { "type": "string", "value": "{\"v\":\"NaN\"}" } }))?;
    assert!(matches!(text.result.text(), Unmasked::Text(text) if text == r#"{"v":"NaN"}"#));
    let none = reply(json!({ "result": { "type": "undefined" } }))?;
    assert!(matches!(none.result.text(), Unmasked::Undefined));
    let number = reply(json!({ "result": { "type": "number", "value": 1 } }))?;
    assert!(
        matches!(number.result.text(), Unmasked::Unreadable),
        "the main-world function answers text or nothing; anything else is a tampered page"
    );
    Ok(())
}

/// upstream's `getExceptionMessage`: the description, stack lines and all, or the thrown
/// primitive as `String()` writes it.
#[test]
fn tests_that_a_throw_reads_as_upstreams_exception_message() -> TestResult {
    let error = reply(json!({
        "result": { "type": "object", "subtype": "error" },
        "exceptionDetails": { "text": "Uncaught", "exception": { "type": "object",
            "description": "TypeError: boom\n    at eval (eval at evaluate (:1:1))" } },
    }))?;
    assert_eq!(
        error.thrown().map(|thrown| thrown.0),
        Some("TypeError: boom\n    at eval (eval at evaluate (:1:1))".to_owned())
    );
    let string = reply(json!({
        "result": { "type": "string" },
        "exceptionDetails": { "text": "Uncaught", "exception": { "type": "string", "value": "plain" } },
    }))?;
    assert_eq!(
        string.thrown().map(|thrown| thrown.0),
        Some("plain".to_owned())
    );
    let null = reply(json!({
        "result": { "type": "object" },
        "exceptionDetails": { "text": "Uncaught", "exception": { "type": "object", "subtype": "null", "value": null } },
    }))?;
    assert_eq!(
        null.thrown().map(|thrown| thrown.0),
        Some("null".to_owned())
    );
    let answered = reply(json!({ "result": { "type": "string", "value": "1" } }))?;
    assert!(answered.thrown().is_none());
    Ok(())
}

#[test]
fn tests_that_a_gone_document_is_an_outcome_and_other_errors_fail() {
    let protocol = |message: &str| {
        CoreError::Cdp(CdpError::Protocol {
            code: -32000,
            message: message.to_owned(),
        })
    };
    let ended = |message: &str| match not_run::<()>(protocol(message)) {
        Ok(Called::Ended(Outcome::NotRun)) => "not run",
        Ok(Called::Ended(Outcome::Destroyed)) => "destroyed",
        Ok(_) => "another outcome",
        Err(_) => "failed",
    };
    assert_eq!(ended("Cannot find context with specified id"), "not run");
    assert_eq!(ended("Could not find object with given id"), "not run");
    assert_eq!(ended("Execution context was destroyed."), "destroyed");
    assert_eq!(
        ended("Internal error"),
        "failed",
        "an unknown CDP error is the op's failure"
    );
}

/// upstream's `JSHandle.toString()` for a settled value: `String(value)` for a primitive, where a
/// `BigInt` is lost to `parseUnserializableValue`, and CDP's description of an object.
#[test]
fn tests_that_a_settled_value_previews_as_upstreams_handle() -> TestResult {
    let preview = |value: Value| object(value).map(RemoteObject::preview);
    assert_eq!(
        preview(json!({ "type": "string", "value": "ready" }))?,
        "ready"
    );
    assert_eq!(
        preview(json!({ "type": "boolean", "value": true }))?,
        "true"
    );
    assert_eq!(preview(json!({ "type": "number", "value": 42 }))?, "42");
    assert_eq!(
        preview(json!({ "type": "number", "unserializableValue": "Infinity" }))?,
        "Infinity"
    );
    let bigint =
        object(json!({ "type": "bigint", "unserializableValue": "10n", "description": "10n" }))?;
    assert!(matches!(
        bigint.json_value(Unmasked::Text(r#"{"bi":"10"}"#.to_owned())),
        Unmasked::Undefined
    ));
    assert_eq!(bigint.preview(), "undefined");
    assert_eq!(
        preview(
            json!({ "type": "object", "className": "Object", "description": "Object",
                        "objectId": "1" })
        )?,
        "Object"
    );
    assert_eq!(
        preview(
            json!({ "type": "object", "subtype": "array", "description": "Array(2)",
                        "objectId": "2" })
        )?,
        "Array(2)"
    );
    assert_eq!(
        preview(json!({ "type": "object", "subtype": "regexp", "objectId": "3" }))?,
        "JSHandle@regexp"
    );
    let array = object(json!({ "type": "object", "subtype": "array", "objectId": "4" }))?;
    assert!(
        matches!(
            array.json_value(Unmasked::Text("[]".to_owned())),
            Unmasked::Text(_)
        ),
        "an object's value is the page's own serialized text"
    );
    Ok(())
}

/// A value leaves in Playwright's format, masked; one upstream could not return, or one no
/// serializer writes, fails as the page's, with nothing of it in the message.
#[tokio::test]
async fn tests_that_an_answer_leaves_masked_or_fails_as_the_pages() -> TestResult {
    let env = TestEnv::new();
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let swept = FieldValues::from_sweep(&json!(["hunter2-secret"]));
    let text = r#"{"o":[{"k":"hunter2-secret","v":"you typed hunter2-secret"}],"id":1}"#;
    assert_eq!(
        masked(&scope, &swept, Unmasked::Text(text.to_owned()))?.into_json(),
        json!({ "o": [{ "k": "[private]", "v": "you typed [private]" }], "id": 1 })
    );
    assert_eq!(
        masked(&scope, &swept, Unmasked::Undefined)?.into_json(),
        json!({ "v": "undefined" })
    );
    let deep = format!("{}0{}", "[".repeat(298), "]".repeat(298));
    for (unmasked, message) in [
        (
            Unmasked::Text(deep),
            "page.evaluate: Cannot serialize result: object reference chain is too long.",
        ),
        (
            Unmasked::Text(r#"{"h":0,"secret":"hunter2-secret"}"#.to_owned()),
            "page.evaluate: the page answered with a value outside Playwright's serialized \
             format, as a page that replaced the globals its serializer uses would, so Neo \
             withheld it",
        ),
        (
            Unmasked::Unreadable,
            "page.evaluate: the page answered with a value outside Playwright's serialized \
             format, as a page that replaced the globals its serializer uses would, so Neo \
             withheld it",
        ),
    ] {
        let failed = masked(&scope, &swept, unmasked)
            .map(MaskedValue::into_json)
            .err()
            .ok_or("an unreadable answer must not leave")?;
        assert_eq!(
            (failed.error_code(), failed.message()),
            (Some(ErrorCode::PageError), message)
        );
    }
    Ok(())
}

/// A function named `__pw_fn_…` comes back as `{fn}`, which upstream's server cannot serialize
/// again: the op fails with its message, the first such function's place in it.
#[tokio::test]
async fn tests_that_a_binding_function_fails_as_upstreams_server_fails_it() -> TestResult {
    let env = TestEnv::new();
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let none = FieldValues::default();
    let caller = "(...args) => globalThis[kBindingsControllerProperty].callBinding(name, ...args)";
    for (text, at) in [
        (r#"{"fn":"__pw_fn_demo"}"#, String::new()),
        (
            r#"{"o":[{"k":"f","v":{"fn":"__pw_fn_demo"}}],"id":1}"#,
            r#" at position "f""#.to_owned(),
        ),
        (
            r#"{"a":[1,{"o":[{"k":"x","v":{"a":[{"fn":"__pw_fn_a"},{"fn":"__pw_fn_b"}],"id":3}}],"id":2}],"id":1}"#,
            r#" at position "[1].x[0]""#.to_owned(),
        ),
    ] {
        let failed = returned(
            &scope,
            masked(&scope, &none, Unmasked::Text(text.to_owned()))?,
        )
        .err()
        .ok_or("a binding function must not leave")?;
        assert_eq!(
            (failed.error_code(), failed.message()),
            (
                Some(ErrorCode::PageError),
                format!("page.evaluate: Attempting to serialize unexpected value{at}: {caller}")
                    .as_str()
            ),
            "{text}"
        );
    }
    Ok(())
}

/// `Pending`, the state after a send, can only be swept: nothing on it lets an answer out. The
/// compiler cannot be asked from a doc test, which sees no `pub(crate)` item and so would pass
/// whatever `Pending` had; this reads the methods its one `impl` block defines.
#[test]
fn tests_that_a_pending_guard_can_only_be_swept() {
    let source = include_str!("../evaluate_guard.rs");
    let methods = |owner: &str| -> Vec<String> {
        let opening = format!("impl<'op> {owner}<'op> {{");
        let block = source
            .split(opening.as_str())
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .unwrap_or_default();
        block
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("pub(crate) async fn ")
                    .or_else(|| line.trim().strip_prefix("pub(crate) fn "))
            })
            .filter_map(|signature| signature.split(['(', '<']).next())
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        source.matches("impl<'op> Pending<'op> {").count(),
        1,
        "one impl block holds Pending's methods"
    );
    assert_eq!(
        methods("Pending"),
        ["sweep"],
        "a pending guard can only be swept"
    );
    assert_eq!(
        methods("EvaluateGuard"),
        ["open", "run", "finish"],
        "only a swept guard sends or lets an answer out"
    );
}

/// What a send produced leaves `run` only inside the `Pending`, and the page's text in it is
/// opaque to the rest of the crate: the types that hold it have no crate-visible variant or
/// field, so `finish` is the one function that reads it. A leaf can branch on the outcome, but
/// never on the text.
#[test]
fn tests_that_page_text_is_opaque_outside_the_guard() {
    let source = include_str!("../evaluate_guard.rs");
    for declaration in [
        "pub(crate) struct Answer(Answered);",
        "\nenum Answered {",
        "\nenum Unmasked {",
        "\nstruct UnmaskedText(String);",
    ] {
        assert!(
            source.contains(declaration),
            "page text is declared opaque: {declaration}"
        );
    }
    assert!(
        source.contains("-> Result<Pending<'op>, PlaywrightError> {"),
        "a send hands back only its Pending"
    );
    assert!(
        source.contains("-> Result<(EvaluateGuard<'op>, Outcome), PlaywrightError> {"),
        "the outcome leaves with the guard its sweep made"
    );
}

const PAGE_TARGET: &str = "page-target";
const PAGE_SESSION: &str = "page-session";
const OOPIF: &str = "oopif-target";
const OOPIF_SESSION: &str = "oopif-session";

/// How long each half of reaching and sweeping the slow frame takes: its attach plus its
/// frame tree, then its world's install plus the sweep.
const HALF: Duration = Duration::from_millis(600);

/// A browser with one tab on `https://a.test/` and an out-of-process frame of the same
/// site whose renderer is slow: its attach, frame tree, world install and sweep take
/// [`HALF`] a pair. Every other command answers at once.
struct SlowFrame {
    events: broadcast::Sender<CdpEvent>,
    targeted: TargetedEvents,
}

impl SlowFrame {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: broadcast::channel(16).0,
            targeted: TargetedEvents::default(),
        })
    }

    async fn answer(&self, method: &str, params: &Value, session: Option<&SessionId>) -> Value {
        let oopif = session.is_some_and(|session| session.as_str() == OOPIF_SESSION);
        match method {
            "Browser.getTabs" => json!({ "tabs": [{ "tabId": 7, "targetId": PAGE_TARGET,
                "windowId": 1, "url": "https://a.test/", "title": "A", "isActive": true,
                "isLoading": false, "loadProgress": 1.0, "isPinned": false,
                "isHidden": false }] }),
            "Target.attachToTarget" => {
                let target = params["targetId"].as_str().unwrap_or_default();
                stage(target == OOPIF).await;
                let session = if target == OOPIF {
                    OOPIF_SESSION
                } else {
                    PAGE_SESSION
                };
                json!({ "sessionId": session })
            }
            "Target.getTargets" => json!({ "targetInfos": [
                { "targetId": PAGE_TARGET, "type": "page", "url": "https://a.test/" },
                { "targetId": OOPIF, "type": "iframe", "url": "https://pay.a.test/",
                  "parentId": PAGE_TARGET },
            ] }),
            "Page.getFrameTree" => {
                stage(oopif).await;
                let (id, url) = if oopif {
                    (OOPIF, "https://pay.a.test/")
                } else {
                    (PAGE_TARGET, "https://a.test/")
                };
                json!({ "frameTree": { "frame": { "id": id, "loaderId": "L", "url": url,
                    "domainAndRegistry": "a.test" } } })
            }
            "Page.createIsolatedWorld" => {
                stage(oopif).await;
                json!({ "executionContextId": 7 })
            }
            "Runtime.evaluate" => json!({ "result": { "type": "object", "objectId": "g" } }),
            // The hidden-root scan finds no shadow root.
            "DOM.describeNode" => json!({ "node": { "backendNodeId": 1,
                "nodeName": "#document", "childNodeCount": 0, "children": [] } }),
            "Runtime.callFunctionOn" => {
                let request = &params["arguments"][0]["value"];
                if request["op"] == "sweep" {
                    stage(oopif).await;
                    json!({ "result": { "type": "object",
                        "value": { "kind": "sweep", "values": [] } } })
                } else if params.get("arguments").is_none() {
                    if params["functionDeclaration"]
                        .as_str()
                        .is_some_and(|function| function.contains("takeHost"))
                    {
                        // The seal reported no shadow host.
                        json!({ "result": { "type": "object", "subtype": "null",
                            "value": null } })
                    } else if params["functionDeclaration"]
                        .as_str()
                        .is_some_and(|function| function.contains("detachedTops"))
                    {
                        // It watches no tree out of the document.
                        json!({ "result": { "type": "object", "subtype": "array",
                            "objectId": "tops", "description": "Array(0)" } })
                    } else {
                        // The world's document, where the scan starts.
                        json!({ "result": { "type": "object", "objectId": "d" } })
                    }
                } else {
                    json!({ "result": { "type": "object", "value": { "kind": "described",
                        "protocol": PROBE_PROTOCOL, "seq": request["frameSeq"], "url": "" } } })
                }
            }
            _ => json!({}),
        }
    }
}

/// One stage of the slow frame, half of a [`HALF`]; nothing for any other frame.
async fn stage(slow: bool) {
    if slow {
        time::sleep(HALF / 2).await;
    }
}

impl CdpConnection for SlowFrame {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { Ok(self.answer(method, &params, session).await) })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params_json: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params_json)?;
            let reply = self.answer(method, &params, session).await;
            Ok(serde_json::to_string(&reply)?)
        })
    }

    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    fn events_targeted(&self, method: &str) -> mpsc::UnboundedReceiver<CdpEvent> {
        self.targeted.subscribe(method)
    }

    fn is_connected(&self) -> bool {
        true
    }

    fn connection_epoch(&self) -> u64 {
        1
    }
}

/// Reaching the slow frame and sweeping it each fit in a budget of their own, 600 ms against
/// 1 s, but the frame has one budget for both: it refuses the script at 1 s rather than
/// passing at 1.2 s.
#[tokio::test(start_paused = true)]
async fn tests_that_a_frame_has_one_budget_from_its_attach_to_its_sweep() -> TestResult {
    let browser = BrowserSession::new(
        SlowFrame::new() as Arc<dyn CdpConnection>,
        BrowserSessionHooks::default(),
    );
    let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
    let listed = browser.pages.list().await?;
    let listed = listed.first().ok_or("the tab is listed")?;
    let page: PageRef = serde_json::from_value(
        json!({ "$neo": "page", "id": listed.page_id, "tab": listed.tab_id }),
    )?;
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let started = Instant::now();
    let call = MainWorldCall::once(script("() => 1"), Subject::Page);
    let swept = EvaluateGuard::open(&playwright, &scope, &page, call).await;
    let took = started.elapsed();
    let error = swept.err().ok_or(format!(
        "a frame 1.2 s from its attach to its sweep passed, after {took:?}"
    ))?;
    assert_eq!(
        (error.error_code(), error.owner()),
        (Some(ErrorCode::Unsupported), ErrorOwner::Neo),
        "{error}"
    );
    assert!(
        took >= FRAME_BUDGET && took < FRAME_BUDGET + HALF / 2,
        "refused when the frame's one budget ran out, after {took:?}"
    );
    Ok(())
}

/// A host serves an op unheld unless its spec declares `MainWorld`, so the guard refuses any
/// other op, here one that declares only `Capture`, before it admits or sweeps anything. A
/// script the prelude refused is raised next, with upstream's message and nothing admitted,
/// while a script to run goes on to admit its page, which fails here for want of a browser.
/// Paused: core waits out its own attach timeout before it gives up on the page.
#[tokio::test(start_paused = true)]
async fn tests_that_the_guard_lets_in_only_a_declared_op_and_a_script_to_run() -> TestResult {
    let playwright = Playwright::new(
        test_env::offline_session(),
        Arc::new(RecordingHooks::default()),
    );
    let env = TestEnv::new();
    let screenshot = Op::from_name("page.screenshot").ok_or("page.screenshot is an op")?;
    let undeclared = OpScope::new(&env, screenshot, CallHeader::default());
    let call = MainWorldCall::once(script("() => 1"), Subject::Page);
    let refused = EvaluateGuard::open(&playwright, &undeclared, &page_ref(1, 1)?, call)
        .await
        .err()
        .ok_or("an op that does not declare MainWorld must get no guard")?;
    assert_eq!(
        (refused.owner(), refused.error_code()),
        (ErrorOwner::Neo, Some(ErrorCode::Internal)),
        "{refused}"
    );
    let evaluate = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let declared = OpScope::new(&env, evaluate, CallHeader::default());
    let unserializable = MainWorldCall::once(
        Script::Refused("Passed function is not well-serializable!".to_owned()),
        Subject::Page,
    );
    let raised = EvaluateGuard::open(&playwright, &declared, &page_ref(1, 1)?, unserializable)
        .await
        .err()
        .ok_or("a refused script gets no guard")?;
    assert_eq!(
        (raised.owner(), raised.error_code(), raised.message()),
        (
            ErrorOwner::Script,
            Some(ErrorCode::InvalidInput),
            "page.evaluate: Passed function is not well-serializable!"
        ),
        "upstream's server-side message, before the page is admitted"
    );
    let call = MainWorldCall::once(script("() => 1"), Subject::Page);
    let admitting = EvaluateGuard::open(&playwright, &declared, &page_ref(1, 1)?, call)
        .await
        .err()
        .ok_or("no browser, no guard")?;
    assert_eq!(
        admitting.error_code(),
        Some(ErrorCode::BrowserDisconnected),
        "the declared op goes on to admit its page: {admitting}"
    );
    assert!(
        env.calls().is_empty(),
        "no op rechecked or marked anything sent"
    );
    Ok(())
}
