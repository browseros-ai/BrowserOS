//! End-to-end MCP dispatch over disposable protocol fixtures. These tests cross
//! the same capture/handler/script seams as hosts; they are not native acceptance.

use crate::{
    BrowserMcpService, BrowserToolDefaults, BrowserToolOptions, InnerCallHook, InnerCallRecord,
    StaticTargetStore, TargetStore, ToolCtx, ToolResult, catalog, execute_tool,
    output_file::create_browser_output_file_access,
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserProfileId, BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection, SessionId,
    TargetBinding, TargetIntent,
};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
struct Call {
    method: String,
    params: Value,
    session: Option<String>,
}

struct NativeFixture {
    profiles: Vec<Value>,
    windows: Vec<Value>,
    tabs: Vec<Value>,
    calls: Vec<Call>,
}

struct FixtureConnection {
    state: Mutex<NativeFixture>,
    events: broadcast::Sender<CdpEvent>,
}

fn profile(id: &str, name: &str, loaded: bool) -> Value {
    let mut value =
        json!({ "browserProfileId": id, "name": name, "directory": id, "loaded": loaded });
    if loaded {
        value["browserContextId"] = json!(format!("context-{id}"));
    }
    value
}

fn window(id: i64, profile: &str, kind: &str, active: bool) -> Value {
    json!({ "windowId": id, "windowType": kind, "bounds": {}, "isActive": active,
        "isVisible": true, "tabCount": 1, "browserProfileId": profile,
        "liveBrowserContextId": format!("context-{profile}"),
        "browserContextId": "legacy-label-must-not-route" })
}

fn tab(id: i64, window: i64, profile: &str) -> Value {
    json!({ "tabId": id, "targetId": format!("target-{id}"), "windowId": window,
        "url": format!("https://{profile}.example.test/{id}"), "title": profile,
        "isActive": true, "isLoading": false, "loadProgress": 1.0, "isPinned": false,
        "isHidden": false, "index": 0, "browserProfileId": profile,
        "liveBrowserContextId": format!("context-{profile}"),
        "browserContextId": "legacy-label-must-not-route" })
}

impl FixtureConnection {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        Arc::new(Self {
            events,
            state: Mutex::new(NativeFixture {
                profiles: vec![
                    profile("eng", "eng", true),
                    profile("sales", "sales", true),
                    profile("closed", "closed", false),
                    profile("duplicate-1", "duplicate", false),
                    profile("duplicate-2", "duplicate", false),
                ],
                // Global focus is sales; eng's own active normal window is 20.
                windows: vec![
                    window(10, "eng", "normal", false),
                    window(20, "eng", "normal", true),
                    window(30, "eng", "popup", true),
                    window(90, "sales", "normal", true),
                ],
                tabs: vec![
                    tab(101, 10, "eng"),
                    tab(102, 20, "eng"),
                    tab(103, 30, "eng"),
                    tab(201, 90, "sales"),
                ],
                calls: Vec::new(),
            }),
        })
    }

    fn change(&self, change: impl FnOnce(&mut NativeFixture)) {
        change(
            &mut self
                .state
                .lock()
                .unwrap_or_else(|err| panic!("fixture poisoned: {err}")),
        );
    }

    fn calls(&self) -> Vec<Call> {
        self.state
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
            .calls
            .clone()
    }

    fn reply(
        &self,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.state.lock().map_err(|err| CdpError::Protocol {
            code: -1,
            message: err.to_string(),
        })?;
        state.calls.push(Call {
            method: method.to_string(),
            params: params.clone(),
            session: session.map(ToString::to_string),
        });
        match method {
            "Browser.getProfiles" => Ok(json!({ "profiles": state.profiles })),
            "Browser.loadProfile" => {
                let id = params["browserProfileId"].as_str().unwrap_or_default();
                let Some(saved) = state
                    .profiles
                    .iter_mut()
                    .find(|profile| profile["browserProfileId"] == id)
                else {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "profile missing".to_string(),
                    });
                };
                let loaded = saved["loaded"] == false;
                saved["loaded"] = json!(true);
                saved["browserContextId"] = json!(format!("context-{id}"));
                Ok(json!({ "profile": saved, "loaded": loaded }))
            }
            "Browser.getWindows" => Ok(json!({ "windows": state.windows })),
            "Browser.getTabs" => Ok(json!({ "tabs": state.tabs })),
            "Browser.getTabInfo" => {
                let Some(tab) = state
                    .tabs
                    .iter()
                    .find(|tab| tab["tabId"] == params["tabId"])
                else {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "tab missing".to_string(),
                    });
                };
                Ok(json!({ "tab": tab }))
            }
            "Browser.getActiveTab" => Ok(
                json!({ "tab": state.tabs.iter().find(|tab| tab["windowId"] == params["windowId"]) }),
            ),
            "Browser.createWindow" => {
                let context = params["browserContextId"]
                    .as_str()
                    .unwrap_or("context-sales");
                let profile = context.strip_prefix("context-").unwrap_or("sales");
                let id = 1 + state
                    .windows
                    .iter()
                    .filter_map(|window| window["windowId"].as_i64())
                    .max()
                    .unwrap_or(0);
                let created = window(id, profile, "normal", false);
                state.windows.push(created.clone());
                Ok(json!({ "window": created }))
            }
            "Browser.createTab" => {
                let window_id = params["windowId"].as_i64().unwrap_or(90);
                let profile = state
                    .windows
                    .iter()
                    .find(|window| window["windowId"] == window_id)
                    .and_then(|window| window["browserProfileId"].as_str())
                    .unwrap_or("sales")
                    .to_string();
                let id = 1 + state
                    .tabs
                    .iter()
                    .filter_map(|tab| tab["tabId"].as_i64())
                    .max()
                    .unwrap_or(0);
                let mut created = tab(id, window_id, &profile);
                created["url"] = params["url"].clone();
                state.tabs.push(created.clone());
                Ok(json!({ "tab": created }))
            }
            "Browser.getTabGroups" => Ok(json!({ "groups": [
                { "groupId": "group-eng", "windowId": 20, "title": "eng", "color": "blue", "collapsed": false, "tabIds": [102] },
                { "groupId": "group-sales", "windowId": 90, "title": "sales", "color": "red", "collapsed": false, "tabIds": [201] }
            ] })),
            "History.getRecent" => Ok(json!({ "entries": [] })),
            "Storage.getCookies" => {
                Ok(json!({ "cookies": [], "routedContext": params.get("browserContextId") }))
            }
            "Target.getTargets" => {
                let mut targets = state.tabs.iter().map(|tab| json!({ "targetId": tab["targetId"], "type": "page", "title": tab["title"], "url": tab["url"], "attached": false, "browserContextId": tab["liveBrowserContextId"] })).collect::<Vec<_>>();
                for (id, profile) in [
                    ("sw-eng-bad", "eng"),
                    ("sw-eng-good", "eng"),
                    ("sw-sales", "sales"),
                ] {
                    targets.push(json!({ "targetId": id, "type": "service_worker", "url": format!("chrome-extension://{id}/worker.js"), "title": id, "attached": false, "browserContextId": format!("context-{profile}") }));
                }
                Ok(json!({ "targetInfos": targets }))
            }
            "Target.attachToTarget" => Ok(
                json!({ "sessionId": format!("session-{}", params["targetId"].as_str().unwrap_or_default()) }),
            ),
            "Runtime.evaluate" => {
                let session = session.map(SessionId::as_str).unwrap_or_default();
                let value = if session.contains("sw-eng-bad") {
                    json!({ "error": "downloads unavailable" })
                } else if session.contains("sw-eng-good") {
                    json!({ "records": [{ "id": 1, "filename": "/synthetic/download.txt", "state": "complete", "bytes": 12 }] })
                } else {
                    json!(session)
                };
                Ok(json!({ "result": { "value": value } }))
            }
            "Page.getFrameTree" => Ok(
                json!({ "frameTree": { "frame": { "id": "frame-1", "loaderId": "loader-1", "url": "https://eng.example.test", "domainAndRegistry": "example.test", "securityOrigin": "https://eng.example.test", "mimeType": "text/html" } } }),
            ),
            "Page.enable"
            | "DOM.enable"
            | "Runtime.enable"
            | "Accessibility.enable"
            | "Runtime.runIfWaitingForDebugger"
            | "Target.setAutoAttach"
            | "Target.detachFromTarget"
            | "Page.reload"
            | "Browser.closeTab"
            | "Browser.closeWindow" => Ok(json!({})),
            _ => Err(CdpError::Protocol {
                code: -1,
                message: format!("unexpected fixture call: {method}"),
            }),
        }
    }
}

impl CdpConnection for FixtureConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(method, params, session) })
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
            Ok(self.reply(method, params, session)?.to_string())
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}

fn intent(profile: &str) -> TargetIntent {
    TargetIntent {
        browser_profile_id: BrowserProfileId(profile.to_string()),
        directory: profile.to_string(),
        name: profile.to_string(),
        window: None,
    }
}

async fn ctx(
    session: Arc<BrowserSession>,
    store: Arc<StaticTargetStore>,
) -> anyhow::Result<ToolCtx> {
    let binding = store.get().await.map_err(anyhow::Error::msg)?;
    Ok(ToolCtx::new(BrowserToolOptions {
        browser: BrowserView::capture(session, binding, None).await,
        target: Some(store),
        defaults: BrowserToolDefaults::default(),
        cancel: CancellationToken::new(),
        output_files: create_browser_output_file_access(),
        inner_call_hook: None,
        preloaded_helpers: Vec::new(),
    }))
}

async fn call(ctx: &ToolCtx, name: &str, args: Value) -> anyhow::Result<ToolResult> {
    let def = catalog()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| anyhow::anyhow!("tool missing: {name}"))?;
    Ok(execute_tool(&def, args, ctx).await?)
}

fn text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text())
        .map(|content| content.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn data(result: ToolResult) -> anyhow::Result<Value> {
    anyhow::ensure!(!result.is_error, "{}", text(&result));
    result
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("structured response missing"))
}

#[tokio::test]
async fn standalone_selection_load_and_window_creation_are_separate_and_persist()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let service = BrowserMcpService::new(super::tests::service_options(session));
    let call = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let listed = call("browser_profiles", json!({})).await?;
    assert!(listed.structured_content.as_ref().is_some_and(|data| {
        data["profiles"].as_array().is_some_and(|profiles| {
            profiles
                .iter()
                .any(|profile| profile["name"] == "closed" && profile["loaded"] == false)
        })
    }));
    let selected = call(
        "browser_target",
        json!({ "action": "set", "name": "closed" }),
    )
    .await?;
    assert_eq!(selected.is_error, Some(false));
    assert!(!connection.calls().iter().any(|call| matches!(
        call.method.as_str(),
        "Browser.loadProfile" | "Browser.createWindow" | "Browser.createTab"
    )));
    let unloaded = call("history", json!({})).await?;
    assert_eq!(unloaded.is_error, Some(true));
    assert!(
        unloaded
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .any(|content| content.text.contains("profile_not_loaded"))
    );
    let loaded = call("browser_profiles", json!({ "action": "load" })).await?;
    assert_eq!(
        loaded
            .structured_content
            .as_ref()
            .map(|data| &data["loaded"]),
        Some(&json!(true))
    );
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Browser.createWindow")
    );
    let empty = call("windows", json!({})).await?;
    assert_eq!(
        empty.structured_content.as_ref().map(|data| &data["count"]),
        Some(&json!(0))
    );
    let created = call("windows", json!({ "action": "create" })).await?;
    assert_eq!(created.is_error, Some(false));
    let created = created
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("window response missing"))?;
    assert_eq!(created["window"]["browserProfileId"], json!("closed"));
    assert_eq!(created["window"]["isActive"], json!(false));
    let opened = call(
        "tabs",
        json!({ "action": "new", "url": "https://closed.example.test" }),
    )
    .await?;
    assert_eq!(opened.is_error, Some(false));
    let calls = connection.calls();
    let native_open = calls
        .iter()
        .find(|call| call.method == "Browser.createTab")
        .ok_or_else(|| anyhow::anyhow!("tab was not created"))?;
    assert_eq!(
        native_open.params["browserContextId"],
        json!("context-closed")
    );
    assert_eq!(
        native_open.params["windowId"],
        created["window"]["windowId"]
    );
    let status = call("browser_target", json!({})).await?;
    assert_eq!(
        status
            .structured_content
            .as_ref()
            .map(|data| &data["binding"]["revision"]),
        Some(&json!(1))
    );
    call("browser_target", json!({ "action": "clear" })).await?;
    let unbound = call("browser_target", json!({})).await?;
    assert_eq!(
        unbound
            .structured_content
            .as_ref()
            .map(|data| &data["bound"]),
        Some(&json!(false))
    );
    Ok(())
}

#[tokio::test]
async fn granular_and_script_paths_project_normal_windows_after_global_reconciliation()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let global = session.pages.list().await?;
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let ctx = ctx(session.clone(), store.clone()).await?;
    let tabs = data(call(&ctx, "tabs", json!({})).await?)?;
    assert_eq!(tabs["pages"].as_array().map(Vec::len), Some(2));
    assert_eq!(session.pages.known_page_ids().await.len(), global.len());
    let windows = data(call(&ctx, "windows", json!({})).await?)?;
    assert_eq!(windows["count"], json!(2));
    assert!(text(&call(&ctx, "windows", json!({})).await?).contains("profile eng"));
    let run = data(call(&ctx, "run", json!({ "code": "return { direct: await browser.pages.list(), tools: await browser.windows({ action: 'list' }), raw: await browser.cdp('Target.getTargets') };" })).await?)?;
    assert_eq!(run["value"]["direct"].as_array().map(Vec::len), Some(2));
    assert_eq!(run["value"]["tools"]["count"], json!(2));
    assert_eq!(
        run["value"]["raw"]["targetInfos"].as_array().map(Vec::len),
        Some(2)
    );
    for tab in global
        .iter()
        .filter(|tab| tab.tab_id.0 == 103 || tab.tab_id.0 == 201)
    {
        let rejected = call(
            &ctx,
            "evaluate",
            json!({ "page": tab.page_id.0, "code": "return 1" }),
        )
        .await?;
        assert!(rejected.is_error);
        assert!(text(&rejected).contains("page_out_of_scope"));
        let script = call(&ctx, "run", json!({ "code": format!("return await browser.page({}).cdp('Runtime.evaluate', '{{}}');", tab.page_id.0) })).await?;
        assert!(script.is_error);
        assert!(text(&script).contains("page_out_of_scope"));
    }
    assert_eq!(
        store
            .get()
            .await
            .map_err(anyhow::Error::msg)?
            .map(|binding| binding.intent.browser_profile_id),
        Some(BrowserProfileId("eng".to_string()))
    );
    Ok(())
}

/// Rebind after the first primitive has finished. Later primitives must keep
/// its captured view while a genuinely new request sees the new binding.
struct RebindHook {
    store: Arc<StaticTargetStore>,
    changed: AtomicBool,
}

impl InnerCallHook for RebindHook {
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn record<'a>(&'a self, _record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !self.changed.swap(true, Ordering::SeqCst) {
                self.store
                    .set(intent("sales"))
                    .await
                    .unwrap_or_else(|err| panic!("rebind failed: {err}"));
            }
        })
    }
    fn on_page_created<'a>(&'a self, _page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn direct_tool_and_root_page_raw_calls_share_capture_across_rebinding() -> anyhow::Result<()>
{
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let mut captured = ctx(session.clone(), store.clone()).await?;
    captured.inner_call_hook = Some(Arc::new(RebindHook {
        store: store.clone(),
        changed: AtomicBool::new(false),
    }));
    let output = data(call(&captured, "run", json!({ "code": "const listed = await browser.pages.list(); const page = await browser.open('https://eng.example.test/new'); const info = await page.info(); await browser.history({ maxResults: 2 }); const root = await browser.cdp('Storage.getCookies'); const local = await page.cdp('Runtime.evaluate', '{}'); return { listed, info, root, local };" })).await?)?;
    assert_eq!(output["value"]["info"]["browserProfileId"], json!("eng"));
    assert_eq!(output["value"]["info"]["windowId"], json!(20));
    assert_eq!(
        output["value"]["root"]["routedContext"],
        json!("context-eng")
    );
    assert!(
        output["value"]["local"]["result"]["value"]
            .as_str()
            .is_some_and(|session| session.starts_with("session-target-"))
    );
    let calls = connection.calls();
    for call in calls.iter().filter(|call| {
        matches!(
            call.method.as_str(),
            "Browser.createTab" | "History.getRecent" | "Storage.getCookies"
        )
    }) {
        assert_eq!(
            call.params["browserContextId"],
            json!("context-eng"),
            "{call:?}"
        );
    }
    let next = ctx(session, store).await?;
    let next_windows = data(call(&next, "windows", json!({})).await?)?;
    assert_eq!(next_windows["windows"][0]["windowId"], json!(90));
    assert_eq!(
        captured.browser.binding().map(|binding| binding.revision),
        Some(1)
    );
    assert_eq!(
        next.browser.binding().map(|binding| binding.revision),
        Some(2)
    );
    Ok(())
}

#[tokio::test]
async fn mutating_target_actions_in_run_reject_before_store_or_native_changes() -> anyhow::Result<()>
{
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session, store.clone()).await?;
    let read = data(
        call(
            &captured,
            "run",
            json!({ "code": "return await browser.target({ action: 'get' });" }),
        )
        .await?,
    )?;
    assert_eq!(read["value"]["bound"], json!(true));
    for args in [
        json!({ "action": "set", "name": "sales" }),
        json!({ "action": "set_window", "windowId": 90 }),
        json!({ "action": "clear_window" }),
        json!({ "action": "clear" }),
    ] {
        let result = call(
            &captured,
            "run",
            json!({ "code": format!("return await browser.target({args});") }),
        )
        .await?;
        assert!(result.is_error);
        assert!(text(&result).contains("target_change_in_run"));
    }
    assert_eq!(
        store
            .get()
            .await
            .map_err(anyhow::Error::msg)?
            .map(|binding| binding.revision),
        Some(1)
    );
    assert!(!connection.calls().iter().any(|call| matches!(
        call.method.as_str(),
        "Browser.createWindow" | "Browser.loadProfile"
    )));
    Ok(())
}

#[tokio::test]
async fn missing_ambiguous_and_invalid_selectors_preserve_binding_without_creation()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    let before = store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session, store.clone()).await?;
    for (name, code) in [
        ("missing", "profile_not_found"),
        ("duplicate", "profile_ambiguous"),
    ] {
        for (tool, action) in [("browser_target", "set"), ("browser_profiles", "load")] {
            let result = call(&captured, tool, json!({ "action": action, "name": name })).await?;
            assert!(result.is_error);
            assert!(text(&result).contains(code), "{}", text(&result));
        }
    }
    for args in [
        json!({ "action": "set" }),
        json!({ "action": "set", "name": "eng", "browserProfileId": "eng" }),
    ] {
        assert!(call(&captured, "browser_target", args).await?.is_error);
    }
    assert_eq!(store.get().await.map_err(anyhow::Error::msg)?, Some(before));
    assert!(!connection.calls().iter().any(|call| matches!(
        call.method.as_str(),
        "Browser.createTab" | "Browser.createWindow" | "Browser.loadProfile"
    )));
    Ok(())
}

#[tokio::test]
async fn management_recovers_closed_pin_and_deleted_profile_without_admitting_old_view()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    let unbound = ctx(session.clone(), store.clone()).await?;
    data(
        call(
            &unbound,
            "browser_target",
            json!({ "action": "set_window", "windowId": 20, "strict": true }),
        )
        .await?,
    )?;
    connection.change(|state| state.windows.retain(|window| window["windowId"] != 20));
    let stale = ctx(session.clone(), store.clone()).await?;
    let failed = call(&stale, "history", json!({})).await?;
    assert!(failed.is_error && text(&failed).contains("pinned_window_closed"));
    let status = data(call(&stale, "browser_target", json!({})).await?)?;
    assert!(
        status["status"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("pinned_window_closed"))
    );
    data(call(&stale, "browser_profiles", json!({})).await?)?;
    data(
        call(
            &stale,
            "browser_target",
            json!({ "action": "clear_window" }),
        )
        .await?,
    )?;
    assert!(call(&stale, "history", json!({})).await?.is_error);
    let recovered = ctx(session.clone(), store.clone()).await?;
    data(call(&recovered, "history", json!({})).await?)?;
    connection.change(|state| {
        state
            .profiles
            .retain(|profile| profile["browserProfileId"] != "eng")
    });
    let deleted = ctx(session.clone(), store.clone()).await?;
    assert!(text(&call(&deleted, "history", json!({})).await?).contains("profile_gone"));
    data(
        call(
            &deleted,
            "browser_target",
            json!({ "action": "set", "name": "sales" }),
        )
        .await?,
    )?;
    let rebound = ctx(session, store).await?;
    data(call(&rebound, "history", json!({})).await?)?;
    Ok(())
}

#[tokio::test]
async fn bound_raw_policy_and_wrong_window_group_defaults_fail_before_dispatch()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let pages = session.pages.list().await?;
    let page = pages
        .iter()
        .find(|page| page.tab_id.0 == 102)
        .ok_or_else(|| anyhow::anyhow!("eng page missing"))?
        .page_id
        .0;
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let mut captured = ctx(session.clone(), store.clone()).await?;
    for code in [
        "return await browser.cdp('Target.attachToTarget', { targetId: 'forbidden' });".to_string(),
        format!(
            "return await browser.page({page}).cdp('Storage.setCookies', '{{\"cookies\":[],\"browserContextId\":\"context-sales\"}}');"
        ),
        "return await browser.cdp('Storage.getCookies', { browserContextId: 'context-sales' });"
            .to_string(),
    ] {
        let result = call(&captured, "run", json!({ "code": code })).await?;
        assert!(
            result.is_error && text(&result).contains("target_scope_unsupported"),
            "{}",
            text(&result)
        );
    }
    let wrong = call(
        &captured,
        "windows",
        json!({ "action": "close", "windowId": 90 }),
    )
    .await?;
    assert!(wrong.is_error && text(&wrong).contains("window_out_of_scope"));
    captured.defaults.default_tab_group_id = Some("group-sales".to_string());
    let grouped = call(&captured, "tabs", json!({ "action": "new" })).await?;
    assert!(
        grouped.is_error && text(&grouped).contains("group_out_of_scope"),
        "{}",
        text(&grouped)
    );
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Browser.createTab"
                || call.method == "Browser.closeWindow"
                || (call.method == "Target.attachToTarget"
                    && call.params["targetId"] == "forbidden"))
    );
    store.clear().await.map_err(anyhow::Error::msg)?;
    let unbound = ctx(session, store).await?;
    data(call(&unbound, "run", json!({ "code": "return await browser.cdp('Target.attachToTarget', { targetId: 'forbidden' });" })).await?)?;
    assert!(connection.calls().iter().any(
        |call| call.method == "Target.attachToTarget" && call.params["targetId"] == "forbidden"
    ));
    Ok(())
}

#[tokio::test]
async fn downloads_adapter_evaluates_only_captured_extension_workers_and_detaches()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session, store).await?;
    let records = crate::downloads_api::read_recent_downloads(&captured.browser).await?;
    assert_eq!(records[0].filename, "/synthetic/download.txt");
    let calls = connection.calls();
    assert!(!calls.iter().any(
        |call| call.method == "Target.attachToTarget" && call.params["targetId"] == "sw-sales"
    ));
    for call in calls
        .iter()
        .filter(|call| call.method == "Runtime.evaluate")
    {
        assert!(
            call.session
                .as_ref()
                .is_some_and(|session| session.contains("sw-eng-"))
        );
    }
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == "Target.attachToTarget")
            .count(),
        calls
            .iter()
            .filter(|call| call.method == "Target.detachFromTarget")
            .count()
    );
    Ok(())
}

#[tokio::test]
async fn static_store_serializes_revisions_and_clear_does_not_reuse_them() -> anyhow::Result<()> {
    let store = StaticTargetStore::new();
    let (a, b) = tokio::join!(store.set(intent("eng")), store.set(intent("sales")));
    let a = a.map_err(anyhow::Error::msg)?;
    let b = b.map_err(anyhow::Error::msg)?;
    assert_ne!(a.revision, b.revision);
    assert_eq!(a.revision.max(b.revision), 2);
    store.clear().await.map_err(anyhow::Error::msg)?;
    assert!(store.get().await.map_err(anyhow::Error::msg)?.is_none());
    let next: TargetBinding = store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    assert_eq!(next.revision, 4);
    Ok(())
}

#[tokio::test]
async fn loading_inside_run_is_allowed_but_does_not_refresh_its_captured_context()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    store
        .set(intent("closed"))
        .await
        .map_err(anyhow::Error::msg)?;
    let captured = ctx(session.clone(), store.clone()).await?;
    let result = call(&captured, "run", json!({ "code": "await browser.profiles({ action: 'load' }); return await browser.cdp('Storage.getCookies');" })).await?;
    assert!(
        result.is_error && text(&result).contains("profile_not_loaded"),
        "{}",
        text(&result)
    );
    let calls = connection.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.method == "Browser.loadProfile")
            .count(),
        1
    );
    assert!(!calls.iter().any(|call| call.method == "Storage.getCookies"));
    let next = ctx(session, store).await?;
    let cookies = data(
        call(
            &next,
            "run",
            json!({ "code": "return await browser.cdp('Storage.getCookies');" }),
        )
        .await?,
    )?;
    assert_eq!(cookies["value"]["routedContext"], json!("context-closed"));
    Ok(())
}

struct ClosePinHook {
    connection: Arc<FixtureConnection>,
    closed: AtomicBool,
}

impl InnerCallHook for ClosePinHook {
    fn authorize<'a>(&'a self, _page: Option<u32>) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn record<'a>(&'a self, _record: InnerCallRecord<'a>) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !self.closed.swap(true, Ordering::SeqCst) {
                self.connection.change(|state| {
                    state.windows.retain(|window| window["windowId"] != 20);
                });
            }
        })
    }
    fn on_page_created<'a>(&'a self, _page: u32) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test]
async fn closing_a_strict_pin_during_run_never_uses_another_same_profile_window()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new(connection.clone(), BrowserSessionHooks::default());
    let store = Arc::new(StaticTargetStore::new());
    let unbound = ctx(session.clone(), store.clone()).await?;
    data(
        call(
            &unbound,
            "browser_target",
            json!({ "action": "set_window", "windowId": 20, "strict": true }),
        )
        .await?,
    )?;
    let mut captured = ctx(session, store.clone()).await?;
    captured.inner_call_hook = Some(Arc::new(ClosePinHook {
        connection: connection.clone(),
        closed: AtomicBool::new(false),
    }));
    let result = call(&captured, "run", json!({ "code": "await browser.pages.list(); return await browser.open('https://eng.example.test/after-close');" })).await?;
    assert!(
        result.is_error && text(&result).contains("pinned_window_closed"),
        "{}",
        text(&result)
    );
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Browser.createTab")
    );
    assert_eq!(
        store
            .get()
            .await
            .map_err(anyhow::Error::msg)?
            .map(|binding| binding.revision),
        Some(1)
    );
    Ok(())
}
