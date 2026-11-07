//! Drives the shipped executable, not a substitute router: both startup paths, skill
//! replacement/restoration and real browser calls are checked with a private HOME/profile.
//! Set NEO_E2E_BROWSER to a BrowserOS neo binary; no existing browser or sidecar is touched.

#![cfg(unix)]

use std::{
    env, fs,
    io::{self, Write},
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
    anyhow::ensure!(status.is_success(), "MCP: HTTP {status}: {body}");
    let reply = if body.trim_start().starts_with('{') {
        serde_json::from_str(body)?
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .find(|reply| reply["id"] == id)
            .ok_or_else(|| anyhow::anyhow!("no RPC reply: {body}"))?
    };
    Ok(reply)
}

/// Older MCP clients cancel with a notification on their transport session. Exercise
/// that actual request token, independently of the runtime's own timeout interrupt.
async fn check_cancellation(client: &Client, base: &str, page: &str) -> anyhow::Result<()> {
    let response = client.post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":40,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"proto-e2e","version":"1"}
        }})).send().await?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("legacy MCP session was not established"))?
        .to_string();
    response.error_for_status()?.text().await?;
    client
        .post(format!("{base}/mcp"))
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session)
        .header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await?
        .error_for_status()?;
    let pending = client.post(format!("{base}/mcp")).header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session).header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({"jsonrpc":"2.0","id":41,"method":"tools/call","params":{"name":"run","arguments":{
            "page":page,"code":"await new Promise(resolve => setTimeout(resolve, 700)); await page.getByRole('button', {name:'Continue'}).click();"
        }}}));
    let pending = tokio::spawn(async move {
        let response = pending.send().await?;
        response.error_for_status()?.text().await
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    client.post(format!("{base}/mcp")).header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session).header("MCP-Protocol-Version", "2025-06-18")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":41,"reason":"test cancellation"}}))
        .send().await?.error_for_status()?;
    // A cancelled legacy request need not send a result. Check the observable browser
    // effect after the script would have clicked, rather than expecting a cancelled reply.
    tokio::time::timeout(Duration::from_secs(3), pending).await???;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let after = run(
        client,
        base,
        42,
        json!({"page":page,"code":"return await page.locator('p').getAttribute('data-clicks');"}),
    )
    .await?;
    assert_eq!(after["structuredContent"]["value"], "1", "{after}");
    Ok(())
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

async fn wait_skill(root: &Path, needle: &str) -> anyhow::Result<String> {
    let path = root.join("home/.agents/skills/browseros-neo/SKILL.md");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(content) = fs::read_to_string(&path)
            && content.contains(needle)
        {
            return Ok(content);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "skill was not installed: {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn stop_server(client: &Client, base: &str, process: &mut Process) -> anyhow::Result<()> {
    client
        .post(format!("{base}/system/shutdown"))
        .send()
        .await?
        .error_for_status()?;
    process.wait().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn flag_selects_playwright_and_skills_without_legacy_effects() -> anyhow::Result<()> {
    let Some(binary) = env::var_os("NEO_E2E_BROWSER") else {
        writeln!(
            io::stderr(),
            "SKIP: set NEO_E2E_BROWSER to a real BrowserOS neo executable"
        )?;
        return Ok(());
    };
    let root = TempDir::new()?;
    fs::create_dir_all(root.path().join("home"))?;
    let profile = root.path().join("browser-profile");
    let mut command = Command::new(binary);
    command
        .arg("--headless=new")
        .arg("--disable-browseros-server")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("about:blank");
    let mut browser = spawn(&mut command, root.path().join("browser.log"))?;
    let client = Client::builder().timeout(Duration::from_secs(20)).build()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let port_file = profile.join("DevToolsActivePort");
    let cdp_port = loop {
        if let Ok(content) = fs::read_to_string(&port_file)
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

    // Establish what flag-off actually ships, including the managed guide. No special
    // test-only handler or dependency injection can accidentally hide a boot regression.
    let (mut legacy, base) = start_server(root.path(), cdp_port, "0", &client).await?;
    let tools = rpc(&client, &base, 1, "tools/list", json!({})).await?;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .is_some_and(|tools| tools.len() > 1),
        "{tools}"
    );
    let old_result = run(&client, &base, 2, json!({"code":"return 17;"})).await?;
    assert_ne!(old_result["isError"], true, "{old_result}");
    assert!(old_result.to_string().contains("17"), "{old_result}");
    wait_skill(root.path(), "Core loop: snapshot -> act -> verify").await?;
    stop_server(&client, &base, &mut legacy).await?;
    let database = root.path().join("state/browserclaw.sqlite");
    let database_before = fs::read(&database)?;
    writeln!(
        io::stderr(),
        "PASS flag off: legacy tool catalog, run and skill"
    )?;

    let (mut prototype, base) = start_server(root.path(), cdp_port, "1", &client).await?;
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .header("Origin", "https://example.com")
            .json(&json!({}))
            .send()
            .await?
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .header("Content-Type", "text/plain")
            .body("{}")
            .send()
            .await?
            .status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let skill = wait_skill(root.path(), "Write Playwright JavaScript").await?;
    assert!(!skill.contains("name_session") && !skill.contains("snapshot -> act"));
    let tools = rpc(&client, &base, 3, "tools/list", json!({})).await?;
    let listed = tools["result"]["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing tools: {tools}"))?;
    assert_eq!(listed.len(), 1, "{tools}");
    assert_eq!(listed[0]["name"], "run");
    assert!(
        listed[0]["inputSchema"]["properties"]
            .get("session")
            .is_none()
    );
    assert!(
        client
            .get(format!("{base}/api/v1/sessions"))
            .send()
            .await?
            .status()
            .is_client_error()
    );
    let settings: Value = client
        .put(format!("{base}/api/v1/settings/agent"))
        .json(&json!({"humanHelpEnabled":false}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(settings["humanHelpEnabled"], false);

    let fixture_listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let fixture_url = format!("http://{}/", fixture_listener.local_addr()?);
    let fixture = tokio::spawn(async move {
        axum::serve(fixture_listener, Router::new()
            .route("/", get(|| async { axum::response::Html("<button onclick=\"const p=document.querySelector('p');p.textContent='clicked';p.setAttribute('data-clicks',String(Number(p.getAttribute('data-clicks')||0)+1))\">Continue</button><p>ready</p>") }))
            .route("/slow", get(|| async { tokio::time::sleep(Duration::from_secs(6)).await; axum::response::Html("<p>slow page</p>") }))
        ).await
    });
    let first = run(&client, &base, 4, json!({"code":format!("await page.goto({}); await page.getByRole('button', {{name:'Continue'}}).click(); console.log('clicked once'); return await page.locator('p').textContent();", serde_json::to_string(&fixture_url)?)})).await?;
    let report = &first["structuredContent"];
    assert_eq!(report["status"], "ok", "{first}");
    assert_eq!(report["value"], "clicked", "{first}");
    assert!(report["logs"].to_string().contains("clicked once"));
    assert!(
        report["calls"]
            .as_array()
            .is_some_and(|calls| calls.len() >= 3)
    );
    let page = report["pages"][0]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no page id: {first}"))?;
    assert_eq!(report["pages"][0]["created"], true);
    writeln!(
        io::stderr(),
        "PASS real BrowserOS MCP: goto, click, textContent -> clicked; page {page}"
    )?;

    let second = run(&client, &base, 5, json!({"page":page,"code":"return {text: await page.locator('p').textContent(), url: page.url()};"})).await?;
    assert_eq!(
        second["structuredContent"]["value"]["text"], "clicked",
        "{second}"
    );
    assert_eq!(second["structuredContent"]["value"]["url"], fixture_url);
    assert_eq!(second["structuredContent"]["pages"][0]["id"], page);
    assert_eq!(second["structuredContent"]["pages"][0]["created"], false);
    writeln!(
        io::stderr(),
        "PASS second independent MCP request continues on the same page id"
    )?;

    let timeout = run(
        &client,
        &base,
        6,
        json!({"page":page,"code":"while (true) {}","timeout":150}),
    )
    .await?;
    assert_eq!(
        timeout["structuredContent"]["status"], "timeout",
        "{timeout}"
    );
    assert_eq!(timeout["isError"], true);
    check_cancellation(&client, &base, page).await?;
    writeln!(
        io::stderr(),
        "PASS legacy MCP cancellation prevents the script's delayed click"
    )?;
    let slow = run(&client, &base, 8, json!({"page":page,"timeout":12000,"code":format!("await page.goto({}); return await page.locator('p').textContent();", serde_json::to_string(&format!("{fixture_url}slow"))?)})).await?;
    assert_eq!(slow["structuredContent"]["value"], "slow page", "{slow}");
    writeln!(
        io::stderr(),
        "PASS six-second navigation respects the twelve-second run budget"
    )?;
    let error = run(&client, &base, 7, json!({"page":page,"code":"console.log('before error'); throw new Error('expected failure');"})).await?;
    assert_eq!(error["structuredContent"]["status"], "error");
    assert!(
        error["structuredContent"]["logs"]
            .to_string()
            .contains("before error")
    );
    assert_eq!(error["isError"], true);
    assert!(
        client
            .get(format!("{base}/system/health"))
            .send()
            .await?
            .status()
            .is_success()
    );
    assert_eq!(
        fs::read(&database)?,
        database_before,
        "prototype touched the legacy database"
    );
    writeln!(
        io::stderr(),
        "PASS timeouts/errors; legacy database unchanged, legacy HTTP effects absent, Playwright skill installed"
    )?;
    stop_server(&client, &base, &mut prototype).await?;
    fixture.abort();

    let (mut restored, base) = start_server(root.path(), cdp_port, "0", &client).await?;
    wait_skill(root.path(), "Core loop: snapshot -> act -> verify").await?;
    stop_server(&client, &base, &mut restored).await?;
    writeln!(io::stderr(), "PASS flag off restores the legacy skill")?;
    let _ = cdp.send("Browser.close", json!({}), None).await;
    cdp.disconnect().await;
    browser.wait().await?;
    Ok(())
}
