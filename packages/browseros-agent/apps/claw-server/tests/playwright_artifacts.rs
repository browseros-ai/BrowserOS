//! `path` options through the shipped MCP binary: every run gets `<state_dir>/artifacts/`
//! (REAL fly-unified-3, opendining-2, dashdish-3: absolute paths were refused for want of an
//! artifact directory). Set NEO_E2E_BROWSER; every process/profile/port belongs to this test.

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

/// The structured report, checked against the text content the model reads.
fn report(result: &Value) -> &Value {
    let text = result["content"][0]["text"]
        .as_str()
        .expect("a text content block");
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("JSON text"),
        result["structuredContent"]
    );
    &result["structuredContent"]
}

fn is_png(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
}

/// An isolated headless BrowserOS neo and its CDP port.
async fn start_browser(root: &Path, binary: &std::ffi::OsStr) -> anyhow::Result<(Process, u16)> {
    let profile = root.join("browser-profile");
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
    let mut browser = spawn(&mut command, root.join("browser.log"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let port = loop {
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
    Ok((browser, port))
}

#[expect(clippy::print_stderr, reason = "Report opt-in browser skips")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tests_that_path_options_write_into_the_shared_artifact_directory() -> anyhow::Result<()> {
    let Some(binary) = env::var_os("NEO_E2E_BROWSER") else {
        eprintln!("SKIP: set NEO_E2E_BROWSER to a real BrowserOS neo executable");
        return Ok(());
    };
    let root = TempDir::new()?;
    fs::create_dir_all(root.path().join("home"))?;
    let (mut browser, cdp_port) = start_browser(root.path(), &binary).await?;
    let cdp = CdpClient::connect(ConnectOptions {
        reconnect_policy: ReconnectPolicy::KeepTrying,
        ..ConnectOptions::new(cdp_port)
    })
    .await?;
    let client = Client::builder().timeout(Duration::from_secs(60)).build()?;
    let (mut server, base) = start_server(root.path(), cdp_port, "1", &client).await?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let fixture = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/",
                get(|| async {
                    axum::response::Html(
                        "<!doctype html><title>tall</title><style>body{margin:0}</style>\
                         <input type=file><div style='height:3000px;background:linear-gradient(#f80,#08f)'></div>",
                    )
                }),
            ),
        )
        .await
    });
    let artifacts = root.path().join("state/artifacts");
    let outside = root.path().join("results/fly-unified-3/columns.png");

    // The REAL snippet, plus a relative path; the reply lists both with absolute files.
    let saved = run(&client, &base, 1, json!({"code":format!(
        "await page.goto({url}); await page.screenshot({{path:'shots/a.png'}}); \
         await page.screenshot({{path:{outside},fullPage:true}}); return 'Screenshot saved to columns.png';",
        url = serde_json::to_string(&url)?,
        outside = serde_json::to_string(&outside)?,
    )})).await?;
    let saved = report(&saved);
    assert_eq!(saved["status"], "ok", "{saved}");
    assert_eq!(saved["value"], "Screenshot saved to columns.png");
    let files = saved["artifacts"].as_array().expect("artifacts listed");
    assert_eq!(files.len(), 2, "{saved}");
    assert_eq!(files[0]["path"], "shots/a.png");
    assert_eq!(
        files[0]["file"],
        artifacts.join("shots/a.png").display().to_string()
    );
    assert!(files[0].get("requested").is_none(), "{saved}");
    assert_eq!(files[1]["path"], "columns.png");
    assert_eq!(files[1]["requested"], outside.display().to_string());
    assert_eq!(
        files[1]["file"],
        artifacts.join("columns.png").display().to_string()
    );
    assert!(is_png(&artifacts.join("shots/a.png")) && is_png(&artifacts.join("columns.png")));
    assert!(
        !outside.exists(),
        "nothing written outside the artifact directory"
    );
    let logs = saved["logs"].as_array().expect("logs");
    assert!(
        logs.iter().any(|line| line.as_str().is_some_and(|line| {
            line.contains(&outside.display().to_string())
                && line.contains(&artifacts.join("columns.png").display().to_string())
        })),
        "one log line says where it went: {logs:?}"
    );
    let page = saved["pages"][0]["id"].as_str().expect("a page id");

    // A later run names an earlier run's file by its reported absolute path: honored as is for
    // a write, and readable for an upload.
    let file = files[0]["file"].as_str().expect("file");
    let again = run(&client, &base, 2, json!({"page":page,"code":format!(
        "await page.screenshot({{path:{file}}}); \
         await page.locator('input[type=file]').setInputFiles({file}); \
         return await page.locator('input[type=file]').evaluate((input) => input.files[0].name + ':' + (input.files[0].size > 0));",
        file = serde_json::to_string(file)?,
    )})).await?;
    let again = report(&again);
    assert_eq!(again["status"], "ok", "{again}");
    assert_eq!(again["value"], "a.png:true");
    assert_eq!(again["artifacts"][0]["path"], "shots/a.png");
    assert!(again["artifacts"][0].get("requested").is_none(), "{again}");

    // A path naming a directory still fails like Node's fs, and is not listed.
    let refused = run(
        &client,
        &base,
        3,
        json!({"page":page,"code":
        "return await page.screenshot({path:'shots/'}).then(() => 'ok', (error) => error.name);"}),
    )
    .await?;
    let refused = report(&refused);
    assert_eq!(refused["value"], "Error", "{refused}");
    assert!(refused.get("artifacts").is_none(), "{refused}");

    stop_server(&client, &base, &mut server).await?;
    fixture.abort();
    let _ = cdp.send("Browser.close", json!({}), None).await;
    cdp.disconnect().await;
    browser.wait().await?;
    Ok(())
}
