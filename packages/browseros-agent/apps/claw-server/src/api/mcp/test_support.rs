use crate::{
    AppState,
    api::mcp::dispatch::{ToolCall, ToolIdentity, linked_cancel_token},
    config::Config,
    identity::{ClientIdentity, ConversationIdentity},
    ids::SessionId,
    services::sessions::{ClawTargetStore, Session},
};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub async fn tool_call(tool_name: &str, raw_args: Value) -> anyhow::Result<ToolCall> {
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
    pub fail_inventory: std::sync::atomic::AtomicBool,
    pub group_created: tokio::sync::Notify,
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
            fail_inventory: std::sync::atomic::AtomicBool::new(false),
            group_created: tokio::sync::Notify::new(),
        })
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
        _session: Option<&'a browseros_core::SessionId>,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, browseros_cdp::CdpError>> {
        Box::pin(async move {
            use serde_json::json;
            use std::sync::atomic::Ordering;
            self.calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((method.into(), params.clone()));
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
                    let group = if method == "Browser.createTabGroup" {
                        let group = json!({"groupId":format!("G{}", groups.len()+1), "windowId":window,
                            "title":params.get("title").and_then(Value::as_str).unwrap_or_default(), "color":"grey", "collapsed":false, "tabIds":tab_ids});
                        groups.push(group.clone());
                        group
                    } else {
                        let group = groups
                            .iter_mut()
                            .find(|group| group["groupId"] == params["groupId"])
                            .ok_or_else(|| error("group missing"))?;
                        if let Some(members) = group["tabIds"].as_array_mut() {
                            members.extend(tab_ids.iter().cloned());
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
                        .ok_or_else(|| error("group missing"))?;
                    for field in ["title", "color", "collapsed"] {
                        if let Some(value) = params.get(field) {
                            group[field] = value.clone();
                        }
                    }
                    Ok(json!({"group":group.clone()}))
                }
                "Browser.closeTabGroup" => {
                    self.groups
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .retain(|group| group["groupId"] != params["groupId"]);
                    Ok(json!({}))
                }
                "Target.detachFromTarget" => Ok(json!({})),
                _ => Err(error(&format!("unexpected CDP method {method}"))),
            }
        })
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
    fn events(&self) -> tokio::sync::broadcast::Receiver<browseros_cdp::CdpEvent> {
        self.events.subscribe()
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn connection_epoch(&self) -> u64 {
        1
    }
}
