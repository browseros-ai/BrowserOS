//! A script's `signal` from the realm to the call it aborts, through `Runtime::run` against the
//! fake browser in `support/fake_browser.rs`: the realm's `AbortController` and `AbortSignal`
//! (`abort.rs` `REALM_GLOBALS`), a signal aborted before the call (nothing is sent), and one
//! aborted while the call waits (it replies `AbortError` and its record says so). No browser
//! needed; the parity files compare the same behaviour with Playwright on BrowserOS.

#[path = "support/fake_browser.rs"]
#[expect(
    dead_code,
    reason = "the page's interceptor and click readers serve click_lifecycle.rs"
)]
mod fake_browser;

use std::time::Duration;

use fake_browser::{FakeBrowser, Replies};
use neo_runtime::{CallResult, RunOutcome, RunStatus};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn run(code: &str) -> RunOutcome {
    let browser = FakeBrowser::start(Replies::default()).await;
    browser
        .runtime()
        .await
        .run(fake_browser::run_request(
            code,
            Duration::from_secs(10),
            CancellationToken::new(),
        ))
        .await
}

fn value(outcome: &RunOutcome) -> &Value {
    assert_eq!(outcome.status, RunStatus::Ok, "{outcome:#?}");
    outcome.value.as_ref().unwrap_or(&Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_realm_has_whatwgs_abort_controller_and_signal() {
    let code = r"
const controller = new AbortController()
const signal = controller.signal
const order = []
const once = () => order.push('once')
signal.addEventListener('abort', once, { once: true })
signal.addEventListener('abort', once, { once: true })
signal.onabort = function (event) { order.push(`onabort ${event.type} ${this === signal}`) }
signal.addEventListener('abort', { handleEvent: (event) => order.push(`object ${event.target === signal} ${event.currentTarget === signal}`) })
const removed = () => order.push('removed')
signal.addEventListener('abort', removed)
signal.removeEventListener('abort', removed)
const before = { aborted: signal.aborted, reason: signal.reason, threw: (() => { try { signal.throwIfAborted(); return false } catch { return true } })() }
controller.abort()
controller.abort('again')
const reason = signal.reason
let thrown
try { signal.throwIfAborted() } catch (error) { thrown = error }
let illegal
try { new AbortSignal() } catch (error) { illegal = error.name }
const preset = AbortSignal.abort('why')
const presetDefault = AbortSignal.abort()
const timed = AbortSignal.timeout(20)
await new Promise((resolve) => setTimeout(resolve, 80))
const first = new AbortController()
const second = new AbortController()
const any = AbortSignal.any([first.signal, second.signal])
let anyEvents = 0
any.addEventListener('abort', () => anyEvents++)
second.abort('second')
first.abort('first')
return {
  types: [typeof AbortController, typeof AbortSignal, Object.prototype.toString.call(signal), signal instanceof AbortSignal],
  before,
  after: { aborted: signal.aborted, thrownIsReason: thrown === reason },
  reason: { isDomException: reason instanceof DOMException, isError: reason instanceof Error, name: reason.name, message: reason.message },
  order,
  illegal,
  preset: [preset.aborted, preset.reason, presetDefault.reason.name],
  timed: [timed.aborted, timed.reason.name, timed.reason.message],
  any: [any.aborted, any.reason, anyEvents, AbortSignal.any([preset]).reason],
}";
    let outcome = run(code).await;
    assert_eq!(
        value(&outcome),
        &json!({
            "types": ["function", "function", "[object AbortSignal]", true],
            "before": { "aborted": false, "threw": false },
            "after": { "aborted": true, "thrownIsReason": true },
            "reason": {
                "isDomException": true,
                "isError": true,
                "name": "AbortError",
                "message": "This operation was aborted",
            },
            "order": ["once", "onabort abort true", "object true true"],
            "illegal": "TypeError",
            "preset": [true, "why", "AbortError"],
            "timed": [true, "TimeoutError", "The operation was aborted due to timeout"],
            "any": [true, "second", 1, "why"],
        }),
        "one `once` listener for a repeated add, the handler in its place, no second dispatch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abort_listener_that_throws_ends_the_run_with_its_error() {
    let code = r"
const controller = new AbortController()
let later = false
controller.signal.addEventListener('abort', () => { throw new Error('listener broke') })
controller.signal.addEventListener('abort', () => { later = true })
controller.abort()
await new Promise((resolve) => setTimeout(resolve, 50))
return { later }";
    let outcome = run(code).await;
    assert_eq!(outcome.status, RunStatus::Error, "{outcome:#?}");
    let error = outcome.error.as_ref().expect("the run failed");
    assert_eq!(
        (error.name.as_str(), error.message.as_str()),
        ("Error", "listener broke")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_aborted_signal_rejects_before_the_call_is_sent() {
    let code = r"
try {
  await page.locator('#go').click({ signal: AbortSignal.abort() })
  return 'clicked'
} catch (error) {
  return { name: error.name, message: error.message, cause: error.cause.name }
}";
    let outcome = run(code).await;
    assert_eq!(
        value(&outcome),
        &json!({
            "name": "AbortError",
            "message": "locator.click: The operation was aborted",
            "cause": "AbortError",
        })
    );
    assert!(
        outcome.calls.is_empty(),
        "nothing reached the browser: {:?}",
        outcome.calls
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signal_that_aborts_mid_wait_ends_the_call_with_abort_error() {
    let code = r"
const controller = new AbortController()
setTimeout(() => controller.abort(new Error('stop')), 200)
try {
  await page.locator('#missing').click({ signal: controller.signal, timeout: 5000 })
  return 'clicked'
} catch (error) {
  return { name: error.name, message: error.message.split('\n')[0], cause: error.cause.message, log: error.log }
}";
    let outcome = run(code).await;
    let value = value(&outcome);
    assert_eq!(
        (&value["name"], &value["message"], &value["cause"]),
        (
            &json!("AbortError"),
            &json!("locator.click: stop"),
            &json!("stop")
        )
    );
    let log = value["log"].as_array().expect("a call log");
    assert_eq!(
        log.last(),
        Some(&json!("  - operation was aborted: stop")),
        "Playwright's last line for an aborted call: {log:?}"
    );
    let [call] = outcome.calls.as_slice() else {
        panic!("one call: {:?}", outcome.calls);
    };
    assert_eq!(
        call.result,
        CallResult::Error {
            name: "AbortError".to_string(),
            message: "stop".to_string(),
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_abort_for_a_call_that_replied_or_never_ran_is_ignored() {
    let code = r"
__neo_runtime.abort(999, 'nobody')
const controller = new AbortController()
const text = await page.title({ signal: controller.signal })
controller.abort('late')
await page.locator('#go').click({ signal: controller.signal }).catch((error) => error.name)
const clicked = await page.locator('#go').click({ signal: new AbortController().signal })
return { text, clicked: clicked === undefined }";
    let outcome = run(code).await;
    assert_eq!(
        value(&outcome),
        &json!({ "text": "Fake page", "clicked": true }),
        "`title` reads no signal, as Playwright sends it with `kNoTimeout`"
    );
}
