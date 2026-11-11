use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use browseros_cdp::{CdpError, CdpEvent, TargetedEvents};
use browseros_core::sensitive_fields::FieldValues;
use browseros_core::{
    BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, CoreError, SessionId,
};
use futures_util::future::BoxFuture;
use neo_trace::ErrorOwner;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{self, Instant};

use super::{
    ArgText, Called, EvaluateGuard, FIRST_POLL, FRAME_BUDGET, MainWorldCall, NAVIGATED, Outcome,
    RemoteObject, Reply, SETTLED, Script, Serialized, Subject, decode, not_run, parsed, returned,
};
use crate::op_scope::OpScope;
use crate::page::OpPage;
use crate::page::op_page::UNTIMED_RETRY;
use crate::page::site_sweep::SiteSweep;
use crate::probe::{PROBE_PROTOCOL, ProbeFailure};
use crate::target::{CallHeader, PageRef};
use crate::test_env::{self, TestEnv};
use crate::test_support::{
    ProbeAnswer, RecordingHooks, SCRIPTED_CHILD, SCRIPTED_DOCUMENT, SCRIPTED_OOPIF, ScriptedRun,
    ScriptedTab, TabScript,
};
use crate::world::Place;
use crate::{ErrorCode, Op, Playwright, PlaywrightError, resolve};

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
    assert!(matches!(text.result.text(), Serialized::Text(text) if text == r#"{"v":"NaN"}"#));
    let none = reply(json!({ "result": { "type": "undefined" } }))?;
    assert!(matches!(none.result.text(), Serialized::Undefined));
    let number = reply(json!({ "result": { "type": "number", "value": 1 } }))?;
    assert!(
        matches!(number.result.text(), Serialized::Unreadable),
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
        error.thrown(),
        Some("TypeError: boom\n    at eval (eval at evaluate (:1:1))".to_owned())
    );
    let string = reply(json!({
        "result": { "type": "string" },
        "exceptionDetails": { "text": "Uncaught", "exception": { "type": "string", "value": "plain" } },
    }))?;
    assert_eq!(string.thrown(), Some("plain".to_owned()));
    let null = reply(json!({
        "result": { "type": "object" },
        "exceptionDetails": { "text": "Uncaught", "exception": { "type": "object", "subtype": "null", "value": null } },
    }))?;
    assert_eq!(null.thrown(), Some("null".to_owned()));
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
        bigint.json_value(Serialized::Text(r#"{"bi":"10"}"#.to_owned())),
        Serialized::Undefined
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
            array.json_value(Serialized::Text("[]".to_owned())),
            Serialized::Text(_)
        ),
        "an object's value is the page's own serialized text"
    );
    Ok(())
}

/// A value leaves in Playwright's format as the page answered it, a key and a number spelled
/// like a field's value included; one upstream could not return, or one no serializer writes,
/// fails as the page's, with nothing of it in the message.
#[tokio::test]
async fn tests_that_an_answer_leaves_as_the_page_gave_it_or_fails_as_the_pages() -> TestResult {
    let env = TestEnv::new();
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let scope = OpScope::new(&env, op, CallHeader::default());
    let text = r#"{"o":[{"k":"hunter2-secret","v":"you typed hunter2-secret"},{"k":"n","v":482913}],"id":1}"#;
    assert_eq!(
        parsed(&scope, Serialized::Text(text.to_owned()))?.into_json(),
        json!({ "o": [{ "k": "hunter2-secret", "v": "you typed hunter2-secret" },
                      { "k": "n", "v": 482_913 }], "id": 1 })
    );
    assert_eq!(
        parsed(&scope, Serialized::Undefined)?.into_json(),
        json!({ "v": "undefined" })
    );
    let deep = format!("{}0{}", "[".repeat(298), "]".repeat(298));
    for (serialized, message) in [
        (
            Serialized::Text(deep),
            "page.evaluate: Cannot serialize result: object reference chain is too long.",
        ),
        (
            Serialized::Text(r#"{"h":0,"secret":"hunter2-secret"}"#.to_owned()),
            "page.evaluate: the page answered with a value outside Playwright's serialized \
             format, as a page that replaced the globals its serializer uses would, so Neo \
             withheld it",
        ),
        (
            Serialized::Unreadable,
            "page.evaluate: the page answered with a value outside Playwright's serialized \
             format, as a page that replaced the globals its serializer uses would, so Neo \
             withheld it",
        ),
    ] {
        let failed = parsed(&scope, serialized)
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
        let failed = returned(&scope, parsed(&scope, Serialized::Text(text.to_owned()))?)
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
/// 1 s, but the frame has one budget for both: a read's sweep withholds the text at 1 s rather
/// than passing at 1.2 s, the op's 1.5 s leaving no room for a second try.
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
    let op = Op::from_name("locator.textContent").ok_or("locator.textContent is an op")?;
    let scope = OpScope::new(&env, op, timeout_ms(1_500));
    let page = OpPage::admit_handle(&playwright, &scope, &page).await?;
    let world = page
        .world(&page.main_endpoint(), Place::Main)
        .await
        .map_err(|failure| failure.to_string())?;
    let started = Instant::now();
    let swept = SiteSweep::of(&page, world).await.values(
        &playwright,
        &scope,
        page.tab(),
        FieldValues::default(),
    );
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
/// other op, here one that declares only `Capture`, before it admits anything. A
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

/// A read's sweep beside a [`LateChild`] tab, with `header`: what it withheld the text with, if
/// it did, and how long the sweep took.
async fn read_beside_late_child(
    tab: &Arc<ScriptedTab<LateChild>>,
    header: CallHeader,
) -> Result<(Option<PlaywrightError>, Duration), Box<dyn Error>> {
    let run = ScriptedRun::over(tab).await?;
    let op = Op::from_name("locator.textContent").ok_or("an op")?;
    let scope = OpScope::new(&run.env, op, header);
    let page = OpPage::admit_handle(&run.playwright, &scope, &run.page).await?;
    let world = page
        .world(&page.main_endpoint(), Place::Main)
        .await
        .map_err(|failure| failure.to_string())?;
    let started = Instant::now();
    let withheld = SiteSweep::of(&page, world)
        .await
        .values(&run.playwright, &scope, page.tab(), FieldValues::default())
        .err();
    Ok((withheld, started.elapsed()))
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
/// on its budget, and the second would take 2 s, 900 ms on the read's session and then the
/// frame's own budget; the bet on the first try's length starts the second with 1.5 s of the
/// op's 2.5 s left. The second is cut at the op's deadline, as late, so inside `OpScope::within`
/// the cut ends the sweep before the op's own timeout can replace Neo's refusal. Both tries'
/// values reach the host.
#[tokio::test(start_paused = true)]
async fn tests_that_a_try_longer_than_the_last_is_cut_at_the_deadline_as_late() -> TestResult {
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

/// An op without a Playwright timeout tries a frame late on every try for [`UNTIMED_RETRY`]
/// from the first try, and no longer.
#[tokio::test(start_paused = true)]
async fn tests_that_an_op_without_a_timeout_tries_a_late_frame_for_a_bounded_window() -> TestResult
{
    let tab = ScriptedTab::new(LateChild::new(every));
    let (withheld, took) = read_beside_late_child(&tab, CallHeader::default()).await?;
    let withheld = withheld.ok_or("a frame late on every try withholds the text")?;
    assert_eq!(
        withheld.error_code(),
        Some(ErrorCode::Unsupported),
        "{withheld}"
    );
    assert!(
        took + FRAME_BUDGET > UNTIMED_RETRY && took <= UNTIMED_RETRY + Duration::from_millis(1),
        "tried for the window and no longer: {took:?}"
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
/// whose next document the predicate holds at once: every probe request and main-world call, in
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
        self.push("probe");
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
/// there, and no probe request goes out around the polls.
#[tokio::test(start_paused = true)]
async fn tests_that_a_wait_a_navigation_cut_off_polls_again_in_the_next_document() -> TestResult {
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
        ["build", "build", "first poll", "settled"],
        "the poller built again after the cut-off call, with nothing else sent to the probe"
    );
    Ok(())
}
