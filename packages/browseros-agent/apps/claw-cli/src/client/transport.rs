//! One POST per call, carrying a persisted `mcp-session-id`.
//!
//! Hand written rather than taken from rmcp because rmcp 3.2's
//! `StreamableHttpClientTransportConfig` has no field for an initial session
//! id, so its client re-initializes in every process. Tab ownership is keyed to
//! the MCP connection, so a CLI that re-initialized per command would re-orphan
//! its own tabs on every command.
//!
//! Four rules come out of the server rather than out of taste, and each one
//! turns into an intermittent mystery if it is guessed:
//!
//! 1. Never send `Origin` or `Sec-Fetch-Site`. The `/mcp` hygiene middleware
//!    rejects both with 403, because browser fetches carry them and native MCP
//!    clients do not.
//! 2. POST must carry `content-type: application/json`, or 415.
//! 3. DELETE must carry NO content-type. The middleware exempts DELETE from the
//!    JSON check only when the header is absent, which is the shape of rmcp's
//!    session teardown.
//! 4. The reply may be `text/event-stream`, because `Accept` advertises both
//!    framings and the server chooses.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::exit::{Code, Failure};

/// Advertises both framings, which is why [`decode_body`] has to handle both.
const ACCEPT: &str = "application/json, text/event-stream";
const SESSION_HEADER: &str = "mcp-session-id";
/// Newest in the server's `SUPPORTED_PROTOCOL_VERSIONS`. The optional
/// `agentName` argument is gated on this version or later.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// A dead port should fail fast so endpoint resolution can try the next candidate.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Must exceed the server's longest deliberately blocking tool. `await_human_help`
/// holds the call for `HELP_WAIT_CHUNK` (75s) by design and `wait for="time"` is
/// honored up to 90s, so a conventional 30s client timeout would make both look
/// like hangs.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(180);

/// What a single POST came back as, before it is interpreted as a tool result.
pub struct Reply {
    pub status: u16,
    pub session_id: Option<String>,
    pub body: Option<Value>,
    pub raw: String,
}

impl Reply {
    /// Whether this is the server saying the transport session is gone.
    #[must_use]
    pub fn is_session_gone(&self) -> bool {
        self.status == 404
    }
}

pub struct Transport {
    http: reqwest::Client,
    endpoint: String,
}

impl Transport {
    /// # Errors
    /// Fails only if the HTTP client cannot be constructed.
    pub fn new(endpoint: &str, read_timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(read_timeout)
            // No default headers: reqwest sends neither Origin nor
            // Sec-Fetch-Site, and nothing here may ever add them.
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            endpoint: endpoint.trim_end_matches('/').to_string(),
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn mcp_url(&self) -> String {
        format!("{}/mcp", self.endpoint)
    }

    /// POSTs one JSON-RPC message, optionally carrying a session id.
    ///
    /// # Errors
    /// Connection failures become [`Code::NoBrowser`] and read timeouts become
    /// [`Code::Timeout`], so the caller never has to inspect a reqwest error.
    pub async fn post(&self, message: &Value, session: Option<&str>) -> Result<Reply> {
        let mut request = self
            .http
            .post(self.mcp_url())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, ACCEPT);
        if let Some(session) = session {
            request = request.header(SESSION_HEADER, session);
        }
        let response = request.json(message).send().await.map_err(classify)?;

        let status = response.status().as_u16();
        let session_id = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let raw = response.text().await.map_err(classify)?;
        let body = decode_body(&content_type, &raw);

        Ok(Reply {
            status,
            session_id,
            body,
            raw,
        })
    }

    /// Releases the transport session. The server answers 202.
    ///
    /// # Errors
    /// Propagates a classified connection error; a non-202 is not an error,
    /// because the session being gone already is the desired end state.
    pub async fn delete_session(&self, session: &str) -> Result<u16> {
        let response = self
            .http
            .delete(self.mcp_url())
            // Deliberately no content-type: the hygiene middleware exempts
            // DELETE from the JSON check only when the header is absent.
            .header(reqwest::header::ACCEPT, ACCEPT)
            .header(SESSION_HEADER, session)
            .send()
            .await
            .map_err(classify)?;
        Ok(response.status().as_u16())
    }

    /// `GET /system/health`, used to prove an endpoint before trusting it.
    ///
    /// Health rather than ready on purpose: health answers 200 whenever the
    /// process is alive and carries the browser link state in the body, which
    /// is the right question for discovery. Readiness is the strict gate and
    /// belongs to `status`.
    pub async fn health(&self, timeout: Duration) -> Option<Value> {
        let response = self
            .http
            .get(format!("{}/system/health", self.endpoint))
            .timeout(timeout)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Value>().await.ok()
    }

    /// `GET /system/ready`: 503 while the browser link is down.
    pub async fn ready(&self) -> Option<(u16, Value)> {
        let response = self
            .http
            .get(format!("{}/system/ready", self.endpoint))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        Some((status, body))
    }

    /// `GET /api/v1/system`, for the server version that keys the tool cache.
    pub async fn system(&self) -> Option<Value> {
        let response = self
            .http
            .get(format!("{}/api/v1/system", self.endpoint))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        response.json::<Value>().await.ok()
    }
}

fn classify(error: reqwest::Error) -> anyhow::Error {
    if error.is_timeout() {
        return Failure::new(
            Code::Timeout,
            "the server did not answer in time. Raise --timeout, or check the cockpit.",
        )
        .into();
    }
    if error.is_connect() || error.is_request() {
        return Failure::no_browser(format!(
            "cannot reach BrowserOS neo: {error}\n  Check that it is running, or pass --endpoint."
        ))
        .into();
    }
    anyhow::Error::new(error)
}

/// Parses either framing into one JSON value.
///
/// An SSE reply frames the message as `data:` lines inside an event. A client
/// that assumes `application/json` will one day parse `data: {...}` as JSON and
/// fail in a way that looks like a server bug.
#[must_use]
pub fn decode_body(content_type: &str, raw: &str) -> Option<Value> {
    if raw.trim().is_empty() {
        return None;
    }
    if content_type.contains("text/event-stream") {
        let joined = raw
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect::<Vec<_>>()
            .join("\n");
        if joined.is_empty() {
            return None;
        }
        return serde_json::from_str(&joined).ok();
    }
    serde_json::from_str(raw).ok()
}

/// The `initialize` request body.
#[must_use]
pub fn initialize_message(client_name: &str, client_version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": client_name, "version": client_version }
        }
    })
}

/// The notification that completes the handshake. It carries no id and the
/// server answers with no body.
#[must_use]
pub fn initialized_message() -> Value {
    json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
}

/// A `tools/call` request body.
#[must_use]
pub fn call_message(id: u64, tool: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    })
}

/// A `tools/list` request body.
#[must_use]
pub fn list_message(id: u64) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": {} })
}

/// Tells the server to stop a call this client is abandoning.
///
/// Sent on SIGINT. `dispatch.rs` has a `client_cancel` path that stops the tool
/// server side; dying without this leaves the server running a tool nobody will
/// read.
#[must_use]
pub fn cancelled_message(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": { "requestId": id, "reason": "client interrupted" }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_sse_framed_reply_decodes_the_same_as_a_json_one() {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } });
        let plain = decode_body("application/json", &payload.to_string());
        let sse = decode_body(
            "text/event-stream",
            &format!("event: message\ndata: {payload}\n\n"),
        );
        assert_eq!(plain, sse);
        assert_eq!(plain, Some(payload));
    }

    #[test]
    fn a_multi_line_sse_payload_is_rejoined_before_parsing() {
        let decoded = decode_body(
            "text/event-stream",
            "event: message\ndata: {\"a\":\ndata: 1}\n\n",
        );
        assert_eq!(decoded, Some(json!({ "a": 1 })));
    }

    #[test]
    fn an_empty_body_decodes_to_nothing_rather_than_an_error() {
        assert_eq!(decode_body("application/json", "   "), None);
        assert_eq!(decode_body("text/event-stream", "event: ping\n\n"), None);
    }

    #[test]
    fn initialize_negotiates_the_version_that_gates_agent_name() {
        let message = initialize_message("browseros-cli", "0.0.1");
        assert_eq!(message["params"]["protocolVersion"], PROTOCOL_VERSION);
    }
}
