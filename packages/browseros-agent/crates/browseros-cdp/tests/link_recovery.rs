//! The link supervisor has to converge on a live socket however the loss and
//! recovery events interleave.
//!
//! The case exercised here is a socket that dies the instant it is opened: the
//! handshake succeeds, so the open reports success, and the reader then fails
//! immediately. These cover the invariant the supervisor guarantees rather than
//! the interleaving that used to break it; that one needs the loss to land inside
//! a few instructions of the recovery and cannot be forced from outside the
//! client. The invariant is the part worth pinning down anyway: convergence must
//! not depend on which event happens to arrive first.

use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use futures_util::StreamExt;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{Instant, sleep},
};

/// Serves `/json/version`, pointing the debugger URL at `ws_port`.
async fn spawn_discovery(ws_port: u16) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|err| panic!("bind discovery: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("discovery addr: {err}"))
        .port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut scratch = [0_u8; 1024];
                let _ = stream.read(&mut scratch).await;
                let body = format!(
                    "{{\"webSocketDebuggerUrl\":\"ws://127.0.0.1:{ws_port}/devtools/browser/test\"}}"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    port
}

/// Accepts websocket upgrades and counts them. The first `collapses` are hung up
/// on the moment the handshake completes; any later one is held open.
async fn spawn_websocket(collapses: usize) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|err| panic!("bind ws: {err}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|err| panic!("ws addr: {err}"))
        .port();
    let upgrades = Arc::new(AtomicUsize::new(0));
    let counter = upgrades.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let seen = counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                if seen < collapses {
                    // Dropping the stream resets the connection, which is what a
                    // browser mid-sleep or mid-restart does to its debugger socket.
                    return;
                }
                while socket.next().await.is_some() {}
            });
        }
    });
    (port, upgrades)
}

fn options(discovery_port: u16) -> ConnectOptions {
    ConnectOptions {
        port: discovery_port,
        connect_timeout: Duration::from_secs(2),
        connect_max_retries: 1,
        reconnect_delay: Duration::from_millis(20),
        link_check_interval: Duration::from_millis(20),
        // Without this the default policy exits the process after three failures,
        // which would take the test runner with it.
        reconnect_policy: ReconnectPolicy::KeepTrying,
        reconnect_max_retries: usize::MAX,
        ..ConnectOptions::new(discovery_port)
    }
}

async fn wait_until(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if done() {
            return true;
        }
        sleep(Duration::from_millis(10)).await;
    }
    done()
}

#[tokio::test]
async fn reopens_a_socket_that_dies_the_instant_it_is_opened() {
    let (ws_port, upgrades) = spawn_websocket(3).await;
    let discovery_port = spawn_discovery(ws_port).await;

    let client = CdpClient::connect(options(discovery_port))
        .await
        .unwrap_or_else(|err| {
            panic!("the first handshake should succeed even though the socket then dies: {err}")
        });

    // Both halves matter. Connected alone is true for the instant before the
    // reader notices the first socket died, so the count is what proves each
    // collapse was actually retried rather than silently dropped.
    let converged = wait_until(Duration::from_secs(10), || {
        upgrades.load(Ordering::SeqCst) > 3 && client.is_connected()
    })
    .await;

    assert!(
        converged,
        "the link should converge on a live socket; upgrades seen: {}, connected: {}",
        upgrades.load(Ordering::SeqCst),
        client.is_connected()
    );
    assert!(
        client.down_for().is_none(),
        "a restored link reports no outage"
    );
    client.disconnect().await;
}

#[tokio::test]
async fn keeps_retrying_and_reports_the_outage_while_the_link_stays_down() {
    let (ws_port, upgrades) = spawn_websocket(usize::MAX).await;
    let discovery_port = spawn_discovery(ws_port).await;

    let client = CdpClient::connect(options(discovery_port))
        .await
        .unwrap_or_else(|err| panic!("the handshake should succeed: {err}"));

    let reported = wait_until(Duration::from_secs(5), || client.down_for().is_some()).await;
    assert!(reported, "a link that is down should report how long for");
    assert!(!client.is_connected());

    let before = upgrades.load(Ordering::SeqCst);
    sleep(Duration::from_millis(300)).await;
    assert!(
        upgrades.load(Ordering::SeqCst) > before,
        "the supervisor should still be retrying"
    );
    client.disconnect().await;
}

/// A stop that lands while the supervisor is reopening has to win.
///
/// Reopening used to clear the stop flag as part of installing the socket, so a
/// disconnect arriving in that window was overwritten and the client quietly came
/// back up. The ticket is explicit that recovery must not undo an intentional stop.
#[tokio::test]
async fn a_stop_during_a_reopen_stays_stopped() {
    // The first socket collapses, so a reopen is in flight around the disconnect.
    let (ws_port, _upgrades) = spawn_websocket(1).await;
    let discovery_port = spawn_discovery(ws_port).await;

    let client = CdpClient::connect(options(discovery_port))
        .await
        .unwrap_or_else(|err| panic!("the handshake should succeed: {err}"));
    client.disconnect().await;

    // Long enough for several reopen passes at a 20ms delay.
    for _ in 0..30 {
        sleep(Duration::from_millis(20)).await;
        assert!(
            !client.is_connected(),
            "a stopped client must stay stopped, even if a reopen was in flight"
        );
    }
}

#[tokio::test]
async fn stops_retrying_once_disconnected_on_purpose() {
    let (ws_port, upgrades) = spawn_websocket(usize::MAX).await;
    let discovery_port = spawn_discovery(ws_port).await;

    let client = CdpClient::connect(options(discovery_port))
        .await
        .unwrap_or_else(|err| panic!("the handshake should succeed: {err}"));
    client.disconnect().await;

    let after_disconnect = upgrades.load(Ordering::SeqCst);
    sleep(Duration::from_millis(300)).await;

    assert_eq!(
        upgrades.load(Ordering::SeqCst),
        after_disconnect,
        "an intentional disconnect must not be undone by automatic recovery"
    );
    assert!(!client.is_connected());
}
