//! Exercise the targeting metadata contract across actual HTTP serialization.
//! A disposable native peer supplies identity and loading state; all requests go
//! through the real router, task store, tool dispatch, effects, and wire envelope.

use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{
    BrowserSession, BrowserSessionHooks, CdpConnection, NativeCapabilities, SessionId,
};
use claw_server::{AppState, build_router, config::Config};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, sync::broadcast, task::JoinHandle};

/// The first native release with profile targeting, which this wire test models.
fn current_native() -> NativeCapabilities {
    NativeCapabilities::from_browseros_version(Some("0.51.1.3"))
}

struct NativeProfiles {
    profiles: Mutex<Vec<Value>>,
    windows: Mutex<Vec<Value>>,
    events: broadcast::Sender<CdpEvent>,
}

impl NativeProfiles {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            profiles: Mutex::new(vec![
                loaded_engineering_profile(),
                json!({"browserProfileId":"native-ops", "name":"Operations",
                    "directory":"Synthetic-Ops", "loaded":false}),
                original_profile(),
            ]),
            // Engineering's window is the focused one; the original profile has its own.
            windows: Mutex::new(
                [11, 12]
                    .into_iter()
                    .map(|id| {
                        json!({"windowId":id, "windowType":"normal", "bounds":{},
                        "isActive":id == 11, "isVisible":true, "tabCount":0,
                        "browserProfileId":"native-eng", "liveBrowserContextId":"ctx-eng"})
                    })
                    .chain([json!({"windowId":21, "windowType":"normal", "bounds":{},
                        "isActive":false, "isVisible":true, "tabCount":0,
                        "browserProfileId":"native-default", "liveBrowserContextId":"ctx-default"})])
                    .collect(),
            ),
            events: broadcast::channel(8).0,
        })
    }
}

/// The browser's original profile: directory "Default", renamed by the user.
fn original_profile() -> Value {
    json!({"browserProfileId":"native-default", "name":"Personal",
        "directory":"Default", "loaded":true, "browserContextId":"ctx-default"})
}

fn loaded_engineering_profile() -> Value {
    json!({"browserProfileId":"native-eng", "name":"Engineering",
        "directory":"Synthetic-Eng", "loaded":true, "browserContextId":"ctx-eng"})
}

impl CdpConnection for NativeProfiles {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        self.send_at_epoch(1, method, params, session)
    }

    fn send_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: Value,
        _session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move {
            // This transport never reconnects. There is no await between checking
            // its fixed epoch and applying the synchronous synthetic operation.
            if expected_epoch != 1 {
                return Err(CdpError::ConnectionEpochChanged {
                    expected: expected_epoch,
                    actual: 1,
                });
            }
            match method {
                "Browser.getProfiles" => Ok(json!({"profiles":self.profiles.lock()
                    .unwrap_or_else(|p| p.into_inner()).clone()})),
                "Browser.loadProfile" => {
                    let mut profiles = self.profiles.lock().unwrap_or_else(|p| p.into_inner());
                    let profile = profiles
                        .iter_mut()
                        .find(|profile| profile["browserProfileId"] == params["browserProfileId"])
                        .ok_or_else(|| CdpError::Protocol {
                            code: -1,
                            message: "synthetic profile missing".into(),
                        })?;
                    let initiated = profile["loaded"] == false;
                    profile["loaded"] = json!(true);
                    profile["browserContextId"] = json!("ctx-ops");
                    Ok(json!({"profile":profile, "loaded":initiated}))
                }
                "Browser.getWindows" => Ok(json!({"windows":self.windows.lock()
                    .unwrap_or_else(|p| p.into_inner()).clone()})),
                "Browser.getTabs" => Ok(json!({"tabs":[]})),
                "Browser.getTabGroups" => Ok(json!({"groups":[]})),
                "Target.detachFromTarget" => Ok(json!({})),
                _ => Err(CdpError::Protocol {
                    code: -1,
                    message: format!("unexpected synthetic method {method}"),
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
        self.send_raw_json_at_epoch(1, method, params, session)
    }

    fn send_raw_json_at_epoch<'a>(
        &'a self,
        expected_epoch: u64,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            let params = serde_json::from_str(params).map_err(|error| CdpError::Protocol {
                code: -1,
                message: error.to_string(),
            })?;
            Ok(self
                .send_at_epoch(expected_epoch, method, params, session)
                .await?
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
        1
    }
}

struct HttpPeer {
    client: reqwest::Client,
    url: String,
    native: Arc<NativeProfiles>,
    task: JoinHandle<std::io::Result<()>>,
    _dir: TempDir,
}

impl HttpPeer {
    async fn start() -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let config = Arc::new(Config {
            server_port: 9200,
            cdp_port: 49337,
            proxy_port: None,
            resources_dir: dir.path().join("resources"),
            browserclaw_dir: dir.path().join("claw"),
            session_idle: Duration::from_secs(300),
            session_retention: Duration::from_secs(7_200),
            session_sweep_interval: Duration::from_secs(60),
            replay_retention_days: 7,
            dev_mode: false,
            native: claw_server::config::NativeIdentity {
                capabilities: current_native(),
                asserted: false,
            },
            mode: Default::default(),
        });
        let state = AppState::new_with_home(config, dir.path().join("home")).await?;
        let native = NativeProfiles::new();
        state
            .browser
            .set_session_for_testing(BrowserSession::new_with_native(
                native.clone(),
                BrowserSessionHooks::default(),
                current_native(),
            ))
            .await;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/mcp", listener.local_addr()?);
        let router = build_router(state);
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
            url,
            native,
            task,
            _dir: dir,
        })
    }

    async fn request(&self, method: &str, mut params: Value) -> anyhow::Result<Value> {
        // Each modern stateless request supplies protocol metadata, while the
        // returned application handle carries task binding across HTTP requests.
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientCapabilities":{},
            "io.modelcontextprotocol/clientInfo":{"name":"Codex", "version":"synthetic"}
        });
        let mut request = self
            .client
            .post(&self.url)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2026-07-28")
            .header("mcp-method", method);
        if method == "tools/call" {
            request = request.header("mcp-name", params["name"].as_str().unwrap_or_default());
        }
        let response = request
            .json(&json!({"jsonrpc":"2.0", "id":1,
            "method":method, "params":params}))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        let encoded = body
            .lines()
            .find_map(|line| line.strip_prefix("data:"))
            .unwrap_or(&body);
        let value: Value = serde_json::from_str(encoded.trim())?;
        assert!(value["error"].is_null(), "{value}");
        assert_ne!(value["result"]["isError"], true, "{value}");
        Ok(value["result"].clone())
    }

    async fn call(
        &self,
        name: &str,
        mut arguments: Value,
        handle: Option<&str>,
    ) -> anyhow::Result<Value> {
        if let Some(handle) = handle {
            arguments["session"] = json!(handle);
        }
        self.request("tools/call", json!({"name":name, "arguments":arguments}))
            .await
    }
}

impl Drop for HttpPeer {
    fn drop(&mut self) {
        // Only this disposable HTTP listener is owned here; no daemon or live
        // browser process is started or stopped by these wire regressions.
        self.task.abort();
    }
}

fn task_handle(result: &Value) -> anyhow::Result<&str> {
    result["_meta"]["com.browseros.neo/session"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("application handle missing: {result}"))
}

fn structured(result: &Value) -> anyhow::Result<&Value> {
    result
        .get("structuredContent")
        .ok_or_else(|| anyhow::anyhow!("structured metadata missing on HTTP wire: {result}"))
}

#[tokio::test]
async fn browser_target_http_roundtrip_preserves_window_revision_and_native_status()
-> anyhow::Result<()> {
    let peer = HttpPeer::start().await?;
    let unbound = peer
        .call("browser_target", json!({"action":"get"}), None)
        .await?;
    let handle = task_handle(&unbound)?;
    // No stored selection: the call reports the original Default profile it uses, not the
    // focused Engineering window and not browser-wide routing.
    let default_target = json!({"action":"get", "bound":false, "default":{"directory":"Default",
        "status":{"profile":original_profile(), "windows":[21], "placementWindowId":21, "error":null}}});
    assert_eq!(structured(&unbound)?, &default_target);
    peer.call(
        "browser_target",
        json!({"action":"set", "browserProfileId":"native-eng"}),
        Some(handle),
    )
    .await?;
    for (strict, mode, revision) in [(false, "working", 2), (true, "pinned", 3)] {
        peer.call(
            "browser_target",
            json!({"action":"set_window", "windowId":11, "strict":strict}),
            Some(handle),
        )
        .await?;
        let result = peer
            .call("browser_target", json!({"action":"get"}), Some(handle))
            .await?;
        let data = structured(&result)?;
        assert_eq!(data["action"], "get");
        assert_eq!(data["bound"], true);
        assert_eq!(
            data["binding"],
            json!({"browserProfileId":"native-eng",
            "name":"Engineering", "directory":"Synthetic-Eng",
            "window":{"mode":mode, "windowId":11}, "revision":revision})
        );
        assert_eq!(
            data["status"],
            json!({"profile":loaded_engineering_profile(),
            "windows":[11,12], "placementWindowId":11, "error":null})
        );
        assert_eq!(task_handle(&result)?, handle);
    }

    let tools = peer.request("tools/list", json!({})).await?;
    for name in ["browser_profiles", "browser_target"] {
        let tool = tools["tools"]
            .as_array()
            .and_then(|tools| tools.iter().find(|tool| tool["name"] == name))
            .ok_or_else(|| anyhow::anyhow!("missing {name}"))?;
        assert!(tool["outputSchema"].is_object(), "{tool}");
    }
    let tabs = peer
        .call("tabs", json!({"action":"list"}), Some(handle))
        .await?;
    assert!(tabs.get("structuredContent").is_none(), "{tabs}");
    assert!(tabs["content"].is_array());

    // get remains a recovery operation after pin closure. Its wire status exposes
    // native failure without erasing the retained window selection or revision.
    peer.native
        .windows
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|window| window["windowId"] != 11);
    let stale = peer
        .call("browser_target", json!({"action":"get"}), Some(handle))
        .await?;
    let data = structured(&stale)?;
    assert_eq!(
        data["binding"]["window"],
        json!({"mode":"pinned", "windowId":11})
    );
    assert_eq!(data["binding"]["revision"], 3);
    assert_eq!(data["status"]["profile"], loaded_engineering_profile());
    assert_eq!(data["status"]["windows"], json!([12]));
    assert!(data["status"]["placementWindowId"].is_null());
    assert!(
        data["status"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("pinned_window_closed"))
    );

    // Clearing the stored override returns the task to Default; nothing new is stored.
    peer.call("browser_target", json!({"action":"clear"}), Some(handle))
        .await?;
    let cleared = peer
        .call("browser_target", json!({"action":"get"}), Some(handle))
        .await?;
    assert_eq!(structured(&cleared)?, &default_target);
    Ok(())
}

#[tokio::test]
async fn browser_profiles_http_roundtrip_preserves_descriptors_and_load_outcome()
-> anyhow::Result<()> {
    let peer = HttpPeer::start().await?;
    let first = peer
        .call("browser_profiles", json!({"action":"list"}), None)
        .await?;
    let handle = task_handle(&first)?;
    let listed = structured(&first)?;
    assert_eq!(
        listed,
        &json!({"action":"list", "profiles":[loaded_engineering_profile(),
        {"browserProfileId":"native-ops", "name":"Operations", "directory":"Synthetic-Ops", "loaded":false},
        original_profile()]})
    );
    peer.call(
        "browser_target",
        json!({"action":"set", "browserProfileId":"native-eng"}),
        Some(handle),
    )
    .await?;
    let bound = peer
        .call("browser_profiles", json!({"action":"list"}), Some(handle))
        .await?;
    assert_eq!(structured(&bound)?["boundBrowserProfileId"], "native-eng");
    assert_eq!(structured(&bound)?["profiles"], listed["profiles"]);
    for initiated in [true, false] {
        let loaded = peer
            .call(
                "browser_profiles",
                json!({"action":"load", "browserProfileId":"native-ops"}),
                Some(handle),
            )
            .await?;
        assert_eq!(
            structured(&loaded)?,
            &json!({"action":"load", "loaded":initiated,
            "profile":{"browserProfileId":"native-ops", "name":"Operations", "directory":"Synthetic-Ops",
                "loaded":true, "browserContextId":"ctx-ops"}})
        );
        assert_eq!(task_handle(&loaded)?, handle);
    }
    let target = peer
        .call("browser_target", json!({"action":"get"}), Some(handle))
        .await?;
    assert_eq!(
        structured(&target)?["binding"]["browserProfileId"],
        "native-eng"
    );
    assert_eq!(structured(&target)?["binding"]["revision"], 1);
    assert_eq!(
        structured(&target)?["status"]["profile"],
        loaded_engineering_profile()
    );
    Ok(())
}
