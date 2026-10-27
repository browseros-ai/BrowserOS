//! The cockpit's live-view socket, `/api/v1/screencasts`, served on a real port: a scripted
//! browser stands in for Chromium and a static table for the sessions' tabs, so these check
//! what claw-app relies on (who may connect, the watch message, the frame layout).

use claw_server::{
    AppState, build_router,
    config::{Config, ServerMode},
    services::capture::{
        CaptureHub, CaptureTab, SessionView,
        testing::{ScriptedCdp, ScriptedTransport, StaticSessionTabs},
    },
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};
use tokio_util::sync::CancellationToken;

const EXTENSION_ORIGIN: &str = "chrome-extension://pjimfkbpehlcllblajnpfamdfjhhlgkc";

struct TestServer {
    url: String,
    browser: Arc<ScriptedCdp>,
    tabs: Arc<StaticSessionTabs>,
    _dir: TempDir,
}

async fn server() -> anyhow::Result<TestServer> {
    let dir = tempfile::tempdir()?;
    let config = Arc::new(Config {
        server_port: 9200,
        cdp_port: 49337,
        proxy_port: None,
        resources_dir: dir.path().join("resources"),
        browserclaw_dir: dir.path().join("browserclaw"),
        session_idle: Duration::from_secs(300),
        session_retention: Duration::from_secs(7_200),
        session_sweep_interval: Duration::from_secs(60),
        replay_retention_days: 7,
        dev_mode: false,
        native: Default::default(),
        mode: ServerMode::Desktop,
        behavior: claw_server::config::Behavior::defaults(ServerMode::Desktop),
        behavior_ignored: Vec::new(),
    });
    let mut state = AppState::new_with_home(config, dir.path().join("home")).await?;
    let browser = ScriptedCdp::new();
    let tabs = Arc::new(StaticSessionTabs::default());
    state.capture = CaptureHub::new(
        tabs.clone(),
        Arc::new(ScriptedTransport(browser.clone())),
        CancellationToken::new(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let router = build_router(state);
    tokio::spawn(async move { axum::serve(listener, router.into_make_service()).await });
    Ok(TestServer {
        url: format!("ws://{address}/api/v1/screencasts"),
        browser,
        tabs,
        _dir: dir,
    })
}

fn live(tab_ids: &[i64]) -> SessionView {
    SessionView {
        live: true,
        tabs: tab_ids
            .iter()
            .map(|&tab_id| CaptureTab {
                tab_id,
                target_id: format!("target-{tab_id}"),
                url: format!("https://news.ycombinator.com/item?id={tab_id}"),
            })
            .collect(),
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect(
    server: &TestServer,
    origin: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = server.url.as_str().into_client_request()?;
    if let Ok(origin) = HeaderValue::from_str(origin) {
        request.headers_mut().insert("origin", origin);
    }
    connect_async(request).await.map(|(socket, _)| socket)
}

/// The next binary frame split into its JSON header and JPEG, or fails after 5 s.
async fn next_frame(socket: &mut Socket) -> anyhow::Result<(Value, Vec<u8>)> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("socket closed"))??;
        let Message::Binary(data) = message else {
            continue;
        };
        let length = u32::from_be_bytes(data[..4].try_into()?) as usize;
        let header = serde_json::from_slice(&data[4..4 + length])?;
        return Ok((header, data[4 + length..].to_vec()));
    }
}

#[tokio::test]
async fn a_page_from_another_origin_cannot_open_the_socket() -> anyhow::Result<()> {
    let server = server().await?;

    let refused = connect(&server, "https://evil.example").await;

    match refused {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), 403);
        }
        other => panic!("expected a 403, got {:?}", other.map(|_| "an open socket")),
    }
    Ok(())
}

#[tokio::test]
async fn the_cockpit_receives_low_quality_frames_for_the_sessions_it_watches() -> anyhow::Result<()>
{
    let server = server().await?;
    server.tabs.set("s1", live(&[5]));
    server.tabs.set("s2", live(&[6, 7]));
    let mut socket = connect(&server, EXTENSION_ORIGIN).await?;

    socket
        .send(Message::text(
            json!({ "watch": [{ "sessionId": "s1" }] }).to_string(),
        ))
        .await?;
    let start = server.browser.wait_for("Page.startScreencast", 1).await;
    assert_eq!(start[0].params["maxWidth"], 640);
    server
        .browser
        .emit_frame("capture-1", b"jpeg-s1", 1_800_000_000.25);

    let (header, jpeg) = next_frame(&mut socket).await?;
    assert_eq!(
        header,
        json!({
            "sessionId": "s1",
            "tabId": 5,
            "url": "https://news.ycombinator.com/item?id=5",
            "capturedAt": 1_800_000_000_250_i64,
        })
    );
    assert_eq!(jpeg, b"jpeg-s1");

    // A pinned watch is echoed with its tab.
    socket
        .send(Message::text(
            json!({ "watch": [{ "sessionId": "s1" }, { "sessionId": "s2", "browserTabId": 7 }] })
                .to_string(),
        ))
        .await?;
    server.browser.wait_for("Page.startScreencast", 2).await;
    server
        .browser
        .emit_frame("capture-2", b"jpeg-s2", 1_800_000_001.0);
    let (header, jpeg) = next_frame(&mut socket).await?;
    assert_eq!(header["sessionId"], "s2");
    assert_eq!(header["browserTabId"], 7);
    assert_eq!(header["tabId"], 7);
    assert_eq!(jpeg, b"jpeg-s2");
    Ok(())
}
