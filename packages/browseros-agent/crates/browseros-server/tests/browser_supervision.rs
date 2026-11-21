//! The engine child's supervision without a browser: an engine that cannot start keeps chat
//! working, a call fails with the reason once the next restart is far off, and the server's
//! stop ends the supervisor. The engine here is `false`, which exits 1 at every start.

#![cfg(unix)]

use std::time::{Duration, Instant};

use browseros_server::browser::{Access, Browser, BrowserConfig, Scope};
use serde_json::json;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn an_engine_that_cannot_start_fails_calls_with_its_reason_and_stops_with_the_server()
-> TestResult {
    let state = tempfile::tempdir()?;
    let stop = CancellationToken::new();
    let browser = Browser::start_with_engine_exe(
        BrowserConfig::Neo {
            cdp_port: 9,
            state_dir: state.path().to_owned(),
            execution_dir: state.path().join(".browseros"),
            browseros_version: None,
        },
        9200,
        stop.clone(),
        "false".into(),
    )
    .await;
    let scope = Scope {
        task: "c1".into(),
        access: Access::Full,
    };
    assert!(!browser.connected());
    let tools = browser.tools();
    assert_eq!(tools.len(), 1, "the built-in run before the engine answers");
    assert_eq!(tools[0].name, "run");
    let armed = browser.arm(scope.clone()).ok_or("an engine arms")?;
    assert_eq!(armed.url(), "http://127.0.0.1:9200/mcp/c1");

    // Starts at 0 s, 1 s and 6 s fail; after the third the restart is 30 s away, past what a
    // call waits, so the call returns then with the engine's reason.
    let started = Instant::now();
    let output = browser
        .call(
            &scope,
            "run",
            json!({ "code": "return 1" }),
            &CancellationToken::new(),
        )
        .await;
    assert!(output.is_error);
    assert!(
        output
            .text
            .starts_with("The browser is not available: the browser engine exited"),
        "{}",
        output.text
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );

    stop.cancel();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let output = browser
            .call(
                &scope,
                "run",
                json!({ "code": "return 1" }),
                &CancellationToken::new(),
            )
            .await;
        if output.text.contains("shutting down") {
            break;
        }
        assert!(Instant::now() < deadline, "{}", output.text);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

#[tokio::test]
async fn a_call_waiting_for_the_engine_ends_with_its_stop_token() -> TestResult {
    let state = tempfile::tempdir()?;
    let browser = Browser::start_with_engine_exe(
        BrowserConfig::Neo {
            cdp_port: 9,
            state_dir: state.path().to_owned(),
            execution_dir: state.path().join(".browseros"),
            browseros_version: None,
        },
        9200,
        CancellationToken::new(),
        "false".into(),
    )
    .await;
    let scope = Scope {
        task: "c1".into(),
        access: Access::Read,
    };
    let cancel = CancellationToken::new();
    let stopper = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        stopper.cancel();
    });
    let started = Instant::now();
    let output = browser
        .call(&scope, "run", json!({ "code": "return 1" }), &cancel)
        .await;
    assert_eq!(output.text, "Stopped");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    Ok(())
}
