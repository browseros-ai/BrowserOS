//! The browser module against a real browser: the engine child, `run` calls, pictures, large
//! results, stop, a killed engine, read-only scope, the tab census, and `/mcp/{task}` from a
//! real MCP client. Ignored by default; run it under the private-browser launcher, from
//! `packages/browseros-agent`:
//!
//! ```text
//! node tools/test-browser.mjs -- cargo test -p browseros-server --test browser_live -- --ignored --nocapture
//! ```
//!
//! The launcher starts a fresh headless browser (Neo on macOS unless `NEO_E2E_BROWSER` names
//! another, such as the desktop BrowserOS) and sets `BROWSEROS_CDP_PORT`. When
//! `BROWSEROS_MCP_CLIENT` names a Node script, the endpoint is also driven by it: the script
//! takes an MCP URL and a script body, connects with a real MCP client, lists the tools, calls
//! `run`, and prints one JSON line. Each step prints one `[browser_live]` line as evidence.

#![cfg(unix)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use browseros_server::browser::{Access, Browser, BrowserConfig, Scope, ToolOutput};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const FIXTURE: &str = r#"<!doctype html><title>Probe fixture</title>
<h1>Probe fixture</h1><p>Fixture on loopback</p>
<button onclick="document.getElementById('n').textContent = ++window.clicks">Count click</button>
<output id="n">0</output><script>window.clicks = 0</script>"#;

/// 720×540 of random pixels: a PNG of about 1.2 MB, over the engine's 256 KiB inline limit.
const NOISE: &str = r#"<!doctype html><title>Noise</title><body style="margin:0">
<canvas id="c" width="720" height="540"></canvas><script>
const g = document.getElementById('c').getContext('2d'); const d = g.createImageData(720, 540);
for (let i = 0; i < d.data.length; i++) d.data[i] = (i % 4 === 3) ? 255 : (Math.random() * 256) | 0;
g.putImageData(d, 0, 0);</script>"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a private browser: run under tools/test-browser.mjs"]
async fn the_engine_drives_a_real_browser() -> TestResult {
    let cdp_port: u16 = std::env::var("BROWSEROS_CDP_PORT")?.parse()?;
    let fixture = serve_fixture().await?;
    let state = tempfile::tempdir()?;
    // The endpoint's listener is bound first, so `Armed::url` names its real port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let server_port = listener.local_addr()?.port();
    let stop = CancellationToken::new();
    let started = Instant::now();
    let browser = Browser::start_with_engine_exe(
        BrowserConfig::Neo {
            cdp_port,
            state_dir: state.path().to_owned(),
            execution_dir: state.path().join("profile").join(".browseros"),
            browseros_version: None,
        },
        server_port,
        stop.clone(),
        PathBuf::from(env!("CARGO_BIN_EXE_browseros_server")),
    )
    .await;

    // Readiness: not linked until /system/ready and a real `return 1`.
    assert!(
        !browser.connected(),
        "connected before the engine was ready"
    );
    let builtin = browser.tools();
    wait_until(Duration::from_secs(30), || browser.connected()).await?;
    note(format!(
        "ready: connected() true after {} ms",
        started.elapsed().as_millis()
    ));
    let live = browser.tools();
    assert_eq!(live.len(), 1);
    assert_eq!(
        builtin,
        live,
        "src/browser/neo/run_tool.json is stale; the engine's definition is:\n{}",
        json!({ "name": live[0].name, "description": live[0].description, "inputSchema": live[0].input_schema })
    );
    note("tools(): the built-in definition equals the engine's tools/list");

    let task = Scope {
        task: "6a1c9f0e-2d4b-4e8a-9f3c-1b2d3e4f5a01".into(),
        access: Access::Full,
    };
    let never = CancellationToken::new();

    let two = run(&browser, &task, "return 1 + 1").await;
    assert!(!two.is_error, "{}", two.text);
    assert_eq!(inner(&two), "2");
    note(format!(
        "return 1 + 1: {:?}, fenced: {}",
        inner(&two),
        two.text.starts_with("[UNTRUSTED_PAGE_CONTENT nonce=")
    ));

    let aria = run(
        &browser,
        &task,
        &format!(
            "await page.goto({:?}); return await page.ariaSnapshot();",
            fixture.page
        ),
    )
    .await;
    assert!(!aria.is_error, "{}", aria.text);
    assert!(
        inner(&aria).contains(r#"heading \"Probe fixture\" [level=1]"#),
        "{}",
        aria.text
    );
    assert!(aria.text.starts_with("[UNTRUSTED_PAGE_CONTENT nonce="));
    note(format!("goto + ariaSnapshot: {}", first_line(inner(&aria))));

    let small = run(
        &browser,
        &task,
        "return await page.screenshot({ clip: { x: 0, y: 0, width: 40, height: 40 } });",
    )
    .await;
    assert!(!small.is_error, "{}", small.text);
    assert_eq!(inner(&small), r#""[picture 1]""#);
    assert_eq!(small.pictures.len(), 1);
    assert_eq!(small.pictures[0].media_type, "image/png");
    note(format!(
        "small screenshot: inline, text {}, {} PNG bytes",
        inner(&small),
        small.pictures[0].bytes.len()
    ));

    let full = run(
        &browser,
        &task,
        &format!(
            "await page.goto({:?}); return await page.screenshot({{ fullPage: true }});",
            fixture.noise
        ),
    )
    .await;
    assert!(!full.is_error, "{}", full.text);
    assert_eq!(inner(&full), r#""[picture 1]""#);
    assert_eq!(full.pictures.len(), 1);
    assert!(
        full.pictures[0].bytes.len() > 256 * 1024,
        "{} bytes",
        full.pictures[0].bytes.len()
    );
    note(format!(
        "full-page screenshot: a blob fetched from the host API, {} PNG bytes",
        full.pictures[0].bytes.len()
    ));

    let big = run(&browser, &task, r#"return "x".repeat(400000);"#).await;
    assert!(big.is_error);
    assert!(inner(&big).contains("result_too_large"), "{}", big.text);
    note(format!(
        "400000 bytes of text: tool error {:?}",
        first_line(inner(&big))
    ));

    let cancel = CancellationToken::new();
    let waiting = tokio::spawn({
        let browser = browser.clone();
        let task = task.clone();
        let cancel = cancel.clone();
        async move {
            let script = "await page.locator('#missing').waitFor({ timeout: 60000 }); return 1;";
            browser
                .call(&task, "run", json!({ "code": script }), &cancel)
                .await
        }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        browser.engine_running_runs().await,
        Some(1),
        "the script is running"
    );
    let cancelled_at = Instant::now();
    cancel.cancel();
    let stopped = tokio::time::timeout(Duration::from_secs(1), waiting).await??;
    let returned_ms = cancelled_at.elapsed().as_millis();
    assert_eq!(stopped.text, "Stopped");
    // The engine's own record, not only this side: the run is no longer in flight.
    let deadline = cancelled_at + Duration::from_secs(1);
    while browser.engine_running_runs().await != Some(0) {
        assert!(
            Instant::now() < deadline,
            "the engine still runs the cancelled script"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ended_ms = cancelled_at.elapsed().as_millis();
    let after = run(&browser, &task, "return 1").await;
    assert_eq!(inner(&after), "1", "{}", after.text);
    note(format!(
        "cancel a 60 s script: call returned {returned_ms} ms after cancel, the engine had no run in flight {ended_ms} ms after cancel, the next call returned {}",
        inner(&after)
    ));

    let read_only = Scope {
        access: Access::Read,
        ..task.clone()
    };
    let refused = run(
        &browser,
        &read_only,
        &format!(
            "await page.goto({:?}); await page.getByRole('button', {{ name: 'Count click' }}).click(); return 1;",
            fixture.page
        ),
    )
    .await;
    assert!(refused.is_error);
    assert!(
        inner(&refused).contains("read_only_run"),
        "{}",
        refused.text
    );
    note(format!("read-only click: {}", first_line(inner(&refused))));

    // The tab census. A fresh conversation whose first script uses the built-in `page`: the run
    // carries only the implicit host task, and the tab it opens still counts.
    let tabs_task = Scope {
        task: "6a1c9f0e-2d4b-4e8a-9f3c-1b2d3e4f5a02".into(),
        access: Access::Full,
    };
    let implicit = run(
        &browser,
        &tabs_task,
        &format!("await page.goto({:?}); return page.tabId;", fixture.page),
    )
    .await;
    let implicit_tab: i64 = inner(&implicit).parse()?;
    assert_eq!(
        browser.take_opened_tabs(&tabs_task.task),
        vec![implicit_tab]
    );
    assert_eq!(browser.take_opened_tabs(&tabs_task.task), Vec::<i64>::new());
    let opened = run(
        &browser,
        &tabs_task,
        "const p = await context.newPage(); return p.tabId;",
    )
    .await;
    let opened_tab: i64 = inner(&opened).parse()?;
    let again = run(&browser, &tabs_task, "return context.pages().length").await;
    assert!(!again.is_error, "{}", again.text);
    assert_eq!(browser.take_opened_tabs(&tabs_task.task), vec![opened_tab]);
    assert_eq!(browser.take_opened_tabs(&tabs_task.task), Vec::<i64>::new());
    note(format!(
        "opened tabs: built-in page {implicit_tab} taken once; context.newPage() {opened_tab} taken once, not again after a later call"
    ));

    // Kill the engine child by its pid and watch the supervisor bring it back.
    let pid = browser.engine_pid().ok_or("no engine pid")?;
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()?;
    assert!(killed.success());
    let killed_at = Instant::now();
    // A call sent in the moment the child dies may have reached it, and is never repeated; so
    // the next call goes out once the supervisor has the new child up.
    wait_until(Duration::from_secs(20), || {
        browser.engine_pid().is_some_and(|new| new != pid)
    })
    .await?;
    let restarted_ms = killed_at.elapsed().as_millis();
    let new_pid = browser
        .engine_pid()
        .ok_or("no engine pid after the restart")?;
    let revived = run(&browser, &task, "return 1 + 1").await;
    assert_eq!(inner(&revived), "2", "{}", revived.text);
    note(format!(
        "killed engine pid {pid}: restarted as pid {new_pid} after {restarted_ms} ms, next call returned {}",
        inner(&revived)
    ));

    endpoint(&browser, listener, &never).await?;

    // Shutdown: the lifeline closes and the child is gone within the 2 s grace.
    let pid = browser.engine_pid().ok_or("no engine pid")?;
    stop.cancel();
    wait_until(Duration::from_secs(3), || !alive(pid)).await?;
    note(format!(
        "shutdown: engine pid {pid} exited after its stdin closed"
    ));
    drop(state);
    Ok(())
}

/// `/mcp/{task}` over the real engine: refusals, the handshake from raw JSON-RPC and from a
/// real MCP client, and dropping `Armed` stopping a call in flight.
async fn endpoint(
    browser: &Browser,
    listener: tokio::net::TcpListener,
    never: &CancellationToken,
) -> TestResult {
    let router = axum::Router::new().nest_service("/mcp", browser.mcp_router());
    let serving = never.clone();
    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { serving.cancelled().await })
            .await
    });
    let task = Scope {
        task: "6a1c9f0e-2d4b-4e8a-9f3c-1b2d3e4f5a03".into(),
        access: Access::Full,
    };
    let client = reqwest::Client::builder().no_proxy().build()?;
    let unarmed = mcp_post(&client, &url_of(browser, &task)?, &initialize(), None).await?;
    assert_eq!(unarmed.0, 404);

    let armed = browser.arm(task.clone()).ok_or("the engine arms")?;
    let url = armed.url().to_owned();
    let from_page = mcp_post(&client, &url, &initialize(), Some("https://example.com")).await?;
    assert_eq!(from_page.0, 403);
    note(format!(
        "endpoint: unarmed task {}, armed task with Origin {}",
        unarmed.0, from_page.0
    ));

    let init = mcp_post(&client, &url, &initialize(), None).await?;
    let initialized = mcp_post(
        &client,
        &url,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        None,
    )
    .await?;
    let listed = mcp_post(
        &client,
        &url,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
        None,
    )
    .await?;
    let called = mcp_post(
        &client,
        &url,
        &json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "run", "arguments": { "code": "return 1 + 1" } }
        }),
        None,
    )
    .await?;
    assert_eq!(init.0, 200);
    assert_eq!(initialized.0, 202);
    assert_eq!(listed.1["result"]["tools"][0]["name"], "run");
    let text = called.1["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(text.contains("\n2\n"), "{text}");
    note(format!(
        "endpoint, raw JSON-RPC: initialize {} ({}), initialized {}, tools/list {:?}, tools/call isError {}",
        init.0,
        init.1["result"]["protocolVersion"],
        initialized.0,
        listed.1["result"]["tools"][0]["name"],
        called.1["result"]["isError"]
    ));

    if let Ok(script) = std::env::var("BROWSEROS_MCP_CLIENT") {
        let output = tokio::process::Command::new("node")
            .arg(&script)
            .arg(&url)
            .arg("return 1 + 1")
            .output()
            .await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(report["tools"], json!(["run"]));
        assert_eq!(report["result"]["isError"], false, "{report}");
        note(format!(
            "endpoint, real MCP client: server {}, protocol {}, tools {}, run isError {}",
            report["server"]["name"],
            report["protocol"],
            report["tools"],
            report["result"]["isError"]
        ));
    } else {
        note("endpoint, real MCP client: not run (BROWSEROS_MCP_CLIENT is not set)");
    }

    let in_flight = tokio::spawn({
        let client = client.clone();
        let url = url.clone();
        async move {
            let call = json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "run", "arguments": {
                    "code": "await page.locator('#missing').waitFor({ timeout: 60000 }); return 1;"
                } }
            });
            mcp_post(&client, &url, &call, None).await
        }
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    let dropped_at = Instant::now();
    drop(armed);
    let (status, reply) = tokio::time::timeout(Duration::from_secs(2), in_flight).await???;
    let dropped_ms = dropped_at.elapsed().as_millis();
    assert_eq!(status, 200);
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(reply["result"]["content"][0]["text"], "Stopped");
    let deadline = dropped_at + Duration::from_secs(1);
    while browser.engine_running_runs().await != Some(0) {
        assert!(
            Instant::now() < deadline,
            "the engine still runs the agent's script"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ended_ms = dropped_at.elapsed().as_millis();
    let closed = mcp_post(&client, &url, &initialize(), None).await?;
    assert_eq!(closed.0, 404);
    note(format!(
        "endpoint: dropping Armed stopped a 60 s call in {dropped_ms} ms, the engine had no run in flight after {ended_ms} ms; the task then answers {}",
        closed.0
    ));
    Ok(())
}

fn url_of(
    browser: &Browser,
    task: &Scope,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Arm and drop at once, to learn the address an unarmed task would have.
    let armed = browser.arm(task.clone()).ok_or("the engine arms")?;
    Ok(armed.url().to_owned())
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "browser-live-test", "version": "1" }
        }
    })
}

/// One POST as an MCP client sends it; the status and the message answering the request's id
/// (or `Null` for a notification).
async fn mcp_post(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    origin: Option<&str>,
) -> Result<(u16, Value), Box<dyn std::error::Error + Send + Sync>> {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(body);
    if let Some(origin) = origin {
        request = request.header("Origin", origin);
    }
    let response = request.send().await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    let messages: Vec<Value> = match serde_json::from_str::<Value>(&text) {
        Ok(single) => vec![single],
        Err(_) => text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|data| serde_json::from_str(data.trim_start()).ok())
            .collect(),
    };
    let reply = messages
        .into_iter()
        .find(|message| message.get("id").is_some() && message["id"] == body["id"])
        .unwrap_or(Value::Null);
    Ok((status, reply))
}

async fn run(browser: &Browser, scope: &Scope, code: &str) -> ToolOutput {
    browser
        .call(
            scope,
            "run",
            json!({ "code": code }),
            &CancellationToken::new(),
        )
        .await
}

/// The text between the fence's markers.
fn inner(output: &ToolOutput) -> &str {
    let after_open = output.text.split_once('\n').map_or("", |(_, rest)| rest);
    after_open.rsplit_once('\n').map_or("", |(body, _)| body)
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

async fn wait_until(
    within: Duration,
    mut condition: impl FnMut() -> bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let deadline = Instant::now() + within;
    while !condition() {
        if Instant::now() > deadline {
            return Err(format!("not true within {} s", within.as_secs()).into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

struct Fixture {
    page: String,
    noise: String,
}

/// The fixture pages on a loopback port of their own, for the life of the test.
async fn serve_fixture() -> Result<Fixture, std::io::Error> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let router = axum::Router::new()
        .route(
            "/",
            axum::routing::get(|| async { axum::response::Html(FIXTURE) }),
        )
        .route(
            "/noise",
            axum::routing::get(|| async { axum::response::Html(NOISE) }),
        );
    tokio::spawn(async move { axum::serve(listener, router).await });
    Ok(Fixture {
        page: format!("http://127.0.0.1:{port}/"),
        noise: format!("http://127.0.0.1:{port}/noise"),
    })
}

#[expect(clippy::print_stderr, reason = "the live test's evidence lines")]
fn note(line: impl std::fmt::Display) {
    eprintln!("[browser_live] {line}");
}
