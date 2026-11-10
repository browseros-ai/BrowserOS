//! Neo: real background-page capture, cancellation and modal-dialog behavior on an owned browser.
//! `NEO_E2E_BROWSER` is required to run. The default gate reports a skip and opens no window.

#![expect(
    clippy::unwrap_used,
    reason = "test fixtures fail immediately when setup or response decoding fails"
)]
#![expect(
    clippy::expect_used,
    reason = "test helper deadlines and missing fixture data are assertion failures"
)]

use std::{env, fs};

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy, SessionId};
use neo_http::NeoCapture;
use neo_screencast::CdpPortTransport;
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::time::{self, Instant};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Own the profile and process group before any assertion can fail.
struct Browser {
    child: Child,
    profile: TempDir,
}

impl Drop for Browser {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn launch(binary: &Path) -> (Browser, u16) {
    let profile = tempfile::tempdir().unwrap();
    let temporary = profile.path().join("tmp");
    fs::create_dir(&temporary).unwrap();
    let mut command = Command::new(binary);
    command
        .args([
            "--headless=new",
            "--no-first-run",
            "--use-mock-keychain",
            "--disable-browseros-server",
            "--disable-browseros-extensions",
            "--disable-background-networking",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .env("MAC_CHROMIUM_TMPDIR", &temporary)
        .env("TMPDIR", &temporary)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut browser = Browser {
        child: command.spawn().unwrap(),
        profile,
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(port) = fs::read_to_string(browser.profile.path().join("DevToolsActivePort"))
            .ok()
            .and_then(|text| text.lines().next()?.parse().ok())
        {
            return (browser, port);
        }
        assert!(
            browser.child.try_wait().unwrap().is_none(),
            "owned browser exited during startup"
        );
        assert!(
            Instant::now() < deadline,
            "owned browser did not publish its port"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
}

async fn send(cdp: &CdpClient, method: &str, params: Value, session: Option<&SessionId>) -> Value {
    time::timeout(Duration::from_secs(5), cdp.send(method, params, session))
        .await
        .unwrap()
        .unwrap()
}

async fn create(cdp: &CdpClient, background: bool) -> String {
    send(
        cdp,
        "Target.createTarget",
        json!({"url":"about:blank","background":background}),
        None,
    )
    .await["targetId"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn attach(cdp: &CdpClient, target: &str) -> SessionId {
    let reply = send(
        cdp,
        "Target.attachToTarget",
        json!({"targetId":target,"flatten":true}),
        None,
    )
    .await;
    SessionId::from(reply["sessionId"].as_str().unwrap())
}

async fn evaluate(cdp: &CdpClient, session: &SessionId, expression: &str) -> Value {
    send(
        cdp,
        "Runtime.evaluate",
        json!({"expression":expression,"returnByValue":true}),
        Some(session),
    )
    .await["result"]["value"]
        .clone()
}

async fn get(client: &Client, url: &str) -> Response {
    time::timeout(Duration::from_secs(7), client.get(url).send())
        .await
        .unwrap()
        .unwrap()
}

async fn parts(response: &mut Response, count: usize, duration: Duration) -> Vec<u8> {
    time::timeout(duration, async {
        let mut bytes = Vec::new();
        while bytes.windows(7).filter(|b| *b == b"--frame").count() < count {
            bytes.extend_from_slice(&response.chunk().await.unwrap().expect("live MJPEG body"));
        }
        bytes
    })
    .await
    .expect("MJPEG frame deadline")
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one browser lifecycle verifies capture and its cleanup together"
)]
#[expect(
    clippy::print_stderr,
    reason = "browser evidence must identify a real run or an explicit skip"
)]
async fn captures_background_pages_and_bounds_modal_failures() {
    let Some(binary) = env::var_os("NEO_E2E_BROWSER") else {
        eprintln!("SKIP browser_e2e: set NEO_E2E_BROWSER to the BrowserOS neo binary");
        return;
    };
    let (_browser, port) = launch(Path::new(&binary)).await;
    let client = Client::new();
    let version: Value = client
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    eprintln!(
        "Browser path={} version={} headless=true owned_port={port}",
        Path::new(&binary).display(),
        version["Browser"]
    );
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(port)
    })
    .await
    .unwrap();
    let background = create(&cdp, true).await;
    let observer = attach(&cdp, &background).await;
    evaluate(&cdp, &observer, "document.body.innerHTML='<canvas width=640 height=400></canvas>'; let n=0; setInterval(()=>{const c=document.querySelector('canvas').getContext('2d');c.fillStyle=`hsl(${n++%360},100%,50%)`;c.fillRect(0,0,640,400)},30); 'ready'").await;
    let foreground = create(&cdp, false).await;
    send(
        &cdp,
        "Target.activateTarget",
        json!({"targetId":foreground}),
        None,
    )
    .await;
    assert_eq!(
        evaluate(&cdp, &observer, "document.visibilityState").await,
        "hidden",
        "fixture really is a background page before presence"
    );
    let info = send(
        &cdp,
        "Target.getTargetInfo",
        json!({"targetId":background}),
        None,
    )
    .await;
    let tab_id = info["targetInfo"]["tabId"]
        .as_i64()
        .expect("BrowserOS tab identity");

    let shutdown = CancellationToken::new();
    // Use the production transports explicitly so the test can break only the capture socket.
    // The injected transports remain fixture-owned and are closed after the service drains.
    let capture_port = Arc::new(CdpPortTransport::new(port));
    let page_port = Arc::new(CdpPortTransport::new(port));
    let capture = NeoCapture::start_with(
        Arc::<CdpPortTransport>::clone(&capture_port),
        Arc::<CdpPortTransport>::clone(&page_port),
        shutdown.clone(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let stop = shutdown.clone();
    let app = capture.router();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .unwrap();
    });
    let page = format!("{host}/v1/pages/{background}");
    let started = Instant::now();
    let shot = get(&client, &format!("{page}/screenshot")).await;
    assert_eq!(shot.status(), StatusCode::OK, "real background screenshot");
    let jpeg = shot.bytes().await.unwrap();
    assert!(
        jpeg.starts_with(&[0xff, 0xd8]) && jpeg.ends_with(&[0xff, 0xd9]),
        "real JPEG markers"
    );
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "background screenshot within one second"
    );

    let mut frames = get(&client, &format!("{page}/screencast?quality=low")).await;
    assert_eq!(frames.status(), StatusCode::OK, "real screencast opens");
    let bytes = parts(&mut frames, 3, Duration::from_secs(3)).await;
    assert!(
        String::from_utf8_lossy(&bytes).contains(&format!("X-Tab-Id: {tab_id}\r\n")),
        "real multipart BrowserOS tab identity"
    );
    capture_port.close().await;
    let generation_eof = time::timeout(Duration::from_secs(2), async {
        while frames.chunk().await.unwrap().is_some() {}
    })
    .await;
    assert!(
        generation_eof.is_ok(),
        "capture-only socket loss ends the existing MJPEG body"
    );
    assert_eq!(
        evaluate(&cdp, &observer, "document.visibilityState").await,
        "visible",
        "separate page presence survives capture socket loss"
    );
    frames = get(&client, &format!("{page}/screencast?quality=low")).await;
    assert_eq!(
        frames.status(),
        StatusCode::OK,
        "new response opens after capture generation loss"
    );
    parts(&mut frames, 3, Duration::from_secs(3)).await;

    let slow = get(&client, &format!("{page}/screencast?quality=low")).await;
    time::sleep(Duration::from_secs(5)).await;
    assert!(
        !parts(&mut frames, 3, Duration::from_secs(3))
            .await
            .is_empty(),
        "stalled viewer does not block another"
    );
    drop(slow);
    drop(frames);

    let mut video = get(&client, &format!("{page}/video?quality=low")).await;
    assert_eq!(
        video.status(),
        StatusCode::OK,
        "BrowserOS AV1 recording opens"
    );
    let mp4 = time::timeout(Duration::from_secs(6), async {
        let mut bytes = Vec::new();
        loop {
            bytes.extend_from_slice(&video.chunk().await.unwrap().expect("live video body"));
            if bytes.windows(4).any(|b| b == b"ftyp") && bytes.windows(4).any(|b| b == b"moof") {
                return bytes;
            }
        }
    })
    .await
    .expect("AV1 initialization and fragment deadline");
    assert!(
        mp4.windows(4).position(|b| b == b"ftyp") < mp4.windows(4).position(|b| b == b"moof"),
        "init precedes video fragment"
    );
    drop(video);
    time::sleep(Duration::from_secs(6)).await;
    assert_eq!(
        evaluate(&cdp, &observer, "document.visibilityState").await,
        "visible",
        "presence survives viewer grace period"
    );

    send(&cdp, "Page.enable", json!({}), Some(&observer)).await;
    let mut events = cdp.events();
    evaluate(
        &cdp,
        &observer,
        "setTimeout(()=>alert('capture deadline'),0); 'armed'",
    )
    .await;
    time::timeout(Duration::from_secs(3), async {
        loop {
            if events.recv().await.unwrap().method == "Page.javascriptDialogOpening" {
                break;
            }
        }
    })
    .await
    .expect("test dialog really opened");
    let start = Instant::now();
    let response = get(&client, &format!("{page}/screenshot")).await;
    assert_eq!(
        response.status(),
        StatusCode::GATEWAY_TIMEOUT,
        "modal still timeout"
    );
    assert_eq!(
        response.json::<Value>().await.unwrap()["code"],
        "capture_timeout",
        "modal error code"
    );
    assert!(
        start.elapsed() < Duration::from_millis(3500),
        "modal still bounded by 3 seconds"
    );

    // A fresh presence setup on the modal page shares no map lock with the other page.
    let blocked = create(&cdp, true).await;
    let blocked_session = attach(&cdp, &blocked).await;
    send(&cdp, "Page.enable", json!({}), Some(&blocked_session)).await;
    let mut events = cdp.events();
    evaluate(
        &cdp,
        &blocked_session,
        "setTimeout(()=>alert('stream deadline'),0); 'armed'",
    )
    .await;
    time::timeout(Duration::from_secs(3), async {
        loop {
            if events.recv().await.unwrap().method == "Page.javascriptDialogOpening" {
                break;
            }
        }
    })
    .await
    .unwrap();
    let blocked_url = format!("{host}/v1/pages/{blocked}/screencast");
    let blocked_client = client.clone();
    let start = Instant::now();
    let blocked_request = tokio::spawn(async move { get(&blocked_client, &blocked_url).await });
    send(
        &cdp,
        "Page.handleJavaScriptDialog",
        json!({"accept":true}),
        Some(&observer),
    )
    .await;
    assert_eq!(
        get(&client, &format!("{page}/screenshot")).await.status(),
        StatusCode::OK,
        "still recovers after dismiss"
    );
    let mut healthy = get(&client, &format!("{page}/screencast?quality=low")).await;
    parts(&mut healthy, 3, Duration::from_secs(3)).await;
    let response = blocked_request.await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::GATEWAY_TIMEOUT,
        "modal stream setup timeout"
    );
    assert_eq!(
        response.json::<Value>().await.unwrap()["code"],
        "capture_timeout",
        "stream timeout code"
    );
    assert!(
        start.elapsed() < Duration::from_millis(5500),
        "single five-second stream deadline"
    );
    send(
        &cdp,
        "Page.handleJavaScriptDialog",
        json!({"accept":true}),
        Some(&blocked_session),
    )
    .await;

    for (header, value) in [("origin", "https://evil.test"), ("sec-fetch-site", "none")] {
        let response = client
            .get(format!("{page}/screenshot"))
            .header(header, value)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "real request hygiene"
        );
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin"),
            "no CORS"
        );
    }
    shutdown.cancel();
    let ended = time::timeout(Duration::from_secs(1), async {
        while healthy.chunk().await.unwrap().is_some() {}
    })
    .await;
    assert!(
        ended.is_ok(),
        "shutdown ends a real open stream within one second"
    );
    time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    capture.close().await;
    capture_port.close().await;
    page_port.close().await;
    send(
        &cdp,
        "Target.closeTarget",
        json!({"targetId":background}),
        None,
    )
    .await;
    // A separate service proves closed-target mapping without asking the closed service to reopen.
    let check = NeoCapture::start(port, CancellationToken::new());
    let response = ServiceExt::oneshot(
        check.router(),
        Request::builder()
            .uri(format!("/v1/pages/{background}/screenshot"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "closed page is missing"
    );
    check.close().await;
    cdp.disconnect().await;
    eprintln!(
        "PASS real screenshot, MJPEG, AV1, capture-generation EOF/recovery, background presence, modal deadlines, concurrent page, header rejection and stream shutdown"
    );
}
