//! End-to-end MCP dispatch over disposable protocol fixtures. These tests cross
//! the same capture/handler/script seams as hosts; they are not native acceptance.

use crate::{
    BrowserMcpService, BrowserToolDefaults, BrowserToolOptions, InnerCallHook, InnerCallRecord,
    StaticTargetStore, TargetStore, ToolCtx, ToolResult, catalog, execute_tool,
    output_file::create_browser_output_file_access, response::ToolResponse,
};
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserProfileId, BrowserSession, BrowserSessionHooks, BrowserView, CdpConnection,
    NativeCapabilities, SessionId, TargetBinding, TargetIntent, WindowId, WindowSelection,
    pages::NewPageOptions,
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

type FixtureChange = Box<dyn FnOnce(&mut NativeFixture) + Send>;

struct NativeFixture {
    epoch: u64,
    profiles: Vec<Value>,
    windows: Vec<Value>,
    tabs: Vec<Value>,
    groups: Vec<Value>,
    calls: Vec<Call>,
    /// Applied under the reply lock once metrics are answered, so the change lands between
    /// a screenshot's viewport measurement and its capture without timing-dependent sleeps.
    after_layout_metrics: Option<FixtureChange>,
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

fn grouped_tab(id: i64, window: i64, profile: &str, group: &str) -> Value {
    let mut tab = tab(id, window, profile);
    tab["groupId"] = json!(group);
    tab
}

impl FixtureConnection {
    fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        Arc::new(Self {
            events,
            state: Mutex::new(NativeFixture {
                epoch: 1,
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
                    grouped_tab(102, 20, "eng", "group-eng"),
                    tab(103, 30, "eng"),
                    grouped_tab(201, 90, "sales", "group-sales"),
                ],
                groups: vec![
                    json!({ "groupId": "group-eng", "windowId": 20, "title": "eng", "color": "blue", "collapsed": false, "tabIds": [102] }),
                    json!({ "groupId": "group-sales", "windowId": 90, "title": "sales", "color": "red", "collapsed": false, "tabIds": [201] }),
                ],
                calls: Vec::new(),
                after_layout_metrics: None,
            }),
        })
    }

    /// Adds the browser's original profile (directory "Default", renamed "Work") with one
    /// normal window, as a capable native has. Fixtures without it model a deleted Default.
    fn with_default_profile(self: Arc<Self>) -> Arc<Self> {
        self.change(|state| {
            let mut original = profile("original", "Work", true);
            original["directory"] = json!("Default");
            state.profiles.push(original);
            state.windows.push(window(50, "original", "normal", false));
            state.tabs.push(tab(501, 50, "original"));
        });
        self
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
        expected_epoch: Option<u64>,
        method: &str,
        params: Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let mut state = self.state.lock().map_err(|err| CdpError::Protocol {
            code: -1,
            message: err.to_string(),
        })?;
        // Reconnect simulation uses this same lock. Check after acquiring it,
        // before publishing or applying dispatch; there is no awaited gap that
        // could send an admitted request to the replacement fixture epoch.
        if let Some(expected) = expected_epoch
            && expected != state.epoch
        {
            return Err(CdpError::ConnectionEpochChanged {
                expected,
                actual: state.epoch,
            });
        }
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
                // Like the native, a window created with a URL opens it as its first tab.
                if let Some(url) = params["url"].as_str() {
                    let tab_id = 1 + state
                        .tabs
                        .iter()
                        .filter_map(|tab| tab["tabId"].as_i64())
                        .max()
                        .unwrap_or(0);
                    let mut first = tab(tab_id, id, profile);
                    first["url"] = json!(url);
                    state.tabs.push(first);
                }
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
            "Browser.getTabGroups" => Ok(json!({ "groups": state.groups })),
            "Browser.addTabsToGroup" => {
                let group_id = params["groupId"].as_str().unwrap_or_default();
                let tab_ids = params["tabIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .collect::<Vec<_>>();
                let Some(group) = state
                    .groups
                    .iter_mut()
                    .find(|group| group["groupId"] == group_id)
                else {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "group missing".to_string(),
                    });
                };
                let members = group["tabIds"]
                    .as_array_mut()
                    .ok_or_else(|| CdpError::Protocol {
                        code: -1,
                        message: "fixture group members missing".to_string(),
                    })?;
                for id in &tab_ids {
                    if !members.contains(&json!(id)) {
                        members.push(json!(id));
                    }
                }
                let updated_group = group.clone();
                for tab in &mut state.tabs {
                    if tab["tabId"]
                        .as_i64()
                        .is_some_and(|id| tab_ids.contains(&id))
                    {
                        tab["groupId"] = json!(group_id);
                    }
                }
                Ok(json!({ "group": updated_group }))
            }
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
            "Page.getLayoutMetrics" => {
                if let Some(change) = state.after_layout_metrics.take() {
                    change(&mut state);
                }
                Ok(json!({
                    "layoutViewport": { "pageX": 0, "pageY": 0, "clientWidth": 2560, "clientHeight": 1600 },
                    "cssLayoutViewport": { "pageX": 3, "pageY": 4, "clientWidth": 1280, "clientHeight": 800 }
                }))
            }
            "Page.captureScreenshot" => Ok(json!({ "data": "cG5n" })),
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
        Box::pin(async move { self.reply(None, method, params, session) })
    }
    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(Some(expected_epoch), method, params, session) })
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
            Ok(self.reply(None, method, params, session)?.to_string())
        })
    }
    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params).map_err(|err| CdpError::Protocol {
                code: -1,
                message: err.to_string(),
            })?;
            Ok(self
                .reply(Some(expected_epoch), method, params, session)?
                .to_string())
        })
    }
    fn events(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
            .epoch
    }
}

#[tokio::test]
async fn fixture_epoch_dispatch_rechecks_before_recording_or_mutating() -> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let expected = connection.connection_epoch();
    let params = json!({ "windowId": 20, "url": "https://eng.example.test/epoch" });
    let raw_params = params.to_string();
    // Both dispatch futures were prepared on the original connection. Changing
    // its epoch before polling must reject even if the IDs are still reusable.
    let decoded = connection.send_at_epoch(expected, "Browser.createTab", params.clone(), None);
    let raw = connection.send_raw_json_at_epoch(expected, "Browser.createTab", &raw_params, None);
    connection.change(|state| state.epoch += 1);
    assert!(matches!(
        decoded.await,
        Err(CdpError::ConnectionEpochChanged {
            expected: 1,
            actual: 2
        })
    ));
    assert!(matches!(
        raw.await,
        Err(CdpError::ConnectionEpochChanged {
            expected: 1,
            actual: 2
        })
    ));
    assert!(connection.calls().is_empty());
    connection.change(|state| assert_eq!(state.tabs.len(), 4));

    let current = connection.connection_epoch();
    connection
        .send_at_epoch(current, "Browser.createTab", params, None)
        .await?;
    let created = connection
        .send_raw_json_at_epoch(current, "Browser.createTab", &raw_params, None)
        .await?;
    assert!(serde_json::from_str::<Value>(&created)?["tab"].is_object());
    assert_eq!(connection.calls().len(), 2);
    connection.change(|state| assert_eq!(state.tabs.len(), 6));
    Ok(())
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
async fn target_output_schema_preserves_full_readback_through_content_gate() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let mut options = super::tests::service_options(session);
    // Hosts suppress legacy structured responses on the wire. Declared output
    // schemas are the existing exception that must retain management readbacks.
    options.include_structured_content = Some(false);
    let service = BrowserMcpService::new(options);
    let call = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let get = call("browser_target", json!({})).await?;
    assert_eq!(get.is_error, Some(false));
    // No stored selection: the readback names the original Default profile in use.
    assert_eq!(
        get.structured_content,
        Some(json!({ "action": "get", "bound": false, "default": {
            "directory": "Default",
            "status": {
                "profile": { "browserProfileId": "original", "name": "Work", "directory": "Default",
                    "loaded": true, "browserContextId": "context-original" },
                "windows": [50], "placementWindowId": 50, "error": null
            }
        }}))
    );

    let initial =
        json!({ "browserProfileId": "eng", "name": "eng", "directory": "eng", "revision": 1 });
    let selected = call("browser_target", json!({ "action": "set", "name": "eng" })).await?;
    assert_eq!(
        selected.structured_content,
        Some(json!({ "action": "set", "binding": initial }))
    );
    let working = json!({ "browserProfileId": "eng", "name": "eng", "directory": "eng", "revision": 2, "window": { "mode": "working", "windowId": 10 } });
    let changed = call(
        "browser_target",
        json!({ "action": "set_window", "windowId": 10 }),
    )
    .await?;
    assert_eq!(
        changed.structured_content,
        Some(json!({ "action": "set_window", "binding": working, "previous": initial }))
    );
    let get = call("browser_target", json!({})).await?;
    assert_eq!(get.is_error, Some(false));
    assert_eq!(
        get.structured_content,
        Some(json!({ "action": "get", "bound": true, "binding": working,
        "status": { "profile": profile("eng", "eng", true), "windows": [10, 20], "placementWindowId": 10, "error": null } }))
    );

    let pinned = json!({ "browserProfileId": "eng", "name": "eng", "directory": "eng", "revision": 3, "window": { "mode": "pinned", "windowId": 20 } });
    let changed = call(
        "browser_target",
        json!({ "action": "set_window", "windowId": 20, "strict": true }),
    )
    .await?;
    assert_eq!(
        changed.structured_content,
        Some(json!({ "action": "set_window", "binding": pinned, "previous": working }))
    );
    let get = call("browser_target", json!({})).await?;
    assert_eq!(
        get.structured_content,
        Some(json!({ "action": "get", "bound": true, "binding": pinned,
        "status": { "profile": profile("eng", "eng", true), "windows": [10, 20], "placementWindowId": 20, "error": null } }))
    );

    let cleared_window =
        json!({ "browserProfileId": "eng", "name": "eng", "directory": "eng", "revision": 4 });
    let cleared = call("browser_target", json!({ "action": "clear_window" })).await?;
    assert_eq!(
        cleared.structured_content,
        Some(json!({ "action": "clear_window", "binding": cleared_window, "previous": pinned }))
    );
    let tabs = call("tabs", json!({ "action": "list" })).await?;
    assert_eq!(tabs.is_error, Some(false));
    assert!(tabs.structured_content.is_none());
    let cleared = call("browser_target", json!({ "action": "clear" })).await?;
    assert_eq!(
        cleared.structured_content,
        Some(json!({ "action": "clear", "bound": false }))
    );
    Ok(())
}

#[tokio::test]
async fn profiles_output_schema_preserves_descriptors_and_load_outcomes() -> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let mut options = super::tests::service_options(session);
    options.include_structured_content = Some(false);
    let service = BrowserMcpService::new(options);
    let call = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let listed = call("browser_profiles", json!({})).await?;
    assert_eq!(listed.is_error, Some(false));
    assert_eq!(
        listed.structured_content,
        Some(json!({ "action": "list", "profiles": [
        profile("eng", "eng", true), profile("sales", "sales", true), profile("closed", "closed", false),
        profile("duplicate-1", "duplicate", false), profile("duplicate-2", "duplicate", false)
    ] }))
    );
    let selected = call(
        "browser_target",
        json!({ "action": "set", "name": "closed" }),
    )
    .await?;
    let before = selected
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("selection missing"))?["binding"]
        .clone();
    let listed = call("browser_profiles", json!({})).await?;
    let listed = listed
        .structured_content
        .ok_or_else(|| anyhow::anyhow!("profile list missing"))?;
    assert_eq!(listed["boundBrowserProfileId"], "closed");
    assert_eq!(listed["profiles"][2], profile("closed", "closed", false));
    let loaded = call("browser_profiles", json!({ "action": "load" })).await?;
    assert_eq!(
        loaded.structured_content,
        Some(
            json!({ "action": "load", "profile": profile("closed", "closed", true), "loaded": true })
        )
    );
    let loaded = call("browser_profiles", json!({ "action": "load" })).await?;
    assert_eq!(
        loaded.structured_content,
        Some(
            json!({ "action": "load", "profile": profile("closed", "closed", true), "loaded": false })
        )
    );
    let get = call("browser_target", json!({})).await?;
    assert_eq!(
        get.structured_content
            .ok_or_else(|| anyhow::anyhow!("readback missing"))?["binding"],
        before
    );
    assert!(!connection.calls().iter().any(|call| matches!(
        call.method.as_str(),
        "Browser.createWindow" | "Browser.createTab"
    )));
    Ok(())
}

#[tokio::test]
async fn standalone_selection_load_and_window_creation_are_separate_and_persist()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
async fn bound_raw_policy_and_explicit_window_group_conflicts_fail_before_dispatch()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let pages = session.pages.list().await?;
    let page = pages
        .iter()
        .find(|page| page.tab_id.0 == 102)
        .ok_or_else(|| anyhow::anyhow!("eng page missing"))?
        .page_id
        .0;
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session.clone(), store.clone()).await?;
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
    let grouped = call(
        &captured,
        "tabs",
        json!({ "action": "new", "groupId": "group-sales" }),
    )
    .await?;
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
    // Clearing the override returns later calls to the original Default profile, which keeps
    // the same scoped raw policy instead of opening browser-wide passthrough.
    connection.clone().with_default_profile();
    store.clear().await.map_err(anyhow::Error::msg)?;
    let defaulted = ctx(session.clone(), store).await?;
    let refused = call(&defaulted, "run", json!({ "code": "return await browser.cdp('Target.attachToTarget', { targetId: 'forbidden' });" })).await?;
    assert!(
        refused.is_error && text(&refused).contains("target_scope_unsupported"),
        "{}",
        text(&refused)
    );
    assert!(!connection.calls().iter().any(
        |call| call.method == "Target.attachToTarget" && call.params["targetId"] == "forbidden"
    ));
    // Global passthrough remains available to host infrastructure through an unbound view.
    let global = BrowserView::unbound(session, None);
    global
        .cdp(
            "Target.attachToTarget",
            json!({ "targetId": "forbidden" }),
            None,
        )
        .await?;
    assert!(connection.calls().iter().any(
        |call| call.method == "Target.attachToTarget" && call.params["targetId"] == "forbidden"
    ));
    Ok(())
}

/// The default viewport clip measures the page before capture, so both protocol calls must ride
/// the bound page capability. Out-of-scope pages are refused before either call; a target
/// incarnation replaced while the measurement is in flight fails the post-send check uncaptured.
#[tokio::test]
async fn bound_viewport_screenshot_measures_and_captures_only_through_guarded_page()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let pages = session.pages.list().await?;
    let page_for_tab = |tab: i64| {
        pages
            .iter()
            .find(|page| page.tab_id.0 == tab)
            .map(|page| page.page_id.0)
            .ok_or_else(|| anyhow::anyhow!("fixture page for tab {tab} missing"))
    };
    let (eng, sales) = (page_for_tab(102)?, page_for_tab(201)?);
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session, store).await?;

    data(call(&captured, "screenshot", json!({ "page": eng })).await?)?;
    let calls = connection.calls();
    let page_calls = calls
        .iter()
        .filter(|call| {
            call.method == "Page.getLayoutMetrics" || call.method == "Page.captureScreenshot"
        })
        .collect::<Vec<_>>();
    assert_eq!(page_calls.len(), 2, "{page_calls:?}");
    for call in &page_calls {
        assert_eq!(
            call.session.as_deref(),
            Some("session-target-102"),
            "{call:?}"
        );
    }
    assert_eq!(
        page_calls[1].params["clip"],
        json!({ "x": 3.0, "y": 4.0, "width": 1280.0, "height": 800.0, "scale": 1.0 })
    );

    let before = connection.calls().len();
    let foreign = call(
        &captured,
        "screenshot",
        json!({ "page": sales, "size": { "width": 640 } }),
    )
    .await?;
    assert!(
        foreign.is_error && text(&foreign).contains("page_out_of_scope"),
        "{}",
        text(&foreign)
    );
    assert!(
        !connection.calls()[before..]
            .iter()
            .any(|call| call.method.starts_with("Page.")),
        "an out-of-scope page must not be measured or captured"
    );

    connection.change(|state| {
        state.after_layout_metrics = Some(Box::new(|state| {
            for tab in &mut state.tabs {
                if tab["tabId"] == 102 {
                    tab["targetId"] = json!("target-102-replaced");
                }
            }
        }));
    });
    let before = connection.calls().len();
    let replaced = call(&captured, "screenshot", json!({ "page": eng })).await?;
    assert!(
        replaced.is_error && text(&replaced).contains("page_target_changed"),
        "{}",
        text(&replaced)
    );
    let after = &connection.calls()[before..];
    assert!(
        after
            .iter()
            .any(|call| call.method == "Page.getLayoutMetrics")
    );
    assert!(
        !after
            .iter()
            .any(|call| call.method == "Page.captureScreenshot"),
        "a page replaced during measurement must not be captured: {}",
        text(&replaced)
    );
    Ok(())
}

#[tokio::test]
async fn script_creation_default_group_does_not_restrict_explicit_working_window()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let store = Arc::new(StaticTargetStore::new());
    let mut selected = intent("eng");
    selected.window = Some(WindowSelection::Working {
        window_id: WindowId(20),
    });
    store.set(selected).await.map_err(anyhow::Error::msg)?;
    let mut captured = ctx(session, store).await?;
    captured.defaults.default_tab_group_id = Some("group-eng".to_string());

    // This is the production core guard, not a mocked group rejection. It must
    // keep rejecting an explicit A group for B while MCP drops inherited A.
    let rejected = captured
        .browser
        .new_page(
            "https://eng.example.test/guard",
            NewPageOptions {
                window_id: Some(WindowId(10)),
                tab_group_id: Some("group-eng".to_string()),
                ..NewPageOptions::default()
            },
        )
        .await;
    assert!(rejected.is_err_and(|error| error.to_string().contains("group_out_of_scope")));
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Browser.createTab")
    );

    data(call(&captured, "run", json!({ "code": "return await browser.pages.newPage('https://eng.example.test/window-b', { windowId: 10 });" })).await?)?;
    let calls = connection.calls();
    let created = calls
        .iter()
        .filter(|call| call.method == "Browser.createTab")
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].params["windowId"], json!(10));
    assert_eq!(created[0].params["browserContextId"], json!("context-eng"));
    assert!(
        !calls
            .iter()
            .any(|call| call.method == "Browser.addTabsToGroup")
    );

    let explicit = call(&captured, "run", json!({ "code": "return await browser.pages.newPage('https://eng.example.test/conflict', { windowId: 10, tabGroupId: 'group-eng' });" })).await?;
    assert!(
        explicit.is_error && text(&explicit).contains("group_out_of_scope"),
        "{}",
        text(&explicit)
    );
    assert_eq!(
        connection
            .calls()
            .iter()
            .filter(|call| call.method == "Browser.createTab")
            .count(),
        1
    );

    data(call(&captured, "run", json!({ "code": "return await browser.pages.newPage('https://eng.example.test/window-a');" })).await?)?;
    assert!(connection.calls().iter().any(
        |call| call.method == "Browser.addTabsToGroup" && call.params["groupId"] == "group-eng"
    ));
    Ok(())
}

#[tokio::test]
async fn granular_creation_default_group_checks_its_actual_window() -> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let mut captured = ctx(session, store).await?;
    captured.defaults.default_tab_group_id = Some("group-eng".to_string());
    assert_eq!(
        captured
            .browser
            .scope()?
            .and_then(|scope| scope.placement_window),
        Some(WindowId(20))
    );

    // A default can move after capture. Consult its actual native window;
    // comparing only the requested window to the old placement is insufficient.
    connection.change(|state| {
        state.groups[0]["windowId"] = json!(10);
        state
            .tabs
            .iter_mut()
            .find(|tab| tab["tabId"] == 102)
            .unwrap_or_else(|| panic!("grouped tab missing"))["windowId"] = json!(10);
    });
    data(
        call(
            &captured,
            "tabs",
            json!({ "action": "new", "url": "https://eng.example.test/granular" }),
        )
        .await?,
    )?;
    let calls = connection.calls();
    let created = calls
        .iter()
        .filter(|call| call.method == "Browser.createTab")
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].params["windowId"], json!(20));
    assert!(
        !calls
            .iter()
            .any(|call| call.method == "Browser.addTabsToGroup")
    );

    let explicit = call(
        &captured,
        "tabs",
        json!({ "action": "new", "groupId": "group-eng" }),
    )
    .await?;
    assert!(
        explicit.is_error && text(&explicit).contains("group_out_of_scope"),
        "{}",
        text(&explicit)
    );
    assert_eq!(
        connection
            .calls()
            .iter()
            .filter(|call| call.method == "Browser.createTab")
            .count(),
        1
    );

    data(call(&captured, "run", json!({ "code": "return await browser.pages.newPage('https://eng.example.test/moved-group', { windowId: 10 });" })).await?)?;
    assert!(connection.calls().iter().any(
        |call| call.method == "Browser.addTabsToGroup" && call.params["groupId"] == "group-eng"
    ));

    captured.defaults.default_tab_group_id = Some("group-sales".to_string());
    data(call(&captured, "tabs", json!({ "action": "new" })).await?)?;
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Browser.addTabsToGroup"
                && call.params["groupId"] == "group-sales")
    );
    Ok(())
}

#[tokio::test]
async fn downloads_adapter_evaluates_only_captured_extension_workers_and_detaches()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
async fn stale_response_readback_does_not_expose_global_dialog_text() -> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let page = session
        .pages
        .list()
        .await?
        .into_iter()
        .find(|page| page.tab_id.0 == 102)
        .ok_or_else(|| anyhow::anyhow!("fixture page missing"))?
        .page_id;
    let store = Arc::new(StaticTargetStore::new());
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let captured = ctx(session.clone(), store).await?;

    // Signals are shared infrastructure: losing admission must not drain or
    // disclose an existing dialog through an advisory response readback.
    let signal_session = SessionId::from("readback-signal-session");
    session
        .page_signals
        .attach_page(page.clone(), signal_session.clone());
    connection.events.send(CdpEvent {
        method: "Page.javascriptDialogOpening".to_string(),
        params: json!({ "type": "confirm", "message": "private dialog content" }),
        session_id: Some(signal_session),
    })?;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while session.page_signals.pending_dialog(&page).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    connection.change(|state| {
        state
            .profiles
            .retain(|profile| profile["browserProfileId"] != "eng");
    });

    let mut response = ToolResponse::new();
    response.include_snapshot(page.0);
    let result = response
        .build_for_session(&captured, Some(page.clone()))
        .await?
        .into_tool_result();
    let output = text(&result);
    assert!(output.contains("unavailable"), "{output}");
    assert!(!output.contains("private dialog content"), "{output}");
    assert!(session.page_signals.pending_dialog(&page).is_some());
    assert!(
        !connection
            .calls()
            .iter()
            .any(|call| call.method == "Accessibility.getFullAXTree")
    );
    Ok(())
}

#[tokio::test]
async fn loading_inside_run_is_allowed_but_does_not_refresh_its_captured_context()
-> anyhow::Result<()> {
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
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

fn older_native() -> NativeCapabilities {
    NativeCapabilities::from_browseros_version(Some("0.51.1.2"))
}

fn profile_operations(connection: &FixtureConnection) -> Vec<String> {
    connection
        .calls()
        .into_iter()
        .map(|call| call.method)
        .filter(|method| {
            matches!(
                method.as_str(),
                "Browser.getProfiles" | "Browser.loadProfile"
            )
        })
        .collect()
}

#[tokio::test]
async fn older_native_refuses_profile_tools_and_run_primitives_before_sending() -> anyhow::Result<()>
{
    for native in [older_native(), NativeCapabilities::default()] {
        let connection = FixtureConnection::new();
        let session = BrowserSession::new_with_native(
            connection.clone(),
            BrowserSessionHooks::default(),
            native,
        );
        let store = Arc::new(StaticTargetStore::new());
        let ctx = ctx(session, store.clone()).await?;
        // Direct calls stand in for a client still holding a list from a capable browser.
        for (name, args) in [
            ("browser_profiles", json!({})),
            (
                "browser_profiles",
                json!({ "action": "load", "name": "eng" }),
            ),
            ("browser_target", json!({ "action": "get" })),
            ("browser_target", json!({ "action": "set", "name": "eng" })),
            (
                "browser_target",
                json!({ "action": "set_window", "windowId": 20 }),
            ),
            ("browser_target", json!({ "action": "clear" })),
        ] {
            let result = call(&ctx, name, args.clone()).await?;
            assert!(result.is_error, "{native}: {name} {args}");
            assert!(
                text(&result).contains(
                    "native_feature_unavailable: profile targeting needs BrowserOS 0.51.1.3"
                ),
                "{native}: {}",
                text(&result)
            );
        }
        assert!(store.get().await.map_err(anyhow::Error::msg)?.is_none());

        // The SDK omits the exports, and the primitive behind them is refused in Rust even
        // when a script reaches it through the raw bridge.
        let run = data(
            call(
                &ctx,
                "run",
                json!({ "code": "const kinds = [typeof browser.profiles, typeof browser.target]; let refused = null; try { await __browserosCall('tool:browser_profiles', '[{}]', false); } catch (error) { refused = String(error.message || error); } const pages = await browser.pages.list(); return { kinds, refused, pages: pages.length };" }),
            )
            .await?,
        )?;
        assert_eq!(run["value"]["kinds"], json!(["undefined", "undefined"]));
        assert!(
            run["value"]["refused"]
                .as_str()
                .is_some_and(|message| message.contains("native_feature_unavailable")),
            "{run}"
        );
        // Ordinary unbound browsing still sees every tab.
        assert_eq!(run["value"]["pages"], json!(4));
        assert!(profile_operations(&connection).is_empty(), "{native}");
    }
    Ok(())
}

#[tokio::test]
async fn capable_native_exposes_run_profile_exports() -> anyhow::Result<()> {
    let session = BrowserSession::new_with_native(
        FixtureConnection::new().with_default_profile(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let ctx = ctx(session, Arc::new(StaticTargetStore::new())).await?;
    let run = data(
        call(
            &ctx,
            "run",
            json!({ "code": "const listed = await browser.profiles({ action: 'list' }); return { kinds: [typeof browser.profiles, typeof browser.target], count: listed.profiles.length };" }),
        )
        .await?,
    )?;
    assert_eq!(run["value"]["kinds"], json!(["function", "function"]));
    assert_eq!(run["value"]["count"], json!(6));
    Ok(())
}

#[test]
fn discovery_omits_profile_surfaces_for_older_natives() {
    let names = |native: NativeCapabilities| {
        catalog()
            .into_iter()
            .filter(|tool| tool.available(native))
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };
    let description = |name: &str, native: NativeCapabilities| {
        catalog()
            .into_iter()
            .find(|tool| tool.name == name)
            .map(|tool| {
                tool.to_mcp_tool(native)
                    .description
                    .unwrap_or_default()
                    .to_string()
            })
            .unwrap_or_default()
    };
    let current = crate::tests::current_native();
    assert!(names(current).contains(&"browser_profiles"));
    assert!(names(current).contains(&"browser_target"));
    assert!(description("run", current).contains("Profile management: browser.profiles"));
    assert!(description("tabs", current).contains("the original Default profile"));
    assert!(description("windows", current).contains("browser_target task selection"));
    for older in [older_native(), NativeCapabilities::default()] {
        let listed = names(older);
        assert!(!listed.contains(&"browser_profiles"), "{older}");
        assert!(!listed.contains(&"browser_target"), "{older}");
        assert!(listed.contains(&"run") && listed.contains(&"tabs"));
        for tool in ["run", "tabs", "windows"] {
            let text = description(tool, older);
            assert!(!text.contains("browser_profiles"), "{older} {tool}: {text}");
            assert!(!text.contains("browser_target"), "{older} {tool}: {text}");
        }
    }
}

#[tokio::test]
async fn hidden_tools_keep_stable_schemas_for_cached_clients() -> anyhow::Result<()> {
    use rmcp::handler::server::ServerHandler;
    let session = BrowserSession::new_with_native(
        FixtureConnection::new(),
        BrowserSessionHooks::default(),
        older_native(),
    );
    let service = BrowserMcpService::new(super::tests::service_options(session));
    let schema = |name: &str| {
        service
            .get_tool(name)
            .map(|tool| serde_json::to_value(tool.input_schema.as_ref()))
            .transpose()
    };
    let expected = catalog()
        .into_iter()
        .find(|tool| tool.name == "browser_target")
        .map(|tool| serde_json::to_value(tool.input_schema.as_ref()))
        .transpose()?;
    assert!(expected.is_some());
    assert_eq!(schema("browser_target")?, expected);
    assert!(schema("browser_profiles")?.is_some());
    let refused = service
        .call_tool_for_testing("browser_profiles", json!({}), CancellationToken::new())
        .await?;
    assert_eq!(refused.is_error, Some(true));
    Ok(())
}

#[tokio::test]
async fn saved_binding_on_an_older_native_runs_unbound_with_one_outer_note() -> anyhow::Result<()> {
    // The same in-memory selection outlives a browser swap: a capable session saves it, the
    // provider then yields an older native, and a capable native later applies it again.
    let connection = FixtureConnection::new();
    let capable = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let older = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        older_native(),
    );
    let use_older = Arc::new(AtomicBool::new(false));
    let mut options = super::tests::service_options(capable.clone());
    options.browser_session = None;
    options.browser_session_provider = Some({
        let use_older = use_older.clone();
        Arc::new(move || {
            let session = if use_older.load(Ordering::SeqCst) {
                older.clone()
            } else {
                capable.clone()
            };
            Box::pin(async move { Some(session) })
        })
    });
    let service = BrowserMcpService::new(options);
    let call = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let selected = call("browser_target", json!({ "action": "set", "name": "eng" })).await?;
    assert_eq!(selected.is_error, Some(false));

    use_older.store(true, Ordering::SeqCst);
    let profile_reads = profile_operations(&connection).len();
    let note = |result: &rmcp::model::CallToolResult| {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text())
            .filter(|content| content.text.contains("was not applied"))
            .map(|content| content.text.clone())
            .collect::<Vec<_>>()
    };
    let listed = call("tabs", json!({ "action": "list" })).await?;
    assert_eq!(listed.is_error, Some(false));
    let notes = note(&listed);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("saved profile target \"eng\" (eng)"));
    assert!(notes[0].contains("BrowserOS 0.51.1.3 or newer"));
    assert!(notes[0].contains("this browser reports BrowserOS 0.51.1.2"));
    // Unbound routing: the other profile's tab is visible too.
    assert!(
        listed
            .structured_content
            .as_ref()
            .and_then(|data| data["pages"].as_array())
            .is_some_and(|pages| pages.len() == 4)
    );
    let script = call(
        "run",
        json!({ "code": "const text = JSON.stringify(await browser.pages.list()); return text.includes('was not applied');" }),
    )
    .await?;
    assert_eq!(note(&script).len(), 1);
    assert_eq!(
        script
            .structured_content
            .as_ref()
            .map(|data| &data["value"]),
        Some(&json!(false))
    );
    assert_eq!(profile_operations(&connection).len(), profile_reads);

    // A capable native retries the retained selection with live identity checks.
    use_older.store(false, Ordering::SeqCst);
    let bound = call("tabs", json!({ "action": "list" })).await?;
    assert!(note(&bound).is_empty());
    assert!(
        bound
            .structured_content
            .as_ref()
            .and_then(|data| data["pages"].as_array())
            .is_some_and(|pages| pages.len() == 2)
    );
    assert!(profile_operations(&connection).len() > profile_reads);
    Ok(())
}

fn creations(connection: &FixtureConnection) -> Vec<(String, Value)> {
    connection
        .calls()
        .into_iter()
        .filter(|call| {
            matches!(
                call.method.as_str(),
                "Browser.createTab" | "Browser.createWindow"
            )
        })
        .map(|call| (call.method, call.params))
        .collect()
}

/// With no selection, granular tools, run's direct primitives, tool:* and raw CDP all use the
/// original Default profile (renamed "Work"), even though sales owns the focused window.
#[tokio::test]
async fn tools_and_run_share_the_default_profile_without_a_selection() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let store = Arc::new(StaticTargetStore::new());
    let ctx = ctx(session, store.clone()).await?;

    data(
        call(
            &ctx,
            "tabs",
            json!({ "action": "new", "url": "https://tool.test/" }),
        )
        .await?,
    )?;
    assert_eq!(creations(&connection)[0].1["windowId"], json!(50));
    let run = data(
        call(
            &ctx,
            "run",
            json!({ "code": "const page = await browser.open('https://run.test/'); const info = await page.info(); const direct = await browser.pages.newPage('https://direct.test/'); const listed = await browser.pages.list(); const windows = await browser.windows({ action: 'list' }); await browser.cdp('History.getRecent', { maxResults: 1 }); return { info, direct, listed: listed.map((p) => p.windowId), windows: windows.count };" }),
        )
        .await?,
    )?;
    assert_eq!(run["value"]["info"]["windowId"], json!(50), "{run}");
    // pages.newPage returns its page id; the listing below shows it in Default's window too.
    assert!(run["value"]["direct"].is_number(), "{run}");
    assert!(
        run["value"]["listed"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty() && ids.iter().all(|id| *id == json!(50))),
        "{run}"
    );
    assert_eq!(run["value"]["windows"], json!(1));
    data(call(&ctx, "history", json!({})).await?)?;
    let created = data(call(&ctx, "windows", json!({ "action": "create" })).await?)?;
    assert!(
        created["window"]["windowId"].as_i64().is_some(),
        "{created}"
    );

    for (method, params) in creations(&connection) {
        match method.as_str() {
            "Browser.createTab" => assert_eq!(params["windowId"], json!(50), "{params}"),
            _ => assert_eq!(
                params["browserContextId"],
                json!("context-original"),
                "{params}"
            ),
        }
    }
    assert!(
        connection
            .calls()
            .iter()
            .filter(|call| call.method == "History.getRecent")
            .all(|call| call.params["browserContextId"] == "context-original")
    );
    // The default is never written as a task selection.
    assert!(store.get().await.map_err(anyhow::Error::msg)?.is_none());
    Ok(())
}

#[tokio::test]
async fn default_without_a_window_gets_one_from_tabs_and_run() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    connection.change(|state| {
        state.windows.retain(|window| window["windowId"] != 50);
        state.tabs.retain(|tab| tab["windowId"] != 50);
    });
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let listing_ctx = ctx(session.clone(), Arc::new(StaticTargetStore::new())).await?;
    data(call(&listing_ctx, "tabs", json!({ "action": "list" })).await?)?;
    assert!(creations(&connection).is_empty(), "listing opened a window");

    data(
        call(
            &listing_ctx,
            "tabs",
            json!({ "action": "new", "url": "https://first.test/" }),
        )
        .await?,
    )?;
    // An inactive normal Default window first, then the page created in it by createTab.
    let created = creations(&connection);
    assert_eq!(created.len(), 2, "{created:?}");
    let (method, params) = &created[0];
    assert_eq!(method, "Browser.createWindow");
    assert_eq!(params["browserContextId"], json!("context-original"));
    assert_eq!(params["windowType"], json!("normal"));
    let window = connection
        .state
        .lock()
        .unwrap_or_else(|err| panic!("fixture poisoned: {err}"))
        .windows
        .last()
        .cloned()
        .unwrap_or_default();
    assert_eq!(window["isActive"], json!(false));
    assert_eq!(created[1].0, "Browser.createTab");
    assert_eq!(created[1].1["windowId"], window["windowId"]);
    assert_eq!(created[1].1["url"], json!("https://first.test/"));

    // The next call captures that window as Default's placement and creates tabs there.
    let run_ctx = ctx(session, Arc::new(StaticTargetStore::new())).await?;
    let run = data(
        call(&run_ctx, "run", json!({ "code": "const page = await browser.open('https://second.test/'); return await page.info();" }))
            .await?,
    )?;
    assert_eq!(run["value"]["windowId"], window["windowId"], "{run}");
    let created = creations(&connection);
    assert_eq!(created.len(), 3, "{created:?}");
    assert_eq!(created[2].0, "Browser.createTab");
    assert_eq!(created[2].1["windowId"], window["windowId"]);
    Ok(())
}

#[tokio::test]
async fn explicit_selection_overrides_default_and_clear_restores_it() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let service = BrowserMcpService::new(super::tests::service_options(session.clone()));
    let call_service = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let tab_window = |result: &rmcp::model::CallToolResult| {
        connection
            .calls()
            .into_iter()
            .rfind(|call| call.method == "Browser.createTab")
            .map(|call| call.params["windowId"].clone())
            .filter(|_| result.is_error != Some(true))
    };

    let defaulted = call_service("tabs", json!({ "action": "new" })).await?;
    assert_eq!(tab_window(&defaulted), Some(json!(50)));
    call_service("browser_target", json!({ "action": "set", "name": "eng" })).await?;
    let explicit = call_service("tabs", json!({ "action": "new" })).await?;
    assert_eq!(tab_window(&explicit), Some(json!(20)));

    // A strict pin stays strict; it never widens back to Default.
    call_service(
        "browser_target",
        json!({ "action": "set_window", "windowId": 10, "strict": true }),
    )
    .await?;
    let pinned = call_service("tabs", json!({ "action": "new" })).await?;
    assert_eq!(tab_window(&pinned), Some(json!(10)));

    call_service("browser_target", json!({ "action": "clear" })).await?;
    let cleared = call_service("tabs", json!({ "action": "new" })).await?;
    assert_eq!(tab_window(&cleared), Some(json!(50)));
    let get = call_service("browser_target", json!({})).await?;
    let data = get.structured_content.unwrap_or_default();
    assert_eq!(data["bound"], json!(false));
    assert_eq!(
        data["default"]["status"]["profile"]["directory"],
        json!("Default")
    );
    Ok(())
}

#[tokio::test]
async fn an_admitted_default_view_ignores_a_later_selection() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let store = Arc::new(StaticTargetStore::new());
    let admitted = ctx(session, store.clone()).await?;
    // Rebinding after admission affects later calls only.
    store.set(intent("eng")).await.map_err(anyhow::Error::msg)?;
    let run = data(
        call(&admitted, "run", json!({ "code": "const page = await browser.open('https://late.test/'); return await page.info();" }))
            .await?,
    )?;
    assert_eq!(run["value"]["windowId"], json!(50), "{run}");
    Ok(())
}

#[tokio::test]
async fn missing_default_refuses_work_but_selection_recovers() -> anyhow::Result<()> {
    // This fixture has no profile with directory "Default" (deleted or never created).
    let connection = FixtureConnection::new();
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let service = BrowserMcpService::new(super::tests::service_options(session));
    let call_service = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    for (name, args) in [
        ("tabs", json!({ "action": "new" })),
        (
            "run",
            json!({ "code": "return await browser.open('https://nowhere.test/');" }),
        ),
        ("windows", json!({ "action": "create" })),
    ] {
        let refused = call_service(name, args).await?;
        assert_eq!(refused.is_error, Some(true), "{name}");
        assert!(
            refused
                .content
                .iter()
                .filter_map(|content| content.as_text())
                .any(|content| content.text.contains("default_profile_unavailable")),
            "{name}: {refused:?}"
        );
    }
    assert!(creations(&connection).is_empty());
    let get = call_service("browser_target", json!({})).await?;
    assert!(get.structured_content.as_ref().is_some_and(|data| {
        data["default"]["status"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("default_profile_unavailable"))
    }));
    // Profile management is admitted without the default and recovers explicitly.
    let selected =
        call_service("browser_target", json!({ "action": "set", "name": "eng" })).await?;
    assert_eq!(selected.is_error, Some(false));
    let opened = call_service("tabs", json!({ "action": "new" })).await?;
    assert_eq!(opened.is_error, Some(false), "{opened:?}");
    assert_eq!(creations(&connection)[0].1["windowId"], json!(20));
    Ok(())
}

#[tokio::test]
async fn profile_management_never_loads_an_unloaded_default() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    connection.change(|state| {
        let original = state
            .profiles
            .iter_mut()
            .find(|profile| profile["directory"] == "Default");
        if let Some(original) = original {
            original["loaded"] = json!(false);
            if let Some(fields) = original.as_object_mut() {
                fields.remove("browserContextId");
            }
        }
    });
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let service = BrowserMcpService::new(super::tests::service_options(session));
    let call_service = |name: &'static str, args: Value| {
        service.call_tool_for_testing(name, args, CancellationToken::new())
    };
    let loads = || {
        connection
            .calls()
            .iter()
            .filter(|call| call.method == "Browser.loadProfile")
            .count()
    };
    for (name, args) in [
        ("browser_profiles", json!({ "action": "list" })),
        ("browser_target", json!({})),
        (
            "browser_target",
            json!({ "action": "set", "name": "sales" }),
        ),
        ("browser_target", json!({ "action": "clear" })),
    ] {
        let result = call_service(name, args.clone()).await?;
        assert_eq!(result.is_error, Some(false), "{name} {args}: {result:?}");
    }
    assert_eq!(loads(), 0, "management loaded the default profile");
    // Ordinary work needs Default's live context, so it loads that existing profile.
    let listed = call_service("tabs", json!({ "action": "list" })).await?;
    assert_eq!(listed.is_error, Some(false), "{listed:?}");
    assert_eq!(loads(), 1);
    Ok(())
}

#[tokio::test]
async fn one_run_without_a_default_window_opens_one_window_for_all_pages() -> anyhow::Result<()> {
    let connection = FixtureConnection::new().with_default_profile();
    connection.change(|state| {
        state.windows.retain(|window| window["windowId"] != 50);
        state.tabs.retain(|tab| tab["windowId"] != 50);
    });
    let session = BrowserSession::new_with_native(
        connection.clone(),
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let ctx = ctx(session, Arc::new(StaticTargetStore::new())).await?;
    let run = data(
        call(
            &ctx,
            "run",
            json!({ "code": "const a = await browser.open('https://a.test/'); const b = await browser.open('https://b.test/'); const [c, d] = await Promise.all([browser.open('https://c.test/'), browser.open('https://d.test/')]); return [await a.info(), await b.info(), await c.info(), await d.info()].map((page) => page.windowId);" }),
        )
        .await?,
    )?;
    let windows = run["value"].as_array().cloned().unwrap_or_default();
    assert_eq!(windows.len(), 4, "{run}");
    assert!(windows.iter().all(|window| *window == windows[0]), "{run}");
    let created = creations(&connection);
    assert_eq!(
        created
            .iter()
            .filter(|(method, _)| method == "Browser.createWindow")
            .count(),
        1,
        "{created:?}"
    );
    Ok(())
}
