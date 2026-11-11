//! `GET /v1/display` as zuzu reaches it: parameter errors before anything opens, and whole
//! WebSocket sessions through `host_service` on a real Unix socket with the token, carrying RFB
//! between a test viewer and the fake Xvnc.

use crate::support::{BoxError, Harness, TestResult, harness};
use futures_util::{SinkExt, StreamExt};
use neo_service::{HostAuth, HostToken, PeerRule, host_service};
use neo_takeover::{OwnerHasScreen, ScreenHolder};
use std::future::IntoFuture;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::handshake::client::Response;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, client_async};

const TOKEN: &str = "display-test-token-0123456789abcdef";

/// How long a test waits for a frame.
const WITHIN: Duration = Duration::from_secs(5);

/// The version, security type 1, and a ClientInit that asks for exclusive access (0).
const HANDSHAKE: &[u8] = b"RFB 003.008\n\x01\x00";
/// What Xvnc sends before any message: its version, security type None, SecurityResult 0, and
/// ServerInit (1600×1000, 32 bits per pixel, the name "neo").
const SERVER_HANDSHAKE: usize = 12 + 2 + 4 + 24 + 3;
/// An update request answered at once (not incremental).
const UPDATE: [u8; 10] = [3, 0, 0, 0, 0, 0, 0, 1, 0, 1];
/// Key `a` down, as noVNC sends it: a QEMU extended key event.
pub(crate) const QEMU_A: [u8; 12] = [255, 0, 0, 1, 0, 0, 0, 0x61, 0, 0, 0, 0x1e];

/// The routes served as the app serves them: `host_service` on a Unix socket, with the peer rule
/// and the token.
pub(crate) struct Socket {
    pub(crate) path: PathBuf,
    server: JoinHandle<std::io::Result<()>>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub(crate) fn serve(h: &Harness) -> Result<Socket, BoxError> {
    let path = h.dir.path().join("display.sock");
    let listener = UnixListener::bind(&path)?;
    let auth = HostAuth {
        token: HostToken::parse(TOKEN)?,
        peer: Some(PeerRule::from_socket(&path)?),
    };
    let server =
        tokio::spawn(axum::serve(listener, host_service(h.routes.clone(), auth)).into_future());
    Ok(Socket { path, server })
}

/// How a test viewer opens a display.
#[derive(Clone, Copy)]
pub(crate) struct Open<'a> {
    query: &'a str,
    token: bool,
    binary: bool,
}

impl<'a> Open<'a> {
    pub(crate) fn new(query: &'a str) -> Self {
        Self {
            query,
            token: true,
            binary: false,
        }
    }
}

pub(crate) type Display = WebSocketStream<UnixStream>;

async fn open(socket: &Path, how: Open<'_>) -> Result<(Display, Response), WsError> {
    let stream = UnixStream::connect(socket).await?;
    let uri = format!("ws://neo/v1/display?{}", how.query)
        .parse()
        .map_err(|error: axum::http::uri::InvalidUri| WsError::HttpFormat(error.into()))?;
    let mut request = ClientRequestBuilder::new(uri);
    if how.token {
        request = request.with_header("Authorization", format!("Bearer {TOKEN}"));
    }
    if how.binary {
        request = request.with_sub_protocol("binary");
    }
    client_async(request, stream).await
}

/// The refusal a display got instead of an upgrade: its status and the contract's `code`.
pub(crate) async fn refused(socket: &Path, how: Open<'_>) -> Result<(u16, String), BoxError> {
    match open(socket, how).await {
        Err(WsError::Http(response)) => {
            let body: serde_json::Value =
                serde_json::from_slice(response.body().as_deref().unwrap_or_default())?;
            assert!(body["requestId"].is_string(), "{body}");
            let code = body["code"].as_str().unwrap_or_default().to_owned();
            Ok((response.status().as_u16(), code))
        }
        Err(error) => Err(error.into()),
        Ok(_) => Err("the display opened".into()),
    }
}

/// The next `count` RFB bytes from the display.
async fn read(display: &mut Display, count: usize) -> Result<Vec<u8>, BoxError> {
    let mut bytes = Vec::new();
    while bytes.len() < count {
        match timeout(WITHIN, display.next())
            .await?
            .ok_or("the display ended")??
        {
            Message::Binary(chunk) => bytes.extend_from_slice(&chunk),
            other => return Err(format!("not RFB: {other:?}").into()),
        }
    }
    Ok(bytes)
}

/// The close the display sent, as (code, reason), skipping RFB bytes still in flight.
async fn closed(display: &mut Display) -> Result<(u16, String), BoxError> {
    loop {
        match timeout(WITHIN, display.next())
            .await?
            .ok_or("no close frame")??
        {
            Message::Binary(_) => {}
            Message::Close(Some(frame)) => {
                return Ok((frame.code.into(), frame.reason.to_string()));
            }
            other => return Err(format!("not a close frame: {other:?}").into()),
        }
    }
}

/// Opens a display and does the RFB handshake through it: Xvnc's handshake arrives untouched.
pub(crate) async fn handshake(
    socket: &Path,
    how: Open<'_>,
) -> Result<(Display, Response), BoxError> {
    let (mut display, response) = open(socket, how).await?;
    assert_eq!(read(&mut display, 12).await?, b"RFB 003.008\n");
    display.send(Message::binary(HANDSHAKE)).await?;
    read(&mut display, SERVER_HANDSHAKE - 12).await?;
    Ok((display, response))
}

fn protocol(response: &Response) -> Option<&str> {
    response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn bad_display_parameters_are_refused_before_anything_opens() -> TestResult {
    let h = harness().await?;
    let dials = h.xvnc.connections();
    for query in [
        "",
        "?mode=watch",
        "?mode=view&mode=control",
        "?mode=view&followTab=12&followTask=checkout",
        "?mode=view&followTab=front",
        "?mode=view&followTask=",
        "?mode=view&release=tk_01900000-0000-7000-8000-000000000000",
        "?mode=control&followTab=12",
        "?mode=control&followTask=checkout",
    ] {
        h.get(&format!("/v1/display{query}"))
            .await?
            .assert_error(axum::http::StatusCode::BAD_REQUEST, "invalid_request");
    }
    // No takeover exists to bind a control display to, whatever the id.
    for release in ["tk_01900000-0000-7000-8000-000000000000", "nope"] {
        h.get(&format!("/v1/display?mode=control&release={release}"))
            .await?
            .assert_error(axum::http::StatusCode::NOT_FOUND, "not_found");
    }
    // A request that is not a WebSocket upgrade never becomes a display.
    h.get("/v1/display?mode=view")
        .await?
        .assert_error(axum::http::StatusCode::BAD_REQUEST, "invalid_request");
    assert_eq!(h.xvnc.connections(), dials, "nothing dialed Xvnc");
    Ok(())
}

#[tokio::test]
async fn the_display_takes_no_subprotocol_and_echoes_binary_when_offered() -> TestResult {
    let h = harness().await?;
    let socket = serve(&h)?;
    let (_, response) = open(&socket.path, Open::new("mode=view")).await?;
    assert_eq!(response.status(), 101);
    assert_eq!(protocol(&response), None, "noVNC 1.3.0 offers none");
    let how = Open {
        binary: true,
        ..Open::new("mode=view")
    };
    let (_, response) = open(&socket.path, how).await?;
    assert_eq!(protocol(&response), Some("binary"));
    Ok(())
}

#[tokio::test]
async fn a_display_needs_the_host_token() -> TestResult {
    let h = harness().await?;
    let socket = serve(&h)?;
    let how = Open {
        token: false,
        ..Open::new("mode=view")
    };
    assert_eq!(
        refused(&socket.path, how).await?,
        (401, "unauthorized".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn the_owners_first_key_closes_views_with_4001_until_their_display_closes() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let socket = serve(&h)?;
    let first = h.xvnc.connections();

    let (mut view, _) = handshake(&socket.path, Open::new("mode=view")).await?;
    view.send(Message::binary(UPDATE.to_vec())).await?;
    assert!(
        !read(&mut view, 4).await?.is_empty(),
        "an update from the screen"
    );

    let how = Open {
        binary: true,
        ..Open::new("mode=control")
    };
    let (mut control, _) = handshake(&socket.path, how).await?;
    control.send(Message::binary(QEMU_A.to_vec())).await?;
    let (code, reason) = closed(&mut view).await?;
    assert_eq!(code, 4001, "{reason}");
    assert!(reason.starts_with("paused"), "{reason}");
    assert_eq!(
        refused(&socket.path, Open::new("mode=view")).await?,
        (409, "display_paused".to_owned())
    );

    timeout(WITHIN, async {
        while !h.xvnc.received(first + 1).contains(&QEMU_A.to_vec()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;
    // The owner closes, and Neo answers the close (RFC 6455) once the display has fully ended.
    control.close(None).await?;
    let answer = timeout(WITHIN, control.next()).await?;
    assert!(
        matches!(answer, Some(Ok(Message::Close(_)))),
        "the owner's close was not answered: {answer:?}"
    );

    let (mut back, _) = handshake(&socket.path, Open::new("mode=view")).await?;
    back.send(Message::binary(UPDATE.to_vec())).await?;
    assert!(
        !read(&mut back, 4).await?.is_empty(),
        "views are let in again once the owner leaves"
    );
    Ok(())
}

/// The takeover's `display` is a control display bound with `release=`: it is the only display
/// while the takeover holds spectators off, its release closes it with 4002, and after that the
/// same path is refused before any upgrade.
#[tokio::test]
async fn a_takeovers_display_closes_with_4002_at_its_release() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let socket = serve(&h)?;
    let takeover = h.post("/v1/takeovers", r#"{"tabId": 12}"#).await?;
    assert_eq!(takeover.status, 200, "{}", takeover.body);
    let query = takeover.body["display"]
        .as_str()
        .and_then(|path| path.strip_prefix("/v1/display?"))
        .ok_or("the takeover names its display")?
        .to_owned();
    assert_eq!(
        refused(&socket.path, Open::new("mode=view")).await?,
        (409, "display_paused".to_owned()),
        "the takeover holds spectators off"
    );
    let (mut control, _) = handshake(&socket.path, Open::new(&query)).await?;

    let id = takeover.body["id"].as_str().ok_or("an id")?;
    let released = h.post(&format!("/v1/takeovers/{id}/release"), "").await?;
    assert_eq!(released.status, 200, "{}", released.body);
    let (code, reason) = closed(&mut control).await?;
    assert_eq!(code, 4002, "{reason}");
    assert!(reason.starts_with("released"), "{reason}");
    assert_eq!(
        refused(&socket.path, Open::new(&query)).await?,
        (409, "takeover_released".to_owned())
    );
    let (_, response) = open(&socket.path, Open::new("mode=view")).await?;
    assert_eq!(
        response.status(),
        101,
        "spectators are back after the release"
    );
    Ok(())
}

#[tokio::test]
async fn a_text_frame_or_a_message_the_filter_refuses_closes_the_display() -> TestResult {
    let h = harness().await?;
    let socket = serve(&h)?;
    let (mut display, _) = handshake(&socket.path, Open::new("mode=view")).await?;
    display.send(Message::text("RFB as text")).await?;
    assert_eq!(closed(&mut display).await?.0, 1003);

    let (mut display, _) = handshake(&socket.path, Open::new("mode=view")).await?;
    display.send(Message::binary(vec![9])).await?;
    let (code, reason) = closed(&mut display).await?;
    assert_eq!(code, 1008);
    assert!(reason.contains("does not know"), "{reason}");
    Ok(())
}

#[tokio::test]
async fn displays_can_open_while_the_browser_cannot_list_its_tabs() -> TestResult {
    let h = harness().await?;
    let socket = serve(&h)?;
    h.browser.become_unreachable();
    for mode in ["mode=view", "mode=control"] {
        let (mut display, _) = handshake(&socket.path, Open::new(mode)).await?;
        display.close(None).await?;
    }
    Ok(())
}

#[tokio::test]
async fn a_display_whose_screen_closes_gets_1011() -> TestResult {
    let h = harness().await?;
    let socket = serve(&h)?;
    let (mut display, _) = handshake(&socket.path, Open::new("mode=control")).await?;
    h.xvnc.drop_connections();
    assert_eq!(closed(&mut display).await?.0, 1011);
    Ok(())
}

#[tokio::test]
async fn an_open_control_display_holds_the_screen_until_it_closes() -> TestResult {
    let h = harness().await?;
    h.browser.open_tab(12);
    let socket = serve(&h)?;
    let front = h.service.front();
    assert_eq!(front.check(), Ok(()));

    let (mut control, _) = handshake(&socket.path, Open::new("mode=control")).await?;
    assert_eq!(
        front.check(),
        Err(OwnerHasScreen {
            by: ScreenHolder::ControlDisplay
        }),
        "open, with no input yet"
    );
    // Neo answers the owner's close once the display has fully ended.
    control.close(None).await?;
    let answer = timeout(WITHIN, control.next()).await?;
    assert!(
        matches!(answer, Some(Ok(Message::Close(_)))),
        "the owner's close was not answered: {answer:?}"
    );
    assert_eq!(front.check(), Ok(()), "the closed display gave it back");
    Ok(())
}
