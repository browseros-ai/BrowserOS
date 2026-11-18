//! One stateless POST per call. No handshake, no session, nothing carried.
//!
//! Every command is its own process, so it could not hold a connection if it
//! wanted to, and the server does not ask it to: on the 2026-07-28 stateless
//! path each request names its protocol version and client in `_meta`, and the
//! CLI's task travels as the tab group it works in, which the caller puts in
//! the arguments. Hand written because that is all it is, one POST.
//!
//! Four rules come out of the server rather than out of taste, and each one
//! turns into an intermittent mystery if it is guessed:
//!
//! 1. Never send `Origin` or `Sec-Fetch-Site`. The `/mcp` hygiene middleware
//!    rejects both with 403, because browser fetches carry them and native MCP
//!    clients do not.
//! 2. POST must carry `content-type: application/json`, or 415.
//! 3. A stateless request names its protocol version in the
//!    `mcp-protocol-version` header and its method in `mcp-method` (and the
//!    tool in `mcp-name`), which the server checks against the body.
//! 4. The reply may be `text/event-stream`, because `Accept` advertises both
//!    framings and the server chooses.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::cli::exit::{Code, Failure};

/// Advertises both framings, which is why [`decode_body`] has to handle both.
const ACCEPT: &str = "application/json, text/event-stream";
/// The stateless protocol: no handshake, and per-request metadata instead of a
/// session. The optional `agentName` argument is gated on this version too.
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The `clientInfo.name` the server recognises the CLI by. Every group-keyed
/// task behaviour on the server is gated on it, so it must never change alone.
pub const CLIENT_NAME: &str = "browseros-cli";

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
    pub body: Option<Value>,
    pub raw: String,
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

    /// POSTs one JSON-RPC message.
    ///
    /// # Errors
    /// Connection failures become [`Code::NoBrowser`] and read timeouts become
    /// [`Code::Timeout`], so the caller never has to inspect a reqwest error.
    pub async fn post(&self, message: &Value) -> Result<Reply> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let mut request = self
            .http
            .post(self.mcp_url())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, ACCEPT)
            .header("mcp-protocol-version", PROTOCOL_VERSION)
            .header("mcp-method", method);
        if let Some(tool) = message.pointer("/params/name").and_then(Value::as_str) {
            request = request.header("mcp-name", tool);
        }
        let response = request.json(message).send().await.map_err(classify)?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let raw = response.text().await.map_err(classify)?;
        let body = decode_body(&content_type, &raw);

        Ok(Reply { status, body, raw })
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

/// The metadata a stateless request carries in place of a session: which
/// protocol it speaks and which client it is. The client name is what the
/// server resolves the CLI's task model from.
#[must_use]
pub fn request_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {
            "name": CLIENT_NAME,
            "version": crate::VERSION,
        }
    })
}

/// A `tools/call` request body.
#[must_use]
pub fn call_message(id: u64, tool: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": arguments, "_meta": request_meta() }
    })
}

/// A `tools/list` request body.
#[must_use]
pub fn list_message(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": { "_meta": request_meta() }
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
    fn every_request_names_its_protocol_and_the_cli_as_its_client() {
        for message in [call_message(1, "tabs", &json!({})), list_message(2)] {
            let meta = &message["params"]["_meta"];
            assert_eq!(
                meta["io.modelcontextprotocol/protocolVersion"],
                PROTOCOL_VERSION
            );
            assert_eq!(
                meta["io.modelcontextprotocol/clientInfo"]["name"],
                CLIENT_NAME
            );
        }
    }
}
