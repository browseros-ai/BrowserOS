//! Neo: Real HTTP protocol helpers shared by the two host-shutdown suites. They parse the
//! negotiated JSON/SSE wire rather than bypassing rmcp, and keep long streams open explicitly.

use std::cmp::Ordering;
use std::time::Duration;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use rmcp::model::SubscriptionsListenResult;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};

pub fn modern(
    client: &Client,
    url: &str,
    id: u64,
    method: &str,
    mut params: Value,
) -> RequestBuilder {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = client
        .post(url)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .header("Accept", "application/json, text/event-stream");
    if method == "tools/call" {
        request = request.header("Mcp-Name", "run");
    }
    request.json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
}

pub fn parse_result(id: u64, text: &str) -> Value {
    let message: Value = if text.trim_start().starts_with('{') {
        serde_json::from_str(text).unwrap()
    } else {
        text.replace("\r\n", "\n")
            .split_inclusive("\n\n")
            .find_map(|event| {
                let data = event
                    .lines()
                    .filter_map(|line| line.strip_prefix("data:"))
                    .map(str::trim_start)
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() {
                    return None;
                }
                let message: Value = serde_json::from_str(&data).unwrap();
                if message["id"] != id {
                    return None;
                }
                assert!(
                    event.ends_with("\n\n"),
                    "terminal SSE event lacks its blank-line terminator: {event}"
                );
                Some(message)
            })
            .unwrap_or_else(|| panic!("no terminal reply for {id}: {text}"))
    };
    assert_eq!(message["id"], id, "{message}");
    assert!(message.get("error").is_none(), "{message}");
    message["result"].clone()
}

fn media_type(response: &Response) -> &str {
    response
        .headers()
        .get(CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .trim()
}

pub async fn result(response: Response, id: u64) -> Value {
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a run reply must not become HTTP 500"
    );
    let json = match media_type(&response) {
        "application/json" => true,
        "text/event-stream" => false,
        other => panic!("unexpected reply Content-Type: {other}"),
    };
    let text = time::timeout(Duration::from_secs(15), response.text())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        text.trim_start().starts_with('{'),
        json,
        "reply framing must match its JSON/SSE Content-Type"
    );
    parse_result(id, &text)
}

pub async fn legacy_result(response: Response, id: u64) -> Value {
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "legacy reply must be HTTP200"
    );
    assert_eq!(
        media_type(&response),
        "text/event-stream",
        "legacy POST is SSE"
    );
    result(response, id).await
}

pub async fn legacy_session(client: &Client, url: &str) -> String {
    let response = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .json(
            &json!({"jsonrpc":"2.0","id":100,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":{},
                "clientInfo":{"name":"shutdown-regression","version":"1"}
            }}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "legacy initialization must establish a usable session"
    );
    let session = response
        .headers()
        .get("Mcp-Session-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let _initialized = result(response, 100).await;
    let response = client
        .post(url)
        .header("MCP-Protocol-Version", "2025-06-18")
        .header("Mcp-Session-Id", &session)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let _body = response.bytes().await.unwrap();
    session
}

pub async fn legacy_get(client: &Client, url: &str, session: &str) -> Response {
    let response = client
        .get(url)
        .header("MCP-Protocol-Version", "2025-06-18")
        .header("Mcp-Session-Id", session)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the legacy GET must open before shutdown"
    );
    assert_eq!(
        media_type(&response),
        "text/event-stream",
        "legacy GET is SSE"
    );
    response
}

pub fn legacy_run(
    client: &Client,
    url: &str,
    session: &str,
    id: u64,
    params: &Value,
) -> RequestBuilder {
    client
        .post(url)
        .header("MCP-Protocol-Version", "2025-06-18")
        .header("Mcp-Session-Id", session)
        .header("Accept", "application/json, text/event-stream")
        .json(
            &json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
                "name":"run","arguments":params
            }}),
        )
}

/// The acknowledgment proves the listen handler was established before shutdown is triggered.
/// Keep its prefix because a TCP chunk may also contain part of the eventual terminal result.
pub struct Listen {
    response: Response,
    prefix: String,
    id: u64,
}

impl Listen {
    pub async fn open(client: &Client, url: &str, id: u64) -> Self {
        let mut response = modern(
            client,
            url,
            id,
            "subscriptions/listen",
            json!({"notifications":{}}),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "listen must open before shutdown"
        );
        assert_eq!(media_type(&response), "text/event-stream", "listen is SSE");
        let prefix = time::timeout(Duration::from_secs(5), async {
            let mut prefix = Vec::new();
            loop {
                let Some(chunk) = response.chunk().await.unwrap() else {
                    panic!("listen ended before its acknowledgment");
                };
                prefix.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&prefix)
                    .contains("notifications/subscriptions/acknowledged")
                {
                    break String::from_utf8(prefix).unwrap();
                }
            }
        })
        .await
        .unwrap();
        Self {
            response,
            prefix,
            id,
        }
    }

    pub async fn complete(self) {
        let tail = time::timeout(Duration::from_secs(15), self.response.text())
            .await
            .unwrap()
            .unwrap();
        let text = format!("{}{tail}", self.prefix);
        let result = parse_result(self.id, &text);
        assert_eq!(
            result["resultType"], "complete",
            "listen must end with the protocol result"
        );
        let result: SubscriptionsListenResult = serde_json::from_value(result).unwrap();
        assert_eq!(
            serde_json::to_value(result.meta.subscription_id()).unwrap(),
            self.id,
            "the final listen result must retain its subscription identity"
        );
    }
}

pub async fn get_eof(response: Response) {
    let _body = time::timeout(Duration::from_secs(10), response.text())
        .await
        .unwrap()
        .unwrap();
}

/// Require the strict POST outcome and clean GET EOF concurrently. The returned observation
/// order is diagnostic: separate connections and client tasks have no server-guaranteed order.
/// Modern POST order is T3c's `transport_cannot_end_after_runs_drain_while_a_production_body_is_unread`
/// plus the source fact that GET ends through transport; T3c-L covers the legacy POST ending
/// on its own before transport cancellation. Neither body drop nor server handoff is receipt.
pub async fn reply_and_get_eof(reply: JoinHandle<Value>, get: Response) -> (Value, &'static str) {
    time::timeout(Duration::from_secs(20), async {
        let ((result, post_at), get_at) = tokio::join!(
            async {
                let result = reply.await;
                (result, Instant::now())
            },
            async {
                get_eof(get).await;
                Instant::now()
            }
        );
        let order = match post_at.cmp(&get_at) {
            Ordering::Less => "post_first",
            Ordering::Greater => "get_first",
            Ordering::Equal => "same_tick",
        };
        (result.unwrap(), order)
    })
    .await
    .unwrap()
}

pub async fn until(mut condition: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{description}");
        time::sleep(Duration::from_millis(2)).await;
    }
}
