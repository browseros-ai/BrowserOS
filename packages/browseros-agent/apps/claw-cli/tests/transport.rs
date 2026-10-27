//! The transport, against a fixture that records what the CLI actually sent.
//!
//! Every assertion here is a rule read out of the server rather than invented:
//! the /mcp hygiene middleware 403s a request carrying Origin or
//! Sec-Fetch-Site, POST must be JSON, DELETE must carry no content-type, and
//! the reply may be SSE framed. Each of those is an intermittent mystery if it
//! is only ever tested against a real server that happens to behave.

use std::sync::{Arc, Mutex};

use claw_cli::client::transport::{
    Transport, call_message, decode_body, initialize_message, initialized_message,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// One recorded request: the head, and the body if there was one.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// What the fixture answers with, per request, in order.
#[derive(Clone)]
struct Canned {
    status: u16,
    content_type: &'static str,
    body: String,
    session_id: Option<&'static str>,
}

impl Canned {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
            session_id: None,
        }
    }

    fn with_session(mut self, id: &'static str) -> Self {
        self.session_id = Some(id);
        self
    }

    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: body.to_string(),
            session_id: None,
        }
    }
}

struct Fixture {
    url: String,
    seen: Arc<Mutex<Vec<Recorded>>>,
}

/// A deliberately tiny HTTP/1.1 server. claw-cli does not depend on a server
/// framework and should not grow one for a test.
async fn fixture(responses: Vec<Canned>) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("binding the fixture: {error}"));
    let port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or(0);
    let seen: Arc<Mutex<Vec<Recorded>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    tokio::spawn(async move {
        for canned in responses {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut raw = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read the head, then exactly content-length bytes of body.
            loop {
                let Ok(read) = socket.read(&mut buffer).await else {
                    return;
                };
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&raw).to_string();
                let Some(split) = text.find("\r\n\r\n") else {
                    continue;
                };
                let head = &text[..split];
                let body_so_far = text.len() - (split + 4);
                let length = head
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if body_so_far >= length {
                    break;
                }
            }

            let text = String::from_utf8_lossy(&raw).to_string();
            let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
            let mut lines = head.lines();
            let method = lines
                .next()
                .and_then(|line| line.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            let headers = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .collect();
            if let Ok(mut log) = recorder.lock() {
                log.push(Recorded {
                    method,
                    headers,
                    body: body.to_string(),
                });
            }

            let session = canned
                .session_id
                .map(|id| format!("mcp-session-id: {id}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 {} OK\r\ncontent-type: {}\r\ncontent-length: {}\r\n{session}connection: close\r\n\r\n{}",
                canned.status,
                canned.content_type,
                canned.body.len(),
                canned.body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });

    Fixture {
        url: format!("http://127.0.0.1:{port}"),
        seen,
    }
}

fn transport(url: &str) -> Transport {
    Transport::new(url, std::time::Duration::from_secs(5))
        .unwrap_or_else(|error| panic!("building the transport: {error}"))
}

#[tokio::test]
async fn a_post_never_carries_the_headers_the_hygiene_middleware_rejects() {
    let fixture = fixture(vec![Canned::json(200, json!({ "result": {} }))]).await;
    let transport = transport(&fixture.url);
    let _ = transport
        .post(&call_message(1, "tabs", &json!({})), Some("abc"))
        .await;

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let request = seen.first().unwrap_or_else(|| panic!("nothing was sent"));
    // The middleware answers 403 to either of these, because a browser fetch
    // carries them and a native MCP client does not.
    assert_eq!(request.header("origin"), None, "{:?}", request.headers);
    assert_eq!(
        request.header("sec-fetch-site"),
        None,
        "{:?}",
        request.headers
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("mcp-session-id"), Some("abc"));
    assert!(
        request
            .header("accept")
            .unwrap_or_default()
            .contains("text/event-stream"),
        "both framings must be advertised"
    );
}

#[tokio::test]
async fn delete_carries_no_content_type_because_that_is_the_only_shape_exempted() {
    let fixture = fixture(vec![Canned::text(202, "")]).await;
    let transport = transport(&fixture.url);
    let status = transport.delete_session("abc").await.unwrap_or(0);
    assert_eq!(status, 202);

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let request = seen.first().unwrap_or_else(|| panic!("nothing was sent"));
    assert_eq!(request.method, "DELETE");
    assert_eq!(
        request.header("content-type"),
        None,
        "the middleware exempts DELETE from the JSON check only when the header is absent"
    );
    assert_eq!(request.header("mcp-session-id"), Some("abc"));
}

#[tokio::test]
async fn an_sse_framed_reply_is_read_exactly_like_a_json_one() {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [] } });
    let fixture = fixture(vec![Canned {
        status: 200,
        content_type: "text/event-stream",
        body: format!("event: message\ndata: {payload}\n\n"),
        session_id: None,
    }])
    .await;
    let transport = transport(&fixture.url);
    let reply = transport
        .post(&call_message(1, "tabs", &json!({})), Some("abc"))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(reply.body, Some(payload));
}

#[tokio::test]
async fn the_session_id_is_taken_from_the_response_header() {
    let fixture = fixture(vec![
        Canned::json(
            200,
            json!({ "result": { "protocolVersion": "2026-07-28" } }),
        )
        .with_session("sess-123"),
    ])
    .await;
    let transport = transport(&fixture.url);
    let reply = transport
        .post(&initialize_message("browseros-cli", "0.0.1"), None)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(reply.session_id.as_deref(), Some("sess-123"));

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let request = seen.first().unwrap_or_else(|| panic!("nothing was sent"));
    assert_eq!(
        request.header("mcp-session-id"),
        None,
        "initialize is the one call that must not present an id"
    );
    assert!(request.body.contains("\"initialize\""), "{}", request.body);
}

#[tokio::test]
async fn a_dead_session_is_recognised_from_the_status_alone() {
    let fixture = fixture(vec![Canned::text(404, "Not Found: Session not found")]).await;
    let transport = transport(&fixture.url);
    let reply = transport
        .post(&call_message(1, "tabs", &json!({})), Some("gone"))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(reply.is_session_gone());
}

#[tokio::test]
async fn health_proves_an_endpoint_and_a_dead_port_fails_fast() {
    let fixture = fixture(vec![Canned::json(200, json!({ "status": "ok" }))]).await;
    let live = transport(&fixture.url);
    assert!(
        live.health(std::time::Duration::from_secs(2))
            .await
            .is_some(),
        "a live server must answer health"
    );

    let dead = transport("http://127.0.0.1:1");
    assert!(
        dead.health(std::time::Duration::from_millis(400))
            .await
            .is_none(),
        "a dead port must not look live"
    );
}

#[tokio::test]
async fn a_connection_failure_is_classified_as_no_browser_rather_than_a_tool_error() {
    let transport = transport("http://127.0.0.1:1");
    let error = transport
        .post(&call_message(1, "tabs", &json!({})), None)
        .await
        .err()
        .unwrap_or_else(|| panic!("a dead port must fail"));
    assert_eq!(
        claw_cli::exit::code_of(&error),
        claw_cli::exit::Code::NoBrowser
    );
}

#[test]
fn the_notification_that_completes_the_handshake_carries_no_id() {
    let message = initialized_message();
    assert_eq!(message["method"], "notifications/initialized");
    assert!(message.get("id").is_none(), "a notification has no id");
}

#[test]
fn an_empty_or_unparsable_body_decodes_to_nothing_rather_than_panicking() {
    assert_eq!(decode_body("application/json", ""), None);
    assert_eq!(decode_body("application/json", "not json"), None);
}
