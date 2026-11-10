//! Neo: T3c exercises the real inner MCP router, not a substitute route. An unpolled
//! response must still own its POST token, closing the handler-to-body loss window W2.

use std::pin::Pin;
use std::str;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use axum::Router;
use axum::body::{self, Body};
use axum::http::{Method, Request, StatusCode, header};
use axum::response::Response;
use futures_util::future;
use http_body::Body as HttpBody;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::task;
use tokio::time;
use tower::ServiceExt;

use super::NeoSdkServer;
use crate::Config;
use tokio_util::sync::CancellationToken;

fn modern(method: &Method) -> Request<Body> {
    // Oneshot has no HTTP connection to synthesize Host. Supply a valid loopback
    // authority so rmcp's host guard admits the protocol reply this test must track.
    let mut request = Request::builder()
        .method(method.clone())
        .uri("/mcp")
        .header("Host", "127.0.0.1")
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Accept", "application/json, text/event-stream");
    let body = if *method == Method::POST {
        request = request
            .header("Content-Type", "application/json")
            .header("Mcp-Method", "tools/list");
        Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{
                "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientCapabilities":{}}
            }})
            .to_string(),
        )
    } else {
        Body::empty()
    };
    request.body(body).unwrap()
}

#[tokio::test]
async fn production_router_counts_unpolled_posts_until_eof_or_drop_but_never_gets() {
    let state = TempDir::new().unwrap();
    let server = NeoSdkServer::new(
        Config::new(1, state.path().to_path_buf()),
        CancellationToken::new(),
    );
    let router = server.router();
    let response = router.clone().oneshot(modern(&Method::POST)).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "ownership must cover a successful tools/list reply, not a guard error"
    );
    assert_eq!(
        server.replies.len(),
        1,
        "R1: the production fallback body is unpolled"
    );
    let bytes = body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        !bytes.is_empty(),
        "tools/list produced a real protocol reply"
    );
    assert_eq!(
        server.replies.len(),
        0,
        "EOF releases production reply ownership"
    );

    let response = router.clone().oneshot(modern(&Method::POST)).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "unread-drop ownership must cover a successful tools/list reply"
    );
    assert_eq!(server.replies.len(), 1);
    drop(response);
    assert_eq!(
        server.replies.len(),
        0,
        "unread response drop releases ownership"
    );

    let response = router.oneshot(modern(&Method::GET)).await.unwrap();
    assert_eq!(
        server.replies.len(),
        0,
        "GET is never part of the POST drain"
    );
    drop(response);
    server.shutdown().await;
}

#[tokio::test]
async fn transport_cannot_end_after_runs_drain_while_a_production_body_is_unread() {
    let state = TempDir::new().unwrap();
    let server = NeoSdkServer::new(
        Config::new(1, state.path().to_path_buf()),
        CancellationToken::new(),
    );
    let response = server
        .router()
        .oneshot(modern(&Method::POST))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the transport-order proof must hold a successful protocol reply"
    );
    assert_eq!(server.replies.len(), 1);
    let closing = server.clone();
    let close = tokio::spawn(async move { closing.close_transport().await });
    server.shutdown.cancelled().await;
    server.runs.wait().await;
    reply_drain_entered(&server).await;
    assert!(
        !server.transport.is_cancelled(),
        "run completion is not body delivery"
    );
    assert!(!close.is_finished());
    drop(response);
    close.await.unwrap();
    assert!(server.transport.is_cancelled());
    assert_eq!(server.replies.len(), 0);
    server.close_transport().await;
    server.shutdown().await;
}

fn legacy(method: Method, session: Option<&str>, message: Option<&Value>) -> Request<Body> {
    let accept = if method == Method::GET {
        "text/event-stream"
    } else {
        "application/json, text/event-stream"
    };
    let mut request = Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::HOST, "127.0.0.1")
        .header("MCP-Protocol-Version", "2025-06-18")
        .header(header::ACCEPT, accept);
    if let Some(session) = session {
        request = request.header("Mcp-Session-Id", session);
    }
    let body = if let Some(message) = message {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(message.to_string())
    } else {
        Body::empty()
    };
    request.body(body).unwrap()
}

/// Keep one session manager by cloning the caller's router for both handshake requests.
async fn legacy_session(router: &Router) -> String {
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"legacy-body-proof","version":"1"}
    }});
    let response = router
        .clone()
        .oneshot(legacy(Method::POST, None, Some(&initialize)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "initialize must succeed");
    let session = response.headers()["Mcp-Session-Id"]
        .to_str()
        .unwrap()
        .to_owned();
    let _initialized = body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let notification = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    let response = router
        .clone()
        .oneshot(legacy(Method::POST, Some(&session), Some(&notification)))
        .await
        .unwrap();
    assert!(response.status().is_success(), "initialized must succeed");
    drop(response);
    session
}

fn assert_sse(response: &Response) {
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "legacy stream must succeed"
    );
    assert_eq!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .trim(),
        "text/event-stream",
        "the proof requires a legacy SSE body, not a finite JSON/error body"
    );
}

fn assert_tools_result(bytes: &[u8], id: u64) {
    let text = str::from_utf8(bytes).unwrap().replace("\r\n", "\n");
    assert!(
        text.ends_with("\n\n"),
        "terminal SSE event must be complete"
    );
    let result = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line.trim()).unwrap())
        .find(|message| message["id"] == id)
        .unwrap();
    assert!(result.get("error").is_none(), "{result}");
    assert_eq!(result["result"]["tools"][0]["name"], "run", "{result}");
}

/// Observing the tracker close proves phase 1 reached the reply drain, not just `runs.wait()`.
async fn reply_drain_entered(server: &NeoSdkServer) {
    time::timeout(Duration::from_secs(20), async {
        while !server.replies.is_closed() {
            task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// A nonblocking GET probe permits priming/keep-alive frames but must stop at Pending.
fn assert_get_is_pending(get: &mut Body) {
    let mut context = Context::from_waker(Waker::noop());
    loop {
        match Pin::new(&mut *get).poll_frame(&mut context) {
            Poll::Ready(Some(Ok(_frame))) => {}
            Poll::Ready(Some(Err(error))) => panic!("legacy GET body error: {error}"),
            Poll::Ready(None) => panic!("legacy GET ended while the POST was unread"),
            Poll::Pending => break,
        }
    }
}

// T3c-L/C1: current-thread scheduling is essential. At POST EOF the test can assert that
// transport is still live before the awakened close task runs. A grace-cut POST cannot pass.
#[tokio::test]
async fn legacy_post_self_terminates_before_transport_cancellation() {
    let state = TempDir::new().unwrap();
    let server = NeoSdkServer::new(
        Config::new(1, state.path().to_path_buf()),
        CancellationToken::new(),
    );
    let router = server.router();
    let session = legacy_session(&router).await;
    assert_eq!(server.replies.len(), 0, "handshake ownership must be gone");
    let response = router
        .clone()
        .oneshot(legacy(Method::GET, Some(&session), None))
        .await
        .unwrap();
    assert_sse(&response);
    let mut get = response.into_body();
    let tools = json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}});
    let response = router
        .clone()
        .oneshot(legacy(Method::POST, Some(&session), Some(&tools)))
        .await
        .unwrap();
    assert_sse(&response);
    assert_eq!(server.replies.len(), 1, "the POST is still unpolled");
    let mut post = response.into_body();
    let closing = server.clone();
    let close = tokio::spawn(async move { closing.close_transport().await });
    server.shutdown.cancelled().await;
    server.runs.wait().await;
    reply_drain_entered(&server).await;
    assert!(
        !server.transport.is_cancelled(),
        "drain entered with POST held"
    );
    assert!(!close.is_finished(), "unread POST still owns the drain");

    // Probe without awaiting: priming and keep-alive frames are allowed, but EOF is not.
    assert_get_is_pending(&mut get);
    let mut bytes = Vec::new();
    loop {
        match future::poll_fn(|cx| Pin::new(&mut post).poll_frame(cx)).await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    bytes.extend_from_slice(&data);
                }
            }
            Some(Err(error)) => panic!("legacy POST body error: {error}"),
            None => {
                // No await before either assertion: this EOF must precede transport cancel,
                // rather than result from take_until cutting an open stream at the grace.
                assert!(!server.transport.is_cancelled(), "POST must end on its own");
                assert_tools_result(&bytes, 2);
                break;
            }
        }
    }
    time::timeout(Duration::from_secs(20), close)
        .await
        .unwrap()
        .unwrap();
    assert!(
        server.transport.is_cancelled(),
        "transport ends after ownership"
    );
    assert_eq!(server.replies.len(), 0, "POST EOF released its token");
    let _closed_get = time::timeout(Duration::from_secs(20), body::to_bytes(get, 1024 * 1024))
        .await
        .unwrap()
        .unwrap();
    server.shutdown().await;
}
