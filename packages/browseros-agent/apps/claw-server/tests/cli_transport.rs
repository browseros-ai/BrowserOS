//! The transport, against a fixture that records what the CLI actually sent.
//!
//! Every assertion here is a rule read out of the server rather than invented:
//! the /mcp hygiene middleware 403s a request carrying Origin or
//! Sec-Fetch-Site, POST must be JSON, a stateless request names its protocol
//! and method in headers, and the reply may be SSE framed. Each of those is an
//! intermittent mystery if it is only ever tested against a real server that
//! happens to behave. There is no session: a call carries the task's tab group
//! and nothing else, and the client follows the group the server names back.

use std::sync::{Arc, Mutex};

use claw_server::cli::client::Client;
use claw_server::cli::client::transport::{
    CLIENT_NAME, PROTOCOL_VERSION, Transport, call_message, decode_body,
};
use claw_server::cli::state::{Store, scope};
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
}

impl Canned {
    fn json(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: body.to_string(),
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

            let response = format!(
                "HTTP/1.1 {} OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
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
    let _ = transport.post(&call_message(1, "tabs", &json!({}))).await;

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
    // Stateless: the protocol and the method ride in headers the server
    // checks against the body, and no session is ever presented.
    assert_eq!(
        request.header("mcp-protocol-version"),
        Some(PROTOCOL_VERSION)
    );
    assert_eq!(request.header("mcp-method"), Some("tools/call"));
    assert_eq!(request.header("mcp-name"), Some("tabs"));
    assert_eq!(request.header("mcp-session-id"), None);
    assert!(
        request
            .header("accept")
            .unwrap_or_default()
            .contains("text/event-stream"),
        "both framings must be advertised"
    );
}

#[tokio::test]
async fn an_sse_framed_reply_is_read_exactly_like_a_json_one() {
    let payload = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [] } });
    let fixture = fixture(vec![Canned {
        status: 200,
        content_type: "text/event-stream",
        body: format!("event: message\ndata: {payload}\n\n"),
    }])
    .await;
    let transport = transport(&fixture.url);
    let reply = transport
        .post(&call_message(1, "tabs", &json!({})))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(reply.body, Some(payload));
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
        .post(&call_message(1, "tabs", &json!({})))
        .await
        .err()
        .unwrap_or_else(|| panic!("a dead port must fail"));
    assert_eq!(
        claw_server::cli::exit::code_of(&error),
        claw_server::cli::exit::Code::NoBrowser
    );
}

#[test]
fn an_empty_or_unparsable_body_decodes_to_nothing_rather_than_panicking() {
    assert_eq!(decode_body("application/json", ""), None);
    assert_eq!(decode_body("application/json", "not json"), None);
}

/// The JSON-RPC body of a recorded request.
fn body_of(request: &Recorded) -> serde_json::Value {
    serde_json::from_str(&request.body).unwrap_or_else(|error| panic!("{error}"))
}

fn client_for(url: &str, dir: &std::path::Path) -> Client {
    let scope = scope(std::path::Path::new("/repo"), None);
    let store = Store::open_in(dir, &scope).unwrap_or_else(|error| panic!("{error}"));
    let transport = Transport::new(url, std::time::Duration::from_secs(5))
        .unwrap_or_else(|error| panic!("{error}"));
    let state = store.load();
    Client::new(transport, store, state)
}

/// A tool result naming `group` as its task's, the way the server answers the CLI.
fn result_in_group(group: &str) -> Canned {
    Canned::json(
        200,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "ok" }],
                "_meta": { "com.browseros.neo/group": group }
            }
        }),
    )
}

#[tokio::test]
async fn a_call_is_one_request_with_no_handshake() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![result_in_group("G1")]).await;
    let mut client = client_for(&fixture.url, dir.path());

    client
        .call("tabs", json!({ "action": "list" }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(seen.len(), 1, "no initialize, no notification: {seen:?}");
    let body = body_of(&seen[0]);
    assert_eq!(body["method"], "tools/call");
    // The server recognises the CLI by this name; it is what turns on the
    // group-keyed task model for this call.
    assert_eq!(
        body["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
        CLIENT_NAME
    );
}

#[tokio::test]
async fn a_call_presents_the_recorded_group_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![result_in_group("G1")]).await;
    let mut client = client_for(&fixture.url, dir.path());
    client
        .persist(|state| state.group_id = Some("G1".into()))
        .unwrap_or_else(|error| panic!("{error}"));

    client
        .call("snapshot", json!({ "page": 3 }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let arguments = &body_of(&seen[0])["params"]["arguments"];
    assert_eq!(arguments, &json!({ "page": 3, "group": "G1" }));
    assert_eq!(seen[0].header("mcp-session-id"), None);
}

#[tokio::test]
async fn the_group_the_server_names_is_kept_for_the_next_command() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![result_in_group("G7")]).await;
    let mut client = client_for(&fixture.url, dir.path());

    client
        .call("tabs", json!({ "action": "new", "url": "https://x.test" }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(client.state().group_id.as_deref(), Some("G7"));
    // And on disk, which is what the next process reads.
    assert_eq!(client.store().load().group_id.as_deref(), Some("G7"));
}

#[tokio::test]
async fn a_result_naming_no_group_ends_the_recorded_task() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    // The user closed the task's tabs, so the server resolved no open task.
    let fixture = fixture(vec![Canned::json(
        200,
        json!({ "jsonrpc": "2.0", "id": 2, "result": { "content": [] } }),
    )])
    .await;
    let mut client = client_for(&fixture.url, dir.path());
    client
        .persist(|state| {
            state.group_id = Some("G1".into());
            state.task = Some("price check".into());
        })
        .unwrap_or_else(|error| panic!("{error}"));

    client
        .call("tabs", json!({ "action": "list" }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(client.state().group_id, None);
    assert_eq!(client.state().task, None);
}

#[tokio::test]
async fn the_plan_refusal_exits_with_its_own_code_and_keeps_the_task() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![Canned::json(
        200,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "isError": true,
                "content": [{
                    "type": "text",
                    "text": "ultrafast_required: the browseros command line is part of the ultrafast plan, which this computer does not have."
                }]
            }
        }),
    )])
    .await;
    let mut client = client_for(&fixture.url, dir.path());
    client
        .persist(|state| state.group_id = Some("G1".into()))
        .unwrap_or_else(|error| panic!("{error}"));

    let error = client
        .call("tabs", json!({ "action": "list" }))
        .await
        .err()
        .unwrap_or_else(|| panic!("the refusal must fail the command"));

    assert_eq!(
        claw_server::cli::exit::code_of(&error),
        claw_server::cli::exit::Code::NotOnPlan
    );
    assert!(error.to_string().contains("ultrafast plan"), "{error}");
    assert_eq!(
        client.state().group_id.as_deref(),
        Some("G1"),
        "a refused call never reached a task, so it cannot have ended one"
    );
}

/// One chunk of a saved output part, as `read_output` answers it.
fn output_chunk(text: &str, offset: u64, next: Option<u64>, total: u64) -> Canned {
    let cursor = json!({
        "outputId": "out-1", "part": "read.txt", "offset": offset, "bytes": text.len(),
        "totalBytes": total, "nextOffset": next, "eof": next.is_none()
    });
    let mut structured = cursor.clone();
    structured["text"] = json!(text);
    Canned::json(
        200,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": format!("Output chunk: {cursor}\n{text}") }],
                "structuredContent": structured
            }
        }),
    )
}

#[tokio::test]
async fn a_saved_output_is_read_to_its_end_as_one_text() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![
        output_chunk("first ", 0, Some(6), 17),
        output_chunk("second ", 6, Some(13), 17),
        output_chunk("last", 13, None, 17),
    ])
    .await;
    let mut client = client_for(&fixture.url, dir.path());
    let args = json!({ "outputId": "out-1", "part": "read.txt", "limit": 65536 });
    let mut output = client
        .call("read_output", args.clone())
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let chunks = claw_server::cli::exec::read_rest_of_output(
        &mut client,
        args.as_object().unwrap_or_else(|| unreachable!()),
        &mut output,
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(chunks, 3);
    assert_eq!(output.text, "first \nsecond \nlast");
    assert!(
        !output.text.contains("Output chunk:"),
        "the cursor lines are paged away"
    );
    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let offsets: Vec<_> = seen
        .iter()
        .map(|request| body_of(request)["params"]["arguments"]["offset"].clone())
        .collect();
    assert_eq!(offsets, vec![json!(null), json!(6), json!(13)]);
}

#[tokio::test]
async fn a_pending_task_name_rides_on_the_call_that_creates_the_task() {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let fixture = fixture(vec![result_in_group("G1")]).await;
    let mut client = client_for(&fixture.url, dir.path());
    client
        .persist(|state| {
            state.pending_task = Some(claw_server::cli::state::PendingTask {
                name: "price check".into(),
                ..Default::default()
            });
        })
        .unwrap_or_else(|error| panic!("{error}"));

    client
        .call("tabs", json!({ "action": "new", "url": "https://x.test" }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let seen = fixture
        .seen
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(
        body_of(&seen[0])["params"]["arguments"]["taskName"],
        "price check"
    );
}
