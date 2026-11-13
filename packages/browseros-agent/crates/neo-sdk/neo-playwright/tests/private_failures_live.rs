//! Private failure diagnostics through the real SDK prelude in QuickJS, Rust retry/reporting,
//! and the browser's isolated probe. The hosted acceptance suite runs the same case script
//! before inspecting persistence; this suite additionally compares native conformance facts
//! and delivers trusted CDP events after an interceptor's arming call has returned.
//!
//! Run under `test:browser` with `cargo test -p neo-playwright --test private_failures_live
//! -- --ignored`. Every test owns and closes its tab; it never changes the browser's others.

#[path = "support/fixture_server.rs"]
#[expect(
    dead_code,
    reason = "the shared server also serves scripts and nested fixtures"
)]
mod fixture_server;
#[path = "support/js_runner.rs"]
mod js_runner;
#[path = "support/live_run.rs"]
#[expect(
    dead_code,
    reason = "the shared harness covers other input and takeover tests"
)]
mod live_run;
#[path = "support/locator.rs"]
#[expect(dead_code, reason = "these fixtures use only CSS selectors")]
mod locator;

use std::collections::HashMap;
use std::sync::Arc;

use browseros_core::{ProtocolSession, TabId};
use fixture_server::{FixtureServer, Page};
use futures_util::future::BoxFuture;
use js_runner::OpHost;
use live_run::{EnvCall, LiveRun, TestPage, TestResult, brand};
use neo_playwright::PlaywrightError;
use serde_json::{Value, json};

const HTML: &str = include_str!("data/private-failure-fields/page.html");
const CASES: &str = include_str!("data/private-failure-fields/cases.js");
const CONFORMANCE: &str = include_str!("data/private-failure-fields/conformance.js");
const CANARIES: [&str; 3] = ["AttrCanary-1", "TypedCanary-2", "TextCanary-3"];
const WRONG: &str = "Wrong expected string without a private marker";

/// One fixture server and one SDK-owned tab. Keeping the server alive until tab cleanup also
/// keeps later native-reference requests on the exact document the SDK assertions inspected.
struct Fixture {
    _server: FixtureServer,
    url: String,
    run: Arc<LiveRun>,
    page: TestPage,
}

impl Fixture {
    async fn open() -> TestResult<Self> {
        let server = FixtureServer::start(HashMap::from([(
            "/private-failures.html".to_owned(),
            Page::html(HTML),
        )]))
        .await?;
        let url = server.url("127.0.0.1", "/private-failures.html");
        let run = Arc::new(LiveRun::connect().await?);
        let page = run.new_page().await?;
        Ok(Self {
            _server: server,
            url,
            run,
            page,
        })
    }

    async fn script(&self, source: &str, conformance: bool) -> TestResult<Value> {
        let host: Arc<dyn OpHost> = if conformance {
            Arc::new(Conformance(Arc::clone(&self.run)))
        } else {
            Arc::clone(&self.run) as Arc<dyn OpHost>
        };
        let script = source.replace("__FIXTURE_URL__", &json!(self.url).to_string());
        Ok(js_runner::run_script(&host, &json!({ "page": brand(&self.page) }), &script).await?)
    }

    async fn close(&self) {
        self.run.close_page(&self.page).await;
    }
}

/// Only the host installs the conformance flag before the prelude captures it. All operations,
/// cache facts and page evaluation still go through the same `LiveRun` and browser document.
struct Conformance(Arc<LiveRun>);

impl OpHost for Conformance {
    fn invoke<'a>(
        &'a self,
        name: &'a str,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, PlaywrightError>> {
        OpHost::invoke(&*self.0, name, input)
    }

    fn event_head(&self) -> u64 {
        OpHost::event_head(&*self.0)
    }

    fn page_facts(&self, tabs: Vec<TabId>) -> BoxFuture<'_, Vec<Value>> {
        OpHost::page_facts(&*self.0, tabs)
    }

    fn gesture(&self, page: TestPage, expression: String) -> BoxFuture<'_, Result<Value, String>> {
        OpHost::gesture(&*self.0, page, expression)
    }

    fn upstream_snapshots(&self) -> bool {
        true
    }
}

fn assert_no_canaries(text: &str) {
    for canary in CANARIES {
        assert!(
            !text.contains(canary),
            "diagnostic exposes {canary}: {text}"
        );
    }
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn private_matcher_failures_hide_values_and_keep_explicit_reads_clear() -> TestResult {
    let fixture = Fixture::open().await?;
    let checked = async {
        let result = fixture.script(CASES, false).await?;
        let failures = result
            .get("failures")
            .and_then(Value::as_object)
            .ok_or("the script reports every named failure")?;
        assert!(
            failures.len() >= 40,
            "the complete matcher and preview matrix"
        );
        for field in ["failures", "clearTemplates", "snapshots"] {
            assert_no_canaries(&result.get(field).ok_or("the diagnostic field")?.to_string());
        }
        for call in fixture.run.calls() {
            if let EnvCall::Note(line) = call {
                assert_no_canaries(&line);
            }
        }
        assert_eq!(
            result
                .get("reads")
                .and_then(|reads| reads.get("inputValue")),
            Some(&json!("TypedCanary-2")),
            "successful deliberate reads retain their value after failure cleanup"
        );
        Ok(())
    }
    .await;
    fixture.close().await;
    checked
}

/// Return the already-booted engine context. The SDK operation installs the bundle first;
/// requesting the same named world reuses it, including the engine and interceptor slots.
/// The native endpoint and its isolated context must stay paired for probe comparisons.
struct EngineContext {
    session: ProtocolSession,
    context: i64,
}

async fn engine_context(fixture: &Fixture) -> TestResult<EngineContext> {
    let page = fixture.run.core_page(&fixture.page).await?;
    let session = fixture.run.browser().pages.get_session(page).await?.session;
    let tree = session.send_value("Page.getFrameTree", json!({})).await?;
    let frame = tree
        .get("frameTree")
        .and_then(|tree| tree.get("frame"))
        .and_then(|frame| frame.get("id"))
        .and_then(Value::as_str)
        .ok_or("the fixture has a main frame")?;
    let world = session
        .send_value(
            "Page.createIsolatedWorld",
            json!({ "frameId": frame, "worldName": "browseros-engine" }),
        )
        .await?;
    let context = world
        .get("executionContextId")
        .and_then(Value::as_i64)
        .ok_or("the engine world has an execution context")?;
    Ok(EngineContext { session, context })
}

async fn isolated(session: &ProtocolSession, context: i64, expression: &str) -> TestResult<Value> {
    let reply = session
        .send_value(
            "Runtime.evaluate",
            json!({ "contextId": context, "expression": expression,
                    "returnByValue": true, "awaitPromise": true }),
        )
        .await?;
    if let Some(exception) = reply.get("exceptionDetails") {
        return Err(format!("the isolated fixture threw: {exception}").into());
    }
    reply
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .ok_or_else(|| format!("the engine returned no JSON value: {reply}").into())
}

/// The generated private ABI receives policy alongside the request. This test-only sender is
/// used for native reference facts and for trusted-event scheduling, never as an SDK bypass
/// in the case matrix above.
async fn probe(
    session: &ProtocolSession,
    context: i64,
    request: Value,
    conformance: bool,
) -> TestResult<Value> {
    let call = json!({
        "request": request,
        "policy": { "conformance": conformance, "mayHaveOtherFrame": false },
    });
    isolated(
        session,
        context,
        &format!("globalThis.__neoProbe.call({call}, null, [])"),
    )
    .await
}

fn target(selector: &str) -> Value {
    json!({ "chunk": { "parts": [locator::css(selector)] } })
}

/// Every family has a named native reference. Comparison parameters match the prelude's,
/// while the actual expected string deliberately has no canary to excuse in a failure.
fn native_cases() -> Vec<(&'static str, &'static str, Value)> {
    let text = json!([{ "string": WRONG, "normalizeWhiteSpace": true }]);
    let value = json!([{ "string": WRONG }]);
    vec![
        (
            "toHaveValue",
            "#private-input",
            json!({ "expression": "to.have.value", "expectedText": value }),
        ),
        (
            "toHaveAttribute.value",
            "#private-input",
            json!({ "expression": "to.have.attribute.value", "expressionArg": "value", "expectedText": value }),
        ),
        (
            "toHaveAttribute.aria-valuetext",
            "#private-input",
            json!({ "expression": "to.have.attribute.value", "expressionArg": "aria-valuetext", "expectedText": value }),
        ),
        (
            "toHaveAttribute.aria-valuenow",
            "#private-input",
            json!({ "expression": "to.have.attribute.value", "expressionArg": "aria-valuenow", "expectedText": value }),
        ),
        (
            "toHaveText.direct",
            "#private-textarea",
            json!({ "expression": "to.have.text", "expectedText": text }),
        ),
        (
            "toHaveText.ancestor",
            "#private-ancestor",
            json!({ "expression": "to.have.text", "expectedText": text }),
        ),
        (
            "toContainText.direct",
            "#private-textarea",
            json!({ "expression": "to.have.text", "expectedText": [{ "string": WRONG, "normalizeWhiteSpace": true, "matchSubstring": true }] }),
        ),
        (
            "toHaveText.array",
            ".private-array",
            json!({ "expression": "to.have.text.array", "expectedText": [{ "string": WRONG, "normalizeWhiteSpace": true }, { "string": "Public tail", "normalizeWhiteSpace": true }] }),
        ),
        (
            "toContainText.array",
            ".private-array",
            json!({ "expression": "to.contain.text.array", "expectedText": [{ "string": WRONG, "normalizeWhiteSpace": true, "matchSubstring": true }] }),
        ),
        (
            "toHaveAccessibleName",
            "#name-from-private",
            json!({ "expression": "to.have.accessible.name", "expectedText": text }),
        ),
        (
            "toHaveAccessibleDescription",
            "#description-from-private",
            json!({ "expression": "to.have.accessible.description", "expectedText": text }),
        ),
        (
            "toHaveCSS.content.own",
            "#private-input",
            json!({ "expression": "to.have.css", "expressionArg": "content", "expectedText": value }),
        ),
        (
            "toHaveCSS.content.before",
            "#private-input",
            json!({ "expression": "to.have.css", "expressionArg": "content", "pseudo": "before", "expectedText": value }),
        ),
        (
            "toHaveCSS.content.after",
            "#private-input",
            json!({ "expression": "to.have.css", "expressionArg": "content", "pseudo": "after", "expectedText": value }),
        ),
        (
            "toMatchAriaSnapshot.locator",
            "#private-input",
            json!({ "expression": "to.match.aria", "expectedValue": { "kind": "role", "role": "button", "name": WRONG } }),
        ),
        (
            "toMatchAriaSnapshot.page",
            "document",
            json!({ "expression": "to.match.aria", "expectedValue": { "kind": "role", "role": "button", "name": WRONG } }),
        ),
        (
            "toBeEmpty",
            "#private-input",
            json!({ "expression": "to.be.empty" }),
        ),
        (
            "toBeHidden",
            "#private-input",
            json!({ "expression": "to.be.hidden" }),
        ),
    ]
}

async fn assert_native_facts(
    session: &ProtocolSession,
    context: i64,
    failures: &Value,
) -> TestResult {
    for (name, selector, mut params) in native_cases() {
        params
            .as_object_mut()
            .ok_or("native parameters are an object")?
            .insert("isNot".to_owned(), json!(false));
        let request = json!({ "op": "expect", "target": if selector == "document" {
            json!("document")
        } else { target(selector) }, "strictness": "strict", "frameVisible": true,
            "params": params });
        let native = probe(session, context, request, true).await?;
        assert_eq!(
            native.get("kind"),
            Some(&json!("expect")),
            "{name}: {native}"
        );
        assert_eq!(
            native.get("matches"),
            Some(&json!(false)),
            "{name}: native comparison failed"
        );
        let failure = failures
            .get(name)
            .ok_or("the SDK captured the native case")?;
        let received = native.get("received").unwrap_or(&Value::Null);
        let actual = received.get("raw").unwrap_or(received);
        assert_eq!(
            failure.get("actual"),
            Some(actual),
            "{name}: native received bytes"
        );
        assert_eq!(
            failure.get("ariaSnapshot").unwrap_or(&Value::Null),
            native.get("ariaSnapshot").unwrap_or(&Value::Null),
            "{name}: native failure tree bytes"
        );
        let resolved = native
            .get("resolved")
            .and_then(Value::as_str)
            .ok_or("native resolution preview")?;
        assert!(
            failure
                .get("log")
                .and_then(Value::as_array)
                .is_some_and(|lines| lines
                    .iter()
                    .any(|line| line.as_str().is_some_and(|line| line.contains(resolved)))),
            "{name}: the native preview is preserved: {failure}"
        );
    }
    Ok(())
}

/// Strict failures finish on their first resolution, so their full native message and log
/// have deterministic bytes. Unlike timeout failures, no retry-count normalization is needed.
async fn assert_native_strict_failures(
    session: &ProtocolSession,
    context: i64,
    failures: &Value,
) -> TestResult {
    for (name, selector, matcher, expression) in [
        (
            "strict.fields",
            ".strict-fields",
            "toHaveValue",
            "to.have.value",
        ),
        (
            "strict.alias",
            ".strict-names",
            "toHaveText",
            "to.have.text",
        ),
    ] {
        let native = probe(
            session,
            context,
            json!({
                "op": "expect", "target": target(selector), "strictness": "strict",
                "frameVisible": true, "params": { "expression": expression,
                    "isNot": false, "expectedText": [{ "string": WRONG }] },
            }),
            true,
        )
        .await?;
        assert_eq!(
            native.get("kind"),
            Some(&json!("strict")),
            "{name}: {native}"
        );
        let violation = native
            .get("message")
            .and_then(Value::as_str)
            .ok_or("the native strict failure")?;
        let call_log = format!(
            "\nCall log:\n  - Expect \"{matcher}\" locator('{selector}') with timeout 100ms\n  - waiting for locator('{selector}')\n"
        );
        let expected = format!(
            "expect(locator).{matcher}(expected) failed\n\n\
             Locator: locator('{selector}')\nExpected: {expected}\nError: {violation}{call_log}",
            expected = json!(WRONG),
        );
        assert_eq!(
            failures
                .get(name)
                .and_then(|failure| failure.get("message")),
            Some(&json!(expected)),
            "{name}: full upstream failure bytes"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn captured_conformance_preserves_native_failure_facts_and_messages() -> TestResult {
    let fixture = Fixture::open().await?;
    let checked = async {
        let result = fixture.script(CONFORMANCE, true).await?;
        let failures = result
            .get("failures")
            .ok_or("the captured conformance failures")?;
        let EngineContext { session, context } = engine_context(&fixture).await?;
        assert_native_facts(&session, context, failures).await?;
        assert_native_strict_failures(&session, context, failures).await?;
        for canary in CANARIES {
            assert!(
                result.to_string().contains(canary),
                "the native diagnostic matrix retains {canary}"
            );
        }
        let message = failures
            .get("toHaveValue")
            .and_then(|failure| failure.get("message"))
            .and_then(Value::as_str)
            .ok_or("the native value failure message")?;
        assert_eq!(
            message.split("\n\nCall log:\n").next(),
            Some(
                concat!(
                    "expect(locator).toHaveValue(expected) failed\n\n",
                    "Locator:  locator('#private-input')\n",
                    "Expected: \"Wrong expected string without a private marker\"\n",
                    "Received: \"TypedCanary-2\"\nTimeout:  100ms\n"
                )
                .trim_end_matches('\n')
            ),
            "the existing upstream failure header is byte-for-byte unchanged"
        );
        let fallback = probe(
            &session,
            context,
            json!({ "op": "document", "part": "expectSnapshot" }),
            true,
        )
        .await?;
        assert_eq!(
            failures
                .get("missing.fallback")
                .and_then(|failure| failure.get("ariaSnapshot")),
            fallback.get("text"),
            "missing-element fallback keeps native bytes"
        );
        Ok(())
    }
    .await;
    fixture.close().await;
    checked
}

/// A successful SDK read boots the real world. The raw probe is then used only to separate
/// the arming reply from trusted input, which a public click intentionally keeps atomic.
#[tokio::test]
#[ignore = "needs BrowserOS with CDP on BROWSEROS_CDP_PORT"]
async fn delayed_trusted_interception_keeps_its_arming_policy() -> TestResult {
    let fixture = Fixture::open().await?;
    let checked = async {
        fixture.script("await page.goto(__FIXTURE_URL__); return await page.locator('#private-input').inputValue();", false).await?;
        let EngineContext { session, context } = engine_context(&fixture).await?;
        let native = probe(&session, context, json!({ "op": "resolve", "target": target("#delayed-cover"), "strictness": "strict", "want": "element" }), true).await?;
        let preview = native.get("preview").and_then(Value::as_str).ok_or("the native covering field's preview")?;
        for (token, conformance) in [(91, false), (92, true)] {
            assert_delayed_verdict(&fixture, &session, context, token, conformance, preview).await?;
        }
        Ok(())
    }.await;
    fixture.close().await;
    checked
}

async fn assert_delayed_verdict(
    fixture: &Fixture,
    session: &ProtocolSession,
    context: i64,
    token: u32,
    conformance: bool,
    native_preview: &str,
) -> TestResult {
    fixture
        .run
        .eval(
            &fixture.page,
            "document.querySelector('#delayed-cover').style.display = 'none'",
        )
        .await?;
    session
        .send_value(
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": 1, "y": 1, "button": "none" }),
        )
        .await?;
    let before = probe(session, context, json!({ "op": "moves" }), conformance).await?;
    let armed = probe(
        session,
        context,
        json!({
            "op": "act", "target": target("#intercept-target"), "strictness": "strict",
            "retarget": "none", "states": [], "frameVisible": true, "scroll": "none",
            "arm": { "action": "mouse", "blockAll": false, "token": token, "expiryMs": 30_000 },
        }),
        conformance,
    )
    .await?;
    assert_eq!(armed.get("kind"), Some(&json!("armed")), "{armed}");
    // An unrelated opposite-policy read finishes before the page reveals the blocker. A
    // World-wide "last policy" would now select the wrong preview for the trusted event.
    let opposite = probe(session, context, json!({ "op": "expect", "target": target("#private-input"), "strictness": "strict", "frameVisible": true, "params": { "expression": "to.have.value", "isNot": false, "expectedText": [{ "string": WRONG }] } }), !conformance).await?;
    assert_eq!(
        opposite.get("received"),
        Some(&json!(if conformance {
            "<12 characters>"
        } else {
            "AttrCanary-1"
        })),
        "the intervening operation really used the opposite mode"
    );
    let point = armed.get("point").ok_or("the armed point")?;
    let covering = fixture
        .run
        .eval(
            &fixture.page,
            &format!(
                "(() => {{ document.querySelector('#delayed-cover').style.display = 'block'; \
                 return document.elementFromPoint({}, {}).id; }})()",
                point.get("x").ok_or("the armed x coordinate")?,
                point.get("y").ok_or("the armed y coordinate")?,
            ),
        )
        .await?;
    assert_eq!(
        covering,
        json!("delayed-cover"),
        "the delayed overlay is laid out before trusted input"
    );
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        session.send_value("Input.dispatchMouseEvent", json!({ "type": kind,
            "x": point.get("x"), "y": point.get("y"), "button": if kind == "mouseMoved" { "none" } else { "left" }, "clickCount": 1,
        })).await?;
    }
    let after = probe(session, context, json!({ "op": "moves" }), !conformance).await?;
    assert!(
        after.get("moves").and_then(Value::as_u64) > before.get("moves").and_then(Value::as_u64),
        "the probe observed trusted pointer movement: {before} -> {after}"
    );
    let verdict = probe(
        session,
        context,
        json!({ "op": "verdict", "token": token }),
        !conformance,
    )
    .await?;
    assert_eq!(verdict.get("kind"), Some(&json!("verdict")), "{verdict}");
    assert_verdict_preview(&verdict, conformance, native_preview)?;
    let restored = isolated(session, context, "({ value: document.querySelector('#private-input').value, attribute: document.querySelector('#private-input').getAttribute('value'), text: document.querySelector('#private-textarea').textContent })").await?;
    assert_eq!(
        restored,
        json!({ "value": "AttrCanary-1", "attribute": "AttrCanary-1", "text": "TextCanary-3" }),
        "native getters are restored after the delayed verdict"
    );
    Ok(())
}

fn assert_verdict_preview(verdict: &Value, conformance: bool, native_preview: &str) -> TestResult {
    let description = verdict
        .get("hit")
        .and_then(|hit| hit.get("hitTargetDescription"))
        .and_then(Value::as_str)
        .ok_or("the delayed trusted event must hit the covering private field")?;
    let expected = if conformance {
        native_preview.to_owned()
    } else {
        native_preview.replace("AttrCanary-1", "<12 characters>")
    };
    assert_eq!(
        description, expected,
        "the arm's mode owns the exact delayed preview bytes"
    );
    Ok(())
}
