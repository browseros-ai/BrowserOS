use crate::{
    AppState,
    api::mcp::dispatch::{ToolCall, ToolIdentity, linked_cancel_token},
    config::{Config, ServerMode},
    identity::{ClientIdentity, ConversationIdentity},
    ids::SessionId,
    services::sessions::{ClawTargetStore, Session},
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// The first native release with profile targeting. Host fixtures model that browser unless a
/// test is about an older or unknown one.
pub(crate) fn current_native() -> browseros_core::NativeCapabilities {
    browseros_core::NativeCapabilities::from_browseros_version(Some("0.51.1.3"))
}

/// A browser session on `connection` that models the current native (profile targeting on).
pub(crate) fn current_session(
    connection: Arc<dyn browseros_core::CdpConnection>,
) -> Arc<browseros_core::BrowserSession> {
    browseros_core::BrowserSession::new_with_native(
        connection,
        browseros_core::BrowserSessionHooks::default(),
        current_native(),
    )
}

pub async fn tool_call(tool_name: &str, raw_args: Value) -> anyhow::Result<ToolCall> {
    tool_call_in_mode(tool_name, raw_args, ServerMode::Desktop).await
}

pub async fn tool_call_in_mode(
    tool_name: &str,
    raw_args: Value,
    mode: ServerMode,
) -> anyhow::Result<ToolCall> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("browserclaw");
    let home = dir.path().join("home");
    let _persisted = dir.keep();
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: root.join("resources"),
        browserclaw_dir: root,
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: crate::config::NativeIdentity {
            capabilities: current_native(),
            asserted: false,
        },
        mode,
    });
    let state = AppState::new_with_home(config, home).await?;
    let session = Session::new(
        SessionId::new("s1"),
        ClientIdentity::Ephemeral {
            slug: "codex".to_string(),
            label: "Codex".to_string(),
        },
        ConversationIdentity::new("codex", "agile-alpaca".to_string()),
        "Codex".to_string(),
        tokio::time::Instant::now(),
    );
    state.sessions.insert_for_testing(session.clone()).await;
    let catalog = Arc::new(browseros_mcp::catalog());
    let tool_index = catalog
        .iter()
        .position(|tool| tool.name == tool_name)
        .ok_or_else(|| anyhow::anyhow!("tool {tool_name} missing from catalog"))?;
    let client_cancel = CancellationToken::new();
    let dispatch_cancel = CancellationToken::new();
    let cancel = linked_cancel_token(
        session.child_token(),
        client_cancel.clone(),
        dispatch_cancel.clone(),
    );
    let ownership_key = session.convo_id().clone();
    let target_store = Arc::new(ClawTargetStore {
        bindings: state.targets.clone(),
        session: session.id().clone(),
    });
    Ok(ToolCall::new(
        catalog,
        tool_index,
        raw_args,
        session.id().clone(),
        Some(ToolIdentity {
            session: session.clone(),
            agent: session.agent().clone(),
            ownership_key,
            agent_label: "Codex".to_string(),
        }),
        None,
        None,
        target_store,
        cancel,
        client_cancel,
        dispatch_cancel,
        None,
        state,
        browseros_mcp::output_file::create_browser_output_file_access(),
    ))
}

/// Synthetic CDP peer for host integration tests. It models native catalog,
/// tab/window identity and group movement so routing errors have visible effects.
pub(crate) struct ProfileConnection {
    events: tokio::sync::broadcast::Sender<browseros_cdp::CdpEvent>,
    pub profiles: std::sync::Mutex<Vec<Value>>,
    pub windows: std::sync::Mutex<Vec<Value>>,
    pub tabs: std::sync::Mutex<Vec<Value>>,
    pub groups: std::sync::Mutex<Vec<Value>>,
    pub calls: std::sync::Mutex<Vec<(String, Value)>>,
    // Epoch selection and recorded publication share this lock with reconnect.
    dispatch_epoch: std::sync::Mutex<u64>,
    pub fail_inventory: std::sync::atomic::AtomicBool,
    pub group_created: tokio::sync::Notify,
    next_group_id: std::sync::atomic::AtomicU64,
    /// Pause a native group lookup after the callback has admitted the original
    /// page, so tests can replace its target across that awaited handoff.
    pub pause_group_lookup: std::sync::atomic::AtomicBool,
    pub group_lookup_entered: tokio::sync::Notify,
    pub group_lookup_release: tokio::sync::Notify,
    /// Pause a grouping write after its guarded reads, before native publication.
    /// This separates fresh admission from ordering with a tool-backed script.
    pub pause_group_create: std::sync::atomic::AtomicBool,
    pub group_create_entered: tokio::sync::Notify,
    pub group_create_release: tokio::sync::Notify,
    /// Response settling runs after new_page returns but before Claw effects
    /// capture the page. Pause this handoff to reconcile a replacement there.
    pub pause_response_evaluation: std::sync::atomic::AtomicBool,
    pub response_evaluation_entered: tokio::sync::Notify,
    pub response_evaluation_release: tokio::sync::Notify,
}

impl ProfileConnection {
    pub(crate) fn new() -> Arc<Self> {
        let (events, _) = tokio::sync::broadcast::channel(8);
        Arc::new(Self {
            events,
            profiles: std::sync::Mutex::new(vec![
                serde_json::json!({"browserProfileId":"eng", "name":"eng", "directory":"Synthetic-eng", "loaded":true, "browserContextId":"ctx-eng"}),
                serde_json::json!({"browserProfileId":"ops", "name":"ops", "directory":"Synthetic-ops", "loaded":true, "browserContextId":"ctx-ops"}),
            ]),
            windows: std::sync::Mutex::new(vec![
                Self::window(1, "eng"),
                Self::window(2, "ops"),
                Self::window(3, "eng"),
            ]),
            tabs: std::sync::Mutex::new(vec![
                Self::tab(101, 1, "eng"),
                Self::tab(102, 2, "ops"),
                Self::tab(103, 3, "eng"),
            ]),
            groups: std::sync::Mutex::new(Vec::new()),
            calls: std::sync::Mutex::new(Vec::new()),
            dispatch_epoch: std::sync::Mutex::new(1),
            fail_inventory: std::sync::atomic::AtomicBool::new(false),
            group_created: tokio::sync::Notify::new(),
            next_group_id: std::sync::atomic::AtomicU64::new(1),
            pause_group_lookup: std::sync::atomic::AtomicBool::new(false),
            group_lookup_entered: tokio::sync::Notify::new(),
            group_lookup_release: tokio::sync::Notify::new(),
            pause_group_create: std::sync::atomic::AtomicBool::new(false),
            group_create_entered: tokio::sync::Notify::new(),
            group_create_release: tokio::sync::Notify::new(),
            pause_response_evaluation: std::sync::atomic::AtomicBool::new(false),
            response_evaluation_entered: tokio::sync::Notify::new(),
            response_evaluation_release: tokio::sync::Notify::new(),
        })
    }

    pub(crate) fn reconnect(&self) {
        let mut epoch = self
            .dispatch_epoch
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *epoch += 1;
    }

    fn dispatch<'a>(
        &'a self,
        expected_epoch: Option<u64>,
        method: &'a str,
        params: Value,
        _session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
        Box::pin(async move {
            use serde_json::json;
            use std::sync::atomic::Ordering;
            // A paused fixture send has not yet been published. Recheck under
            // the reconnect boundary after release, before recording any dispatch.
            if method == "Browser.getTabGroups"
                && self.pause_group_lookup.swap(false, Ordering::SeqCst)
            {
                self.group_lookup_entered.notify_one();
                self.group_lookup_release.notified().await;
            }
            if method == "Browser.createTabGroup"
                && self.pause_group_create.swap(false, Ordering::SeqCst)
            {
                self.group_create_entered.notify_one();
                self.group_create_release.notified().await;
            }
            if method == "Runtime.evaluate"
                && self.pause_response_evaluation.swap(false, Ordering::SeqCst)
            {
                self.response_evaluation_entered.notify_one();
                self.response_evaluation_release.notified().await;
            }
            {
                let epoch = self
                    .dispatch_epoch
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                if let Some(expected) = expected_epoch
                    && expected != *epoch
                {
                    return Err(browseros_cdp::CdpError::ConnectionEpochChanged {
                        expected,
                        actual: *epoch,
                    });
                }
                self.calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((method.into(), params.clone()));
            }
            let error = |message: &str| browseros_cdp::CdpError::Protocol {
                code: -1,
                message: message.into(),
            };
            match method {
                "Browser.getProfiles" => Ok(
                    json!({"profiles":self.profiles.lock().unwrap_or_else(|p| p.into_inner()).clone()}),
                ),
                "Browser.getWindows" => Ok(
                    json!({"windows":self.windows.lock().unwrap_or_else(|p| p.into_inner()).clone()}),
                ),
                "Browser.getTabs" => {
                    if self.fail_inventory.load(Ordering::SeqCst) {
                        return Err(error("inventory unavailable"));
                    }
                    Ok(json!({"tabs":self.tabs.lock().unwrap_or_else(|p| p.into_inner()).clone()}))
                }
                "Browser.getTabInfo" => {
                    let tab = self
                        .tabs
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .iter()
                        .find(|tab| tab["tabId"] == params["tabId"])
                        .cloned()
                        .ok_or_else(|| error("Tab not found"))?;
                    Ok(json!({"tab":tab}))
                }
                "Browser.createTab" => {
                    let window_id = params.get("windowId").and_then(Value::as_i64).unwrap_or(2);
                    let windows = self.windows.lock().unwrap_or_else(|p| p.into_inner());
                    let window = windows
                        .iter()
                        .find(|window| window["windowId"] == window_id)
                        .ok_or_else(|| error("window missing"))?;
                    let profile = window["browserProfileId"]
                        .as_str()
                        .ok_or_else(|| error("profile missing"))?;
                    let mut tabs = self.tabs.lock().unwrap_or_else(|p| p.into_inner());
                    let id = 101 + i64::try_from(tabs.len()).map_err(|_| error("too many tabs"))?;
                    let mut tab = Self::tab(id, window_id, profile);
                    tab["url"] = params["url"].clone();
                    tabs.push(tab.clone());
                    Ok(json!({"tab":tab}))
                }
                "Browser.getTabGroups" => Ok(
                    json!({"groups":self.groups.lock().unwrap_or_else(|p| p.into_inner()).clone()}),
                ),
                "Browser.createTabGroup" | "Browser.addTabsToGroup" => {
                    let mut tabs = self.tabs.lock().unwrap_or_else(|p| p.into_inner());
                    let tab_ids = params["tabIds"]
                        .as_array()
                        .ok_or_else(|| error("tabIds required"))?;
                    let window = tabs
                        .iter()
                        .find(|tab| tab_ids.contains(&tab["tabId"]))
                        .and_then(|tab| tab["windowId"].as_i64())
                        .ok_or_else(|| error("tab missing"))?;
                    let mut groups = self.groups.lock().unwrap_or_else(|p| p.into_inner());
                    let destination = if method == "Browser.createTabGroup" {
                        format!("G{}", self.next_group_id.fetch_add(1, Ordering::SeqCst))
                    } else {
                        params["groupId"]
                            .as_str()
                            .ok_or_else(|| error("groupId required"))?
                            .to_string()
                    };
                    if method == "Browser.addTabsToGroup"
                        && !groups.iter().any(|group| group["groupId"] == destination)
                    {
                        return Err(error("Tab group not found"));
                    }
                    // Native grouping moves membership. Moving the last tab also
                    // destroys its previous group, so stale returned IDs must fail.
                    for group in groups
                        .iter_mut()
                        .filter(|group| group["groupId"] != destination)
                    {
                        if let Some(members) = group["tabIds"].as_array_mut() {
                            members.retain(|member| !tab_ids.contains(member));
                        }
                    }
                    groups.retain(|group| {
                        group["tabIds"]
                            .as_array()
                            .is_some_and(|members| !members.is_empty())
                    });
                    let group = if method == "Browser.createTabGroup" {
                        let group = json!({"groupId":destination, "windowId":window,
                            "title":params.get("title").and_then(Value::as_str).unwrap_or_default(), "color":"grey", "collapsed":false, "tabIds":tab_ids});
                        groups.push(group.clone());
                        group
                    } else {
                        let group = groups
                            .iter_mut()
                            .find(|group| group["groupId"] == params["groupId"])
                            .ok_or_else(|| error("Tab group not found"))?;
                        if let Some(members) = group["tabIds"].as_array_mut() {
                            for id in tab_ids {
                                if !members.contains(id) {
                                    members.push(id.clone());
                                }
                            }
                        }
                        group.clone()
                    };
                    for tab in tabs
                        .iter_mut()
                        .filter(|tab| tab_ids.contains(&tab["tabId"]))
                    {
                        tab["windowId"] = group["windowId"].clone();
                        tab["groupId"] = group["groupId"].clone();
                    }
                    self.group_created.notify_one();
                    Ok(json!({"group":group}))
                }
                "Browser.updateTabGroup" => {
                    let mut groups = self.groups.lock().unwrap_or_else(|p| p.into_inner());
                    let group = groups
                        .iter_mut()
                        .find(|group| group["groupId"] == params["groupId"])
                        .ok_or_else(|| error("Tab group not found"))?;
                    for field in ["title", "color", "collapsed"] {
                        if let Some(value) = params.get(field) {
                            group[field] = value.clone();
                        }
                    }
                    Ok(json!({"group":group.clone()}))
                }
                "Browser.closeTabGroup" => {
                    let mut tabs = self.tabs.lock().unwrap_or_else(|p| p.into_inner());
                    let mut groups = self.groups.lock().unwrap_or_else(|p| p.into_inner());
                    let group = groups
                        .iter()
                        .find(|group| group["groupId"] == params["groupId"])
                        .ok_or_else(|| error("Tab group not found"))?;
                    let members = group["tabIds"]
                        .as_array()
                        .ok_or_else(|| error("tabIds missing"))?;
                    tabs.retain(|tab| !members.contains(&tab["tabId"]));
                    groups.retain(|group| group["groupId"] != params["groupId"]);
                    Ok(json!({}))
                }
                "Target.detachFromTarget" => Ok(json!({})),
                "Target.attachToTarget" => {
                    Ok(json!({"sessionId":format!("S-{}", params["targetId"])}))
                }
                "Page.enable"
                | "DOM.enable"
                | "Runtime.enable"
                | "Accessibility.enable"
                | "Runtime.runIfWaitingForDebugger"
                | "Target.setAutoAttach" => Ok(json!({})),
                // A snapshot failure is advisory to creation. The retained result
                // still enters Claw effects, exercising the earlier response await.
                "Runtime.evaluate" => Err(error("synthetic snapshot unavailable")),
                _ => Err(error(&format!("unexpected CDP method {method}"))),
            }
        })
    }

    /// Adds the browser's original profile (directory "Default", renamed "Personal") with
    /// one normal window that is not the focused one, as a capable native has. The base
    /// fixture without it models a browser whose Default profile is gone.
    pub(crate) fn with_default_profile(self: Arc<Self>) -> Arc<Self> {
        self.profiles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(
                serde_json::json!({"browserProfileId":"default", "name":"Personal",
                "directory":"Default", "loaded":true, "browserContextId":"ctx-default"}),
            );
        self.windows
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(Self::window(4, "default"));
        self.tabs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(Self::tab(104, 4, "default"));
        self
    }

    fn window(id: i64, profile: &str) -> Value {
        serde_json::json!({"windowId":id, "windowType":"normal", "bounds":{}, "isActive":id == 2,
            "isVisible":true, "tabCount":1, "browserProfileId":profile, "liveBrowserContextId":format!("ctx-{profile}")})
    }

    fn tab(id: i64, window: i64, profile: &str) -> Value {
        serde_json::json!({"tabId":id, "targetId":format!("T{id}"), "url":format!("https://synthetic.test/{id}"),
            "title":format!("Tab {id}"), "isActive":true, "isLoading":false, "loadProgress":1.0,
            "isPinned":false, "isHidden":false, "windowId":window, "index":0,
            "browserProfileId":profile, "liveBrowserContextId":format!("ctx-{profile}")})
    }

    pub(crate) fn binding(
        profile: &str,
        revision: u64,
        window: i64,
    ) -> browseros_core::TargetBinding {
        browseros_core::TargetBinding {
            intent: browseros_core::TargetIntent {
                browser_profile_id: browseros_core::BrowserProfileId::from(profile.to_string()),
                directory: format!("Synthetic-{profile}"),
                name: profile.to_string(),
                window: Some(browseros_core::WindowSelection::Working {
                    window_id: browseros_core::WindowId(window),
                }),
            },
            revision,
        }
    }
}

impl browseros_core::CdpConnection for ProfileConnection {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
        self.dispatch(None, method, params, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
        self.dispatch(Some(expected_epoch), method, params, session)
    }

    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
        Box::pin(async move {
            let value = serde_json::from_str(params).map_err(|error| {
                browseros_cdp::CdpError::Protocol {
                    code: -1,
                    message: error.to_string(),
                }
            })?;
            Ok(self.send(method, value, session).await?.to_string())
        })
    }
    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<String, browseros_cdp::CdpError>> {
        Box::pin(async move {
            let value = serde_json::from_str(params).map_err(|error| {
                browseros_cdp::CdpError::Protocol {
                    code: -1,
                    message: error.to_string(),
                }
            })?;
            Ok(self
                .dispatch(Some(expected_epoch), method, value, session)
                .await?
                .to_string())
        })
    }
    fn events(&self) -> tokio::sync::broadcast::Receiver<browseros_cdp::CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        *self
            .dispatch_epoch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }
}
