//! Playwright leaves translate JSON arguments into locator-engine and trusted
//! input operations. The shared BrowserBridge remains the sole owner of hooks;
//! delegating to an existing tool here must never create a second audit row.

use super::{
    PwCallError, PwCallOutcome, action_error, bounded, deadline, opts, page_arg, page_info,
    selector_arg, string_arg,
};
use crate::{
    framework::{ToolDef, execute_tool},
    tools::run::{BrowserBridge, BrowserCallValue},
};
use browseros_core::{
    CoreError, CoveredElementTarget, PageId, ProtocolSession, WindowId,
    input::{
        ClickOptions, Point, keyboard,
        mouse::{self, MouseButton},
    },
    locator::{Deadline, ElementState, LocatorEngine, Resolved, Strictness},
    pages::NewPageOptions,
};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};

const ACTION_TIMEOUT: u64 = 10_000;

/// Remote handles stay alive through input and its redaction probe, then are
/// released together. This also covers query/evaluate failures and partial arrays.
#[derive(Default)]
struct ActionState {
    typed_target: Option<Resolved>,
    handles: Vec<(ProtocolSession, String)>,
    last_retry: Option<CoreError>,
}
impl ActionState {
    fn hold(&mut self, node: &Resolved) {
        self.handles
            .push((node.session.clone(), node.object_id.clone()));
    }
    async fn release(self, dl: &Deadline) {
        // Never extend the operation/run deadline for cleanup. A destroyed
        // execution context releases any handles that could not be reclaimed.
        for (session, object) in self.handles {
            let _ = bounded(
                dl,
                session.send_value("Runtime.releaseObject", json!({"objectId":object})),
            )
            .await;
        }
    }
}

pub(crate) async fn dispatch(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
) -> Result<PwCallOutcome, PwCallError> {
    let options = opts(args, option_index(method));
    let dl = deadline(bridge, &options, ACTION_TIMEOUT);
    let timeout_ms = options.timeout_ms.unwrap_or(ACTION_TIMEOUT);
    let engine = bridge.ctx.session.locator();
    // Retain the actual node used by the action: resolving again could inspect a
    // replacement input and accidentally expose a password entered in the first.
    let mut state = ActionState::default();
    let result = bounded(
        &dl,
        dispatch_inner(bridge, &engine, method, args, &dl, &mut state),
    )
    .await;
    let mut audit_args = None;
    let mut secrets = Vec::new();
    if let Some(index) = typed_value_index(method) {
        let probe = if let Some(target) = state.typed_target.as_ref() {
            bounded(&dl, engine.describe(target)).await
        } else if method.starts_with("keyboard.") {
            bounded(&dl, async {
                let page = page_arg(args).map_err(CoreError::from)?;
                evaluate_page(bridge, page,
                    "(() => { const e = document.activeElement; return e ? {type:e.type || '', autocomplete:e.autocomplete || ''} : null; })()", None).await
            }).await
        } else {
            Err(CoreError::Message(
                "typed element could not be inspected".into(),
            ))
        };
        if sensitive(probe.as_ref().ok()) {
            (audit_args, secrets) = redact_args(args, index);
        }
    }
    let last_retry = state.last_retry.take();
    state.release(&dl).await;
    match result {
        Ok(mut outcome) => {
            // The facade unwraps {value,url,title}; wrap even user objects so a
            // page.evaluate result that happens to contain these keys stays data.
            // Refresh is best effort after a successful action: a click may close
            // its own tab, and that must not turn its completed input into failure.
            if (method.starts_with("locator.")
                || method.starts_with("page.")
                || method.starts_with("keyboard.")
                || method.starts_with("mouse.")
                || matches!(
                    method,
                    "neo.snapshot" | "neo.read" | "neo.grep" | "neo.download"
                ))
                && method != "page.info"
                && let Ok(page) = page_arg(args)
                && let Ok(mut info) = page_info(bridge, page, &dl).await
            {
                info["value"] = match outcome.value {
                    BrowserCallValue::Json(value) => value,
                    BrowserCallValue::Undefined => Value::Null,
                };
                outcome.value = BrowserCallValue::Json(info);
            }
            outcome.audit_args = audit_args;
            outcome.secrets = secrets;
            Ok(outcome)
        }
        Err(error) => {
            let mut message = action_error(
                method,
                args.get(1)
                    .and_then(Value::as_str)
                    .filter(|_| method.starts_with("locator.")),
                timeout_ms,
                error,
            );
            if let Some(retry) = last_retry {
                let detail = match retry {
                    CoreError::ElementCovered { blocker, .. } => {
                        format!("{blocker} intercepts pointer events")
                    }
                    error => error.to_string(),
                };
                message.push_str(&format!("\n  - {detail}"));
            }
            for secret in &secrets {
                message = message.replace(secret, "[redacted]");
                if let Ok(encoded) = serde_json::to_string(secret) {
                    // Remove exactly the JSON delimiters; trim_matches would also
                    // remove a secret's trailing escaped quote and expose it.
                    message = message.replace(&encoded[1..encoded.len() - 1], "[redacted]");
                }
            }
            Err(PwCallError {
                message,
                audit_args,
                secrets,
            })
        }
    }
}

fn option_index(method: &str) -> usize {
    match method {
        "locator.evaluate" | "locator.evaluateAll" | "locator.query" => 4,
        "locator.fill"
        | "locator.type"
        | "locator.press"
        | "locator.check"
        | "locator.selectOption"
        | "locator.setInputFiles"
        | "locator.dragTo"
        | "page.evaluate"
        | "mouse.click"
        | "mouse.move"
        | "mouse.wheel" => 3,
        "locator.click"
        | "locator.dblclick"
        | "locator.hover"
        | "locator.focus"
        | "locator.blur"
        | "locator.clear"
        | "locator.scrollIntoViewIfNeeded"
        | "keyboard.press"
        | "keyboard.type"
        | "keyboard.insertText" => 2,
        _ => 1,
    }
}

fn typed_value_index(method: &str) -> Option<usize> {
    match method {
        "locator.fill" | "locator.type" => Some(2),
        "keyboard.type" | "keyboard.insertText" => Some(1),
        _ => None,
    }
}

/// Missing/malformed descriptions fail closed, including detached nodes and
/// exhausted deadlines. Empty strings cannot be added to replacement lists.
pub(super) fn sensitive(description: Option<&Value>) -> bool {
    let Some(description) = description else {
        return true;
    };
    let (Some(kind), Some(autocomplete)) = (
        description.get("type").and_then(Value::as_str),
        description.get("autocomplete").and_then(Value::as_str),
    ) else {
        return true;
    };
    kind.eq_ignore_ascii_case("password")
        || [
            "current-password",
            "new-password",
            "one-time-code",
            "cc-number",
            "cc-csc",
            "cc-exp",
        ]
        .iter()
        .any(|token| autocomplete.to_ascii_lowercase().contains(token))
}

pub(super) fn redact_args(args: &[Value], index: usize) -> (Option<Value>, Vec<String>) {
    let mut masked = args.to_vec();
    let secret = args
        .get(index)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    if let Some(value) = masked.get_mut(index) {
        *value = json!("[redacted]");
    }
    (
        Some(Value::Array(masked)),
        secret.map(|s| vec![s.to_string()]).unwrap_or_default(),
    )
}

async fn dispatch_inner(
    bridge: &BrowserBridge,
    engine: &LocatorEngine,
    method: &str,
    args: &[Value],
    dl: &Deadline,
    state: &mut ActionState,
) -> Result<PwCallOutcome, CoreError> {
    if method.starts_with("locator.") {
        return locator_action(bridge, engine, method, args, dl, state).await;
    }
    match method {
        "context.newPage" => {
            let url = args
                .first()
                .and_then(Value::as_str)
                .unwrap_or("about:blank");
            let options = args.get(1).unwrap_or(&Value::Null);
            let page = bridge
                .ctx
                .session
                .pages
                .new_page(
                    url,
                    NewPageOptions {
                        background: Some(true),
                        window_id: options
                            .get("windowId")
                            .and_then(Value::as_i64)
                            .map(WindowId)
                            .or_else(|| bridge.ctx.defaults.default_window_id.clone()),
                        tab_group_id: options
                            .get("tabGroupId")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| bridge.ctx.defaults.default_tab_group_id.clone()),
                    },
                )
                .await?;
            let mut outcome = PwCallOutcome::json(json!(page.0));
            // The outer bridge claims and groups this page before recording or
            // resolving the JavaScript promise, exactly like pages.newPage.
            outcome.created_page = Some(page.0);
            Ok(outcome)
        }
        "context.pages" | "context.lastPage" | "neo.pages" => {
            let pages = bridge.ctx.session.pages.list().await?;
            let mut values: Vec<Value> = pages.into_iter().map(|p| json!({"pageId":p.page_id.0,"url":p.url,"title":p.title,"isActive":p.is_active,"windowId":p.window_id.map(|w|w.0)})).collect();
            if let Some(hook) = &bridge.ctx.inner_call_hook {
                values = hook.annotate_pages(&values).await;
            }
            if method == "context.lastPage" {
                return Ok(PwCallOutcome::json(json!(
                    bridge.pw_activity.last_owned(&values)
                )));
            }
            let scope = if method == "context.pages" {
                "mine"
            } else {
                args.first()
                    .and_then(|v| {
                        v.as_str()
                            .or_else(|| v.get("ownership").and_then(Value::as_str))
                    })
                    .unwrap_or("all")
            };
            if !["all", "mine", "user", "other-agent"].contains(&scope) {
                return Err("unknown ownership scope".into());
            }
            if scope != "all" {
                values.retain(|v| v.get("ownership").and_then(Value::as_str) == Some(scope));
            }
            Ok(PwCallOutcome::json(json!(values)))
        }
        "context.close" => {
            bridge
                .ctx
                .session
                .pages
                .close(page_arg(args).map_err(CoreError::from)?)
                .await?;
            Ok(PwCallOutcome::json(Value::Null))
        }
        "neo.cdp" => {
            let name = string_arg(args, 0, "CDP method").map_err(CoreError::from)?;
            let params = args
                .get(1)
                .cloned()
                .filter(|v| !v.is_null())
                .unwrap_or_else(|| json!({}));
            let value = if let Some(page) = args.get(2).filter(|v| !v.is_null()) {
                let page = page_arg(std::slice::from_ref(page)).map_err(CoreError::from)?;
                bridge
                    .ctx
                    .session
                    .pages
                    .get_session(page)
                    .await?
                    .session
                    .send_value(name, params)
                    .await?
            } else {
                bridge.ctx.session.cdp(name, params, None).await?
            };
            Ok(PwCallOutcome::json(value))
        }
        _ => {
            let page = page_arg(args).map_err(CoreError::from)?;
            let value = match method {
                "page.info" | "neo.page" => page_info(bridge, page, dl).await?,
                "page.title" => page_info(bridge, page, dl).await?["title"].clone(),
                "page.content" => evaluate_page(bridge, page, "(() => (document.doctype ? new XMLSerializer().serializeToString(document.doctype) + '\\n' : '') + document.documentElement.outerHTML)()", None).await?,
                "page.evaluate" => evaluate_page(bridge, page, string_arg(args, 1, "function").map_err(CoreError::from)?, Some(args.get(2).cloned().unwrap_or(Value::Null))).await?,
                "page.screenshot" | "page.pdf" | "neo.read" | "neo.grep" | "neo.download" => return tool_action(bridge, method, args).await,
                "neo.snapshot" => {
                    let snapshot = bridge.ctx.session.observe(page).await.snapshot().await?;
                    let refs: Vec<Value> = snapshot.refs.entries_in_order().into_iter().map(|e| json!({"ref":e.ref_id.as_str(),"backendNodeId":e.backend_node_id,"role":e.role,"name":e.name,"nth":e.nth,"frameId":e.frame_id.as_ref().map(|f|f.as_str())})).collect();
                    json!({"text":snapshot.text,"refs":refs,"url":snapshot.url})
                }
                m if m.starts_with("keyboard.") || m.starts_with("mouse.") => {
                    device_action(bridge, page.clone(), method, args).await?;
                    Value::Null
                }
                _ => return Err(format!("not available in BrowserOS neo: {method}").into()),
            };
            Ok(PwCallOutcome::json(value))
        }
    }
}

async fn locator_action(
    bridge: &BrowserBridge,
    engine: &LocatorEngine,
    method: &str,
    args: &[Value],
    dl: &Deadline,
    state: &mut ActionState,
) -> Result<PwCallOutcome, CoreError> {
    loop {
        match locator_attempt(bridge, engine, method, args, dl, state).await {
            // Both errors occur before input dispatch. Retrying a protocol failure
            // after a down/up pair could duplicate user input, so never do that.
            Err(error @ (CoreError::ElementCovered { .. } | CoreError::DocumentChanged)) => {
                state.last_retry = Some(error);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            result => return result,
        }
    }
}

async fn locator_attempt(
    bridge: &BrowserBridge,
    engine: &LocatorEngine,
    method: &str,
    args: &[Value],
    dl: &Deadline,
    state: &mut ActionState,
) -> Result<PwCallOutcome, CoreError> {
    let page = page_arg(args).map_err(CoreError::from)?;
    let selector = selector_arg(args).map_err(CoreError::from)?;
    if method == "locator.query" {
        return query(engine, page, selector, args, dl, state)
            .await
            .map(PwCallOutcome::json);
    }
    if method == "locator.evaluateAll" {
        let nodes = engine.resolve_all(page.clone(), selector).await?;
        for node in &nodes {
            state.hold(node);
        }
        return evaluate_all(
            bridge,
            page,
            &nodes,
            string_arg(args, 2, "function").map_err(CoreError::from)?,
            args.get(3).cloned().unwrap_or(Value::Null),
            state,
        )
        .await
        .map(PwCallOutcome::json);
    }
    let options = opts(args, option_index(method));
    let strict = if options.strict {
        Strictness::Strict
    } else {
        Strictness::First
    };
    let resolved = engine
        .resolve(page.clone(), selector, strict, None, dl)
        .await?;
    state.hold(&resolved);
    if method == "locator.evaluate" {
        return engine
            .call_on(
                &resolved,
                string_arg(args, 2, "function").map_err(CoreError::from)?,
                args.get(3).cloned().unwrap_or(Value::Null),
            )
            .await
            .map(PwCallOutcome::json);
    }
    state.typed_target = Some(resolved);
    let r = state
        .typed_target
        .as_ref()
        .ok_or_else(|| CoreError::from("missing resolved element"))?;
    let input = bridge.ctx.session.input(page.clone()).await;
    // Hidden file inputs and focus/blur/keyboard operations do not need pointer
    // actionability. Applying the click checks to them breaks valid Playwright.
    let no_scroll = matches!(
        method,
        "locator.setInputFiles"
            | "locator.focus"
            | "locator.blur"
            | "locator.type"
            | "locator.press"
    );
    if !no_scroll {
        scroll(r).await?;
    }
    if !options.force {
        let states: &[ElementState] = match method {
            "locator.fill" | "locator.clear" => &[
                ElementState::Visible,
                ElementState::Enabled,
                ElementState::Editable,
            ],
            "locator.selectOption" => &[ElementState::Visible, ElementState::Enabled],
            "locator.hover" | "locator.dragTo" => &[ElementState::Visible, ElementState::Stable],
            "locator.scrollIntoViewIfNeeded" => &[ElementState::Stable],
            _ if no_scroll => &[],
            _ => &[
                ElementState::Visible,
                ElementState::Enabled,
                ElementState::Stable,
            ],
        };
        if !states.is_empty() {
            engine.wait_for_states(r, states, dl).await?;
        }
    }
    let raw_options = args.get(option_index(method)).unwrap_or(&Value::Null);
    match method {
        "locator.click" | "locator.dblclick" | "locator.hover" => {
            pointer_action(
                bridge,
                engine,
                page.clone(),
                r,
                method,
                raw_options,
                options.force,
            )
            .await?
        }
        "locator.fill" | "locator.clear" => {
            let value = if method == "locator.clear" {
                ""
            } else {
                string_arg(args, 2, "value").map_err(CoreError::from)?
            };
            if value.is_empty() || options.force {
                // The existing core fill uses Backspace for empty strings and
                // always cover-checks. Playwright selects via injected fill and
                // presses Delete; force must bypass the pointer cover check.
                let result = injected_call(r, "fill", json!(value)).await?;
                match result.as_str() {
                    Some("needsinput") if value.is_empty() => {
                        keyboard::press_combo(&r.session, "Delete").await?
                    }
                    Some("needsinput") => {
                        r.session
                            .send_value("Input.insertText", json!({"text":value}))
                            .await?;
                    }
                    Some("done") => {}
                    _ => return Err(format!("fill failed: {result}").into()),
                }
            } else {
                input
                    .fill_backend_node(&r.session, r.backend_node_id, value)
                    .await?;
            }
        }
        "locator.type" | "locator.press" => {
            input
                .focus_backend_node(&r.session, r.backend_node_id)
                .await?;
            let text = string_arg(args, 2, "text or key").map_err(CoreError::from)?;
            // The node may be in an OOPIF: preserve its session for keyboard input.
            if method == "locator.type" {
                type_with_delay(&r.session, text, raw_options).await?;
            } else {
                keyboard::press_combo(&r.session, &platform_key(text)).await?;
            }
        }
        "locator.focus" => {
            input
                .focus_backend_node(&r.session, r.backend_node_id)
                .await?
        }
        "locator.blur" => {
            engine.call_on(r, "el => el.blur()", Value::Null).await?;
        }
        "locator.check" => {
            let checked = args
                .get(2)
                .and_then(Value::as_bool)
                .ok_or_else(|| CoreError::from("checked must be a boolean"))?;
            let actual = if options.force || raw_options.get("position").is_some() {
                let current = injected_call(r, "state", json!("checked")).await?;
                if current.get("matches").and_then(Value::as_bool) != Some(checked) {
                    pointer_action(
                        bridge,
                        engine,
                        page.clone(),
                        r,
                        "locator.click",
                        raw_options,
                        options.force,
                    )
                    .await?;
                }
                if raw_options.get("trial").and_then(Value::as_bool) == Some(true) {
                    checked
                } else {
                    injected_call(r, "state", json!("checked"))
                        .await?
                        .get("matches")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| CoreError::from("Not a checkbox or radio button"))?
                }
            } else if raw_options.get("trial").and_then(Value::as_bool) == Some(true) {
                let point = point(r, None).await?;
                cover_check(engine, r, point).await?;
                checked
            } else {
                input
                    .check_backend_node(&r.session, r.backend_node_id, checked)
                    .await?
            };
            if actual != checked {
                return Err("Clicking the checkbox did not change its state".into());
            }
        }
        "locator.selectOption" => {
            return select_options(
                bridge,
                engine,
                page,
                r,
                args.get(2).cloned().unwrap_or(Value::Null),
            )
            .await
            .map(PwCallOutcome::json);
        }
        "locator.setInputFiles" => {
            let paths = string_list(args.get(2).unwrap_or(&Value::Null), "file paths")?
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            input
                .upload_backend_node(&r.session, r.backend_node_id, &paths)
                .await?;
        }
        "locator.scrollIntoViewIfNeeded" => {}
        "locator.dragTo" => {
            let target = engine
                .resolve(
                    page.clone(),
                    string_arg(args, 2, "target selector").map_err(CoreError::from)?,
                    strict,
                    None,
                    dl,
                )
                .await?;
            state
                .handles
                .push((target.session.clone(), target.object_id.clone()));
            if !r.session.same_session(&target.session) {
                return Err(CoreError::CrossFrameDrag);
            }
            scroll(&target).await?;
            if !options.force {
                engine
                    .wait_for_states(&target, &[ElementState::Visible, ElementState::Stable], dl)
                    .await?;
            }
            let from = point(r, raw_options.get("sourcePosition")).await?;
            let to = point(&target, raw_options.get("targetPosition")).await?;
            if !options.force {
                cover_check(engine, r, from).await?;
                cover_check(engine, &target, to).await?;
            }
            drag_to(bridge, r, from, to).await?;
        }
        _ => return Err(format!("not available in BrowserOS neo: {method}").into()),
    }
    Ok(PwCallOutcome::json(Value::Null))
}

/// Native HTML drags enter Chromium's drag loop, which ordinary mouse-up cannot
/// reliably finish. Intercept that loop and replay its real DataTransfer payload
/// through CDP; custom pointer-based drags still receive normal mouse events.
async fn drag_to(
    bridge: &BrowserBridge,
    source: &Resolved,
    from: Point,
    to: Point,
) -> Result<(), CoreError> {
    let session = &source.session;
    let mut events = bridge.ctx.session.cdp_events();
    let mut cleanup = DragCleanup {
        session: session.clone(),
        point: to,
        watcher: None,
        complete: false,
    };
    let result = async {
        // Retain the listener in the source's isolated world. A canceled
        // dragstart must behave like an ordinary mouse gesture, not wait for a
        // dragIntercepted event that Chromium will never emit.
        let watcher = session.send_value("Runtime.callFunctionOn", json!({
            "objectId":source.object_id,
            "functionDeclaration":r"function() {
                const view = this.ownerDocument.defaultView;
                let started;
                const onStart = event => { started = event; };
                view.addEventListener('dragstart', onStart, true);
                return {
                    dispose() { view.removeEventListener('dragstart', onStart, true); },
                    async started() {
                        await new Promise(resolve => view.setTimeout(resolve, 0));
                        this.dispose();
                        return !!started && !started.defaultPrevented;
                    }
                };
            }"
        })).await?;
        let watcher = watcher["result"]["objectId"].as_str()
            .ok_or_else(|| CoreError::from("could not observe dragstart"))?.to_owned();
        cleanup.watcher = Some(watcher.clone());
        session.send_value("Input.setInterceptDrags", json!({"enabled":true})).await?;
        for (kind, point, buttons) in [
            ("mouseMoved", from, 0),
            ("mousePressed", from, 1),
            ("mouseMoved", to, 1),
        ] {
            session.send_value("Input.dispatchMouseEvent", json!({
                "type":kind,"x":point.x,"y":point.y,"button":if buttons == 1 { "left" } else { "none" },
                "buttons":buttons,"clickCount":if kind == "mousePressed" {1} else {0}
            })).await?;
        }
        let started = evaluated_value(session.send_value("Runtime.callFunctionOn", json!({
            "objectId":watcher,"functionDeclaration":"function() { return this.started(); }",
            "awaitPromise":true,"returnByValue":true
        })).await?)? == json!(true);
        if started {
            let data = loop {
                let event = events.recv().await.map_err(|error| CoreError::from(error.to_string()))?;
                if event.session_id.as_ref() == session.session_id() && event.method == "Input.dragIntercepted" {
                    break event.params["data"].clone();
                }
            };
            for kind in ["dragEnter", "dragOver", "drop"] {
                session.send_value("Input.dispatchDragEvent", json!({
                    "type":kind,"x":to.x,"y":to.y,"data":data
                })).await?;
            }
        } else {
            // A second move delivers the destination's dragover/pointer update
            // even when no native drag session was started.
            session.send_value("Input.dispatchMouseEvent", json!({
                "type":"mouseMoved","x":to.x,"y":to.y,"button":"left","buttons":1
            })).await?;
        }
        Ok(())
    }.await;
    cleanup.release(result.is_err()).await;
    result
}

/// An operation deadline can drop drag_to during any CDP await. Restore browser
/// input state even then; otherwise one timed-out drag can stall later scripts.
struct DragCleanup {
    session: ProtocolSession,
    point: Point,
    watcher: Option<String>,
    complete: bool,
}
impl DragCleanup {
    async fn release(&mut self, cancel: bool) {
        restore_drag(&self.session, self.point, self.watcher.as_deref(), cancel).await;
        self.complete = true;
    }
}
impl Drop for DragCleanup {
    fn drop(&mut self) {
        if !self.complete {
            let session = self.session.clone();
            let point = self.point;
            let watcher = self.watcher.take();
            tokio::spawn(async move {
                restore_drag(&session, point, watcher.as_deref(), true).await;
            });
        }
    }
}
async fn restore_drag(
    session: &ProtocolSession,
    point: Point,
    watcher: Option<&str>,
    cancel: bool,
) {
    // Cleanup is bounded independently because the operation's deadline may
    // already have expired. It only undoes input/listener state, never retries
    // the user action or keeps a dropped script alive.
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        if cancel {
            let _ = session.send_value("Input.dispatchDragEvent", json!({
                "type":"dragCancel","x":point.x,"y":point.y,
                "data":{"items":[],"dragOperationsMask":65535}
            })).await;
        }
        let _ = session.send_value("Input.dispatchMouseEvent", json!({
            "type":"mouseReleased","x":point.x,"y":point.y,"button":"left","buttons":0,"clickCount":1
        })).await;
        let _ = session.send_value("Input.setInterceptDrags", json!({"enabled":false})).await;
        if let Some(watcher) = watcher {
            let _ = session.send_value("Runtime.callFunctionOn", json!({
                "objectId":watcher,"functionDeclaration":"function() { this.dispose(); }"
            })).await;
            let _ = session.send_value("Runtime.releaseObject", json!({"objectId":watcher})).await;
        }
    }).await;
}

async fn scroll(r: &Resolved) -> Result<(), CoreError> {
    r.session
        .send_value(
            "DOM.scrollIntoViewIfNeeded",
            json!({"backendNodeId":r.backend_node_id}),
        )
        .await?;
    Ok(())
}

async fn point(r: &Resolved, position: Option<&Value>) -> Result<Point, CoreError> {
    let result = r
        .session
        .send_value(
            "DOM.getBoxModel",
            json!({"backendNodeId":r.backend_node_id}),
        )
        .await?;
    let quad_name = if position.is_some() {
        "padding"
    } else {
        "content"
    };
    let quad = result
        .get("model")
        .and_then(|m| m.get(quad_name))
        .and_then(Value::as_array)
        .filter(|q| q.len() == 8)
        .ok_or_else(|| CoreError::from("Element is not visible or has no layout box"))?;
    let coordinate = |i: usize| {
        quad.get(i)
            .and_then(Value::as_f64)
            .ok_or_else(|| CoreError::from("Invalid element box"))
    };
    if let Some(position) = position {
        return Ok(Point {
            x: coordinate(0)? + number(position.get("x"), "position.x")?,
            y: coordinate(1)? + number(position.get("y"), "position.y")?,
        });
    }
    Ok(Point {
        x: (coordinate(0)? + coordinate(2)? + coordinate(4)? + coordinate(6)?) / 4.0,
        y: (coordinate(1)? + coordinate(3)? + coordinate(5)? + coordinate(7)?) / 4.0,
    })
}

/// The published LocatorEngine seam has no generic injected-method entry point.
/// Bootstrap a companion engine only for hit targets, state reads, multi-select,
/// and ariaSnapshot. It uses the same pinned bundle and the resolved node's
/// isolated world; user functions still run exclusively through call_on.
/// The document-scoped cache disappears on navigation and is invisible to page JS.
async fn injected_call(r: &Resolved, method: &str, arg: Value) -> Result<Value, CoreError> {
    const BUNDLE: &str =
        include_str!("../../../browseros-core/src/locator/assets/injected_script.js");
    const CACHE: &str = "__browseros_pw_action_engine";
    let cached = evaluated_value(r.session.send_value("Runtime.callFunctionOn", json!({
        "objectId":r.object_id, "functionDeclaration":format!("function() {{ return !!this.ownerDocument.{CACHE}; }}"), "returnByValue":true
    })).await?)?;
    if cached != Value::Bool(true) {
        let source = format!(
            r#"function() {{
            const module = {{}};
            {BUNDLE}
            this.ownerDocument.{CACHE} = new (module.exports.InjectedScript())(globalThis, {{
                isUnderTest:false,sdkLanguage:'javascript',frameSeq:0,testIdAttributeName:'data-testid',
                stableRafCount:1,browserName:'chromium',shouldPrependErrorPrefix:false,isUtilityWorld:true,customEngines:[]
            }});
        }}"#
        );
        evaluated_value(
            r.session
                .send_value(
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId":r.object_id,"functionDeclaration":source,"returnByValue":true
                    }),
                )
                .await?,
        )?;
    }
    let source = format!(
        r#"function(method, arg) {{
        const engine = this.ownerDocument.{CACHE};
        switch (method) {{
            case 'hitTarget': return engine.expectHitTarget(arg, this);
            case 'state': return engine.elementState(this, arg);
            case 'aria': return engine.ariaSnapshot(this, {{}});
            case 'select': return engine.selectOptions(this, arg);
            case 'fill': return engine.fill(this, arg);
        }}
        throw new Error('Unknown injected action');
    }}"#
    );
    evaluated_value(r.session.send_value("Runtime.callFunctionOn",json!({
        "objectId":r.object_id,"functionDeclaration":source,
        "arguments":[{"value":method},{"value":arg}],"awaitPromise":true,"returnByValue":true
    })).await?)
}

async fn cover_check(engine: &LocatorEngine, r: &Resolved, point: Point) -> Result<(), CoreError> {
    // CDP/input use the target session's viewport; the injected hit test queries
    // the resolved node's document. Translate only for the hit test, never input.
    let hit_point = engine.hit_target_point(r, point).await?;
    let result = injected_call(r, "hitTarget", json!({"x":hit_point.x,"y":hit_point.y})).await?;
    if result == "done" {
        return Ok(());
    }
    if let Some(blocker) = result.get("hitTargetDescription").and_then(Value::as_str) {
        return Err(CoreError::ElementCovered {
            target: CoveredElementTarget {
                backend_node_id: Some(r.backend_node_id),
                ..Default::default()
            },
            blocker: blocker.to_string(),
        });
    }
    Err(CoreError::from(format!(
        "Hit target check failed: {result}"
    )))
}

async fn pointer_action(
    bridge: &BrowserBridge,
    engine: &LocatorEngine,
    page: PageId,
    r: &Resolved,
    method: &str,
    options: &Value,
    force: bool,
) -> Result<(), CoreError> {
    let position = point(r, options.get("position")).await?;
    if !force {
        cover_check(engine, r, position).await?;
    }
    if options.get("trial").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let (button, count, modifiers) = pointer_options(options, method == "locator.dblclick")?;
    let page_session = bridge
        .ctx
        .session
        .pages
        .get_session(page.clone())
        .await?
        .session;
    let simple = !force
        && options.get("position").is_none()
        && modifiers == 0
        && count == 1
        && r.session.same_session(&page_session);
    let input = bridge.ctx.session.input(page).await;
    if simple {
        if method == "locator.hover" {
            input.hover_backend_node(r.backend_node_id).await?;
        } else {
            input
                .click_backend_node(
                    r.backend_node_id,
                    ClickOptions {
                        button: Some(button),
                        click_count: Some(count),
                    },
                )
                .await?;
        }
    } else if method == "locator.hover" {
        r.session
            .send_value(
                "Input.dispatchMouseEvent",
                json!({"type":"mouseMoved","x":position.x,"y":position.y,"modifiers":modifiers}),
            )
            .await?;
    } else {
        // Core's coordinate primitive preserves the resolved frame session and
        // modifier bits. Double clicks require two complete down/up pairs.
        for click in 1..=count {
            mouse::dispatch_click(&r.session, position.x, position.y, button, click, modifiers)
                .await?;
        }
    }
    Ok(())
}

fn pointer_options(options: &Value, double: bool) -> Result<(MouseButton, i64, i64), CoreError> {
    let button = match options
        .get("button")
        .and_then(Value::as_str)
        .unwrap_or("left")
    {
        "left" => MouseButton::Left,
        "middle" => MouseButton::Middle,
        "right" => MouseButton::Right,
        _ => return Err("button must be left, middle or right".into()),
    };
    let count = if double {
        2
    } else {
        options
            .get("clickCount")
            .and_then(Value::as_i64)
            .unwrap_or(1)
    };
    if !(1..=100).contains(&count) {
        return Err("clickCount must be between 1 and 100".into());
    }
    let mut bits = 0;
    if let Some(modifiers) = options.get("modifiers") {
        for modifier in string_list(modifiers, "modifiers")? {
            bits |= match modifier.as_str() {
                "Alt" => 1,
                "Control" => 2,
                "Meta" => 4,
                "Shift" => 8,
                "ControlOrMeta" => {
                    if cfg!(target_os = "macos") {
                        4
                    } else {
                        2
                    }
                }
                _ => return Err("unknown keyboard modifier".into()),
            };
        }
    }
    Ok((button, count, bits))
}

fn platform_key(key: &str) -> String {
    key.replace(
        "ControlOrMeta",
        if cfg!(target_os = "macos") {
            "Meta"
        } else {
            "Control"
        },
    )
}
fn number(value: Option<&Value>, name: &str) -> Result<f64, CoreError> {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or_else(|| CoreError::from(format!("{name} must be a finite number")))
}
fn string_list(value: &Value, name: &str) -> Result<Vec<String>, CoreError> {
    if let Some(value) = value.as_str() {
        return Ok(vec![value.to_string()]);
    }
    value
        .as_array()
        .ok_or_else(|| CoreError::from(format!("{name} must be a string or array")))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| CoreError::from(format!("{name} entries must be strings")))
        })
        .collect()
}

async fn type_with_delay(
    session: &ProtocolSession,
    text: &str,
    options: &Value,
) -> Result<(), CoreError> {
    let delay = options.get("delay").and_then(Value::as_u64).unwrap_or(0);
    if delay == 0 {
        return keyboard::type_text(session, text).await;
    }
    for ch in text.chars() {
        keyboard::type_text(session, &ch.to_string()).await?;
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    Ok(())
}

async fn device_action(
    bridge: &BrowserBridge,
    page: PageId,
    method: &str,
    args: &[Value],
) -> Result<(), CoreError> {
    let session = bridge
        .ctx
        .session
        .pages
        .get_session(page.clone())
        .await?
        .session;
    let input = bridge.ctx.session.input(page).await;
    let options = args.get(option_index(method)).unwrap_or(&Value::Null);
    match method {
        "keyboard.press" => {
            input
                .press(&platform_key(
                    string_arg(args, 1, "key").map_err(CoreError::from)?,
                ))
                .await?
        }
        "keyboard.type" => {
            let text = string_arg(args, 1, "text").map_err(CoreError::from)?;
            if options.get("delay").and_then(Value::as_u64).unwrap_or(0) == 0 {
                input.type_text(text).await?;
            } else {
                type_with_delay(&session, text, options).await?;
            }
        }
        "keyboard.insertText" => {
            session
                .send_value(
                    "Input.insertText",
                    json!({"text":string_arg(args,1,"text").map_err(CoreError::from)?}),
                )
                .await?;
        }
        "mouse.click" => {
            let (button, count, modifiers) = pointer_options(options, false)?;
            for click in 1..=count {
                mouse::dispatch_click(
                    &session,
                    number(args.get(1), "x")?,
                    number(args.get(2), "y")?,
                    button,
                    click,
                    modifiers,
                )
                .await?;
            }
        }
        "mouse.move" => {
            mouse::dispatch_hover(
                &session,
                number(args.get(1), "x")?,
                number(args.get(2), "y")?,
            )
            .await?
        }
        "mouse.wheel" => {
            mouse::dispatch_scroll(
                &session,
                0.0,
                0.0,
                number(args.get(1), "deltaX")?,
                number(args.get(2), "deltaY")?,
            )
            .await?
        }
        _ => return Err(format!("not available in BrowserOS neo: {method}").into()),
    }
    Ok(())
}

pub(super) fn evaluated_value(response: Value) -> Result<Value, CoreError> {
    if let Some(exception) = response.get("exceptionDetails") {
        return Err(exception
            .pointer("/exception/description")
            .or_else(|| exception.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("JavaScript evaluation failed")
            .to_string()
            .into());
    }
    Ok(response
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

async fn evaluate_page(
    bridge: &BrowserBridge,
    page: PageId,
    source: &str,
    arg: Option<Value>,
) -> Result<Value, CoreError> {
    let expression = match arg {
        Some(arg) => format!("((fn) => typeof fn === 'function' ? fn({arg}) : fn)(({source}))"),
        None => source.to_string(),
    };
    let session = bridge.ctx.session.pages.get_session(page).await?.session;
    evaluated_value(
        session
            .send_value(
                "Runtime.evaluate",
                json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
            )
            .await?,
    )
}

async fn select_options(
    bridge: &BrowserBridge,
    engine: &LocatorEngine,
    page: PageId,
    r: &Resolved,
    values: Value,
) -> Result<Value, CoreError> {
    let values = if values.is_null() {
        vec![]
    } else if let Some(array) = values.as_array() {
        array.clone()
    } else {
        vec![values]
    };
    let mut options = Vec::new();
    for value in values {
        match value {
            Value::String(value) => options.push(json!({"value":value})),
            value @ Value::Object(_)
                if value.get("value").and_then(Value::as_str).is_some()
                    || value.get("label").and_then(Value::as_str).is_some()
                    || value.get("index").and_then(Value::as_u64).is_some() =>
            {
                options.push(value)
            }
            _ => return Err("selectOption expects values, labels, or option indexes".into()),
        }
    }
    let page_session = bridge
        .ctx
        .session
        .pages
        .get_session(page.clone())
        .await?
        .session;
    let core_value = options
        .first()
        .filter(|_| options.len() == 1)
        .and_then(|v| v.get("value"))
        .and_then(Value::as_str);
    // Wait under the caller's outer Deadline when a site's options arrive later.
    loop {
        if let Some(value) = core_value
            && r.session.same_session(&page_session)
        {
            let present = engine.call_on(r, "(el, value) => { if (!el.options) throw new Error('Element is not a select'); return Array.from(el.options).some(o => o.value === value); }", json!(value)).await?;
            if present == true {
                let selected = bridge
                    .ctx
                    .session
                    .input(page.clone())
                    .await
                    .select_backend_node(r.backend_node_id, value)
                    .await?;
                return selected
                    .map(|value| json!([value]))
                    .ok_or_else(|| CoreError::from("Option was not selected"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        let selected = injected_call(r, "select", json!(options)).await?;
        if selected == "error:optionsnotfound" {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        if !selected.is_array() {
            return Err(format!("selectOption failed: {selected}").into());
        }
        return Ok(selected);
    }
}

async fn query(
    engine: &LocatorEngine,
    page: PageId,
    selector: &str,
    args: &[Value],
    dl: &Deadline,
    state: &mut ActionState,
) -> Result<Value, CoreError> {
    let what = string_arg(args, 2, "query").map_err(CoreError::from)?;
    if what == "count" {
        return Ok(json!(engine.count(page, selector).await?));
    }
    if what == "allTextContents" || what == "allInnerTexts" {
        let mut values = Vec::new();
        let nodes = engine.resolve_all(page, selector).await?;
        for r in &nodes {
            state.hold(r);
        }
        for r in nodes {
            values.push(
                engine
                    .call_on(
                        &r,
                        if what == "allTextContents" {
                            "el => el.textContent || ''"
                        } else {
                            "el => el.innerText"
                        },
                        Value::Null,
                    )
                    .await?,
            );
        }
        return Ok(json!(values));
    }
    let source = match what {
        "textContent" => "el => el.textContent",
        "innerText" => "el => el.innerText",
        "innerHTML" => "el => el.innerHTML",
        "inputValue" => {
            "el => { if (!['INPUT','TEXTAREA','SELECT'].includes(el.tagName)) throw new Error('Node is not an input, textarea or select'); return el.value; }"
        }
        "getAttribute" => "(el, name) => el.getAttribute(name)",
        "isVisible" | "isHidden" | "isEnabled" | "isChecked" | "isEditable" => "",
        "boundingBox" => {
            "el => { const r=el.getBoundingClientRect(); return r.width && r.height ? {x:r.x,y:r.y,width:r.width,height:r.height} : null; }"
        }
        "ariaSnapshot" => "",

        _ => return Err(format!("unknown locator query: {what}").into()),
    };
    let r = if what == "isVisible" || what == "isHidden" {
        let mut nodes = engine.resolve_all(page, selector).await?;
        for node in &nodes {
            state.hold(node);
        }
        if nodes.len() > 1 {
            return Err(format!("{} matches (strict)", nodes.len()).into());
        }
        match nodes.pop() {
            Some(r) => r,
            None => return Ok(json!(what == "isHidden")),
        }
    } else {
        let r = engine
            .resolve(page, selector, Strictness::Strict, None, dl)
            .await?;
        state.hold(&r);
        r
    };
    if what == "ariaSnapshot" {
        return injected_call(&r, "aria", Value::Null).await;
    }
    let state = match what {
        "isVisible" | "isHidden" => Some("visible"),
        "isEnabled" => Some("enabled"),
        "isChecked" => Some("checked"),
        "isEditable" => Some("editable"),
        _ => None,
    };
    let result = if let Some(state) = state {
        let result = injected_call(&r, "state", json!(state)).await?;
        if result.get("received").and_then(Value::as_str) == Some("error:notconnected")
            && what != "isHidden"
            && what != "isVisible"
        {
            return Err("Element is not attached to the DOM".into());
        }
        json!(
            result
                .get("matches")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        )
    } else {
        engine
            .call_on(&r, source, args.get(3).cloned().unwrap_or(Value::Null))
            .await?
    };
    Ok(if what == "isHidden" {
        json!(!result.as_bool().unwrap_or(false))
    } else {
        result
    })
}

async fn evaluate_all(
    bridge: &BrowserBridge,
    page: PageId,
    nodes: &[Resolved],
    source: &str,
    arg: Value,
    state: &mut ActionState,
) -> Result<Value, CoreError> {
    if nodes.is_empty() {
        return evaluate_page(bridge, page, &format!("({source})([], {arg})"), None).await;
    }
    let first = &nodes[0];
    if nodes
        .iter()
        .any(|r| !r.session.same_session(&first.session) || r.frame_id != first.frame_id)
    {
        return Err("evaluateAll requires elements in the same frame".into());
    }
    // Backend node ids cross worlds, object ids do not. Resolve the whole set
    // into the default main world before invoking fn once on an element array.
    let mut handles = Vec::new();
    for node in nodes {
        let resolved = first
            .session
            .send_value(
                "DOM.resolveNode",
                json!({"backendNodeId":node.backend_node_id}),
            )
            .await?;
        let id = resolved
            .pointer("/object/objectId")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::from("Could not resolve main-world element"))?
            .to_string();
        state.handles.push((first.session.clone(), id.clone()));
        handles.push(id);
    }
    let mut arguments = vec![json!({"value":arg})];
    arguments.extend(handles.iter().map(|id| json!({"objectId":id})));
    evaluated_value(first.session.send_value("Runtime.callFunctionOn",json!({"objectId":handles[0],"functionDeclaration":format!("function(arg, ...elements) {{ return ({source})(elements, arg); }}"),"arguments":arguments,"awaitPromise":true,"returnByValue":true})).await?)
}

async fn tool_action(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
) -> Result<PwCallOutcome, CoreError> {
    let (def, text): (ToolDef, bool) = match method {
        "page.screenshot" => (crate::tools::screenshot::definition(), false),
        "page.pdf" => (crate::tools::pdf::definition(), false),
        "neo.read" => (crate::tools::read::definition(), true),
        "neo.grep" => (crate::tools::grep::definition(), true),
        "neo.download" => (crate::tools::download::definition(), false),
        _ => return Err("unknown tool delegate".into()),
    };
    let mut options = args
        .get(1)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    options.remove("timeout");
    if method == "page.screenshot" {
        // The public screenshot tool optimizes for JPEG; Playwright defaults
        // to PNG, including captures with only {fullPage:true} supplied.
        let kind = options.remove("type").unwrap_or_else(|| json!("png"));
        options.insert("format".into(), kind);
    }
    options.insert(
        "page".into(),
        json!(page_arg(args).map_err(CoreError::from)?.0),
    );
    let result = execute_tool(&def, Value::Object(options), &bridge.ctx)
        .await
        .map_err(|e| CoreError::from(e.to_string()))?;
    let content = result
        .content
        .iter()
        .filter_map(|c| {
            if let rmcp::model::ContentBlock::Text(t) = c {
                Some(t.text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error {
        return Err(content.into());
    }
    // The standalone screenshot tool's structured value is only metadata; its
    // image lives in MCP content. Preserve the encoded payload across the JSON
    // bridge so the facade can expose bytes without loading Node's Buffer.
    let value = if method == "page.screenshot" {
        let image = result
            .content
            .iter()
            .find_map(|content| match content {
                rmcp::model::ContentBlock::Image(image) => Some(image),
                _ => None,
            })
            .ok_or_else(|| CoreError::from("screenshot did not return image data"))?;
        json!({"body":image.data,"base64Encoded":true})
    } else if text {
        Value::String(content)
    } else {
        result.structured_content.unwrap_or(Value::String(content))
    };
    Ok(PwCallOutcome {
        value: BrowserCallValue::Json(value),
        audit_args: None,
        created_page: None,
        secrets: Vec::new(),
    })
}
