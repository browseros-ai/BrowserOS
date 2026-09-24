//! Host assertion contracts are tested at the single-probe seam so locator-engine
//! bootstrap details do not couple P6's retry tests to P1's CDP implementation.

use super::expect::{self, Assertion};
use crate::{
    framework::{BrowserToolDefaults, BrowserToolOptions, ToolCtx},
    output_file::create_browser_output_file_access,
    tools::run::{ScriptSpec, execute_script},
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, CdpConnection, ProtocolSession, SessionId,
    locator::{Deadline, ExpectOutcome},
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::broadcast, time::Instant};
use tokio_util::sync::CancellationToken;

struct ExpectConnection {
    sender: broadcast::Sender<CdpEvent>,
    state: Mutex<FakeState>,
}

struct FakeState {
    answers: VecDeque<Value>,
    calls: Vec<(String, Value, Instant)>,
    urls: VecDeque<String>,
}

impl ExpectConnection {
    fn new(answers: Vec<Value>) -> Arc<Self> {
        let (sender, _) = broadcast::channel(8);
        Arc::new(Self {
            sender,
            state: Mutex::new(FakeState {
                answers: answers.into(),
                calls: Vec::new(),
                urls: VecDeque::from([
                    "https://example.test/old".into(),
                    "https://example.test/new".into(),
                ]),
            }),
        })
    }

    fn calls(&self) -> anyhow::Result<Vec<(String, Value, Instant)>> {
        Ok(self
            .state
            .lock()
            .map_err(|err| anyhow::anyhow!(err.to_string()))?
            .calls
            .clone())
    }
}

impl CdpConnection for ExpectConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|err| CdpError::Protocol {
                code: -1,
                message: err.to_string(),
            })?;
            state
                .calls
                .push((method.to_owned(), params, Instant::now()));
            match method {
                "Runtime.callFunctionOn" => {
                    let answer = if state.answers.len() > 1 {
                        state.answers.pop_front()
                    } else {
                        state.answers.front().cloned()
                    }
                    .unwrap_or_else(|| json!({"matches": false, "received": "hidden"}));
                    Ok(json!({ "result": { "value": answer } }))
                }
                "Browser.getTabs" | "Browser.getTabInfo" => {
                    let url = if state.urls.len() > 1 {
                        state.urls.pop_front()
                    } else {
                        state.urls.front().cloned()
                    };
                    let tab = json!({
                        "tabId": 7, "targetId": "target-7", "url": url,
                        "title": " Example  title ", "isActive": false, "isLoading": false,
                        "loadProgress": 1.0, "isPinned": false, "isHidden": false, "windowId": 1, "index": 0
                    });
                    Ok(if method == "Browser.getTabs" {
                        json!({"tabs": [tab]})
                    } else {
                        json!({"tab": tab})
                    })
                }
                _ => Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected CDP call: {method}"),
                }),
            }
        })
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params).map_err(|err| CdpError::Protocol {
                code: -1,
                message: err.to_string(),
            })?;
            self.send(method, params, session)
                .await
                .map(|value| value.to_string())
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.sender.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn assertion(method: &str, expected: Value, opts: Value) -> anyhow::Result<Assertion> {
    Assertion::parse(method, &[json!(1), json!("css=#target"), expected, opts])
        .map_err(anyhow::Error::msg)
}

fn deadline(ms: u64) -> Deadline {
    Deadline {
        at: Instant::now() + Duration::from_millis(ms),
        cancel: CancellationToken::new(),
    }
}

// This adapter represents P1's expect contract. It sends the prepared payload
// over a fake CDP connection and returns one observation, never performs retries.
async fn injected_probe(
    assertion: &Assertion,
    connection: Arc<ExpectConnection>,
) -> Result<ExpectOutcome, String> {
    let mut options = assertion.options.clone();
    options["expression"] = json!(assertion.expression);
    let response = ProtocolSession::root(connection).send_value("Runtime.callFunctionOn", json!({
        "functionDeclaration": "function(options) { return this.expect(null, options, []); }",
        "arguments": [{"value": options}], "returnByValue": true, "awaitPromise": true
    })).await.map_err(|err| err.to_string())?;
    let result = &response["result"]["value"];
    Ok(ExpectOutcome {
        matches: result["matches"].as_bool().ok_or("missing matches")?,
        received: result
            .get("received")
            .cloned()
            .and_then(expect::received_value),
        log: Vec::new(),
        timed_out: false,
    })
}

fn context(connection: Arc<ExpectConnection>) -> ToolCtx {
    ToolCtx::new(BrowserToolOptions {
        session: BrowserSession::new(connection, BrowserSessionHooks::default()),
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    })
}

#[tokio::test(start_paused = true)]
async fn visible_resolves_on_second_probe() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(vec![
        json!({"matches": false, "received": "hidden"}),
        json!({"matches": true}),
    ]);
    let assertion = assertion("expect.toBeVisible", Value::Null, json!({}))?;
    let value = expect::poll(&assertion, &deadline(5_000), || {
        injected_probe(&assertion, connection.clone())
    })
    .await
    .map_err(anyhow::Error::msg)?;
    assert_eq!(value["matches"], true);
    assert!(value.get("timedOut").is_none());
    let calls = connection.calls()?;
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].2.duration_since(calls[0].2),
        Duration::from_millis(100)
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn not_visible_resolves_on_false_without_inverting_wire_matches() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(vec![json!({"matches": false, "received": "hidden"})]);
    let assertion = assertion("expect.toBeVisible", Value::Null, json!({"isNot": true}))?;
    let value = expect::poll(&assertion, &deadline(5_000), || {
        injected_probe(&assertion, connection.clone())
    })
    .await
    .map_err(anyhow::Error::msg)?;
    assert_eq!(value["matches"], false);
    assert!(value.get("timedOut").is_none());
    assert_eq!(connection.calls()?.len(), 1);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn timeout_keeps_last_received_and_uses_retry_schedule() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(vec![json!({"matches": false, "received": "hidden"})]);
    let assertion = assertion("expect.toBeVisible", Value::Null, json!({}))?;
    let start = Instant::now();
    let value = expect::poll(&assertion, &deadline(3_000), || {
        injected_probe(&assertion, connection.clone())
    })
    .await
    .map_err(anyhow::Error::msg)?;
    assert_eq!(value["matches"], false);
    assert_eq!(value["timedOut"], true);
    assert_eq!(value["received"], "hidden");
    let times: Vec<_> = connection
        .calls()?
        .iter()
        .map(|call| call.2.duration_since(start).as_millis())
        .collect();
    assert_eq!(times, [0, 100, 350, 850, 1850, 2850]);
    assert_eq!(
        Instant::now().duration_since(start),
        Duration::from_millis(3_000)
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn regex_payload_preserves_source_flags_and_text_options() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(vec![json!({"matches": true})]);
    let assertion = assertion(
        "expect.toHaveText",
        json!({"regexSource": "(?<=hello )world", "regexFlags": "im"}),
        json!({"ignoreCase": false, "useInnerText": true}),
    )?;
    expect::poll(&assertion, &deadline(5_000), || {
        injected_probe(&assertion, connection.clone())
    })
    .await
    .map_err(anyhow::Error::msg)?;
    let calls = connection.calls()?;
    let payload = &calls[0].1["arguments"][0]["value"];
    assert_eq!(payload["expression"], "to.have.text");
    assert_eq!(payload["useInnerText"], true);
    assert_eq!(
        payload["expectedText"],
        json!([{
            "regexSource": "(?<=hello )world", "regexFlags": "im", "normalizeWhiteSpace": true,
            "matchSubstring": false, "ignoreCase": false
        }])
    );
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn url_polls_fresh_page_info() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(Vec::new());
    let ctx = context(connection.clone());
    let assertion = Assertion::parse(
        "expect.toHaveURL",
        &[
            json!(1),
            Value::Null,
            json!("https://example.test/new"),
            json!({}),
        ],
    )
    .map_err(anyhow::Error::msg)?;
    let dl = deadline(5_000);
    let value = expect::poll(&assertion, &dl, || assertion.probe(&ctx, &dl))
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(value["matches"], true);
    assert_eq!(value["received"], "https://example.test/new");
    let methods: Vec<_> = connection
        .calls()?
        .iter()
        .map(|call| call.0.clone())
        .collect();
    assert_eq!(methods, ["Browser.getTabs", "Browser.getTabInfo"]);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn stalled_probe_is_bounded_and_cancellation_interrupts_sleep() -> anyhow::Result<()> {
    let assertion = assertion("expect.toBeVisible", Value::Null, json!({}))?;
    let value = expect::poll(&assertion, &deadline(100), || {
        futures_util::future::pending()
    })
    .await
    .map_err(anyhow::Error::msg)?;
    assert_eq!(value["timedOut"], true);
    let dl = deadline(5_000);
    let cancel = dl.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
    });
    let connection = ExpectConnection::new(vec![json!({"matches": false})]);
    let result = expect::poll(&assertion, &dl, || {
        injected_probe(&assertion, connection.clone())
    })
    .await;
    assert_eq!(result, Err("cancelled".into()));
    assert_eq!(connection.calls()?.len(), 1);
    Ok(())
}

#[test]
fn all_matchers_and_payload_variants_follow_injected_protocol() -> anyhow::Result<()> {
    for (method, expression, expected, options) in [
        ("toBeVisible", "to.be.visible", Value::Null, json!({})),
        ("toBeHidden", "to.be.hidden", Value::Null, json!({})),
        ("toBeAttached", "to.be.attached", Value::Null, json!({})),
        ("toBeEnabled", "to.be.enabled", Value::Null, json!({})),
        ("toBeDisabled", "to.be.disabled", Value::Null, json!({})),
        (
            "toBeChecked",
            "to.be.checked",
            Value::Null,
            json!({"checked": false}),
        ),
        ("toBeEditable", "to.be.editable", Value::Null, json!({})),
        ("toBeEmpty", "to.be.empty", Value::Null, json!({})),
        ("toBeFocused", "to.be.focused", Value::Null, json!({})),
        ("toHaveText", "to.have.text", json!("text"), json!({})),
        (
            "toHaveText",
            "to.have.text.array",
            json!(["text"]),
            json!({}),
        ),
        ("toContainText", "to.have.text", json!("text"), json!({})),
        (
            "toContainText",
            "to.contain.text.array",
            json!(["text"]),
            json!({}),
        ),
        ("toHaveValue", "to.have.value", json!("value"), json!({})),
        ("toHaveCount", "to.have.count", json!(3), json!({})),
        (
            "toHaveAttribute",
            "to.have.attribute",
            Value::Null,
            json!({"name": "role"}),
        ),
        (
            "toHaveAttribute",
            "to.have.attribute.value",
            json!("button"),
            json!({"name": "role"}),
        ),
        ("toHaveClass", "to.have.class", json!("active"), json!({})),
        (
            "toHaveClass",
            "to.have.class.array",
            json!(["active"]),
            json!({}),
        ),
        ("toHaveId", "to.have.id", json!("target"), json!({})),
        (
            "toHaveCSS",
            "to.have.css",
            json!("block"),
            json!({"name": "display"}),
        ),
        (
            "toHaveAccessibleName",
            "to.have.accessible.name",
            json!("Submit"),
            json!({}),
        ),
        ("toHaveRole", "to.have.role", json!("button"), json!({})),
        ("toHaveTitle", "to.have.title", json!("Title"), json!({})),
        (
            "toHaveURL",
            "to.have.url",
            json!("https://example.test"),
            json!({}),
        ),
        (
            "toBeVisible",
            "to.be.hidden",
            Value::Null,
            json!({"visible": false}),
        ),
        (
            "toBeAttached",
            "to.be.detached",
            Value::Null,
            json!({"attached": false}),
        ),
        (
            "toBeEditable",
            "to.be.readonly",
            Value::Null,
            json!({"editable": false}),
        ),
        (
            "toBeEnabled",
            "to.be.disabled",
            Value::Null,
            json!({"enabled": false}),
        ),
    ] {
        let assertion = assertion(&format!("expect.{method}"), expected, options)?;
        assert_eq!(assertion.expression, expression, "{method}");
    }
    let count = assertion("expect.toHaveCount", json!(3), json!({}))?;
    assert_eq!(count.options["expectedNumber"], 3);
    let checked = assertion("expect.toBeChecked", Value::Null, json!({"checked": false}))?;
    assert_eq!(checked.options["expectedValue"], json!({"checked": false}));
    let attr = assertion(
        "expect.toHaveAttribute",
        json!({"name": "role", "value": "button"}),
        json!({}),
    )?;
    assert_eq!(attr.options["expressionArg"], "role");
    assert_eq!(attr.options["expectedText"][0]["string"], "button");
    let contains = assertion("expect.toContainText", json!("text"), json!({}))?;
    assert_eq!(contains.options["expectedText"][0]["matchSubstring"], true);
    Ok(())
}

#[test]
fn missing_element_rules_do_not_pass_arbitrary_negated_matchers() -> anyhow::Result<()> {
    for (method, expected, is_not, passes) in [
        ("toBeHidden", Value::Null, false, true),
        ("toBeVisible", Value::Null, true, true),
        ("toBeAttached", Value::Null, true, true),
        ("toHaveCount", json!(0), false, true),
        ("toHaveCount", json!(1), false, false),
        ("toBeVisible", Value::Null, false, false),
        ("toHaveText", json!("hello"), true, false),
        ("toBeEnabled", Value::Null, true, false),
    ] {
        let assertion = assertion(
            &format!("expect.{method}"),
            expected,
            json!({"isNot": is_not}),
        )?;
        let outcome = assertion.missing();
        assert_eq!(
            outcome.matches != is_not,
            passes,
            "{method}, isNot={is_not}"
        );
        if method != "toHaveCount" {
            assert_eq!(outcome.received, None);
        }
    }
    Ok(())
}

#[test]
fn sensitive_values_and_aria_snapshots_are_not_diagnostics() {
    assert_eq!(
        expect::received_value(json!({"value": "hidden", "ariaSnapshot": "password secret"})),
        Some(json!("hidden"))
    );
    assert_eq!(
        expect::received_value(json!({"ariaSnapshot": "password secret"})),
        None
    );
    for description in [
        json!({"type": "PASSWORD"}),
        json!({"autocomplete": "section-login one-time-code"}),
        json!({"autocomplete": "cc-number"}),
    ] {
        assert!(expect::is_sensitive(&description));
    }
    assert!(!expect::is_sensitive(
        &json!({"type": "text", "autocomplete": "email"})
    ));
}

#[test]
fn page_matchers_preserve_js_regex_and_whitespace_semantics() -> anyhow::Result<()> {
    assert!(
        expect::page_text_matches(
            &json!({"regexSource": "(?<=example[.])test", "regexFlags": "i"}),
            "https://EXAMPLE.TEST"
        )
        .map_err(anyhow::Error::msg)?
    );
    assert!(
        !expect::page_text_matches(
            &json!({"regexSource": "example", "regexFlags": "i", "ignoreCase": false}),
            "EXAMPLE"
        )
        .map_err(anyhow::Error::msg)?
    );
    assert!(
        expect::page_text_matches(
            &json!({"string": "Example title", "normalizeWhiteSpace": true}),
            " Example\n\u{200b} title "
        )
        .map_err(anyhow::Error::msg)?
    );
    assert!(
        !expect::page_text_matches(
            &json!({"string": "https://example.test", "normalizeWhiteSpace": false}),
            " https://example.test "
        )
        .map_err(anyhow::Error::msg)?
    );
    Ok(())
}

async fn script(
    connection: Arc<ExpectConnection>,
    timeout: u64,
    code: &str,
) -> anyhow::Result<Value> {
    let result = execute_script(
        ScriptSpec {
            bootstrap_js: super::FACADE_JS,
            code: code.to_string(),
            timeout_ms: timeout,
            helpers: false,
        },
        &context(connection),
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err:?}"))?
    .into_tool_result();
    result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("missing structured result"))
}

#[tokio::test]
async fn dispatch_returns_timeout_shape_through_shared_bridge() -> anyhow::Result<()> {
    let result = script(ExpectConnection::new(Vec::new()), 1_000,
        r#"return await __browserosCall('expect.toHaveURL', JSON.stringify([1, null, 'https://never.test', {timeout: 30}]), false);"#).await?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["timedOut"], true);
    assert_eq!(result["value"]["received"], "https://example.test/old");
    Ok(())
}

#[tokio::test]
async fn sixty_second_assertion_is_clamped_to_remaining_run_budget() -> anyhow::Result<()> {
    let connection = ExpectConnection::new(Vec::new());
    let started = Instant::now();
    let result = script(connection.clone(), 150,
        r#"await sleep(50); return await __browserosCall('expect.toHaveURL', JSON.stringify([1, null, 'https://never.test', {timeout: 60000}]), false);"#).await?;
    assert!(started.elapsed() < Duration::from_millis(600), "{result}");
    assert!(started.elapsed() >= Duration::from_millis(140), "{result}");
    // At the exact run deadline the parent runtime can win the race. Either
    // timeout envelope is valid; neither may leave a detached polling task.
    assert!(
        result["value"]["timedOut"] == true || result["error"] == "playwright exceeded 150ms",
        "{result}"
    );
    let calls = connection.calls()?.len();
    assert!(calls > 0);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(connection.calls()?.len(), calls);
    Ok(())
}
