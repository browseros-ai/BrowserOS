//! Web-first assertions translate facade arguments into Playwright's injected probes.
//! The host owns retries, deadlines, missing elements, and safe failure diagnostics.

use super::{PwCallOutcome, deadline, opts};
use crate::{
    framework::ToolCtx,
    tools::run::{BrowserBridge, BrowserCallValue},
};
use browseros_core::{
    PageId,
    locator::{Deadline, ExpectOutcome},
};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tokio::time::{Instant, sleep_until};

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const POLL_MS: [u64; 4] = [100, 250, 500, 1_000];

/// One immutable assertion; only the observed page state changes between probes.
pub(super) struct Assertion {
    page: PageId,
    selector: Option<String>,
    pub(super) expression: &'static str,
    pub(super) options: Value,
    is_not: bool,
    page_field: Option<&'static str>,
}

pub(crate) async fn dispatch(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
) -> Result<PwCallOutcome, String> {
    let assertion = Assertion::parse(method, args)?;
    let dl = deadline(bridge, &opts(args, 3), DEFAULT_TIMEOUT_MS);
    let value = poll(&assertion, &dl, || assertion.probe(&bridge.ctx, &dl)).await?;
    Ok(PwCallOutcome {
        value: BrowserCallValue::Json(value),
        audit_args: None,
        created_page: None,
        secrets: Vec::new(),
    })
}

impl Assertion {
    pub(super) fn parse(method: &str, args: &[Value]) -> Result<Self, String> {
        let page = args
            .first()
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| format!("{method}: expected a pageId"))?;
        let expected = args.get(2).unwrap_or(&Value::Null);
        let opts = args.get(3).unwrap_or(&Value::Null);
        let is_not = opts.get("isNot").and_then(Value::as_bool).unwrap_or(false);
        let mut options = json!({ "isNot": is_not });
        let mut text = None;
        let mut normalize = false;
        let mut substring = false;
        let mut page_field = None;
        let expression = match method {
            "expect.toBeVisible" => {
                state_expression(opts, "visible", "to.be.visible", "to.be.hidden")
            }
            "expect.toBeHidden" => "to.be.hidden",
            "expect.toBeAttached" => {
                state_expression(opts, "attached", "to.be.attached", "to.be.detached")
            }
            "expect.toBeEnabled" => {
                state_expression(opts, "enabled", "to.be.enabled", "to.be.disabled")
            }
            "expect.toBeDisabled" => "to.be.disabled",
            "expect.toBeEditable" => {
                state_expression(opts, "editable", "to.be.editable", "to.be.readonly")
            }
            "expect.toBeEmpty" => "to.be.empty",
            "expect.toBeFocused" => "to.be.focused",
            "expect.toBeChecked" => {
                let mut value = json!({});
                for key in ["checked", "indeterminate"] {
                    if let Some(flag) = opts.get(key).and_then(Value::as_bool) {
                        value[key] = json!(flag);
                    }
                }
                options["expectedValue"] = value;
                "to.be.checked"
            }
            "expect.toHaveCount" => {
                let count = expected
                    .as_u64()
                    .ok_or("toHaveCount: expected a non-negative integer")?;
                options["expectedNumber"] = json!(count);
                "to.have.count"
            }
            "expect.toHaveText" | "expect.toContainText" => {
                text = Some(expected);
                normalize = true;
                substring = method == "expect.toContainText";
                if let Some(inner) = opts.get("useInnerText").and_then(Value::as_bool) {
                    options["useInnerText"] = json!(inner);
                }
                match (substring, expected.is_array()) {
                    (true, true) => "to.contain.text.array",
                    (false, true) => "to.have.text.array",
                    // There is no scalar to.contain.text in the injected engine.
                    (_, false) => "to.have.text",
                }
            }
            "expect.toHaveAttribute" | "expect.toHaveCSS" => {
                let (name, value) = named_expected(expected, opts)?;
                options["expressionArg"] = json!(name);
                text = value;
                if method == "expect.toHaveCSS" {
                    if value.is_none() {
                        return Err("toHaveCSS: expected a value".into());
                    }
                    "to.have.css"
                } else if value.is_some() {
                    "to.have.attribute.value"
                } else {
                    "to.have.attribute"
                }
            }
            "expect.toHaveClass" => {
                text = Some(expected);
                if expected.is_array() {
                    "to.have.class.array"
                } else {
                    "to.have.class"
                }
            }
            "expect.toHaveValue" => {
                text = Some(expected);
                "to.have.value"
            }
            "expect.toHaveId" => {
                text = Some(expected);
                "to.have.id"
            }
            "expect.toHaveRole" => {
                if !expected.is_string() {
                    return Err("toHaveRole: expected a string".into());
                }
                text = Some(expected);
                "to.have.role"
            }
            "expect.toHaveAccessibleName" => {
                text = Some(expected);
                normalize = true;
                "to.have.accessible.name"
            }
            "expect.toHaveTitle" => {
                text = Some(expected);
                normalize = true;
                page_field = Some("title");
                "to.have.title"
            }
            "expect.toHaveURL" => {
                text = Some(expected);
                page_field = Some("url");
                "to.have.url"
            }
            _ => return Err(format!("not available in BrowserOS neo: {method}")),
        };
        if let Some(text) = text {
            let entries = match text {
                Value::Array(values) if expression.ends_with(".array") => values.as_slice(),
                _ => std::slice::from_ref(text),
            };
            options["expectedText"] = Value::Array(
                entries
                    .iter()
                    .map(|entry| expected_text(entry, opts, normalize, substring))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let selector = args.get(1).and_then(Value::as_str).map(str::to_owned);
        if page_field.is_none() && selector.as_deref().is_none_or(str::is_empty) {
            return Err(format!("{method}: expected a locator selector"));
        }
        Ok(Self {
            page: PageId(page),
            selector,
            expression,
            options,
            is_not,
            page_field,
        })
    }

    pub(super) async fn probe(
        &self,
        ctx: &ToolCtx,
        dl: &Deadline,
    ) -> Result<ExpectOutcome, String> {
        if let Some(field) = self.page_field {
            // page.info is backed by refresh, not the PageManager cache: navigation
            // and document.title mutations must be observable on every retry.
            let info = ctx
                .session
                .pages
                .refresh(self.page.clone())
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| format!("Unknown page {}", self.page))?;
            let received = if field == "title" {
                info.title
            } else {
                info.url
            };
            let matches = page_text_matches(&self.options["expectedText"][0], &received)?;
            return Ok(outcome(matches, Some(json!(received))));
        }
        let engine = ctx.session.locator();
        let selector = self
            .selector
            .as_deref()
            .ok_or("expected a locator selector")?;
        let count = engine
            .count(self.page.clone(), selector)
            .await
            .map_err(|err| err.to_string())?;
        if count == 0 {
            return Ok(self.missing());
        }
        let mut result = engine
            .expect(
                self.page.clone(),
                Some(selector),
                self.expression,
                self.options.clone(),
                dl,
            )
            .await
            .map_err(|err| err.to_string())?;
        // P1 returns received.value. Defensively strip the wire wrapper too:
        // ariaSnapshot may include a password from an entirely different element.
        result.received = result.received.and_then(received_value);
        result.log.clear();
        if self.options.get("expectedText").is_some() {
            // Describe the same selection before exposing its text. A failed
            // sensitivity probe masks rather than leaking a value into diagnostics.
            let sensitive = match engine.resolve_all(self.page.clone(), selector).await {
                Ok(elements) if !elements.is_empty() => {
                    let mut sensitive = false;
                    for element in elements {
                        sensitive |= match engine.describe(&element).await {
                            Ok(description) => is_sensitive(&description),
                            Err(_) => true,
                        };
                        // resolve_all gives us remote handles. The description
                        // is copied, so this probe no longer owns a use for them.
                        let _ = element
                            .session
                            .send_value(
                                "Runtime.releaseObject",
                                json!({ "objectId": element.object_id }),
                            )
                            .await;
                    }
                    sensitive
                }
                _ => true,
            };
            if sensitive && result.received.is_some() {
                result.received = Some(json!("[redacted]"));
            }
        }
        Ok(result)
    }

    pub(super) fn missing(&self) -> ExpectOutcome {
        let matches = match self.expression {
            "to.have.count" => self.options["expectedNumber"] == json!(0),
            "to.be.hidden" | "to.be.detached" if !self.is_not => true,
            "to.be.visible" | "to.be.attached" if self.is_not => false,
            // Most negated assertions still require an element. Returning the
            // negation bit keeps matches != isNot false until one exists.
            _ => self.is_not,
        };
        let received = if self.expression == "to.have.count" {
            Some(json!(0))
        } else {
            None
        };
        outcome(matches, received)
    }
}

/// Injected expect is a single probe. Retrying here re-resolves the selector
/// after DOM changes and keeps sleeps and in-flight CDP work inside one budget.
pub(super) async fn poll<F, Fut>(
    assertion: &Assertion,
    dl: &Deadline,
    mut probe: F,
) -> Result<Value, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<ExpectOutcome, String>>,
{
    let target = assertion.selector.as_deref().unwrap_or("page.info");
    let mut log = vec![format!("waiting for {target} ({})", assertion.expression)];
    let mut last = outcome(assertion.is_not, None);
    let mut attempt = 0;
    loop {
        tokio::select! {
            biased;
            () = dl.cancel.cancelled() => return Err("cancelled".into()),
            () = sleep_until(dl.at) => break,
            result = probe() => last = result?,
        }
        attempt += 1;
        if last.matches != assertion.is_not && !last.timed_out {
            log.push(format!("assertion satisfied after {attempt} probe(s)"));
            return Ok(json!({ "matches": last.matches, "received": last.received, "log": log }));
        }
        if last.timed_out {
            break;
        }
        let interval = POLL_MS[(attempt - 1).min(POLL_MS.len() - 1)];
        tokio::select! {
            biased;
            () = dl.cancel.cancelled() => return Err("cancelled".into()),
            () = sleep_until((Instant::now() + Duration::from_millis(interval)).min(dl.at)) => {}
        }
    }
    log.push(format!("assertion timed out after {attempt} probe(s)"));
    Ok(json!({ "matches": last.matches, "received": last.received, "log": log, "timedOut": true }))
}

fn outcome(matches: bool, received: Option<Value>) -> ExpectOutcome {
    ExpectOutcome {
        matches,
        received,
        timed_out: false,
        log: Vec::new(),
    }
}

fn state_expression(opts: &Value, key: &str, yes: &'static str, no: &'static str) -> &'static str {
    if opts.get(key) == Some(&Value::Bool(false)) {
        no
    } else {
        yes
    }
}

fn named_expected<'a>(
    expected: &'a Value,
    opts: &'a Value,
) -> Result<(&'a str, Option<&'a Value>), String> {
    if let Some(name) = opts
        .get("expressionArg")
        .or_else(|| opts.get("name"))
        .and_then(Value::as_str)
    {
        return Ok((name, (!expected.is_null()).then_some(expected)));
    }
    if let Some(name) = expected.get("name").and_then(Value::as_str) {
        return Ok((name, expected.get("value")));
    }
    if let Some(values) = expected.as_array() {
        let name = values
            .first()
            .and_then(Value::as_str)
            .ok_or("expected an attribute or CSS name")?;
        return Ok((name, values.get(1)));
    }
    expected
        .as_str()
        .map(|name| (name, None))
        .ok_or_else(|| "expected an attribute or CSS name".into())
}

fn expected_text(
    expected: &Value,
    opts: &Value,
    normalize: bool,
    substring: bool,
) -> Result<Value, String> {
    let mut entry = json!({ "matchSubstring": substring, "normalizeWhiteSpace": normalize });
    if let Some(text) = expected.as_str() {
        entry["string"] = json!(text);
    } else {
        // RegExp objects cannot cross JSON. The facade transports source + flags;
        // the injected script reconstructs them, preserving JS regex semantics.
        let source = expected
            .get("regexSource")
            .or_else(|| expected.get("source"))
            .and_then(Value::as_str)
            .ok_or("expected a string or serialized regular expression")?;
        let flags = expected
            .get("regexFlags")
            .or_else(|| expected.get("flags"))
            .and_then(Value::as_str)
            .unwrap_or("");
        entry["regexSource"] = json!(source);
        entry["regexFlags"] = json!(flags);
    }
    if let Some(ignore) = opts.get("ignoreCase").and_then(Value::as_bool) {
        entry["ignoreCase"] = json!(ignore);
    }
    Ok(entry)
}

pub(super) fn received_value(value: Value) -> Option<Value> {
    match value {
        Value::Object(mut object) => object.remove("value"),
        value => Some(value),
    }
}

pub(super) fn is_sensitive(description: &Value) -> bool {
    if description
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
    {
        return true;
    }
    description
        .get("autocomplete")
        .and_then(Value::as_str)
        .is_some_and(|autocomplete| {
            autocomplete.split_ascii_whitespace().any(|token| {
                [
                    "current-password",
                    "new-password",
                    "one-time-code",
                    "cc-number",
                    "cc-csc",
                    "cc-exp",
                ]
                .iter()
                .any(|secret| token.eq_ignore_ascii_case(secret))
            })
        })
}

pub(super) fn page_text_matches(expected: &Value, received: &str) -> Result<bool, String> {
    // Page metadata stays in the host. Use its existing QuickJS dependency for
    // RegExp (lookarounds, flags, etc.), rather than translating to Rust regexes.
    // This context has no host functions, and its lifetime ends before any await.
    let runtime = rquickjs::Runtime::new().map_err(|err| err.to_string())?;
    runtime.set_memory_limit(4 * 1024 * 1024);
    let cutoff = std::time::Instant::now() + Duration::from_millis(50);
    runtime.set_interrupt_handler(Some(Box::new(move || std::time::Instant::now() >= cutoff)));
    let context = rquickjs::Context::full(&runtime).map_err(|err| err.to_string())?;
    let source = format!(
        r#"((e, received) => {{
        if (e.regexSource !== undefined) {{
            const flags = new Set(e.regexFlags || '');
            if (e.ignoreCase === false) flags.delete('i');
            if (e.ignoreCase === true) flags.add('i');
            return new RegExp(e.regexSource, [...flags].join('')).test(received);
        }}
        const normalize = s => {{
            if (e.normalizeWhiteSpace) s = s.replace(/[\u200b\u00ad]/g, '').trim().replace(/\s+/g, ' ');
            return e.ignoreCase ? s.toLowerCase() : s;
        }};
        return normalize(e.string) === normalize(received);
    }})({expected}, {})"#,
        json!(received)
    );
    context
        .with(|ctx| ctx.eval::<bool, _>(source))
        .map_err(|err| format!("page assertion pattern: {err}"))
}
