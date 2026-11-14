//! Stateless loopback MCP. Each call owns one POST and response stream, so
//! dropping a run future closes its caller connection and cancels the SDK run.
//! JSON-RPC ids correlate replies; SSE notifications are never mistaken for one.

use std::time::Duration;

use reqwest::Client;
use rmcp::model::CallToolResult;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    Effects, EngineUnavailable, RunCall, RunOutcome, RunStatus, RunTool, TaskKey, TaskTouch,
    TaskTouchKind, reply,
};

const RUN_META: &str = "com.browseros.neo/run";
// The SDK bounds returned JSON at 256 KiB plus console and metadata. Leave
// room for content blocks while refusing an unbounded malformed local reply.
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Wire {
    client: Client,
    endpoint: String,
}

impl Wire {
    pub(super) fn new(port: u16) -> Result<Self, EngineUnavailable> {
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(1))
                .build()
                .map_err(|error| EngineUnavailable::new(error.to_string()))?,
            endpoint: format!("http://127.0.0.1:{port}"),
        })
    }

    pub(super) async fn healthy(&self) -> bool {
        self.client
            .get(format!("{}/system/health", self.endpoint))
            .timeout(Duration::from_millis(500))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    pub(super) async fn tool(&self) -> Result<RunTool, EngineUnavailable> {
        let listed = tokio::time::timeout(
            Duration::from_secs(5),
            self.request("tools/list", json!({})),
        )
        .await
        .map_err(|_| EngineUnavailable::new("the Playwright tool list timed out"))??;
        let tool = listed["tools"]
            .as_array()
            .and_then(|tools| tools.iter().find(|tool| tool["name"] == "run"))
            .cloned()
            .ok_or_else(|| EngineUnavailable::new("the Playwright tool list has no run"))?;
        serde_json::from_value(tool).map_err(|error| {
            EngineUnavailable::new(format!("invalid Playwright run definition: {error}"))
        })
    }

    pub(super) async fn run(&self, call: RunCall) -> Result<RunOutcome, EngineUnavailable> {
        let mut host = json!({ "hostData": call.host_data });
        if let Some(task) = call.task {
            host["taskId"] = json!(task.as_str());
        }
        // Deliberately omit requestId. The SDK then cancels at once when this
        // stream closes, rather than retaining an orphan for a possible retry.
        let result = self
            .request(
                "tools/call",
                json!({
                    "name": "run",
                    "arguments": call.args,
                    "_meta": { RUN_META: host }
                }),
            )
            .await?;
        decode_run(result)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, EngineUnavailable> {
        let id = json!(uuid::Uuid::new_v4().to_string());
        let mut response = self
            .client
            .post(format!("{}/mcp", self.endpoint))
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            // Origin must stay absent. The SDK rejects browser-origin traffic.
            .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| {
                EngineUnavailable::new(format!("the Playwright wire failed: {error}"))
            })?;
        let sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        let mut body = Vec::new();
        let mut events = Events::default();
        let mut bytes = 0;
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            EngineUnavailable::new(format!("the Playwright reply broke: {error}"))
        })? {
            bytes += chunk.len();
            if bytes > MAX_REPLY_BYTES {
                return Err(EngineUnavailable::new(
                    "the Playwright reply exceeded its wire limit",
                ));
            }
            if sse {
                if let Some(result) = events.push(&chunk, &id)? {
                    return Ok(result);
                }
            } else {
                body.extend_from_slice(&chunk);
            }
        }
        if sse {
            // A final SSE event may omit its trailing blank line on EOF.
            events.push(b"\n\n", &id)?.ok_or_else(|| {
                EngineUnavailable::new("the Playwright stream ended without its reply")
            })
        } else {
            let message = serde_json::from_slice(&body).map_err(|error| {
                EngineUnavailable::new(format!("the Playwright reply is not JSON: {error}"))
            })?;
            response_result(message, &id)?.ok_or_else(|| {
                EngineUnavailable::new("the Playwright reply has a different request id")
            })
        }
    }
}

/// Decodes SSE incrementally because a UTF-8 sequence, a line and an event can
/// each cross HTTP chunks. Only a matching JSON-RPC response ends the request.
#[derive(Default)]
struct Events {
    pending: Vec<u8>,
    data: Vec<u8>,
}

impl Events {
    fn push(&mut self, bytes: &[u8], id: &Value) -> Result<Option<Value>, EngineUnavailable> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                if !self.data.is_empty() {
                    let message = serde_json::from_slice(&self.data).map_err(|error| {
                        EngineUnavailable::new(format!("invalid Playwright SSE event: {error}"))
                    })?;
                    self.data.clear();
                    if let Some(result) = response_result(message, id)? {
                        return Ok(Some(result));
                    }
                }
            } else if let Some(data) = line.strip_prefix(b"data:") {
                let data = data.strip_prefix(b" ").unwrap_or(data);
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(data);
            }
        }
        Ok(None)
    }
}

fn response_result(message: Value, id: &Value) -> Result<Option<Value>, EngineUnavailable> {
    if message.get("id") != Some(id) {
        return Ok(None);
    }
    if let Some(result) = message.get("result") {
        return Ok(Some(result.clone()));
    }
    if let Some(error) = message.get("error") {
        return Err(EngineUnavailable::new(format!(
            "the Playwright endpoint refused the request ({})",
            error.get("code").unwrap_or(&Value::Null)
        )));
    }
    Err(EngineUnavailable::new(
        "the Playwright response has no result",
    ))
}

#[derive(Deserialize)]
struct RunMeta {
    id: String,
    status: RunStatus,
    effects: Effects,
    tasks: Vec<WireTask>,
}

#[derive(Deserialize)]
struct WireTask {
    id: TaskKey,
    name: Option<String>,
    kind: TaskTouchKind,
    tabs: Vec<WireTab>,
}

#[derive(Deserialize)]
struct WireTab {
    #[serde(rename = "tabId")]
    tab_id: i64,
}

fn decode_run(value: Value) -> Result<RunOutcome, EngineUnavailable> {
    let reply: CallToolResult = serde_json::from_value(value).map_err(|error| {
        EngineUnavailable::new(format!("invalid Playwright tool reply: {error}"))
    })?;
    let meta = reply
        .meta
        .as_ref()
        .and_then(|meta| meta.0.get(RUN_META))
        .cloned()
        .ok_or_else(|| EngineUnavailable::new("the Playwright reply has no run metadata"))?;
    let meta: RunMeta = serde_json::from_value(meta).map_err(|error| {
        EngineUnavailable::new(format!("invalid Playwright run metadata: {error}"))
    })?;
    Ok(RunOutcome {
        reply: reply::shape(reply),
        run_id: meta.id,
        status: meta.status,
        effects: meta.effects,
        tasks: meta
            .tasks
            .into_iter()
            .map(|task| TaskTouch {
                id: task.id,
                name: task.name,
                kind: task.kind,
                tabs: task.tabs.into_iter().map(|tab| tab.tab_id).collect(),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Events, RUN_META, decode_run};
    use crate::services::neo::{Effects, RunStatus};
    use serde_json::json;

    #[test]
    fn fragmented_sse_ignores_notifications_and_other_requests()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut events = Events::default();
        let input = concat!(
            "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\n",
            "data: {\"id\":4,\"result\":{}}\n\n",
            "data: {\"id\":5,\"result\":{\"value\":\"café\"}}\n\n"
        );
        let mut reply = None;
        for byte in input.as_bytes() {
            if let Some(result) = events.push(&[*byte], &json!(5))? {
                reply = Some(result);
            }
        }
        assert_eq!(reply, Some(json!({"value":"café"})));
        Ok(())
    }

    #[test]
    fn run_reply_preserves_value_and_failed_task_touches() -> Result<(), Box<dyn std::error::Error>>
    {
        let value = json!({
            "content": [{"type":"text","text":"failed"}],
            "structuredContent": {"error":{"message":"failed"}},
            "isError": true,
            "_meta": { RUN_META: {
                "id":"run-one", "status":"error", "effects":"unknown", "unknown":[],
                "tasks":[{"id":"task-one", "name":"work", "kind":"created", "tabs":[{"tabId":8,"url":"https://example.test"}]}]
            }}
        });
        let outcome = decode_run(value.clone())?;
        assert_eq!(outcome.run_id, "run-one");
        assert_eq!(outcome.status, RunStatus::Error);
        assert_eq!(outcome.effects, Effects::Unknown);
        assert_eq!(outcome.tasks[0].tabs, vec![8]);
        assert_eq!(serde_json::to_value(outcome.reply)?, value);
        Ok(())
    }
}
