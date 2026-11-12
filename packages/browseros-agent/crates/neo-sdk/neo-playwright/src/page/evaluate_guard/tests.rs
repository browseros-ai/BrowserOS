use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use browseros_cdp::CdpError;
use browseros_core::CoreError;
use neo_trace::ErrorOwner;
use serde_json::{Value, json};

use super::{
    ArgText, Called, EvaluateGuard, FIRST_POLL, MainWorldCall, NAVIGATED, OTHER_WORLD, Outcome,
    RemoteObject, Reply, SETTLED, Script, Serialized, Subject, THROUGH_EVALUATOR, decode, not_run,
    parsed, returned,
};
use crate::op_scope::OpScope;
use crate::target::{CallHeader, PageRef};
use crate::test_env::{self, EnvCall, TestEnv};
use crate::test_support::{ProbeAnswer, SCRIPTED_DOCUMENT, ScriptedRun, ScriptedTab, TabScript};
use crate::{ErrorCode, Op, Playwright, Refusal};

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

/// A host serves an op unheld unless its spec declares `MainWorld`, so the guard refuses any
/// other op, here one that declares only `Capture`, before it admits anything. A
/// script the prelude refused is raised next, with upstream's message and nothing admitted,
/// while a script to run goes on to admit its page, which fails here for want of a browser.
/// Paused: core waits out its own attach timeout before it gives up on the page.
#[tokio::test(start_paused = true)]
async fn tests_that_the_guard_lets_in_only_a_declared_op_and_a_script_to_run() -> TestResult {
    let playwright = Playwright::new(test_env::offline_session());
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
        ProbeAnswer::unexpected()
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
                        "expression": "() => window.ran = true", "isFunction": true,
                        "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } });
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
        ProbeAnswer::unexpected()
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
                        "expression": "() => true", "isFunction": true,
                        "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } });
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

/// A tab whose main frame's document is in another context than the session's default one, as
/// a same-process child frame's is: Chromium refuses a call through the evaluator before anything
/// runs, and the direct call answers. Logs each call into the document.
#[derive(Default)]
struct OtherWorld {
    log: Mutex<Vec<&'static str>>,
}

impl OtherWorld {
    fn log(&self) -> Vec<&'static str> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TabScript for OtherWorld {
    fn probe(&self, _frame: &str, _request: &Value) -> ProbeAnswer {
        ProbeAnswer::unexpected()
    }

    fn main_world(&self, object: &str, function: &str) -> Option<Result<Value, &'static str>> {
        if object != SCRIPTED_DOCUMENT {
            return None;
        }
        let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        if function == THROUGH_EVALUATOR {
            log.push("through the evaluator");
            return Some(Err(OTHER_WORLD));
        }
        log.push("direct");
        Some(Ok(json!({ "result": { "type": "string", "value": "7" } })))
    }
}

fn evaluate_input(run: &ScriptedRun) -> Result<Value, serde_json::Error> {
    Ok(json!({ "page": serde_json::to_value(&run.page)?,
               "expression": "() => 7", "isFunction": true,
                        "arg": { "value": r#"{"v":"undefined"}"#, "handles": [] } }))
}

/// A refused evaluator ran nothing, so the direct call goes out, and only after a recheck of
/// its own (STYLE P6): one recheck per send.
#[tokio::test(start_paused = true)]
async fn tests_that_a_refused_evaluator_falls_back_after_a_fresh_recheck() -> TestResult {
    let tab = ScriptedTab::new(OtherWorld::default());
    let run = ScriptedRun::over(&tab).await?;
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let answered = run
        .playwright
        .invoke(&run.env, op, evaluate_input(&run)?)
        .await?;
    assert_eq!(answered["value"], json!(7));
    assert_eq!(tab.script().log(), ["through the evaluator", "direct"]);
    let rechecks = run
        .env
        .calls()
        .iter()
        .filter(|call| matches!(call, EnvCall::Recheck))
        .count();
    assert_eq!(rechecks, 2, "the fallback was rechecked before it went out");
    Ok(())
}

/// A host that takes the tab between the refused evaluator and the fallback stops the fallback:
/// nothing runs in the page after the refusal.
#[tokio::test(start_paused = true)]
async fn tests_that_a_host_refusal_before_the_fallback_sends_nothing() -> TestResult {
    let tab = ScriptedTab::new(OtherWorld::default());
    let run = ScriptedRun::over(&tab).await?;
    let refusal = Refusal {
        owner: ErrorOwner::Host,
        code: "owner_has_control".to_owned(),
        message: "The owner is using this tab.".to_owned(),
        host_code: None,
    };
    let run = ScriptedRun {
        env: run.env.with_refusal_after(1, refusal),
        ..run
    };
    let op = Op::from_name("page.evaluate").ok_or("page.evaluate is an op")?;
    let refused = run
        .playwright
        .invoke(&run.env, op, evaluate_input(&run)?)
        .await
        .err()
        .ok_or("the host's refusal stops the fallback")?;
    assert!(
        refused.message().contains("The owner is using this tab."),
        "{}",
        refused.message()
    );
    assert_eq!(
        tab.script().log(),
        ["through the evaluator"],
        "no direct call after the refusal"
    );
    Ok(())
}
