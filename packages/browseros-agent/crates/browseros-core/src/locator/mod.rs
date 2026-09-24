//! Playwright owns selector, state, and matcher semantics in an isolated world. This
//! module supplies the host half: frame routing, bounded polling, handles and diagnostics.

mod frames;
#[cfg(test)]
mod tests;
mod world;

use crate::{
    CoreError, FrameId, PageId, ProtocolSession, frames::FrameRegistry, pages::PageManager,
};
use frames::{Scope, root_frame};
use serde_json::{Value, json};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use world::{Worlds, object_id, release, runtime_result};

/// One engine per BrowserSession, sharing its transport and frame registry.
pub struct LocatorEngine {
    pages: Arc<PageManager>,
    frames: Arc<FrameRegistry>,
    worlds: Worlds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementState {
    Visible,
    Hidden,
    Enabled,
    Disabled,
    Editable,
    Checked,
    Unchecked,
    Stable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strictness {
    Strict,
    First,
}

impl ElementState {
    fn name(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Editable => "editable",
            Self::Checked => "checked",
            Self::Unchecked => "unchecked",
            Self::Stable => "stable",
        }
    }
}

/// A resolved node retains its frame session so input can address OOPIFs. The object
/// belongs to the isolated document and becomes invalid on navigation; do not persist it.
pub struct Resolved {
    pub session: ProtocolSession,
    pub backend_node_id: i64,
    pub object_id: String,
    pub frame_id: Option<FrameId>,
}

/// Shared wall-clock and cancellation budget, including CDP round trips and RAF waits.
pub struct Deadline {
    pub at: Instant,
    pub cancel: CancellationToken,
}

/// `matches` is the underlying matcher result: assertion success is `matches != isNot`.
/// Received contains only the matcher value, with secret-input values masked.
pub struct ExpectOutcome {
    pub matches: bool,
    pub received: Option<Value>,
    pub timed_out: bool,
    pub log: Vec<String>,
}

const POLL_MS: &[u64] = &[0, 20, 50, 100, 100, 500];

impl Deadline {
    async fn run<T>(
        &self,
        future: impl Future<Output = Result<T, CoreError>>,
    ) -> Result<T, CoreError> {
        // Dropping a CDP future stops waiting, not JavaScript already running in the
        // renderer. Calls here are reads; a late result cannot dispatch trusted input.
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(CoreError::from("Locator operation cancelled")),
            () = tokio::time::sleep_until(self.at) => Err(timeout()),
            result = future => result,
        }
    }

    async fn pause(&self, attempt: usize) -> Result<(), CoreError> {
        let ms = POLL_MS[attempt.min(POLL_MS.len() - 1)];
        self.run(async {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok(())
        })
        .await
    }
}

fn timeout() -> CoreError {
    CoreError::from("TimeoutError: locator deadline exceeded")
}
fn with_log(error: CoreError, log: &[String]) -> CoreError {
    CoreError::Message(format!(
        "{error}\nCall log:\n{}",
        log.iter()
            .map(|line| format!("  - {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}
fn log_line(log: &mut Vec<String>, line: String) {
    if log.last() != Some(&line) {
        log.push(line);
    }
}

impl LocatorEngine {
    #[must_use]
    pub fn new(pages: Arc<PageManager>, frames: Arc<FrameRegistry>) -> Self {
        Self {
            pages,
            frames,
            worlds: Worlds::default(),
        }
    }

    /// Polls [0,20,50,100,100,500] ms; strictness applies before checking the wait state.
    pub async fn resolve(
        &self,
        page: PageId,
        selector: &str,
        strict: Strictness,
        wait: Option<ElementState>,
        dl: &Deadline,
    ) -> Result<Resolved, CoreError> {
        let log = vec![format!(
            "waiting for {selector}{}",
            wait.map(|s| format!(" to be {}", s.name()))
                .unwrap_or_default()
        )];
        let result = async {
            let mut attempt = 0;
            loop {
                dl.pause(attempt).await?;
                attempt += 1;
                let result = dl
                    .run(self.resolve_once(page.clone(), selector, strict, wait))
                    .await?;
                if let Some(result) = result {
                    return Ok(result);
                }
            }
        }
        .await;
        result.map_err(|error| with_log(error, &log))
    }

    async fn resolve_once(
        &self,
        page: PageId,
        selector: &str,
        strict: Strictness,
        wait: Option<ElementState>,
    ) -> Result<Option<Resolved>, CoreError> {
        let Some(scope) = self.scope(page, selector).await? else {
            return Ok(None);
        };
        let remote = self
            .worlds
            .call(
                &scope.session,
                &scope.frame,
                r#"async function(parsed, strict, state) {
            const el = this.querySelector(parsed, document, strict);
            if (!el) return null;
            if (state && await this.checkElementStates(el, [state])) return null;
            return el;
        }"#,
                vec![
                    scope.parsed.clone(),
                    json!(strict == Strictness::Strict),
                    json!(wait.map(ElementState::name)),
                ],
                false,
            )
            .await?;
        if remote.get("objectId").is_none() {
            return Ok(None);
        }
        self.resolved(&scope, object_id(&remote)?).await.map(Some)
    }

    async fn resolved(&self, scope: &Scope, object: String) -> Result<Resolved, CoreError> {
        let described = scope
            .session
            .send_value("DOM.describeNode", json!({"objectId": object, "depth": 0}))
            .await;
        let backend = described.and_then(|d| {
            d["node"]["backendNodeId"]
                .as_i64()
                .ok_or_else(|| CoreError::from("Resolved element has no backendNodeId"))
        });
        match backend {
            Ok(backend_node_id) => Ok(Resolved {
                session: scope.session.clone(),
                backend_node_id,
                object_id: object,
                frame_id: Some(scope.frame.clone()),
            }),
            Err(error) => {
                release(&scope.session, &object).await;
                Err(error)
            }
        }
    }

    pub async fn resolve_all(
        &self,
        page: PageId,
        selector: &str,
    ) -> Result<Vec<Resolved>, CoreError> {
        self.resolve_all_inner(page, selector)
            .await
            .map_err(|e| with_log(e, &[format!("waiting for {selector}")]))
    }

    async fn resolve_all_inner(
        &self,
        page: PageId,
        selector: &str,
    ) -> Result<Vec<Resolved>, CoreError> {
        let Some(scope) = self.scope(page, selector).await? else {
            return Ok(Vec::new());
        };
        let remote = self
            .worlds
            .call(
                &scope.session,
                &scope.frame,
                "function(parsed) { return this.querySelectorAll(parsed, document); }",
                vec![scope.parsed.clone()],
                false,
            )
            .await?;
        let array = object_id(&remote)?;
        let properties = scope
            .session
            .send_value(
                "Runtime.getProperties",
                json!({"objectId": array, "ownProperties": true}),
            )
            .await;
        release(&scope.session, &array).await;
        let properties = properties?;
        let items = properties["result"]
            .as_array()
            .ok_or_else(|| CoreError::from("Element array has no properties"))?;
        let mut objects = items
            .iter()
            .filter_map(|item| {
                let index = item["name"].as_str()?.parse::<usize>().ok()?;
                Some((index, item["value"]["objectId"].as_str()?.to_owned()))
            })
            .collect::<Vec<_>>();
        objects.sort_by_key(|(index, _)| *index);
        let mut resolved = Vec::new();
        for (index, (_, object)) in objects.iter().enumerate() {
            match self.resolved(&scope, object.clone()).await {
                Ok(node) => resolved.push(node),
                Err(error) => {
                    for (_, remaining) in &objects[index + 1..] {
                        release(&scope.session, remaining).await;
                    }
                    for node in &resolved {
                        release(&node.session, &node.object_id).await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(resolved)
    }

    pub async fn count(&self, page: PageId, selector: &str) -> Result<usize, CoreError> {
        let result = async {
            let Some(scope) = self.scope(page, selector).await? else {
                return Ok(0);
            };
            let remote = self
                .worlds
                .call(
                    &scope.session,
                    &scope.frame,
                    "function(parsed) { return this.querySelectorAll(parsed, document).length; }",
                    vec![scope.parsed],
                    true,
                )
                .await?;
            remote["value"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| CoreError::from("Injected count is not an integer"))
        }
        .await;
        result.map_err(|e| with_log(e, &[format!("waiting for {selector}")]))
    }

    pub async fn wait_for_states(
        &self,
        r: &Resolved,
        states: &[ElementState],
        dl: &Deadline,
    ) -> Result<(), CoreError> {
        let names = states.iter().map(|s| s.name()).collect::<Vec<_>>();
        let mut log = vec![format!("waiting for element to be {}", names.join(", "))];
        let result = async {
            let frame = match &r.frame_id { Some(frame) => frame.clone(), None => dl.run(root_frame(&r.session)).await? };
            let mut attempt = 0;
            loop {
                dl.pause(attempt).await?;
                attempt += 1;
                let value = dl.run(async {
                    let world = self.worlds.get(&r.session, &frame).await?;
                    // Use the existing handle, not the backend id: navigation must not
                    // silently turn an already-resolved action into an action on a new node.
                    runtime_result(r.session.send_value("Runtime.callFunctionOn", json!({
                        "objectId": world.object,
                        "functionDeclaration": "async function(el, states) { return await this.checkElementStates(el, states) || null; }",
                        "arguments": [{"objectId": r.object_id}, {"value": names}],
                        "returnByValue": true, "awaitPromise": true
                    })).await?)
                }).await?;
                let status = &value["value"];
                if status.is_null() { return Ok(()); }
                if status == "error:notconnected" { return Err(CoreError::DocumentChanged); }
                if let Some(state) = status["missingState"].as_str() {
                    log_line(&mut log, format!("element is not {state}"));
                }
            }
        }.await;
        result.map_err(|e| with_log(e, &log))
    }

    /// Polls the pinned `expect(element, options, elements)` entry. Missing elements
    /// and negation are host semantics; never pass undefined to a non-array matcher.
    pub async fn expect(
        &self,
        page: PageId,
        selector: Option<&str>,
        expression: &str,
        mut options: Value,
        dl: &Deadline,
    ) -> Result<ExpectOutcome, CoreError> {
        if !options.is_object() {
            options = json!({});
        }
        options["expression"] = json!(expression);
        let is_not = options["isNot"].as_bool().unwrap_or(false);
        options["isNot"] = json!(is_not);
        let selector = selector.unwrap_or(":root");
        let mut outcome = ExpectOutcome {
            matches: is_not,
            received: None,
            timed_out: false,
            log: vec![format!("waiting for {selector}")],
        };
        let mut attempt = 0;
        loop {
            let check = async {
                dl.pause(attempt).await?;
                dl.run(self.expect_once(page.clone(), selector, &options))
                    .await
            }
            .await;
            attempt += 1;
            match check {
                Ok(value) => {
                    outcome.matches = value["matches"].as_bool().unwrap_or(is_not);
                    outcome.received = value.get("received").cloned();
                    if let Some(log) = value["log"].as_str() {
                        log_line(&mut outcome.log, log.to_owned());
                    }
                    if outcome.matches != is_not {
                        return Ok(outcome);
                    }
                    if let Some(received) = &outcome.received {
                        log_line(&mut outcome.log, format!("unexpected value {received}"));
                    }
                }
                Err(error) if error == timeout() => {
                    outcome.timed_out = true;
                    return Ok(outcome);
                }
                Err(error) => return Err(with_log(error, &outcome.log)),
            }
        }
    }

    async fn expect_once(
        &self,
        page: PageId,
        selector: &str,
        options: &Value,
    ) -> Result<Value, CoreError> {
        let Some(scope) = self.scope(page, selector).await? else {
            return Ok(missing_expect(options));
        };
        let result = self
            .worlds
            .call(
                &scope.session,
                &scope.frame,
                EXPECT,
                vec![scope.parsed, options.clone()],
                true,
            )
            .await?;
        Ok(result["value"].clone())
    }

    /// DOM.resolveNode without executionContextId adopts into the node's own main
    /// world (including same-process child frames). Calling the isolated handle directly
    /// would hide page globals and break locator.evaluate's Playwright contract.
    pub async fn call_on(
        &self,
        r: &Resolved,
        fn_source: &str,
        arg: Value,
    ) -> Result<Value, CoreError> {
        let adopted = r
            .session
            .send_value(
                "DOM.resolveNode",
                json!({"backendNodeId": r.backend_node_id}),
            )
            .await?;
        let object = object_id(&adopted["object"])?;
        let result = r.session.send_value("Runtime.callFunctionOn", json!({
            "objectId": object, "functionDeclaration": format!("function(arg) {{ return ({fn_source})(this, arg); }}"),
            "arguments": [{"value": arg}], "returnByValue": true, "awaitPromise": true
        })).await.and_then(runtime_result);
        release(&r.session, &object).await;
        result.map(|remote| remote.get("value").cloned().unwrap_or(Value::Null))
    }

    pub async fn describe(&self, r: &Resolved) -> Result<Value, CoreError> {
        let described = r
            .session
            .send_value(
                "DOM.describeNode",
                json!({"backendNodeId": r.backend_node_id, "depth": 0}),
            )
            .await?;
        let node = &described["node"];
        let tag = node["localName"]
            .as_str()
            .or_else(|| node["nodeName"].as_str())
            .ok_or_else(|| CoreError::from("DOM.describeNode returned no tag"))?
            .to_ascii_lowercase();
        let mut result = json!({"tag": tag, "type": "", "autocomplete": ""});
        if let Some(attributes) = node["attributes"].as_array() {
            for pair in attributes.chunks_exact(2) {
                if let Some(name @ ("type" | "autocomplete")) = pair[0].as_str() {
                    result[name] = pair[1].clone();
                }
            }
        }
        Ok(result)
    }
}

fn missing_expect(options: &Value) -> Value {
    let expression = options["expression"].as_str().unwrap_or("");
    let is_not = options["isNot"].as_bool().unwrap_or(false);
    if expression == "to.have.count" {
        return json!({"matches": options["expectedNumber"] == 0, "received": 0, "log": "  locator resolved to 0 elements"});
    }
    if expression.ends_with(".array") {
        return json!({"matches": options["expectedText"].as_array().is_none_or(Vec::is_empty), "received": [], "log": "  locator resolved to 0 elements"});
    }
    let matches = match (expression, is_not) {
        ("to.be.hidden" | "to.be.detached", false) => true,
        ("to.be.visible" | "to.be.attached" | "to.be.in.viewport", true) => false,
        _ => is_not,
    };
    json!({"matches": matches, "received": null})
}

// Strip diagnostics in the renderer, before they cross CDP. In particular a failed
// visibility matcher can snapshot unrelated password fields in the document body.
// Retarget labels just like value matchers do, and mask the whole array if any member
// is sensitive; diagnostic previews must not reintroduce those values through logs.
const EXPECT: &str = r#"async function(parsed, options) {
    const elements = this.querySelectorAll(parsed, document);
    const array = options.expression === 'to.have.count' || options.expression.endsWith('.array');
    if (!array && elements.length > 1) throw this.strictModeViolationError(parsed, elements);
    if (!elements.length && !array) {
        let matches = options.isNot;
        if (!options.isNot && ['to.be.hidden', 'to.be.detached'].includes(options.expression)) matches = true;
        if (options.isNot && ['to.be.visible', 'to.be.attached', 'to.be.in.viewport'].includes(options.expression)) matches = false;
        return { matches, received: null };
    }
    const sensitive = elements.some(el => {
        el = this.retarget(el, 'follow-label');
        return el && (String(el.getAttribute('type')).toLowerCase() === 'password' ||
            /(?:current-password|new-password|one-time-code|cc-number|cc-csc|cc-exp)/i.test(el.getAttribute('autocomplete') || ''));
    });
    const result = await this.expect(elements[0], options, elements);
    const log = array ? `  locator resolved to ${elements.length} element${elements.length === 1 ? '' : 's'}` :
        `  locator resolved to ${sensitive ? '<input [redacted]>' : this.previewNode(elements[0])}`;
    return { matches: result.matches, received: sensitive ? '[redacted]' : result.received?.value, log };
}"#;
