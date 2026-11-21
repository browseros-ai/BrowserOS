//! Raw JSON-RPC to the engine's loopback `/mcp`, the approach of the unmerged claw-server branch.
//!
//! Each request is one POST whose reply is one JSON body or an SSE stream, so dropping a call's
//! future closes its connection, and the engine cancels the script within about 100 ms (probe
//! p1). The JSON-RPC id is fresh per request and only the matching response ends it; an SSE
//! notification or keep-alive is skipped. No `Origin` header is ever sent: the engine refuses
//! browser-origin traffic.

use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};

use super::super::{Access, Scope, Tool};
use super::RUN;

/// The `_meta` key the engine reads a host's task and access from.
pub(super) const RUN_META: &str = "com.browseros.neo/run";

/// The engine bounds a reply's JSON at 256 KiB, which its text and structured copies double,
/// plus inline picture bytes. This bound only refuses a broken reply that never ends.
const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Wire {
    client: Client,
    endpoint: String,
}

impl Wire {
    pub(super) fn new(port: u16) -> Result<Self, String> {
        let client = Client::builder()
            // A proxy from the environment must never see a loopback call into the browser.
            .no_proxy()
            // A fresh connection per request: a kept-alive one to a child that just died fails
            // in a way that cannot say whether the script started, while a fresh connect to a
            // dead port is refused, which proves nothing ran (`WireError::NotDelivered`).
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(1))
            .build()
            .map_err(|error| format!("building the engine client: {error}"))?;
        Ok(Self {
            client,
            endpoint: format!("http://127.0.0.1:{port}"),
        })
    }

    /// The engine's `GET /system/ready`: 200 once it is linked to the browser, 503 before.
    pub(super) async fn ready(&self) -> bool {
        self.client
            .get(format!("{}/system/ready", self.endpoint))
            .timeout(Duration::from_millis(500))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    pub(super) async fn tool(&self) -> Result<Tool, String> {
        let listed = tokio::time::timeout(
            Duration::from_secs(5),
            self.request("tools/list", json!({})),
        )
        .await
        .map_err(|_| "the engine's tool list timed out".to_owned())?
        .map_err(|error| error.to_string())?;
        let run = listed["tools"]
            .as_array()
            .and_then(|tools| tools.iter().find(|tool| tool["name"] == RUN))
            .ok_or("the engine lists no `run` tool")?;
        tool_from(run)
    }

    /// One `run` call. `scope` is `None` only for the readiness probe. The arguments go through
    /// as the caller gave them, for the engine to validate (`code`, `timeout`, nothing else).
    pub(super) async fn run(&self, args: Value, scope: Option<&Scope>) -> Result<Value, WireError> {
        let mut params = json!({ "name": RUN, "arguments": args });
        if let Some(scope) = scope {
            params["_meta"] = json!({ RUN_META: run_meta(scope) });
        }
        self.request("tools/call", params).await
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, WireError> {
        let id = json!(uuid::Uuid::new_v4().to_string());
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut response = self
            .client
            .post(format!("{}/mcp", self.endpoint))
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                let reason = format!("the engine did not answer: {}", chain(&error));
                // Only a refused connect proves the engine never read the call: rmcp starts the
                // handler before it writes the response headers, so a connection that broke
                // later may have started the script.
                if error.is_connect() {
                    WireError::NotDelivered(reason)
                } else {
                    WireError::Lost(reason)
                }
            })?
            .error_for_status()
            .map_err(|error| WireError::Refused(format!("the engine refused the call: {error}")))?;
        let sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        let mut events = Events::default();
        let mut body = Vec::new();
        let mut read = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| WireError::Lost(format!("the engine's reply broke off: {error}")))?
        {
            read += chunk.len();
            if read > MAX_REPLY_BYTES {
                return Err(WireError::Refused(
                    "the engine's reply exceeded its wire limit".to_owned(),
                ));
            }
            if sse {
                if let Some(result) = events.push(&chunk, &id).map_err(WireError::Refused)? {
                    return Ok(result);
                }
            } else {
                body.extend_from_slice(&chunk);
            }
        }
        if sse {
            // The last event may end at EOF without its blank line.
            events
                .push(b"\n\n", &id)
                .map_err(WireError::Refused)?
                .ok_or_else(|| {
                    WireError::Lost("the engine's stream ended without a reply".to_owned())
                })
        } else {
            let message: Value = serde_json::from_slice(&body).map_err(|error| {
                WireError::Refused(format!("the engine's reply is not JSON: {error}"))
            })?;
            response_result(&message, &id)
                .map_err(WireError::Refused)?
                .ok_or_else(|| {
                    WireError::Refused("the engine replied to another request".to_owned())
                })
        }
    }
}

/// Why a request to the engine failed, by what it says about the script.
pub(super) enum WireError {
    /// The connection was refused: the engine never read the call, which is safe to send again.
    NotDelivered(String),
    /// The connection broke before the reply was whole: the script may have acted.
    Lost(String),
    /// The engine answered, but not with a result. Nothing ran beyond what it reports.
    Refused(String),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotDelivered(reason) | Self::Lost(reason) | Self::Refused(reason) => {
                f.write_str(reason)
            }
        }
    }
}

/// An error with its causes: reqwest's own text ("error sending request") names none of them.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// `taskId` always; `access: "read"` only for read-only scope, because the engine refuses an
/// explicit `null` and reads a missing key as full access. No `requestId`: with one, a dropped
/// request would wait out a 15 s rejoin grace instead of cancelling at once.
fn run_meta(scope: &Scope) -> Value {
    let mut meta = json!({ "taskId": scope.task });
    if scope.access == Access::Read {
        meta["access"] = json!("read");
    }
    meta
}

/// A `run` definition, from `tools/list` or the built-in copy.
pub(super) fn parse_tool(bytes: &[u8]) -> Result<Tool, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    tool_from(&value)
}

fn tool_from(value: &Value) -> Result<Tool, String> {
    let name = value["name"].as_str().ok_or("a tool without a name")?;
    let description = value["description"].as_str().unwrap_or_default();
    let input_schema = value
        .get("inputSchema")
        .filter(|schema| schema.is_object())
        .ok_or("a tool without an input schema")?;
    Ok(Tool {
        name: name.to_owned(),
        description: description.to_owned(),
        input_schema: input_schema.clone(),
    })
}

/// An incremental SSE decoder: a UTF-8 sequence, a line and an event can each cross chunks.
#[derive(Default)]
struct Events {
    pending: Vec<u8>,
    data: Vec<u8>,
}

impl Events {
    fn push(&mut self, bytes: &[u8], id: &Value) -> Result<Option<Value>, String> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=end).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                if self.data.is_empty() {
                    continue;
                }
                let message: Value = serde_json::from_slice(&self.data)
                    .map_err(|error| format!("the engine sent a broken event: {error}"))?;
                self.data.clear();
                if let Some(result) = response_result(&message, id)? {
                    return Ok(Some(result));
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

/// `Some(result)` for the response to `id`; `None` for any other message. A failed script is a
/// result with `isError`; a JSON-RPC error means the engine refused the request itself.
fn response_result(message: &Value, id: &Value) -> Result<Option<Value>, String> {
    if message.get("id") != Some(id) {
        return Ok(None);
    }
    if let Some(result) = message.get("result") {
        return Ok(Some(result.clone()));
    }
    let error = &message["error"];
    Err(format!(
        "the engine refused the request ({}: {})",
        error["code"],
        error["message"].as_str().unwrap_or("no message")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_stream_split_anywhere_yields_only_its_own_reply() -> Result<(), String> {
        let stream = concat!(
            "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\n",
            ": keep-alive\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":\"other\",\"result\":{}}\n\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":\"mine\",\"result\":{\"value\":\"café\"}}"
        );
        let id = json!("mine");
        let mut events = Events::default();
        let mut reply = None;
        for byte in stream.as_bytes() {
            if let Some(result) = events.push(&[*byte], &id)? {
                reply = Some(result);
            }
        }
        if reply.is_none() {
            reply = events.push(b"\n\n", &id)?;
        }
        assert_eq!(reply, Some(json!({ "value": "café" })));
        Ok(())
    }

    #[test]
    fn a_json_rpc_error_names_its_code() {
        let message = json!({"jsonrpc":"2.0","id":7,"error":{"code":-32602,"message":"bad"}});
        assert_eq!(
            response_result(&message, &json!(7)),
            Err("the engine refused the request (-32602: bad)".to_owned())
        );
    }

    #[test]
    fn read_scope_sends_read_access_and_full_scope_sends_no_access_key() {
        let read = run_meta(&Scope {
            task: "c1".into(),
            access: Access::Read,
        });
        assert_eq!(read, json!({ "taskId": "c1", "access": "read" }));
        let full = run_meta(&Scope {
            task: "c1".into(),
            access: Access::Full,
        });
        assert_eq!(full, json!({ "taskId": "c1" }));
    }
}
