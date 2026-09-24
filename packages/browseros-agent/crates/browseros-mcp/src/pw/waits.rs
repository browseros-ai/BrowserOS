//! Deadline-bound Playwright navigation and waits over the shared CDP connection.
//! Receivers are installed before commands that can emit events; page/DOM polling
//! fills gaps for already-completed states without starting a second browser client.

use super::{PwCallOutcome, deadline, opts};
use crate::{
    output_file::{create_download_output_dir, record_browser_output_file},
    tools::run::{BrowserBridge, BrowserCallValue},
};
use browseros_cdp::CdpEvent;
use browseros_core::{
    CoreError, PageId, ProtocolSession,
    locator::{Deadline, ElementState, Strictness},
};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::HashSet, future::Future, path::Path, time::Duration};
use tokio::{
    sync::broadcast,
    time::{Instant, sleep, sleep_until},
};

const POLL: Duration = Duration::from_millis(50);
const DEFAULT_TIMEOUT_MS: u64 = 10_000;

pub(crate) async fn dispatch(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
) -> Result<PwCallOutcome, String> {
    let index = match method {
        "page.goto" | "page.waitForLoadState" | "page.waitForURL" => 2,
        "page.waitForFunction" | "page.waitForEvent" | "locator.waitFor" => 3,
        _ => 1,
    };
    let options = opts(args, index);
    let dl = deadline(bridge, &options, DEFAULT_TIMEOUT_MS);
    let timeout_ms = options.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let log = match method {
        "page.waitForURL" => format!(
            "waiting for navigation to {} until {:?}",
            args.get(1).unwrap_or(&Value::Null),
            options.wait_until.as_deref().unwrap_or("load")
        ),
        "page.goto" => format!(
            "navigating to {}, waiting until {:?}",
            args.get(1).unwrap_or(&Value::Null),
            options.wait_until.as_deref().unwrap_or("load")
        ),
        "page.waitForEvent" => format!("waiting for event {}", args.get(1).unwrap_or(&Value::Null)),
        "locator.waitFor" => format!(
            "waiting for locator({}) to be {}",
            args.get(1).unwrap_or(&Value::Null),
            args.get(2).and_then(Value::as_str).unwrap_or("visible")
        ),
        _ => "waiting for the requested state".to_owned(),
    };
    bounded(
        &dl,
        method,
        timeout_ms,
        &log,
        dispatch_inner(bridge, method, args, &dl),
    )
    .await
}

/// This outer race also bounds session attachment and slow CDP commands, not only
/// polling sleeps. Dropping a wait drops its receiver; no listener task survives it.
async fn bounded<T>(
    dl: &Deadline,
    method: &str,
    ms: u64,
    log: &str,
    work: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        () = dl.cancel.cancelled() => Err(format!("{method}: cancelled")),
        () = sleep_until(dl.at) => Err(timeout_error(method, ms, log)),
        result = work => result.map_err(|error| {
            if error.contains("Timeout") || error.contains("timed out") {
                timeout_error(method, ms, log)
            } else if error.starts_with(method) { error } else { format!("{method}: {error}") }
        }),
    }
}

fn timeout_error(method: &str, ms: u64, log: &str) -> String {
    format!(
        "TimeoutError: {method}: Timeout {ms}ms exceeded.\n=========================== logs ===========================\n{log}"
    )
}

fn outcome(value: Value) -> PwCallOutcome {
    PwCallOutcome {
        value: BrowserCallValue::Json(value),
        audit_args: None,
        created_page: None,
        secrets: Vec::new(),
    }
}

fn text_arg(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("argument {index} must be a string"))
}

fn error(err: impl std::fmt::Display) -> String {
    err.to_string()
}

async fn dispatch_inner(
    bridge: &BrowserBridge,
    method: &str,
    args: &[Value],
    dl: &Deadline,
) -> Result<PwCallOutcome, String> {
    let page = args
        .first()
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .map(PageId)
        .ok_or("pageId must be an unsigned integer")?;
    if method == "locator.waitFor" {
        return locator_wait(bridge, page, args, dl).await.map(outcome);
    }
    if method == "page.dialog" {
        let accept = args
            .get(1)
            .and_then(Value::as_bool)
            .ok_or("accept must be a boolean")?;
        bridge
            .ctx
            .session
            .input(page.clone())
            .await
            .handle_dialog(accept, args.get(2).and_then(Value::as_str))
            .await
            .map_err(error)?;
        bridge.ctx.session.page_signals.clear_dialog(&page);
        return Ok(outcome(Value::Null));
    }
    // Subscribe before attachment/enabling domains, which can replay current state
    // or synchronously open a dialog. Each wait filters the shared stream by session.
    let mut events = bridge.ctx.session.cdp_events();
    if method == "page.waitForEvent"
        && matches!(args.get(1).and_then(Value::as_str), Some("popup" | "page"))
    {
        return popup(bridge, page, text_arg(args, 1)?, &mut events).await;
    }
    let session = bridge
        .ctx
        .session
        .pages
        .get_session(page.clone())
        .await
        .map_err(error)?
        .session;
    let value = match method {
        "page.goto"
        | "page.reload"
        | "page.goBack"
        | "page.goForward"
        | "page.waitForLoadState"
        | "page.waitForURL" => {
            navigation(bridge, page, &session, &mut events, method, args).await?
        }
        "page.waitForFunction" => function_wait(&session, args, dl).await?,
        "page.waitForEvent" => event_wait(bridge, page, &session, &mut events, args).await?,
        "page.frames" => {
            let tree = send(&session, "Page.getFrameTree", json!({})).await?;
            let mut frames = Vec::new();
            flatten_frames(&tree["frameTree"], None, &mut frames);
            Value::Array(frames)
        }
        _ => return Err(format!("unsupported wait method: {method}")),
    };
    Ok(outcome(value))
}

async fn send(session: &ProtocolSession, method: &str, params: Value) -> Result<Value, String> {
    session.send_value(method, params).await.map_err(error)
}

fn flatten_frames(tree: &Value, parent: Option<&str>, result: &mut Vec<Value>) {
    let frame = &tree["frame"];
    if let Some(id) = frame["id"].as_str() {
        result.push(json!({"frameId": id, "parentFrameId": parent, "name": frame["name"].as_str().unwrap_or_default(), "url": frame["url"]}));
        if let Some(children) = tree["childFrames"].as_array() {
            for child in children {
                flatten_frames(child, Some(id), result);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum LoadState {
    Commit,
    Dom,
    Load,
    Idle,
}
impl LoadState {
    fn parse(state: &str) -> Result<Self, String> {
        match state {
            "commit" => Ok(Self::Commit),
            "domcontentloaded" => Ok(Self::Dom),
            "load" => Ok(Self::Load),
            "networkidle" => Ok(Self::Idle),
            _ => Err(format!(
                "expected one of (load|domcontentloaded|networkidle|commit), got {state}"
            )),
        }
    }
    fn event(self) -> &'static str {
        match self {
            Self::Commit => "init",
            Self::Dom => "DOMContentLoaded",
            Self::Load => "load",
            Self::Idle => "networkIdle",
        }
    }
}

/// Tracks the main document incarnation, so a subframe or an old loader's load
/// event cannot complete a navigation. Readiness polling starts only after commit.
struct NavigationState {
    frame: String,
    loader: String,
    expected_loader: Option<String>,
    committed: bool,
    url: String,
    states: HashSet<String>,
    status: Option<Value>,
    requests: HashSet<String>,
}
impl NavigationState {
    fn new(tree: &Value) -> Result<Self, String> {
        let frame = &tree["frameTree"]["frame"];
        Ok(Self {
            frame: frame["id"]
                .as_str()
                .ok_or("Page.getFrameTree returned no main frame")?
                .to_owned(),
            loader: frame["loaderId"].as_str().unwrap_or_default().to_owned(),
            url: frame["url"].as_str().unwrap_or_default().to_owned(),
            expected_loader: None,
            committed: true,
            states: HashSet::new(),
            status: None,
            requests: HashSet::new(),
        })
    }
    fn observe(&mut self, event: &CdpEvent) -> Result<(), String> {
        let p = &event.params;
        match event.method.as_str() {
            "Page.frameNavigated" if p["frame"]["id"].as_str() == Some(&self.frame) => {
                let frame = &p["frame"];
                let loader = frame["loaderId"].as_str().unwrap_or_default();
                if let Some(expected) = &self.expected_loader
                    && loader != expected
                {
                    return Ok(());
                }
                if self.loader != loader {
                    self.states.clear();
                }
                self.loader = loader.to_owned();
                self.committed = true;
                self.url = frame["url"].as_str().unwrap_or_default().to_owned();
            }
            "Page.navigatedWithinDocument"
                if p["frameId"].as_str() == Some(&self.frame) && self.expected_loader.is_none() =>
            {
                self.committed = true;
                self.url = p["url"].as_str().unwrap_or_default().to_owned();
            }
            "Page.lifecycleEvent" if p["frameId"].as_str() == Some(&self.frame) => {
                let loader = p["loaderId"].as_str().unwrap_or_default();
                let expected = self.expected_loader.as_deref().unwrap_or(&self.loader);
                if loader != expected {
                    return Ok(());
                }
                if let Some(name) = p["name"].as_str() {
                    self.states.insert(name.to_owned());
                    // The requested loader's lifecycle proves it committed even if
                    // frameNavigated was delivered before the navigate response.
                    if self.expected_loader.is_some() {
                        self.committed = true;
                        self.loader = loader.to_owned();
                    }
                }
            }
            "Network.requestWillBeSent"
                if p["frameId"].as_str() == Some(&self.frame) && p["type"] == "Document" =>
            {
                if let Some(id) = p["requestId"].as_str() {
                    self.requests.insert(id.to_owned());
                }
            }
            "Network.responseReceived"
                if p["frameId"].as_str() == Some(&self.frame) && p["type"] == "Document" =>
            {
                if self
                    .expected_loader
                    .as_deref()
                    .is_none_or(|loader| p["loaderId"].as_str() == Some(loader))
                {
                    self.status = p["response"].get("status").cloned();
                }
            }
            "Network.loadingFailed"
                if p["requestId"]
                    .as_str()
                    .is_some_and(|id| self.requests.contains(id)) =>
            {
                return Err(p["errorText"]
                    .as_str()
                    .unwrap_or("net::ERR_FAILED")
                    .to_owned());
            }
            _ => {}
        }
        Ok(())
    }
    fn ready(&self, state: LoadState) -> bool {
        self.committed && (state == LoadState::Commit || self.states.contains(state.event()))
    }
}

async fn navigation(
    bridge: &BrowserBridge,
    page: PageId,
    session: &ProtocolSession,
    events: &mut broadcast::Receiver<CdpEvent>,
    method: &str,
    args: &[Value],
) -> Result<Value, String> {
    let is_wait = matches!(method, "page.waitForLoadState" | "page.waitForURL");
    let option_index = if matches!(
        method,
        "page.goto" | "page.waitForLoadState" | "page.waitForURL"
    ) {
        2
    } else {
        1
    };
    let options = opts(args, option_index);
    let state = LoadState::parse(if method == "page.waitForLoadState" {
        args.get(1).and_then(Value::as_str).unwrap_or("load")
    } else {
        options.wait_until.as_deref().unwrap_or("load")
    })?;
    if let Some(dialog) = bridge.ctx.session.page_signals.pending_dialog_line(&page) {
        return Err(dialog);
    }
    let tree = send(session, "Page.getFrameTree", json!({})).await?;
    let mut nav = NavigationState::new(&tree)?;
    send(
        session,
        "Page.setLifecycleEventsEnabled",
        json!({"enabled": true}),
    )
    .await?;
    if !is_wait {
        // Navigation's existing helper only checks readyState and treats its own
        // timeout as advisory. Send the same commands here, retaining loader IDs
        // and CDP errorText so Playwright's stronger completion contract survives.
        send(session, "Network.enable", json!({})).await?;
        // Preserve current lifecycle for same-document history/hash navigation;
        // a new loader clears it below, but a hash change does not fire load again.
        drain_events(events, session, |event| nav.observe(event))?;
        nav.committed = false;
        let reply = match method {
            "page.goto" => {
                send(session, "Page.navigate", json!({"url": text_arg(args, 1)?})).await?
            }
            "page.reload" => send(session, "Page.reload", json!({})).await?,
            _ => {
                let history = send(session, "Page.getNavigationHistory", json!({})).await?;
                let current = history["currentIndex"]
                    .as_i64()
                    .ok_or("missing navigation history index")?;
                let index = current + if method == "page.goBack" { -1 } else { 1 };
                let entry = usize::try_from(index)
                    .ok()
                    .and_then(|i| history["entries"].get(i));
                let Some(entry) = entry else {
                    return Ok(Value::Null);
                };
                send(
                    session,
                    "Page.navigateToHistoryEntry",
                    json!({"entryId": entry["id"]}),
                )
                .await?
            }
        };
        if let Some(message) = reply["errorText"].as_str().filter(|s| !s.is_empty()) {
            return Err(message.to_owned());
        }
        nav.expected_loader = reply["loaderId"].as_str().map(str::to_owned);
        if nav
            .expected_loader
            .as_deref()
            .is_some_and(|loader| loader != nav.loader)
        {
            nav.states.clear();
        }
    }
    let pattern = if method == "page.waitForURL" {
        Some(UrlFilter::parse(args.get(1).unwrap_or(&Value::Null))?)
    } else {
        None
    };
    let initial_loader = nav.loader.clone();
    loop {
        drain_events(events, session, |event| nav.observe(event))?;
        let info = bridge
            .ctx
            .session
            .pages
            .refresh(page.clone())
            .await
            .map_err(error)?
            .ok_or("Target page has been closed")?;
        let url_matches = pattern
            .as_ref()
            .map(|filter| filter.matches(&info.url, &json!({"url": info.url})))
            .transpose()?
            .unwrap_or(true);
        // Browser.getTabInfo may see a new URL before the frame event reaches
        // this receiver. Never combine its URL with the old document's load.
        if info.url != nav.url {
            let tree = send(session, "Page.getFrameTree", json!({})).await?;
            let frame = &tree["frameTree"]["frame"];
            if frame["url"].as_str() == Some(&info.url) {
                nav.observe(&CdpEvent {
                    method: "Page.frameNavigated".to_owned(),
                    params: json!({"frame": frame}),
                    session_id: session.session_id().cloned(),
                })?;
            }
        }
        if url_matches && info.url == nav.url && nav.ready(state) {
            return navigation_result(session, is_wait, &info.url, &info.title, nav.status).await;
        }
        // readyState covers waits started after load; for an initiated navigation
        // confirm the new loader first, otherwise the previous document is "complete".
        if !nav.committed {
            let tree = send(session, "Page.getFrameTree", json!({})).await?;
            let current = &tree["frameTree"]["frame"];
            let loader = current["loaderId"].as_str().unwrap_or_default();
            if nav
                .expected_loader
                .as_deref()
                .map_or(loader != initial_loader, |expected| expected == loader)
            {
                nav.loader = loader.to_owned();
                nav.committed = true;
                nav.url = current["url"].as_str().unwrap_or_default().to_owned();
            }
        }
        if url_matches && info.url == nav.url && nav.committed && state != LoadState::Idle {
            if state == LoadState::Commit {
                return navigation_result(session, is_wait, &info.url, &info.title, nav.status)
                    .await;
            }
            let ready = session
                .send_value(
                    "Runtime.evaluate",
                    json!({"expression": "document.readyState", "returnByValue": true}),
                )
                .await;
            match ready {
                Ok(value)
                    if value["result"]["value"] == "complete"
                        || (state == LoadState::Dom
                            && value["result"]["value"] == "interactive") =>
                {
                    return navigation_result(session, is_wait, &info.url, &info.title, nav.status)
                        .await;
                }
                Err(err) if !transient_context(&err) => return Err(error(err)),
                _ => {}
            }
        }
        if let Some(event) = next_event(events).await?
            && event.session_id.as_ref() == session.session_id()
        {
            nav.observe(&event)?;
        }
    }
}

async fn navigation_result(
    session: &ProtocolSession,
    is_wait: bool,
    url: &str,
    title: &str,
    status: Option<Value>,
) -> Result<Value, String> {
    if is_wait {
        return Ok(Value::Null);
    }
    // The browser tab model can publish its title after the document load event.
    // Read the committed document directly so goto does not return "about:blank"
    // as the title of a page that has already loaded. A dialog may block Runtime,
    // so metadata enrichment must not consume the entire navigation budget.
    let document = tokio::time::timeout(
        Duration::from_millis(250),
        session.send_value(
            "Runtime.evaluate",
            json!({
                "expression": "document.title", "returnByValue": true,
            }),
        ),
    )
    .await;
    let mut value = json!({"url": url, "title": title});
    if let Ok(Ok(document)) = document
        && let Some(title) = document["result"]["value"].as_str()
    {
        value["title"] = json!(title);
    }
    if let Some(status) = status {
        value["status"] = status;
    }
    Ok(value)
}

fn transient_context(err: &CoreError) -> bool {
    let text = err.to_string();
    text.contains("Execution context was destroyed")
        || text.contains("Cannot find context")
        || text.contains("Inspected target navigated")
}

fn drain_events(
    events: &mut broadcast::Receiver<CdpEvent>,
    session: &ProtocolSession,
    mut observe: impl FnMut(&CdpEvent) -> Result<(), String>,
) -> Result<(), String> {
    loop {
        match events.try_recv() {
            Ok(event) if event.session_id.as_ref() == session.session_id() => observe(&event)?,
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Empty) => return Ok(()),
            Err(err) => return Err(format!("CDP event stream interrupted: {err}")),
        }
    }
}

async fn next_event(
    events: &mut broadcast::Receiver<CdpEvent>,
) -> Result<Option<CdpEvent>, String> {
    tokio::select! {
        event = events.recv() => event.map(Some).map_err(|err| format!("CDP event stream interrupted: {err}")),
        () = sleep(POLL) => Ok(None),
    }
}

async fn function_wait(
    session: &ProtocolSession,
    args: &[Value],
    dl: &Deadline,
) -> Result<Value, String> {
    let source = text_arg(args, 1)?;
    let arg = args.get(2).unwrap_or(&Value::Null);
    let polling = args.get(3).and_then(|value| value.get("polling"));
    let interval = match polling {
        None | Some(Value::String(_)) if polling.is_none_or(|value| value == "raf") => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|ms| ms.is_finite() && *ms > 0.0)
                .ok_or("polling must be 'raf' or a positive interval")?,
        ),
        None => None,
    };
    if interval.is_none() {
        send(
            session,
            "Emulation.setFocusEmulationEnabled",
            json!({"enabled": true}),
        )
        .await?;
    }
    // Evaluate in the main world (no contextId); return a truthiness envelope so
    // JS objects and empty arrays remain truthy across JSON. Each host poll owns
    // at most one animation callback, with a timer that cancels it if frames stop.
    let evaluate = format!(
        "async () => {{ const predicate = ({source}); const value = await (typeof predicate === 'function' ? predicate({arg}) : predicate); return {{ matched: !!value, value: value === undefined ? null : value }}; }}"
    );
    loop {
        let budget_ms = dl
            .at
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(100);
        let expression = if interval.is_none() {
            format!(
                "new Promise((resolve, reject) => {{ const timer = setTimeout(() => {{ cancelAnimationFrame(frame); resolve({{matched:false}}); }}, {budget_ms}); const frame = requestAnimationFrame(() => {{ clearTimeout(timer); ({evaluate})().then(resolve, reject); }}); }})"
            )
        } else {
            format!("({evaluate})()")
        };
        match session.send_value("Runtime.evaluate", json!({"expression": expression, "returnByValue": true, "awaitPromise": true, "timeout": dl.at.saturating_duration_since(Instant::now()).as_secs_f64() * 1000.0})).await {
            Ok(reply) => {
                if let Some(exception) = reply.get("exceptionDetails") { return Err(exception["exception"]["description"].as_str().or_else(|| exception["text"].as_str()).unwrap_or("function evaluation failed").to_owned()); }
                if reply["result"]["value"]["matched"] == true { return Ok(reply["result"]["value"]["value"].clone()); }
            }
            Err(err) if transient_context(&err) => {},
            Err(err) => return Err(error(err)),
        }
        if let Some(ms) = interval {
            sleep(Duration::from_secs_f64((ms / 1000.0).min(30.0))).await;
        } else {
            tokio::task::yield_now().await;
        }
    }
}

async fn locator_wait(
    bridge: &BrowserBridge,
    page: PageId,
    args: &[Value],
    dl: &Deadline,
) -> Result<Value, String> {
    let selector = text_arg(args, 1)?;
    let state = args.get(2).and_then(Value::as_str).unwrap_or("visible");
    let engine = bridge.ctx.session.locator();
    let strict = if opts(args, 3).strict {
        Strictness::Strict
    } else {
        Strictness::First
    };
    match state {
        "attached" | "visible" => {
            let resolved = engine
                .resolve(
                    page,
                    selector,
                    strict,
                    (state == "visible").then_some(ElementState::Visible),
                    dl,
                )
                .await
                .map_err(error)?;
            let _ = resolved
                .session
                .send_value(
                    "Runtime.releaseObject",
                    json!({"objectId": resolved.object_id}),
                )
                .await;
        }
        "detached" | "hidden" => loop {
            let count = engine.count(page.clone(), selector).await.map_err(error)?;
            if count == 0 {
                break;
            }
            if strict == Strictness::Strict && count > 1 {
                return Err(format!(
                    "strict mode violation: locator({selector:?}) resolved to {count} elements"
                ));
            }
            if state == "hidden" {
                // Resolve against a short slice: a visible node may disappear while
                // the engine waits. Re-count between slices so missing means hidden.
                let slice = Deadline {
                    at: (Instant::now() + POLL).min(dl.at),
                    cancel: dl.cancel.clone(),
                };
                match engine
                    .resolve(
                        page.clone(),
                        selector,
                        strict,
                        Some(ElementState::Hidden),
                        &slice,
                    )
                    .await
                {
                    Ok(resolved) => {
                        let _ = resolved
                            .session
                            .send_value(
                                "Runtime.releaseObject",
                                json!({"objectId": resolved.object_id}),
                            )
                            .await;
                        break;
                    }
                    Err(err)
                        if err.to_string().to_lowercase().contains("timeout")
                            || err.to_string().contains("timed out") => {}
                    Err(err) => return Err(error(err)),
                }
            }
            sleep(POLL).await;
        },
        _ => {
            return Err(format!(
                "state must be attached, detached, visible or hidden; got {state}"
            ));
        }
    }
    Ok(Value::Null)
}

async fn popup(
    bridge: &BrowserBridge,
    page: PageId,
    kind: &str,
    events: &mut broadcast::Receiver<CdpEvent>,
) -> Result<PwCallOutcome, String> {
    let before = bridge.ctx.session.pages.list().await.map_err(error)?;
    let opener = before
        .iter()
        .find(|info| info.page_id == page)
        .map(|info| info.target_id.as_str().to_owned())
        .ok_or("Target page has been closed")?;
    let ids: HashSet<_> = before.into_iter().map(|info| info.page_id).collect();
    let mut targets = HashSet::new();
    // Discovery is root-scoped and idempotent. Do not disable it when this wait
    // ends: other concurrent waits and FrameRegistry share the same connection.
    bridge
        .ctx
        .session
        .cdp("Target.setDiscoverTargets", json!({"discover": true}), None)
        .await
        .map_err(error)?;
    loop {
        while let Ok(event) = events.try_recv() {
            if matches!(
                event.method.as_str(),
                "Target.targetCreated" | "Target.targetInfoChanged"
            ) {
                let target = &event.params["targetInfo"];
                if target["type"] == "page"
                    && (kind == "page" || target["openerId"].as_str() == Some(&opener))
                    && let Some(id) = target["targetId"].as_str()
                {
                    targets.insert(id.to_owned());
                }
            }
        }
        for info in bridge.ctx.session.pages.list().await.map_err(error)? {
            if !ids.contains(&info.page_id)
                && (kind == "page" || targets.contains(info.target_id.as_str()))
            {
                let mut result = outcome(json!(info.page_id.0));
                // The shared bridge claims/groups only outcomes carrying this flag;
                // returning a bare id would leave an agent-opened popup unowned.
                result.created_page = Some(info.page_id.0);
                return Ok(result);
            }
        }
        // Keep target events queued for the next pass while polling Browser tabs:
        // targetCreated can precede the fork exposing its corresponding tab id.
        sleep(POLL).await;
    }
}

async fn event_wait(
    bridge: &BrowserBridge,
    page: PageId,
    session: &ProtocolSession,
    events: &mut broadcast::Receiver<CdpEvent>,
    args: &[Value],
) -> Result<Value, String> {
    let kind = text_arg(args, 1)?;
    if kind == "download" {
        return download(bridge, session, events).await;
    }
    if !matches!(kind, "dialog" | "request" | "response" | "framenavigated") {
        return Err(format!("unsupported event: {kind}"));
    }
    let include_body = args
        .get(3)
        .and_then(|v| v.get("body"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // Metadata returns immediately at responseReceived. A later body request can
    // reuse this bridge leaf with {requestId} as filter and {body:true} as options;
    // no global response cache or eager buffering is needed across script calls.
    if kind == "response"
        && include_body
        && let Some(id) = args
            .get(2)
            .and_then(|v| v.get("requestId"))
            .and_then(Value::as_str)
    {
        return response_body(session, events, id).await;
    }
    let filter = UrlFilter::parse(args.get(2).unwrap_or(&Value::Null))?;
    if matches!(kind, "request" | "response") {
        // Enabled lazily on this page's session; never disable another wait's
        // Network stream. requestId is retained for on-demand getResponseBody.
        send(session, "Network.enable", json!({})).await?;
    }
    loop {
        if kind == "dialog"
            && let Some(dialog) = bridge.ctx.session.page_signals.pending_dialog(&page)
        {
            return Ok(
                json!({"type": dialog.kind, "message": dialog.message, "defaultValue": dialog.default_prompt.unwrap_or_default()}),
            );
        }
        let Some(event) = next_event(events).await? else {
            continue;
        };
        if event.session_id.as_ref() != session.session_id() {
            continue;
        }
        let p = &event.params;
        let mut value = match (kind, event.method.as_str()) {
            ("dialog", "Page.javascriptDialogOpening") => {
                json!({"type": p["type"], "message": p["message"], "defaultValue": p["defaultPrompt"].as_str().unwrap_or_default()})
            }
            ("framenavigated", "Page.frameNavigated") => {
                json!({"frameId": p["frame"]["id"], "parentFrameId": p["frame"]["parentId"], "name": p["frame"]["name"].as_str().unwrap_or_default(), "url": p["frame"]["url"]})
            }
            ("framenavigated", "Page.navigatedWithinDocument") => {
                json!({"frameId": p["frameId"], "url": p["url"]})
            }
            ("request", "Network.requestWillBeSent") => {
                json!({"url": p["request"]["url"], "status": Value::Null, "headers": p["request"]["headers"], "method": p["request"]["method"], "requestId": p["requestId"]})
            }
            ("response", "Network.responseReceived") => {
                json!({"url": p["response"]["url"], "status": p["response"]["status"], "headers": p["response"]["headers"], "requestId": p["requestId"]})
            }
            _ => continue,
        };
        if filter.matches(value["url"].as_str().unwrap_or_default(), &value)? {
            if kind == "response" && include_body {
                let id = value["requestId"]
                    .as_str()
                    .ok_or("response missing requestId")?;
                let body = response_body(session, events, id).await?;
                value["body"] = body["body"].clone();
                value["base64Encoded"] = body["base64Encoded"].clone();
            }
            return Ok(value);
        }
    }
}

async fn response_body(
    session: &ProtocolSession,
    events: &mut broadcast::Receiver<CdpEvent>,
    request_id: &str,
) -> Result<Value, String> {
    loop {
        match send(
            session,
            "Network.getResponseBody",
            json!({"requestId": request_id}),
        )
        .await
        {
            Ok(body) => return Ok(body),
            Err(err)
                if err.contains("No resource with given identifier")
                    || err.contains("No data found") => {}
            Err(err) => return Err(err),
        }
        // Response headers arrive before the body has finished. Polling also
        // covers a loadingFinished event that preceded this on-demand call.
        if let Some(event) = next_event(events).await?
            && event.session_id.as_ref() == session.session_id()
            && event.params["requestId"].as_str() == Some(request_id)
            && event.method == "Network.loadingFailed"
        {
            return Err(event.params["errorText"]
                .as_str()
                .unwrap_or("net::ERR_FAILED")
                .to_owned());
        }
    }
}

/// The drop guard restores download behavior even when the outer deadline or
/// run cancellation drops capture mid-await. Cleanup has its own short bound.
struct DownloadBehavior(Option<ProtocolSession>);
impl Drop for DownloadBehavior {
    fn drop(&mut self) {
        let Some(session) = self.0.take() else {
            return;
        };
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                Duration::from_millis(500),
                session.send_value("Page.setDownloadBehavior", json!({"behavior": "default"})),
            )
            .await;
        });
    }
}

async fn download(
    bridge: &BrowserBridge,
    session: &ProtocolSession,
    events: &mut broadcast::Receiver<CdpEvent>,
) -> Result<Value, String> {
    let directory = create_download_output_dir().await.map_err(error)?;
    let mut reset = DownloadBehavior(Some(session.clone()));
    send(
        session,
        "Page.setDownloadBehavior",
        json!({"behavior": "allow", "downloadPath": directory}),
    )
    .await?;
    let mut selected: Option<(String, String)> = None;
    loop {
        let Some(event) = next_event(events).await? else {
            continue;
        };
        if event.session_id.as_ref() != session.session_id() {
            continue;
        }
        let p = &event.params;
        match event.method.as_str() {
            "Page.downloadWillBegin" if selected.is_none() => {
                let guid = p["guid"].as_str().ok_or("download missing guid")?;
                let name = p["suggestedFilename"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or("download missing suggested filename")?;
                // The browser chooses a filename, but never allow that string to
                // grant output-file access outside our dedicated download directory.
                if Path::new(name).file_name().and_then(|name| name.to_str()) != Some(name)
                    || name.contains('\\')
                    || matches!(name, "." | "..")
                {
                    return Err("invalid download filename".to_owned());
                }
                selected = Some((guid.to_owned(), name.to_owned()));
            }
            "Page.downloadProgress" => {
                let Some((guid, filename)) = &selected else {
                    continue;
                };
                if p["guid"].as_str() != Some(guid) {
                    continue;
                }
                match p["state"].as_str() {
                    Some("completed") => {
                        let path = directory.join(filename);
                        record_browser_output_file(&bridge.ctx.output_files, path.clone()).await;
                        // Finish ordinary cleanup before another sequential wait
                        // can install its directory. Drop still handles cancellation.
                        send(
                            session,
                            "Page.setDownloadBehavior",
                            json!({"behavior": "default"}),
                        )
                        .await?;
                        reset.0 = None;
                        return Ok(json!({"path": path, "suggestedFilename": filename}));
                    }
                    Some("canceled") => return Err("Download was canceled".to_owned()),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// URL strings are whole-URL globs; regular expressions and predicate source
/// execute in a separate, bounded QuickJS context, never in the inspected page.
/// This keeps JS regex semantics and prevents filters from reading page secrets.
enum UrlFilter {
    Any,
    Glob(Regex),
    Js(String),
}
impl UrlFilter {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::Any);
        }
        if let Some(source) = value.get("source").and_then(Value::as_str) {
            let flags = value
                .get("flags")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Ok(Self::Js(format!(
                "new RegExp({}, {}).test(event.url)",
                json!(source),
                json!(flags)
            )));
        }
        if let Some(source) = value.get("predicate").and_then(Value::as_str) {
            return Ok(Self::predicate(source));
        }
        let source = value
            .as_str()
            .ok_or("URL filter must be a string, RegExp or predicate")?;
        if source.is_empty() {
            return Ok(Self::Any);
        }
        if let Some(tail) = source.strip_prefix('/')
            && let Some(end) = tail.rfind('/')
        {
            let (pattern, flags) = tail.split_at(end);
            if flags[1..].chars().all(|c| "dgimsuvy".contains(c)) {
                return Ok(Self::Js(format!(
                    "new RegExp({}, {}).test(event.url)",
                    json!(pattern),
                    json!(&flags[1..])
                )));
            }
        }
        if source.contains("=>") || source.trim_start().starts_with("function") {
            return Ok(Self::predicate(source));
        }
        Ok(Self::Glob(Regex::new(&glob_regex(source)).map_err(error)?))
    }
    fn predicate(source: &str) -> Self {
        Self::Js(format!(
            "({source})({{...event, url: () => event.url, status: () => event.status, headers: () => event.headers, request: () => ({{url: () => event.url, method: () => event.method}}), method: () => event.method }})"
        ))
    }
    fn matches(&self, url: &str, value: &Value) -> Result<bool, String> {
        match self {
            Self::Any => Ok(true),
            Self::Glob(regex) => Ok(regex.is_match(url)),
            Self::Js(expression) => {
                let runtime = rquickjs::Runtime::new().map_err(error)?;
                runtime.set_memory_limit(2 * 1024 * 1024);
                runtime.set_max_stack_size(128 * 1024);
                let until = std::time::Instant::now() + Duration::from_millis(10);
                runtime.set_interrupt_handler(Some(Box::new(move || {
                    std::time::Instant::now() >= until
                })));
                let context = rquickjs::Context::full(&runtime).map_err(error)?;
                context.with(|ctx| ctx.eval::<bool, _>(format!("(() => {{ const event = {value}; const result = ({expression}); if (result && typeof result.then === 'function') throw new Error('async event predicates are not supported'); return !!result; }})()"))).map_err(|err| format!("invalid event predicate or regular expression: {err}"))
            }
        }
    }
}

fn glob_regex(glob: &str) -> String {
    let chars: Vec<_> = glob.chars().collect();
    let mut pattern = String::from("^");
    let mut i = 0;
    let mut groups = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                i += 1;
                pattern.push_str(&regex::escape(&chars[i].to_string()));
            }
            '*' => {
                let start = i;
                while i + 1 < chars.len() && chars[i + 1] == '*' {
                    i += 1;
                }
                if i > start
                    && (start == 0 || chars[start - 1] == '/')
                    && (i + 1 == chars.len() || chars[i + 1] == '/')
                {
                    pattern.push_str("(?:[^/]*(?:/|$))*");
                    if i + 1 < chars.len() {
                        i += 1;
                    }
                } else {
                    pattern.push_str("[^/]*");
                }
            }
            '{' => {
                groups += 1;
                pattern.push_str("(?:");
            }
            '}' if groups > 0 => {
                groups -= 1;
                pattern.push(')');
            }
            ',' if groups > 0 => pattern.push('|'),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    pattern.push('$');
    pattern
}
