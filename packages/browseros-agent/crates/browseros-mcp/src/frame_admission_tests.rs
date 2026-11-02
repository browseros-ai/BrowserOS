//! Frame admission refusals through the real MCP service. A frame whose admission is refused
//! during a capture is reported by the snapshot tool as `refused`, not as a browser error.
//! Scoped Default-profile view over root -> f1 -> f2, two cross-site OOPIFs whose page trees
//! omit remote frames as Chromium's do. f1 reports f2's owner element once, while the capture
//! admits f2, then another one: the owner was replaced mid-capture.
use crate::BrowserMcpService;
use browseros_cdp::{CdpError, CdpEvent};
use browseros_core::{BrowserSession, BrowserSessionHooks, CdpConnection, SessionId};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

struct Chain {
    replace_owner: AtomicBool,
    f2_owner_reads: AtomicUsize,
    events: broadcast::Sender<CdpEvent>,
}

fn tab() -> Value {
    json!({ "tabId": 501, "targetId": "page", "windowId": 50,
        "url": "https://default.example.test/", "title": "Default", "isActive": true,
        "isLoading": false, "loadProgress": 1.0, "isPinned": false, "isHidden": false,
        "index": 0, "browserProfileId": "original", "liveBrowserContextId": "context-original" })
}

fn level(session: Option<&SessionId>) -> usize {
    match session.map(SessionId::as_str) {
        Some("session-f1") => 1,
        Some("session-f2") => 2,
        _ => 0,
    }
}

fn ax_tree(level: usize) -> Value {
    let button = format!("button-{level}");
    let mut children = vec![button.clone()];
    let mut nodes = vec![json!({"nodeId":button,"backendDOMNodeId":10 + level,
        "role":{"type":"internalRole","value":"button"},
        "name":{"type":"computedString","value":format!("Target {level}")}})];
    if level < 2 {
        let owner = format!("owner-{}", level + 1);
        children.push(owner.clone());
        nodes.push(json!({"nodeId":owner,"backendDOMNodeId":1001 + level,
            "role":{"type":"internalRole","value":"Iframe"},
            "name":{"type":"computedString","value":owner}}));
    }
    nodes.insert(
        0,
        json!({"nodeId":"document","role":{"type":"internalRole","value":"RootWebArea"},"childIds":children}),
    );
    json!({ "nodes": nodes })
}

impl Chain {
    fn reply(
        &self,
        method: &str,
        params: &Value,
        session: Option<&SessionId>,
    ) -> Result<Value, CdpError> {
        let level = level(session);
        Ok(match method {
            "Browser.getProfiles" => {
                json!({"profiles":[{"browserProfileId":"original","name":"Work",
                "directory":"Default","loaded":true,"browserContextId":"context-original"}]})
            }
            "Browser.getWindows" => {
                json!({"windows":[{"windowId":50,"windowType":"normal","bounds":{},
                "isActive":true,"isVisible":true,"tabCount":1,"browserProfileId":"original",
                "liveBrowserContextId":"context-original"}]})
            }
            "Browser.getTabs" => json!({"tabs":[tab()]}),
            "Browser.getTabInfo" => json!({"tab":tab()}),
            "Target.attachToTarget" => {
                json!({"sessionId":format!("session-{}", params["targetId"].as_str().unwrap_or_default())})
            }
            "Page.getFrameTree" => match level {
                0 => {
                    json!({"frameTree":{"frame":{"id":"root","loaderId":"l0","url":"https://default.example.test/"}}})
                }
                level => json!({"frameTree":{"frame":{"id":format!("f{level}")}}}),
            },
            "DOM.getFrameOwner" => match (level, params["frameId"].as_str()) {
                (0, Some("f1")) => json!({"backendNodeId":1001}),
                (1, Some("f2")) => {
                    let reads = self.f2_owner_reads.fetch_add(1, Ordering::SeqCst);
                    let moved = self.replace_owner.load(Ordering::SeqCst) && reads > 0;
                    json!({"backendNodeId": if moved { 9999 } else { 1002 }})
                }
                _ => {
                    return Err(CdpError::Protocol {
                        code: -32000,
                        message: "Frame with the given id does not belong to the target.".into(),
                    });
                }
            },
            "DOM.describeNode" => match params["backendNodeId"].as_i64() {
                Some(1001) => json!({"node":{"frameId":"f1"}}),
                Some(1002) => json!({"node":{"frameId":"f2"}}),
                _ => json!({"node":{}}),
            },
            "Accessibility.getFullAXTree" => ax_tree(level),
            _ => json!({}),
        })
    }
}

impl CdpConnection for Chain {
    fn send<'a>(
        &'a self,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(method, &params, session) })
    }
    fn send_at_epoch<'a>(
        &'a self,
        _epoch: u64,
        method: &'a str,
        params: Value,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<Value, CdpError>> {
        Box::pin(async move { self.reply(method, &params, session) })
    }
    fn send_raw_json<'a>(
        &'a self,
        method: &'a str,
        params: &'a str,
        session: Option<&'a SessionId>,
    ) -> BoxFuture<'a, Result<String, CdpError>> {
        Box::pin(async move {
            Ok(serde_json::to_string(&self.reply(
                method,
                &serde_json::from_str(params)?,
                session,
            )?)?)
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

async fn snapshot(replace_owner: bool) -> anyhow::Result<rmcp::model::CallToolResult> {
    let chain = Arc::new(Chain {
        replace_owner: AtomicBool::new(replace_owner),
        f2_owner_reads: AtomicUsize::new(0),
        events: broadcast::channel(16).0,
    });
    let session = BrowserSession::new_with_native(
        chain,
        BrowserSessionHooks::default(),
        crate::tests::current_native(),
    );
    let service = BrowserMcpService::new(crate::tests::service_options(session));
    let pages = service
        .call_tool_for_testing(
            "tabs",
            json!({ "action": "list" }),
            CancellationToken::new(),
        )
        .await?;
    let page = pages
        .structured_content
        .as_ref()
        .and_then(|data| data["pages"][0]["page"].as_u64())
        .ok_or_else(|| anyhow::anyhow!("no page in {pages:?}"))?;
    Ok(service
        .call_tool_for_testing(
            "snapshot",
            json!({ "page": page }),
            CancellationToken::new(),
        )
        .await?)
}

#[tokio::test]
async fn snapshot_tool_reports_a_frame_admission_refusal_as_refused() -> anyhow::Result<()> {
    let complete = snapshot(false).await?;
    let text = format!("{:?}", complete.content);
    anyhow::ensure!(
        text.contains("Target 2"),
        "the healthy chain is read through: {text}"
    );

    let partial = snapshot(true).await?;
    let structured = partial
        .structured_content
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no structured content: {partial:?}"))?;
    let errors = structured["frameErrors"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let f2 = errors
        .iter()
        .find(|error| error["frameId"] == "f2")
        .ok_or_else(|| anyhow::anyhow!("f2 not reported: {structured}"))?;
    anyhow::ensure!(f2["reason"] == "refused", "f2 reported as {f2}");
    let text = format!("{:?}", partial.content);
    anyhow::ensure!(
        text.contains("Target 1"),
        "readable frames keep their content: {text}"
    );
    anyhow::ensure!(
        !text.contains("Target 2"),
        "the refused frame shows no content: {text}"
    );
    Ok(())
}
