//! Drives the shipped executable, not a substitute router: both startup paths, skill
//! replacement/restoration and real browser calls are checked with a private HOME/profile.
//! Set NEO_E2E_BROWSER to a BrowserOS neo binary; no existing browser or sidecar is touched.

#![cfg(unix)]

use std::{
    collections::BTreeSet,
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
    anyhow::ensure!(status.is_success(), "MCP: HTTP {status}: {body}");
    let reply = if body.trim_start().starts_with('{') {
        serde_json::from_str(&body)?
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

async fn page_targets(cdp: &CdpClient) -> anyhow::Result<BTreeSet<String>> {
    let reply = cdp.send("Target.getTargets", json!({}), None).await?;
    let targets = reply["targetInfos"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing targets: {reply}"))?;
    Ok(targets
        .iter()
        .filter(|target| target["type"] == "page")
        .filter_map(|target| target["targetId"].as_str().map(str::to_string))
        .collect())
}

async fn check_tab_workflow(
    client: &Client,
    base: &str,
    cdp: &CdpClient,
    fixture_url: &str,
) -> anyhow::Result<()> {
    let list_url = format!("{fixture_url}hn");
    let article_urls: BTreeSet<_> = (1..=5)
        .map(|n| format!("{fixture_url}article/{n}"))
        .collect();
    let list = run(
        client,
        base,
        50,
        json!({"code":format!("await page.goto({});", serde_json::to_string(&list_url)?)}),
    )
    .await?;
    assert_eq!(list["structuredContent"]["status"], "ok", "{list}");
    let list_id = list["structuredContent"]["pages"][0]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no list tab: {list}"))?;
    let guide = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../resources/skills/browserclaw/PLAYWRIGHT.md"),
    )?;
    let example = guide
        .split_once("## Tabs\n")
        .and_then(|(_, section)| section.split_once("```js\n"))
        .and_then(|(_, fenced)| fenced.split_once("```"))
        .map(|(code, _)| code)
        .ok_or_else(|| anyhow::anyhow!("Playwright guide has no Tabs example"))?;
    let opened = run(client, base, 51, json!({"page":list_id,"code":example})).await?;
    let report = &opened["structuredContent"];
    assert_eq!(report["status"], "ok", "{opened}");
    let values: Vec<Value> = serde_json::from_value(report["value"].clone())?;
    assert_eq!(
        values,
        (1..=5)
            .map(|n| json!({"title":format!("Article {n}"),"url":format!("{fixture_url}article/{n}")}))
            .collect::<Vec<_>>()
    );
    let pages: Vec<Value> = serde_json::from_value(report["pages"].clone())?;
    let original = pages
        .iter()
        .find(|page| page["id"] == list_id)
        .ok_or_else(|| anyhow::anyhow!("list tab missing: {opened}"))?;
    assert_eq!(original["created"], false);
    assert_eq!(original["url"], list_url);
    let articles: Vec<_> = pages
        .iter()
        .filter(|page| page["created"] == true)
        .collect();
    assert_eq!(articles.len(), 5, "{opened}");
    assert_eq!(
        articles
            .iter()
            .filter_map(|page| page["url"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>(),
        article_urls
    );
    let article_ids: BTreeSet<_> = articles
        .iter()
        .filter_map(|page| page["id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(article_ids.len(), 5);
    let article_id = articles
        .iter()
        .find(|page| page["url"] == format!("{fixture_url}article/1"))
        .and_then(|page| page["id"].as_str())
        .ok_or_else(|| anyhow::anyhow!("no first article tab: {opened}"))?;
    let closed = run(
        client,
        base,
        52,
        json!({"page":article_id,"code":format!(
            "const tabs = context.pages(); const urls = tabs.map(tab => tab.url());
         const heading = await page.locator('h1').textContent();
         for (const tab of tabs) {{ if ({}.includes(tab.url())) await tab.close(); }}
         return {{ urls, heading }};", serde_json::to_string(&article_urls)?
        )}),
    )
    .await?;
    let report = &closed["structuredContent"];
    assert_eq!(report["status"], "ok", "{closed}");
    assert_eq!(report["value"]["heading"], "Article 1");
    let listed_urls: BTreeSet<String> = serde_json::from_value(report["value"]["urls"].clone())?;
    assert!(article_urls.is_subset(&listed_urls));
    assert!(listed_urls.contains(&list_url));
    let pages: Vec<Value> = serde_json::from_value(report["pages"].clone())?;
    assert_eq!(
        pages
            .iter()
            .filter(|page| page["closed"] == true)
            .filter_map(|page| page["id"].as_str().map(str::to_string))
            .collect::<BTreeSet<_>>(),
        article_ids
    );
    let remaining = run(
        client,
        base,
        53,
        json!({"page":list_id,"code":
            "const urls = context.pages().map(tab => tab.url()); await page.close(); return urls;"
        }),
    )
    .await?;
    assert_eq!(
        remaining["structuredContent"]["status"], "ok",
        "{remaining}"
    );
    let remaining_urls: BTreeSet<String> =
        serde_json::from_value(remaining["structuredContent"]["value"].clone())?;
    assert!(article_urls.is_disjoint(&remaining_urls));
    eprintln!("PASS documented Tabs example: five article tabs opened, reused, listed and closed");

    let slow_url = format!("{fixture_url}slow");
    let timeout = run(
        client,
        base,
        54,
        json!({"timeout":2000,"code":format!(
            "await page.goto({});", serde_json::to_string(&slow_url)?
        )}),
    )
    .await?;
    let report = &timeout["structuredContent"];
    assert_eq!(timeout["isError"], true, "{timeout}");
    assert_eq!(report["status"], "timeout", "{timeout}");
    assert_eq!(report["pages"][0]["created"], true, "{timeout}");
    let timeout_id = report["pages"][0]["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("timeout lost its created tab: {timeout}"))?;
    let text: Value = serde_json::from_str(
        timeout["content"][0]["text"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("timeout has no text content: {timeout}"))?,
    )?;
    assert_eq!(text, *report);
    let resumed = run(
        client,
        base,
        55,
        json!({"page":timeout_id,"timeout":15000,"code":
            "await page.waitForLoadState('domcontentloaded'); return page.url();"
        }),
    )
    .await?;
    assert_eq!(resumed["structuredContent"]["status"], "ok", "{resumed}");
    assert_eq!(resumed["structuredContent"]["value"], slow_url);
    let closed = run(
        client,
        base,
        56,
        json!({"page":timeout_id,"code":"await page.close();"}),
    )
    .await?;
    assert_eq!(closed["structuredContent"]["status"], "ok", "{closed}");
    assert_eq!(closed["structuredContent"]["pages"][0]["id"], timeout_id);
    assert_eq!(closed["structuredContent"]["pages"][0]["closed"], true);
    eprintln!(
        "PASS navigation timeout preserves the created tab in structured and text results; resume and close work"
    );

    let before = page_targets(cdp).await?;
    let late = run(
        client,
        base,
        57,
        json!({"timeout":1,"code":"return page.url();"}),
    )
    .await?;
    let pages: Vec<Value> = serde_json::from_value(late["structuredContent"]["pages"].clone())?;
    for page in &pages {
        let id = page["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("tab has no id: {late}"))?;
        let closed = run(
            client,
            base,
            58,
            json!({"page":id,"code":"await page.close();"}),
        )
        .await?;
        assert_eq!(closed["structuredContent"]["status"], "ok", "{closed}");
        assert_eq!(closed["structuredContent"]["pages"][0]["closed"], true);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let after = page_targets(cdp).await?;
        if after.is_subset(&before) {
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "late creation leaked tabs: {after:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!("PASS one-millisecond run leaves no new tab behind");
    Ok(())
}

async fn wait_skill(root: &Path, needle: &str) -> anyhow::Result<String> {
    let path = root.join("home/.agents/skills/browseros-neo/SKILL.md");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(content) = fs::read_to_string(&path) {
            if content.contains(needle) {
                return Ok(content);
            }
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
        eprintln!("SKIP: set NEO_E2E_BROWSER to a real BrowserOS neo executable");
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
        if let Ok(content) = fs::read_to_string(&port_file) {
            if let Some(port) = content
                .lines()
                .next()
                .and_then(|line| line.parse::<u16>().ok())
            {
                break port;
            }
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
    eprintln!("PASS flag off: legacy tool catalog, run and skill");

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
    let description = listed[0]["description"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("run has no description: {tools}"))?;
    assert!(description.contains("context.newPage()"));
    assert!(description.chars().count() <= 4096);
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
    let headlines = format!(
        "<title>Stories</title><table>{}</table>",
        (1..=5)
            .map(|n| format!("<tr><td class=\"titleline\"><a href=\"{fixture_url}article/{n}\">Story {n}</a></td></tr>"))
            .collect::<String>()
    );
    let fixture = tokio::spawn(async move {
        axum::serve(fixture_listener, Router::new()
            .route("/", get(|| async { axum::response::Html("<button onclick=\"const p=document.querySelector('p');p.textContent='clicked';p.setAttribute('data-clicks',String(Number(p.getAttribute('data-clicks')||0)+1))\">Continue</button><p>ready</p>") }))
            .route("/slow", get(|| async { tokio::time::sleep(Duration::from_secs(6)).await; axum::response::Html("<p>slow page</p>") }))
            .route("/hn", get(move || { let html = headlines.clone(); async move { axum::response::Html(html) } }))
            .route("/article/{n}", get(|axum::extract::Path(n): axum::extract::Path<u32>| async move {
                axum::response::Html(format!("<title>Article {n}</title><h1>Article {n}</h1>"))
            }))
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
    eprintln!("PASS real BrowserOS MCP: goto, click, textContent -> clicked; page {page}");

    let second = run(&client, &base, 5, json!({"page":page,"code":"return {text: await page.locator('p').textContent(), url: page.url()};"})).await?;
    assert_eq!(
        second["structuredContent"]["value"]["text"], "clicked",
        "{second}"
    );
    assert_eq!(second["structuredContent"]["value"]["url"], fixture_url);
    assert_eq!(second["structuredContent"]["pages"][0]["id"], page);
    assert_eq!(second["structuredContent"]["pages"][0]["created"], false);
    eprintln!("PASS second independent MCP request continues on the same page id");
    check_tab_workflow(&client, &base, &cdp, &fixture_url).await?;

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
    eprintln!("PASS legacy MCP cancellation prevents the script's delayed click");
    let slow = run(&client, &base, 8, json!({"page":page,"timeout":12000,"code":format!("await page.goto({}); return await page.locator('p').textContent();", serde_json::to_string(&format!("{fixture_url}slow"))?)})).await?;
    assert_eq!(slow["structuredContent"]["value"], "slow page", "{slow}");
    eprintln!("PASS six-second navigation respects the twelve-second run budget");
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
    eprintln!(
        "PASS timeouts/errors; legacy database unchanged, legacy HTTP effects absent, Playwright skill installed"
    );
    stop_server(&client, &base, &mut prototype).await?;
    fixture.abort();

    let (mut restored, base) = start_server(root.path(), cdp_port, "0", &client).await?;
    wait_skill(root.path(), "Core loop: snapshot -> act -> verify").await?;
    stop_server(&client, &base, &mut restored).await?;
    eprintln!("PASS flag off restores the legacy skill");
    let _ = cdp.send("Browser.close", json!({}), None).await;
    cdp.disconnect().await;
    browser.wait().await?;
    Ok(())
}
