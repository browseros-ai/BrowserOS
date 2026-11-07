//! Exercises scoped snapshots, bounded responses and stored retrieval through the shipped MCP binary.
//! Set NEO_E2E_BROWSER; every process/profile/port belongs to this test.

#![cfg(unix)]
#![expect(
    clippy::expect_used,
    reason = "Assertions fail the test on malformed MCP replies"
)]

use std::{
    env, fs,
    net::TcpListener as StdListener,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use axum::{Router, routing::get};
use browseros_cdp::{CdpClient, ConnectOptions, ReconnectPolicy};
use reqwest::Client;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;

/// Owns only its freshly spawned process group. Drop is a failure-path cleanup; normal
/// teardown uses the server's shutdown API and Browser.close and waits for each child.
struct Process {
    child: Child,
    log: PathBuf,
}

impl Process {
    async fn wait(&mut self) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(status) = self.child.try_wait()? {
                anyhow::ensure!(
                    status.success(),
                    "process failed: {status}\n{}",
                    fs::read_to_string(&self.log).unwrap_or_default()
                );
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "process {} did not stop",
                self.child.id()
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status();
            let _ = self.child.wait();
        }
    }
}

fn spawn(command: &mut Command, log: PathBuf) -> anyhow::Result<Process> {
    let stderr = fs::File::create(&log)?;
    let child = command
        .process_group(0)
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()?;
    Ok(Process { child, log })
}

fn free_port() -> anyhow::Result<u16> {
    Ok(StdListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

async fn wait_http(client: &Client, url: &str, process: &mut Process) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if client
            .get(url)
            .send()
            .await
            .is_ok_and(|reply| reply.status().is_success())
        {
            return Ok(());
        }
        anyhow::ensure!(
            process.child.try_wait()?.is_none(),
            "process exited before {url}: {}",
            fs::read_to_string(&process.log).unwrap_or_default()
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "not ready: {url}\n{}",
            fs::read_to_string(&process.log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn start_server(
    root: &Path,
    cdp_port: u16,
    flag: &str,
    client: &Client,
) -> anyhow::Result<(Process, String)> {
    let port = free_port()?;
    let config = root.join("config.json");
    fs::write(
        &config,
        serde_json::to_vec(&json!({
            "ports":{"server":port,"cdp":cdp_port},
            "flags":{"devMode":false},
            "directories":{"resources":Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources")}
        }))?,
    )?;
    let binary = env::var_os("NEO_PROTO_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_browseros-claw-server-rs")));
    let mut command = Command::new(binary);
    // Hermetic discovery: skill installation must never see the developer's real harness
    // paths, profiles, configuration or analytics credentials.
    command
        .env_clear()
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("PATH", "")
        .env("BROWSERCLAW_DIR", root.join("state"))
        .env("BROWSERCLAW_PLAYWRIGHT_RUN", flag)
        .args(["--config"])
        .arg(config);
    let mut process = spawn(&mut command, root.join(format!("server-{flag}-{port}.log")))?;
    let base = format!("http://127.0.0.1:{port}");
    wait_http(client, &format!("{base}/system/ready"), &mut process).await?;
    Ok((process, base))
}

async fn rpc(
    client: &Client,
    base: &str,
    id: u64,
    method: &str,
    mut params: Value,
) -> anyhow::Result<Value> {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientCapabilities":{}
    });
    let mut request = client
        .post(format!("{base}/mcp"))
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", method)
        .header("Accept", "application/json, text/event-stream");
    if method == "tools/call" {
        request = request.header("Mcp-Name", params["name"].as_str().unwrap_or_default());
    }
    let response = request
        .json(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    parse_reply(id, status, &body)
}

fn parse_reply(id: u64, status: reqwest::StatusCode, body: &str) -> anyhow::Result<Value> {
    let reply: Value = if body.trim_start().starts_with('{') {
        serde_json::from_str(body)?
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .find(|reply| reply["id"] == id)
            .ok_or_else(|| anyhow::anyhow!("no RPC reply: {body}"))?
    };
    anyhow::ensure!(
        status.is_success() || reply.get("error").is_some(),
        "MCP: HTTP {status}: {body}"
    );
    Ok(reply)
}

async fn run(client: &Client, base: &str, id: u64, args: Value) -> anyhow::Result<Value> {
    let reply = rpc(
        client,
        base,
        id,
        "tools/call",
        json!({"name":"run","arguments":args}),
    )
    .await?;
    anyhow::ensure!(
        reply.get("error").is_none(),
        "run failed at protocol layer: {reply}"
    );
    Ok(reply["result"].clone())
}

async fn stop_server(client: &Client, base: &str, process: &mut Process) -> anyhow::Result<()> {
    client
        .post(format!("{base}/system/shutdown"))
        .send()
        .await?
        .error_for_status()?;
    process.wait().await
}

fn bounded(result: &Value) -> &Value {
    let text = result["content"][0]["text"]
        .as_str()
        .expect("expected test value");
    assert!(
        text.len() <= 24_000,
        "model-visible JSON: {} bytes",
        text.len()
    );
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("expected test value"),
        result["structuredContent"]
    );
    &result["structuredContent"]
}

#[expect(
    clippy::print_stderr,
    reason = "Report opt-in browser skips and verification measurements"
)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tests_that_scoped_snapshots_and_bounded_results_work_through_mcp() -> anyhow::Result<()> {
    let Some(binary) = env::var_os("NEO_E2E_BROWSER") else {
        eprintln!("SKIP: set NEO_E2E_BROWSER to a real BrowserOS neo executable");
        return Ok(());
    };
    let root = TempDir::new()?;
    fs::create_dir_all(root.path().join("home"))?;
    let profile = root.path().join("browser-profile");
    let mut command = Command::new(binary);
    command
        .args([
            "--headless=new",
            "--disable-browseros-server",
            "--remote-debugging-port=0",
            "--no-first-run",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank");
    let mut browser = spawn(&mut command, root.path().join("browser.log"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let cdp_port = loop {
        if let Ok(content) = fs::read_to_string(profile.join("DevToolsActivePort"))
            && let Some(port) = content
                .lines()
                .next()
                .and_then(|line| line.parse::<u16>().ok())
        {
            break port;
        }
        anyhow::ensure!(
            browser.child.try_wait()?.is_none() && Instant::now() < deadline,
            "BrowserOS did not publish a debugging port"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(cdp_port)
    })
    .await?;
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let (mut server, base) = start_server(root.path(), cdp_port, "1", &client).await?;
    let stories = (1..=30)
        .map(|i| format!("<article><a href='/article/{i}'>Story {i}</a></article>"))
        .collect::<String>();
    let products = (0..3500).map(|i| format!("<article><button onclick=\"document.getElementById('selection').textContent='selected {i}'\">Buy product {i:04} 漢字 🦀</button><p>Useful product details with a long description for item {i}</p></article>")).collect::<String>();
    let html = format!(
        "<!doctype html><html lang='en'><meta charset='utf-8'><title>Fixture shop</title><body><header>Outside header</header><main><section aria-label='Stories'>{stories}</section><section aria-label='Products'>{products}</section><p id='selection' role='status'>none</p></main><footer>Outside footer</footer></body></html>"
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let fixture = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/",
                    get(move || {
                        let html = html.clone();
                        async { axum::response::Html(html) }
                    }),
                )
                .route(
                    "/never",
                    get(|| async {
                        std::future::pending::<axum::response::Html<&'static str>>().await
                    }),
                ),
        )
        .await
    });

    let scoped = run(&client, &base, 1, json!({"code":format!(
        "await page.goto({}); return await page.getByRole('region', {{name:'Stories'}}).ariaSnapshot({{mode:'ai'}});", serde_json::to_string(&url)?)})).await?;
    let report = bounded(&scoped);
    assert_eq!(report["status"], "ok");
    let tree = report["value"].as_str().expect("expected test value");
    assert!(
        tree.contains("Story 30") && !tree.contains("Buy product") && !tree.contains("Outside")
    );
    let page = report["pages"][0]["id"]
        .as_str()
        .expect("expected test value");
    let full = run(&client, &base, 2, json!({"page":page,"code":
        "const tree = await page.ariaSnapshot({mode:'ai'}); if (tree.length < 100000) throw new Error('fixture too small'); let hash=0; for(let i=0;i<tree.length;i++) hash=(Math.imul(hash,31)+tree.charCodeAt(i))>>>0; console.log('hash:'+hash); console.log('pages:'+(await context.pages()).length); return tree;"})).await?;
    let report = bounded(&full);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["pages"][0]["id"], page);
    assert!(
        report["truncated"]["value"]["total"]
            .as_u64()
            .expect("expected test value")
            > 100_000
    );
    let mut text = report["value"]
        .as_str()
        .expect("expected test value")
        .to_string();
    let prefix = text.len();
    let mut next = report["truncated"]["value"]["next"].clone();
    let mut last_ref = None;
    let mut windows = 0;
    while next.is_object() {
        windows += 1;
        assert!(windows < 100, "retrieval must make progress");
        let window = run(&client, &base, 10 + windows, next.clone()).await?;
        let window = bounded(&window);
        assert!(
            window["pages"]
                .as_array()
                .expect("expected test value")
                .is_empty()
        );
        assert!(
            window["truncated"]["end"]
                .as_u64()
                .expect("expected test value")
                > next["offset"].as_u64().expect("expected test value")
        );
        let chunk = window["value"].as_str().expect("expected test value");
        for line in chunk
            .lines()
            .filter(|line| line.contains("button \"Buy product"))
        {
            if let Some((_, reference)) = line.split_once("[ref=") {
                last_ref = reference
                    .split_once(']')
                    .map(|(reference, _)| reference.to_string());
            }
        }
        text.push_str(chunk);
        next = window["truncated"]["next"].clone();
    }
    assert_eq!(
        text.len() as u64,
        report["truncated"]["value"]["total"]
            .as_u64()
            .expect("expected test value")
    );
    let hash = text.encode_utf16().fold(0_u32, |hash, ch| {
        hash.wrapping_mul(31).wrapping_add(u32::from(ch))
    });
    assert!(
        report["logs"]
            .as_array()
            .expect("expected test value")
            .contains(&json!(format!("hash:{hash}")))
    );
    assert!(text[prefix..].contains("Buy product 3499"));
    let reference = last_ref.expect("expected test value");
    let clicked = run(&client, &base, 150, json!({"page":page,"code":format!(
        "await page.locator('aria-ref={reference}').click(); return {{ selected: await page.getByRole('status').textContent(), url:page.url(), count:(await context.pages()).length }};")})).await?;
    let clicked = bounded(&clicked);
    assert_eq!(clicked["status"], "ok");
    assert_eq!(clicked["value"]["selected"], "selected 3499");
    assert_eq!(clicked["value"]["url"], url);
    assert!(
        report["logs"]
            .as_array()
            .expect("expected test value")
            .contains(&json!(format!("pages:{}", clicked["value"]["count"])))
    );

    let id = report["truncated"]["result"]
        .as_str()
        .expect("expected test value");
    for args in [
        json!({"result":id,"code":"return 1"}),
        json!({"result":id,"page":page}),
        json!({"result":id,"timeout":1}),
        json!({"code":"return 1","field":"value"}),
        json!({}),
    ] {
        let reply = rpc(
            &client,
            &base,
            160,
            "tools/call",
            json!({"name":"run","arguments":args}),
        )
        .await?;
        assert_eq!(reply["error"]["code"], -32602);
    }
    let expired = run(&client, &base, 161, json!({"result":"missing"})).await?;
    assert_eq!(expired["isError"], true);
    assert!(
        bounded(&expired)["pages"]
            .as_array()
            .expect("expected test value")
            .is_empty()
    );

    let failed = run(&client, &base, 170, json!({"timeout":5000,"code":format!(
        "console.log('large log\\n'.repeat(20000)); await page.goto({}, {{timeout:200}});", serde_json::to_string(&format!("{url}never"))?)})).await?;
    let failed_report = bounded(&failed);
    assert_eq!(failed["isError"], true);
    assert_eq!(failed_report["status"], "error");
    assert_eq!(failed_report["error"]["name"], "TimeoutError");
    assert!(
        failed_report["pages"][0]["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(failed_report["pages"][0]["created"], true);
    assert!(failed_report["truncated"]["logs"]["next"].is_object());
    eprintln!(
        "PASS MCP scoped subtree; original {} bytes, visible {} bytes, {windows} retrieval windows; retrieved ref clicks; failed navigation keeps page/error",
        text.len(),
        full["content"][0]["text"]
            .as_str()
            .expect("expected test value")
            .len()
    );
    stop_server(&client, &base, &mut server).await?;
    fixture.abort();
    let _ = cdp.send("Browser.close", json!({}), None).await;
    cdp.disconnect().await;
    browser.wait().await?;
    Ok(())
}
