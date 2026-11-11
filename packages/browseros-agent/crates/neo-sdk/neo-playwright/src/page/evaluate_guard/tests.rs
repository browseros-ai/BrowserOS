use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, CoreError, SessionId, TabId,
};
use futures_util::future::{self, BoxFuture};
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{self, Instant};

use super::{
    ArgText, Called, EvaluateGuard, FIRST_POLL, FRAME_BUDGET, MainWorldCall, NAVIGATED, Outcome,
    RemoteObject, Reply, SETTLED, Script, Subject, Unmasked, decode, masked, not_run, returned,
};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::op_page::UNTIMED_RETRY;
use crate::page::page_value::MaskedValue;
use crate::page::site_sweep::SiteSweep;
use crate::probe::{PROBE_PROTOCOL, ProbeFailure};
use crate::target::{CallHeader, PageRef};
use crate::test_env::{self, TestEnv};
use crate::test_support::{
    ProbeAnswer, RecordingHooks, SCRIPTED_CHILD, SCRIPTED_DOCUMENT, SCRIPTED_OOPIF, ScriptedLink,
    ScriptedRun, ScriptedTab, TabScript,
};
use crate::world::Place;
use crate::{ErrorCode, Op, OpEnv, Playwright, PlaywrightError, Refusal, RunState, resolve};

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
fn page_ref(_id: u64, tab: u64) -> Result<PageRef, serde_json::Error> {
    serde_json::from_value(json!({ "$neo": "page", "tab": tab }))
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
        Ok(Called::Ended(Outcome::Navigated)) => "navigated",
        Ok(_) => "another outcome",
        Err(_) => "failed",
    };
    assert_eq!(ended("Cannot find context with specified id"), "not run");
    assert_eq!(ended("Could not find object with given id"), "not run");
    assert_eq!(ended("Execution context was destroyed."), "destroyed");
    assert_eq!(
        ended("Inspected target navigated or closed"),
        "navigated",
        "a call a navigation cut off may have run"
    );
    assert_eq!(
        ended("Internal error"),
        "failed",
        "an unknown CDP error is the op's failure"
    );
    assert!(
        matches!(
            not_run::<()>(CoreError::Cdp(CdpError::SessionGone)),
            Ok(Called::Ended(Outcome::NotRun))
        ),
        "Chromium refuses a call on a session that is gone before it reaches the page"
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
    let document = include_str!("document_write.rs");
    let methods_in = |source: &str, opening: &str| -> Vec<String> {
        let block = source
            .split(opening)
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
        (
            source.matches("> Pending<'op").count(),
            document.matches("> Pending<'op").count()
        ),
        (1, 0),
        "one impl block holds Pending's methods, whatever it guards"
    );
    assert_eq!(
        methods_in(source, "impl<'op, G: Guarded> Pending<'op, G> {"),
        ["sweep"],
        "a pending guard can only be swept"
    );
    assert_eq!(
        methods_in(source, "impl<'op> EvaluateGuard<'op> {"),
        ["open", "run", "finish"],
        "only a swept guard sends a script or lets its answer out"
    );
    assert_eq!(
        methods_in(document, "impl<'op> EvaluateGuard<'op, DocumentWrite> {"),
        ["open_document", "write", "close"],
        "only a swept guard writes a document or lets the write's end out"
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
        source.contains("-> Result<(EvaluateGuard<'op, G>, G::Outcome), PlaywrightError> {"),
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
/// passing at 1.2 s, the op's 1.5 s leaving no room for a second try.
#[tokio::test(start_paused = true)]
async fn tests_that_a_frame_has_one_budget_from_its_attach_to_its_sweep() -> TestResult {
    let browser = BrowserSession::new(
        SlowFrame::new() as Arc<dyn CdpConnection>,
        BrowserSessionHooks::default(),
    );
    let playwright = Playwright::new(Arc::clone(&browser), Arc::new(RecordingHooks::default()));
    let listed = browser.pages.list().await?;
    let listed = listed.first().ok_or("the tab is listed")?;
    let page: PageRef = serde_json::from_value(json!({ "$neo": "page", "tab": listed.tab_id }))?;
    let env = TestEnv::over(BrowserView::unbound(Arc::clone(&browser), None));
    let op = Op::from_name("page.waitForFunction").ok_or("page.waitForFunction is an op")?;
    let scope = OpScope::new(&env, op, timeout_ms(1_500));
    let started = Instant::now();
    let call = MainWorldCall::poll(script("() => 1"), None);
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
    let call = MainWorldCall::once(script("() => 1"), Subject::Frame(None));
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
        Subject::Frame(None),
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
    let call = MainWorldCall::once(script("() => 1"), Subject::Frame(None));
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

/// A call header with Playwright's timeout of `millis`.
fn timeout_ms(millis: u64) -> CallHeader {
    CallHeader {
        api: None,
        timeout_ms: Some(millis),
    }
}

/// How long the slow child frame's sweep takes: past its 1 s budget.
const LATE: Duration = Duration::from_millis(1_200);

/// A tab whose same-site child frame answers the sweeps `late` picks (numbered from 0) only
/// after [`LATE`], and whose main frame finds a new value on each of its sweeps, `main-try-1`
/// first.
struct LateChild {
    late: fn(usize) -> bool,
    child_sweeps: AtomicUsize,
    main_sweeps: AtomicUsize,
}

impl LateChild {
    fn new(late: fn(usize) -> bool) -> Self {
        Self {
            late,
            child_sweeps: AtomicUsize::new(0),
            main_sweeps: AtomicUsize::new(0),
        }
    }
}

/// Late on the child's first sweep only.
fn first(sweep: usize) -> bool {
    sweep == 0
}

/// Late on every sweep of the child.
fn every(_sweep: usize) -> bool {
    true
}

impl TabScript for LateChild {
    fn probe(&self, frame: &str, _request: &Value) -> ProbeAnswer {
        if frame == SCRIPTED_CHILD {
            let swept = self.child_sweeps.fetch_add(1, Ordering::SeqCst);
            return ProbeAnswer {
                after: if (self.late)(swept) {
                    LATE
                } else {
                    Duration::ZERO
                },
                ..ProbeAnswer::empty_sweep()
            };
        }
        let swept = self.main_sweeps.fetch_add(1, Ordering::SeqCst) + 1;
        ProbeAnswer::reply(json!({ "kind": "sweep", "values": [format!("main-try-{swept}")] }))
    }

    fn child(&self) -> bool {
        true
    }
}

/// What opening a guard beside a [`LateChild`] gave.
struct Opened {
    /// The guard's refusal, if any.
    refused: Option<PlaywrightError>,
    /// How long the open took.
    took: Duration,
    /// Every value the host heard by then.
    heard: Vec<String>,
}

/// Opens `op`'s guard with `header` on a [`LateChild`] tab.
async fn open_beside_late_child(
    tab: &Arc<ScriptedTab<LateChild>>,
    op: &str,
    header: CallHeader,
) -> Result<Opened, Box<dyn Error>> {
    let run = ScriptedRun::over(tab).await?;
    let op = Op::from_name(op).ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, header);
    let call = MainWorldCall::poll(script("() => 1"), None);
    let started = Instant::now();
    let refused = EvaluateGuard::open(&run.playwright, &scope, &run.page, call)
        .await
        .err();
    Ok(Opened {
        refused,
        took: started.elapsed(),
        heard: run.hooks.swept(),
    })
}

/// A same-site frame that misses its budget once is swept again on a fresh budget, inside the
/// op's own timeout, and the script goes ahead once every frame of its site answered; the frame
/// is not given up on the first late answer, as upstream would wait on it.
#[tokio::test(start_paused = true)]
async fn tests_that_a_frame_late_once_is_swept_again_and_the_script_runs() -> TestResult {
    let tab = ScriptedTab::new(LateChild::new(first));
    let Opened {
        refused,
        took,
        heard,
    } = open_beside_late_child(&tab, "page.waitForFunction", timeout_ms(5_000)).await?;
    assert!(
        refused.is_none(),
        "the second sweep found every frame: {refused:?}"
    );
    assert_eq!(
        tab.script().child_sweeps.load(Ordering::SeqCst),
        2,
        "the late frame was swept again"
    );
    assert!(
        took >= FRAME_BUDGET && took < FRAME_BUDGET + Duration::from_millis(100),
        "one late try, then one on time: {took:?}"
    );
    assert!(
        ["main-try-1", "main-try-2"]
            .iter()
            .all(|value| heard.iter().any(|heard| heard == value)),
        "both tries' values reached the host: {heard:?}"
    );
    Ok(())
}

/// A frame late on every try fails the op closed with the message one late sweep gave, once
/// the op's deadline leaves no room for a try as long as the last: before it, so the op's own
/// timeout never hides why. Every try's values reach the host as that try ends, though the
/// script never runs.
#[tokio::test(start_paused = true)]
async fn tests_that_a_frame_late_to_the_deadline_fails_closed_as_before() -> TestResult {
    let tab = ScriptedTab::new(LateChild::new(every));
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.waitForFunction").ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, timeout_ms(2_500));
    let call = MainWorldCall::poll(script("() => 1"), None);
    let started = Instant::now();
    let opening = async {
        let opened = EvaluateGuard::open(&run.playwright, &scope, &run.page, call).await;
        (opened, started.elapsed())
    };
    let after_one_try = async {
        time::sleep(FRAME_BUDGET + Duration::from_millis(100)).await;
        run.hooks.swept()
    };
    let ((opened, took), heard_early) = future::join(opening, after_one_try).await;
    let error = opened.err().ok_or("a frame late on every try refuses")?;
    assert_eq!(
        (error.error_code(), error.owner(), error.message()),
        (
            Some(ErrorCode::Unsupported),
            ErrorOwner::Neo,
            "page.waitForFunction: a frame of the script's site did not answer Neo's privacy \
             sweep within 1000ms, so the script was not run; try again once the frame responds"
        ),
    );
    assert!(
        took >= FRAME_BUDGET * 2 && took <= Duration::from_millis(2_500),
        "two tries fit in the op's 2.5 s, and the op ends inside it: {took:?}"
    );
    assert!(
        heard_early.iter().any(|value| value == "main-try-1"),
        "the first try's values reached the host before the second ended: {heard_early:?}"
    );
    let heard = run.hooks.swept();
    assert!(
        ["main-try-1", "main-try-2"]
            .iter()
            .all(|value| heard.iter().any(|heard| heard == value)),
        "every try's values reached the host: {heard:?}"
    );
    assert!(env_sent_nothing(&run.env), "nothing was sent");
    Ok(())
}

/// A tab whose same-site out-of-process frame answers every sweep only after [`LATE`], and
/// whose main frame answers its sweeps (numbered from 0) after what `main` gives, each finding a
/// new value, `main-try-1` first.
struct LateOopif {
    main: fn(usize) -> Duration,
    main_sweeps: AtomicUsize,
}

impl LateOopif {
    fn new(main: fn(usize) -> Duration) -> Self {
        Self {
            main,
            main_sweeps: AtomicUsize::new(0),
        }
    }
}

impl TabScript for LateOopif {
    fn probe(&self, frame: &str, _request: &Value) -> ProbeAnswer {
        if frame == SCRIPTED_OOPIF {
            return ProbeAnswer {
                after: LATE,
                ..ProbeAnswer::empty_sweep()
            };
        }
        let swept = self.main_sweeps.fetch_add(1, Ordering::SeqCst);
        let values = json!([format!("main-try-{}", swept + 1)]);
        ProbeAnswer {
            after: (self.main)(swept),
            ..ProbeAnswer::reply(json!({ "kind": "sweep", "values": values }))
        }
    }

    fn oopif(&self) -> bool {
        true
    }
}

/// The main frame's second sweep takes 900 ms, inside its budget, so the second try reaches the
/// out-of-process frame only at 1.9 s, on a budget of its own.
fn slow_second(sweep: usize) -> Duration {
    if sweep == 1 {
        Duration::from_millis(900)
    } else {
        Duration::ZERO
    }
}

/// A try can outlast the one before it. Here the first takes 1 s, the out-of-process frame late
/// on its budget, and the second would take 2 s, 900 ms on the script's session and then the
/// frame's own budget; the bet on the first try's length starts the second with 1.5 s of the
/// op's 2.5 s left. The second is cut at the op's deadline, as late. Outside `OpScope::within`,
/// as `waitForFunction`'s first sweep is, the op ends on time; inside it, as a read's sweep is,
/// the cut ends the sweep before the op's own timeout can replace Neo's refusal. Both tries'
/// values reach the host either way.
#[tokio::test(start_paused = true)]
async fn tests_that_a_try_longer_than_the_last_is_cut_at_the_deadline_as_late() -> TestResult {
    let deadline = Duration::from_millis(2_500);
    let tab = ScriptedTab::new(LateOopif::new(slow_second));
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.waitForFunction").ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, timeout_ms(2_500));
    let call = MainWorldCall::poll(script("() => 1"), None);
    let started = Instant::now();
    let opened = EvaluateGuard::open(&run.playwright, &scope, &run.page, call).await;
    let took = started.elapsed();
    let refused = opened.err().ok_or("a frame late on both tries refuses")?;
    assert_eq!(
        (refused.error_code(), refused.message()),
        (
            Some(ErrorCode::Unsupported),
            "page.waitForFunction: a frame of the script's site did not answer Neo's privacy \
             sweep within 1000ms, so the script was not run; try again once the frame responds"
        ),
    );
    assert!(
        took > FRAME_BUDGET * 2 && took <= deadline,
        "two tries, the second cut at the op's deadline: {took:?}"
    );
    let heard = run.hooks.swept();
    assert!(
        ["main-try-1", "main-try-2"]
            .iter()
            .all(|value| heard.iter().any(|heard| heard == value)),
        "both tries' values reached the host: {heard:?}"
    );

    let tab = ScriptedTab::new(LateOopif::new(slow_second));
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("locator.textContent").ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, timeout_ms(2_500));
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let world = page
        .world(&page.main_endpoint(), Place::Main)
        .await
        .map_err(|failure| failure.to_string())?;
    let swept = scope
        .within(SiteSweep::of(&page, world))
        .await
        .map_err(|_expired| "the op's own timeout ended the sweep")?;
    let withheld = swept
        .values(&run.playwright, &scope, page.tab(), FieldValues::default())
        .err()
        .ok_or("a frame late on both tries withholds the text")?;
    assert_eq!(
        (withheld.error_code(), withheld.message()),
        (
            Some(ErrorCode::Unsupported),
            "locator.textContent: a frame of the page's site did not answer Neo's privacy sweep \
             within 1000ms, so page text that may hold its values is withheld; try again once \
             the frame responds"
        ),
    );
    let heard = run.hooks.swept();
    assert!(
        ["main-try-1", "main-try-2"]
            .iter()
            .all(|value| heard.iter().any(|heard| heard == value)),
        "both tries' values reached the host: {heard:?}"
    );
    Ok(())
}

/// Whether the host was never asked to mark anything sent.
fn env_sent_nothing(env: &TestEnv) -> bool {
    !env.calls().contains(&test_env::EnvCall::MarkSent)
}

/// `page.evaluate` has no Playwright timeout, so a frame late on every try is tried for
/// [`UNTIMED_RETRY`] from the first try, and no longer.
#[tokio::test(start_paused = true)]
async fn tests_that_an_op_without_a_timeout_tries_a_late_frame_for_a_bounded_window() -> TestResult
{
    let tab = ScriptedTab::new(LateChild::new(every));
    let Opened { refused, took, .. } =
        open_beside_late_child(&tab, "page.evaluate", CallHeader::default()).await?;
    let refused = refused.ok_or("a frame late on every try refuses")?;
    assert_eq!(
        refused.error_code(),
        Some(ErrorCode::Unsupported),
        "{refused}"
    );
    assert!(
        refused
            .message()
            .starts_with("page.evaluate: a frame of the script's site did not answer"),
        "{refused}"
    );
    assert!(
        took + FRAME_BUDGET > UNTIMED_RETRY && took <= UNTIMED_RETRY + Duration::from_millis(1),
        "tried for the window and no longer: {took:?}"
    );
    Ok(())
}

/// [`UNTIMED_RETRY`] bounds each stall of an op without a timeout, not the op: here a poll with
/// `timeout: 0`, whose first sweep meets a stall that recovers on its second try. The sweep after
/// its script meets a frame late for good, which gets a whole window of its own, opened when
/// that sweep started, and fails the op when the window ends: the earlier stall neither shortens
/// it nor keeps the op trying past it.
#[tokio::test(start_paused = true)]
async fn tests_that_a_stall_past_the_untimed_window_fails_after_one_that_recovered() -> TestResult {
    fn all_but_the_second(sweep: usize) -> bool {
        sweep != 1
    }
    let tab = ScriptedTab::new(LateChild::new(all_but_the_second));
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.waitForFunction").ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, timeout_ms(0));
    let call = MainWorldCall::poll(script("() => 1"), None);
    let started = Instant::now();
    let guard = EvaluateGuard::open(&run.playwright, &scope, &run.page, call).await?;
    let recovered = started.elapsed();
    assert!(
        recovered >= FRAME_BUDGET && recovered < FRAME_BUDGET + Duration::from_millis(100),
        "the first stall recovered on its second try: {recovered:?}"
    );
    let pending = guard.run(&scope).await?;
    let stalled = Instant::now();
    let withheld = pending
        .sweep(&scope)
        .await
        .err()
        .ok_or("a frame late for good withholds the script's result")?;
    let took = stalled.elapsed();
    assert_eq!(
        (withheld.error_code(), withheld.message()),
        (
            Some(ErrorCode::Unsupported),
            "page.waitForFunction: a frame of the script's site did not answer Neo's privacy \
             sweep within 1000ms, so the result of the script, which ran, is withheld; try again \
             once the frame responds"
        ),
    );
    assert!(
        took + FRAME_BUDGET > UNTIMED_RETRY && took <= UNTIMED_RETRY + Duration::from_millis(1),
        "the second stall had a whole window of its own, and no more: {took:?}"
    );
    assert_eq!(
        tab.script().child_sweeps.load(Ordering::SeqCst),
        2 + 5,
        "two tries for the first stall, five 1 s tries for the second"
    );
    Ok(())
}

/// A tab whose renderer answers its first `slow` hidden-root scans only after [`LATE`]: a new
/// world's install misses its budget at the scan.
struct LateScan {
    slow: usize,
    scans: AtomicUsize,
}

impl TabScript for LateScan {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::empty_sweep()
    }

    fn delay(&self, method: &str) -> Duration {
        if method == "DOM.describeNode" && self.scans.fetch_add(1, Ordering::SeqCst) < self.slow {
            LATE
        } else {
            Duration::ZERO
        }
    }
}

/// An install whose scan of the document's closed shadow roots misses the frame's budget left no
/// world, so it is installed again while the op's deadline has room for it; late on the last
/// try it fails as one late install did, with Neo's message on why.
#[tokio::test(start_paused = true)]
async fn tests_that_an_install_late_at_its_scan_is_tried_again_inside_the_deadline() -> TestResult {
    for (slow, timeout, installed) in [(1, 5_000, true), (usize::MAX, 2_500, false)] {
        let tab = ScriptedTab::new(LateScan {
            slow,
            scans: AtomicUsize::new(0),
        });
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("locator.textContent").ok_or("an op")?;
        let scope = OpScope::new(&run.env, op, timeout_ms(timeout));
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let started = Instant::now();
        let world = page.world(&page.main_endpoint(), Place::Main).await;
        let took = started.elapsed();
        match world {
            Ok(_) => assert!(
                installed && took >= FRAME_BUDGET && took < FRAME_BUDGET * 2,
                "installed on the second try: {took:?}"
            ),
            Err(failure @ ProbeFailure::Late) => {
                assert!(
                    !installed && took >= FRAME_BUDGET * 2 && took <= Duration::from_millis(2_500),
                    "late on both tries the 2.5 s had room for: {took:?}"
                );
                let error = resolve::failed(&scope, failure);
                assert_eq!(
                    (error.error_code(), error.message()),
                    (
                        Some(ErrorCode::Unsupported),
                        "locator.textContent: a frame did not answer Neo's privacy scan within \
                         1000ms, so Neo cannot see every shadow root of its page and refuses the \
                         op; try again once the frame responds"
                    )
                );
            }
            Err(failure) => return Err(failure.to_string().into()),
        }
    }
    Ok(())
}

/// The sweep after a script is tried again as the one before it is: the script ran once, and
/// what it produced stays inside the `Pending` until a sweep of every frame of its site
/// completes, or is withheld once the op's deadline leaves no room for another try.
#[tokio::test(start_paused = true)]
async fn tests_that_the_sweep_after_a_script_is_tried_again_before_its_result_leaves() -> TestResult
{
    fn second(sweep: usize) -> bool {
        sweep == 1
    }
    fn after_the_first(sweep: usize) -> bool {
        sweep >= 1
    }
    for (late, withheld) in [
        (second as fn(usize) -> bool, false),
        (after_the_first, true),
    ] {
        let tab = ScriptedTab::new(LateChild::new(late));
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("page.waitForFunction").ok_or("an op")?;
        let scope = OpScope::new(&run.env, op, timeout_ms(2_500));
        let call = MainWorldCall::poll(script("() => 1"), None);
        let guard = EvaluateGuard::open(&run.playwright, &scope, &run.page, call).await?;
        let pending = guard.run(&scope).await?;
        let started = Instant::now();
        let swept = pending.sweep(&scope).await;
        let took = started.elapsed();
        let sent = run
            .env
            .calls()
            .iter()
            .filter(|call| **call == test_env::EnvCall::MarkSent)
            .count();
        assert_eq!(
            sent, 1,
            "the script went out once, after the sweep before it"
        );
        match swept {
            Ok(_) => assert!(
                !withheld && took >= FRAME_BUDGET && took < FRAME_BUDGET * 2,
                "the result left after the late try and one on time: {took:?}"
            ),
            Err(error) => {
                assert!(withheld, "{error}");
                assert_eq!(
                    (error.error_code(), error.message()),
                    (
                        Some(ErrorCode::Unsupported),
                        "page.waitForFunction: a frame of the script's site did not answer Neo's \
                         privacy sweep within 1000ms, so the result of the script, which ran, is \
                         withheld; try again once the frame responds"
                    )
                );
                assert!(took >= FRAME_BUDGET, "withheld after a late try: {took:?}");
            }
        }
    }
    Ok(())
}

/// A read's page text leaves only after a sweep of every frame of its site completed, masked
/// with the values every try found: the value the late try saw in the main frame is masked,
/// though that frame holds another by the try that completed. A frame late on every try
/// withholds the text.
#[tokio::test(start_paused = true)]
async fn tests_that_a_read_after_a_late_frame_is_masked_with_every_tries_values() -> TestResult {
    for (late, withheld) in [(first as fn(usize) -> bool, false), (every, true)] {
        let tab = ScriptedTab::new(LateChild::new(late));
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name("locator.textContent").ok_or("an op")?;
        let scope = OpScope::new(&run.env, op, timeout_ms(2_500));
        let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
        let world = page
            .world(&page.main_endpoint(), Place::Main)
            .await
            .map_err(|failure| failure.to_string())?;
        let site = SiteSweep::of(&page, world).await.values(
            &run.playwright,
            &scope,
            page.tab(),
            FieldValues::default(),
        );
        match site {
            Ok(site) => {
                assert!(!withheld, "a frame late on every try withholds the text");
                assert_eq!(
                    site.mask_text("main-try-1, main-try-2".to_owned()),
                    "[private], [private]",
                    "both tries' values mask the text"
                );
            }
            Err(error) => {
                assert!(withheld, "{error}");
                assert_eq!(
                    (error.error_code(), error.message()),
                    (
                        Some(ErrorCode::Unsupported),
                        "locator.textContent: a frame of the page's site did not answer Neo's \
                         privacy sweep within 1000ms, so page text that may hold its values is \
                         withheld; try again once the frame responds"
                    )
                );
            }
        }
    }
    Ok(())
}

/// How Chromium fails a call into a frame that a navigation moved to another renderer while
/// the call was pending (measured by M6a, a poll during a cross-site goto).
const MOVED: &str = "Inspected target navigated or closed";

/// A tab whose main world cuts off every call into its document, as that navigation does,
/// counting them.
#[derive(Default)]
struct CutOff {
    calls: AtomicUsize,
}

impl TabScript for CutOff {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::empty_sweep()
    }

    fn main_world(&self, object: &str, _function: &str) -> Option<Result<Value, &'static str>> {
        (object == SCRIPTED_DOCUMENT).then(|| {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(MOVED)
        })
    }
}

/// A script a navigation cut off may have run, so `page.evaluate` never sends it again and
/// fails as upstream's does, with the message its `rewriteError` gives that failure.
#[tokio::test(start_paused = true)]
async fn tests_that_an_evaluate_a_navigation_cut_off_is_not_sent_again() -> TestResult {
    let tab = ScriptedTab::new(CutOff::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let input = json!({ "page": serde_json::to_value(&run.page)?,
                        "script": { "run": { "expression": "() => window.ran = true",
                                             "isFunction": true, "arg": r#"{"v":"undefined"}"# } } });
    let error = run
        .playwright
        .invoke(&run.env, op, input)
        .await
        .err()
        .ok_or("a script a navigation cut off does not answer")?;
    assert_eq!(
        (error.error_code(), error.message()),
        (
            Some(ErrorCode::PageError),
            format!("page.evaluate: {NAVIGATED}").as_str()
        ),
    );
    assert_eq!(
        tab.script().calls.load(Ordering::SeqCst),
        1,
        "the script went out once"
    );
    Ok(())
}

/// A tab whose first call into its main frame's document is cut off by that navigation, and in
/// whose next document the predicate holds at once: every probe sweep and main-world call, in
/// order.
#[derive(Default)]
struct MovesOnce {
    log: Mutex<Vec<&'static str>>,
}

impl MovesOnce {
    fn log(&self) -> Vec<&'static str> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, entry: &'static str) -> usize {
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        log.push(entry);
        log.iter().filter(|logged| **logged == entry).count()
    }
}

impl TabScript for MovesOnce {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        self.push("sweep");
        ProbeAnswer::empty_sweep()
    }

    fn main_world(&self, object: &str, function: &str) -> Option<Result<Value, &'static str>> {
        match object {
            SCRIPTED_DOCUMENT if self.push("build") == 1 => Some(Err(MOVED)),
            SCRIPTED_DOCUMENT => Some(Ok(
                json!({ "result": { "type": "function", "objectId": "poller" } }),
            )),
            "poller" if function == FIRST_POLL => {
                self.push("first poll");
                Some(Ok(
                    json!({ "result": { "type": "object", "subtype": "array",
                                            "value": [true, "true"] } }),
                ))
            }
            "poller" if function == SETTLED => {
                self.push("settled");
                Some(Ok(
                    json!({ "result": { "type": "boolean", "value": true } }),
                ))
            }
            _ => None,
        }
    }
}

/// A predicate a navigation cut off is side-effect free, as upstream's are meant to be, so the
/// wait goes on in the frame's next document, as upstream's does: the poller is built again
/// there, only after a sweep that followed the cut-off call.
#[tokio::test(start_paused = true)]
async fn tests_that_a_wait_a_navigation_cut_off_polls_again_after_a_sweep() -> TestResult {
    let tab = ScriptedTab::new(MovesOnce::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.waitForFunction").ok_or("page.waitForFunction is an op")?;
    let input = json!({ "call": { "timeoutMs": 5_000 }, "page": serde_json::to_value(&run.page)?,
                        "script": { "run": { "expression": "() => true", "isFunction": true,
                                             "arg": r#"{"v":"undefined"}"# } } });
    let waited = run.playwright.invoke(&run.env, op, input).await?;
    assert_eq!(
        waited,
        json!({ "value": true, "preview": "true" }),
        "settled in the next document"
    );
    assert_eq!(
        tab.script().log(),
        [
            "sweep",
            "build",
            "sweep",
            "build",
            "first poll",
            "settled",
            "sweep"
        ],
        "a sweep between the cut-off call and the poller built again"
    );
    Ok(())
}

/// A sweep the op's own timeout cuts is never dropped with what it found: the frame that
/// answered reports its value to the host, though a same-site frame stalls past the timeout,
/// and the op fails with upstream's timeout, not with Neo's refusal, which would name a budget
/// the frame never had. setContent's sweep before its write and waitForFunction's before its
/// first poll ran inside the op's timeout, which dropped them whole (M2R review F2).
#[tokio::test(start_paused = true)]
async fn tests_that_a_sweep_the_op_timeout_cuts_reports_what_it_found() -> TestResult {
    let wait = json!({ "script": { "run": { "expression": "() => 1", "isFunction": true,
                                            "arg": r#"{"v":"undefined"}"# } } });
    let mut ended = Vec::new();
    for (name, mut input) in [
        ("page.waitForFunction", wait),
        ("page.setContent", json!({ "html": "<p>hi</p>" })),
    ] {
        let tab = ScriptedTab::new(LateChild::new(every));
        let run = ScriptedRun::over(&tab).await?;
        let op = Op::from_name(name).ok_or("an op")?;
        if let Some(fields) = input.as_object_mut() {
            fields.insert("page".to_owned(), serde_json::to_value(&run.page)?);
            fields.insert("call".to_owned(), json!({ "timeoutMs": 500 }));
        }
        let error = run
            .playwright
            .invoke(&run.env, op, input)
            .await
            .err()
            .ok_or(format!("{name}: a frame stalled past the timeout"))?;
        ended.push((
            name,
            error.error_code(),
            error
                .message()
                .starts_with(&format!("{name}: Timeout 500ms exceeded.")),
            run.hooks.swept().iter().any(|value| value == "main-try-1"),
            run.env.calls().contains(&test_env::EnvCall::MarkSent),
        ));
    }
    assert_eq!(
        ended,
        [
            (
                "page.waitForFunction",
                Some(ErrorCode::WaitTimeout),
                true,
                true,
                false
            ),
            (
                "page.setContent",
                Some(ErrorCode::WaitTimeout),
                true,
                true,
                false
            ),
        ],
        "(op, code, upstream's timeout, the answered value reached the host, anything sent)"
    );
    Ok(())
}

/// A tab whose page opened `link`, a popup of another site: the group's other tab, which no
/// sweep of the page's site reads but the host's rules must check. Its main world answers the
/// script with `1`, counted.
struct LinkedPopup {
    link: ScriptedLink,
    ran: AtomicUsize,
}

impl LinkedPopup {
    fn new(link: ScriptedLink) -> Self {
        Self {
            link,
            ran: AtomicUsize::new(0),
        }
    }
}

impl TabScript for LinkedPopup {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::empty_sweep()
    }

    fn linked(&self) -> Option<ScriptedLink> {
        Some(self.link)
    }

    fn main_world(&self, object: &str, _function: &str) -> Option<Result<Value, &'static str>> {
        (object == SCRIPTED_DOCUMENT).then(|| {
            self.ran.fetch_add(1, Ordering::SeqCst);
            Ok(json!({ "result": { "type": "string", "value": "1" } }))
        })
    }
}

/// `page.evaluate(() => 1)` as the prelude sends it, on `page`, for the host `env`: the op's
/// own outcome inside.
async fn evaluate_on(
    pw: &Playwright,
    env: &dyn OpEnv,
    page: &PageRef,
) -> Result<Result<Value, PlaywrightError>, Box<dyn Error>> {
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let input = json!({ "page": serde_json::to_value(page)?,
                        "script": { "run": { "expression": "() => 1", "isFunction": true,
                                             "arg": r#"{"v":"undefined"}"# } } });
    Ok(pw.invoke(env, op, input).await)
}

/// An extension's page the script's page opened is a popup like any other to that page's
/// script, though core's tab registry leaves internal pages out: the host's rules run on its
/// tab, by the id the browser's own tab list gives it, before the script goes out.
#[tokio::test]
async fn tests_that_a_linked_page_core_leaves_out_is_checked_by_its_tab() -> TestResult {
    let tab = ScriptedTab::new(LinkedPopup::new(ScriptedLink {
        target: "extension-popup",
        url: "chrome-extension://abcdefghijklmnop/popup.html",
        tab: Some(11),
    }));
    let run = ScriptedRun::over(&tab).await?;
    let value = evaluate_on(&run.playwright, &run.env, &run.page).await??;
    assert_eq!(value, json!(1), "the script ran");
    assert_eq!(
        run.env.calls(),
        [
            test_env::EnvCall::Recheck,
            test_env::EnvCall::RecheckReach(vec![TabId(11)]),
            test_env::EnvCall::MarkSent
        ],
        "the host's rules ran on the popup's tab, then the op was marked sent"
    );
    Ok(())
}

/// A host that implements only what `OpEnv` requires, over a [`TestEnv`], so its
/// `recheck_reach` is the trait's default.
struct DefaultReach(TestEnv);

impl OpEnv for DefaultReach {
    fn view(&self) -> &BrowserView {
        self.0.view()
    }

    fn run_state(&self) -> &Arc<RunState> {
        self.0.run_state()
    }

    fn recheck(&self) -> Result<(), Refusal> {
        self.0.recheck()
    }

    fn mark_sent(&self) {
        self.0.mark_sent();
    }

    fn mark_sensitive_target(&self) {
        self.0.mark_sensitive_target();
    }

    fn note(&self, line: String) {
        self.0.note(line);
    }
}

/// A tab of the group the browser lists no tab for, as one that closed since the sweep read
/// the targets, is a tab the host's rules cannot be asked about. So the script is refused with
/// nothing sent, as a host that checks no other tab refuses it.
#[tokio::test]
async fn tests_that_a_linked_target_with_no_tab_refuses_the_script_as_an_unchecked_tab_does()
-> TestResult {
    let link = |tab| ScriptedLink {
        target: "closing-popup",
        url: "https://b.test/",
        tab,
    };
    let tab = ScriptedTab::new(LinkedPopup::new(link(None)));
    let run = ScriptedRun::over(&tab).await?;
    let refused = evaluate_on(&run.playwright, &run.env, &run.page)
        .await?
        .err()
        .ok_or("a tab the host cannot be asked about refuses the script")?;
    assert_eq!(
        tab.script().ran.load(Ordering::SeqCst),
        0,
        "the script never reached the page"
    );
    assert!(
        !run.env.calls().contains(&test_env::EnvCall::MarkSent),
        "nothing was marked sent: {:?}",
        run.env.calls()
    );
    let named = ScriptedTab::new(LinkedPopup::new(link(Some(12))));
    let ScriptedRun {
        playwright,
        env,
        page,
        ..
    } = ScriptedRun::over(&named).await?;
    let unchecked = evaluate_on(&playwright, &DefaultReach(env), &page)
        .await?
        .err()
        .ok_or("a host that checks no other tab refuses the script")?;
    assert_eq!(
        (refused.owner(), refused.error_code(), refused.message()),
        (
            unchecked.owner(),
            unchecked.error_code(),
            unchecked.message()
        ),
        "the refusal a host that checks no other tab gives"
    );
    Ok(())
}

/// A popup of the scripted tab's own site, so every sweep of the page's site covers it too.
const SAME_SITE_POPUP: ScriptedLink = ScriptedLink {
    target: "same-site-popup",
    url: "https://a.test/popup",
    tab: Some(13),
};

/// A tab whose page has opened [`SAME_SITE_POPUP`] once the test arms it, and whose browser then
/// answers `Browser.getVersion` only after the CDP client gave up on it. A tab's first page state
/// waits on that read when an earlier one failed, as every one does here (the scripted reply
/// names no user agent), so the popup's first sweep stalls in its page setup, before any frame
/// of it is asked anything. The main frame's sweeps find `swept-alone` before the popup opened
/// and `swept-beside-the-popup` after, and its main world answers a script with `1`.
#[derive(Default)]
struct PopupSetupStalls {
    armed: AtomicBool,
}

impl TabScript for PopupSetupStalls {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        let value = if self.armed.load(Ordering::SeqCst) {
            "swept-beside-the-popup"
        } else {
            "swept-alone"
        };
        ProbeAnswer::reply(json!({ "kind": "sweep", "values": [value] }))
    }

    fn linked(&self) -> Option<ScriptedLink> {
        self.armed.load(Ordering::SeqCst).then_some(SAME_SITE_POPUP)
    }

    fn outlasts_client(&self, method: &str) -> bool {
        method == "Browser.getVersion" && self.armed.load(Ordering::SeqCst)
    }

    fn main_world(&self, object: &str, _function: &str) -> Option<Result<Value, &'static str>> {
        (object == SCRIPTED_DOCUMENT)
            .then(|| Ok(json!({ "result": { "type": "string", "value": "1" } })))
    }
}

/// A run on a [`PopupSetupStalls`] tab whose page state is set up, while the browser still
/// answers, by one `page.evaluate`; then the popup opens and the browser stops answering.
async fn beside_a_stalling_popup() -> Result<ScriptedRun, Box<dyn Error>> {
    let tab = ScriptedTab::new(PopupSetupStalls::default());
    let run = ScriptedRun::over(&tab).await?;
    evaluate_on(&run.playwright, &run.env, &run.page).await??;
    tab.script().armed.store(true, Ordering::SeqCst);
    Ok(run)
}

/// A linked tab's page setup belongs to its sweep's try, and ends with it. The browser-side
/// reads a popup's first page state waits on can stall, and no timeout of the op's runs around
/// a guard's sweep. So an op with a timeout times out as upstream's would, at its own deadline
/// and with nothing sent, and the value the page's main frame answered with in that try still
/// reaches the host. An op without one tries again on fresh budgets for its untimed window, and
/// then fails closed, rather than wait out the CDP client's own 60 s (M3R review F1).
#[tokio::test(start_paused = true)]
async fn tests_that_a_linked_tabs_page_setup_ends_with_its_try() -> TestResult {
    let wait = json!({ "script": { "run": { "expression": "() => 1", "isFunction": true,
                                            "arg": r#"{"v":"undefined"}"# } } });
    let mut ended = Vec::new();
    for (name, mut input) in [
        ("page.waitForFunction", wait),
        ("page.setContent", json!({ "html": "<p>hi</p>" })),
    ] {
        let run = beside_a_stalling_popup().await?;
        let before = run.env.calls().len();
        if let Some(fields) = input.as_object_mut() {
            fields.insert("page".to_owned(), serde_json::to_value(&run.page)?);
            fields.insert("call".to_owned(), json!({ "timeoutMs": 500 }));
        }
        let op = Op::from_name(name).ok_or("an op")?;
        let started = Instant::now();
        let error = run
            .playwright
            .invoke(&run.env, op, input)
            .await
            .err()
            .ok_or(format!(
                "{name}: the popup's setup stalled past the timeout"
            ))?;
        let took = started.elapsed();
        let policed = run.env.calls().into_iter().skip(before).any(|call| {
            matches!(
                call,
                test_env::EnvCall::Recheck
                    | test_env::EnvCall::RecheckReach(_)
                    | test_env::EnvCall::MarkSent
            )
        });
        ended.push((
            name,
            took <= Duration::from_millis(500),
            error.error_code(),
            error
                .message()
                .starts_with(&format!("{name}: Timeout 500ms exceeded.")),
            run.hooks
                .swept()
                .iter()
                .any(|value| value == "swept-beside-the-popup"),
            policed,
        ));
    }
    assert_eq!(
        ended,
        [
            (
                "page.waitForFunction",
                true,
                Some(ErrorCode::WaitTimeout),
                true,
                true,
                false
            ),
            (
                "page.setContent",
                true,
                Some(ErrorCode::WaitTimeout),
                true,
                true,
                false
            ),
        ],
        "(op, ended by its deadline, code, upstream's timeout, the main frame's value reached \
         the host, the host's rules ran for a send)"
    );

    let run = beside_a_stalling_popup().await?;
    let started = Instant::now();
    let refused = evaluate_on(&run.playwright, &run.env, &run.page)
        .await?
        .err()
        .ok_or("a popup whose setup stalls on every try refuses the script")?;
    let took = started.elapsed();
    assert_eq!(
        (refused.error_code(), refused.message()),
        (
            Some(ErrorCode::Unsupported),
            "page.evaluate: a frame of the script's site did not answer Neo's privacy sweep \
             within 1000ms, so the script was not run; try again once the frame responds"
        ),
    );
    assert!(
        took + FRAME_BUDGET > UNTIMED_RETRY && took <= UNTIMED_RETRY + Duration::from_millis(1),
        "tried for the untimed window and no longer: {took:?}"
    );
    Ok(())
}
