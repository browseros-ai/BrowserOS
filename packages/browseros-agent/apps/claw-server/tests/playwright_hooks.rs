//! Neo: host exchanges through the streamable HTTP adapter and a private browser.

#![cfg(unix)]

use axum::{Router, routing::get};
use claw_server::api::mcp::playwright::PlaywrightService;
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    env, fs,
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;

struct Harness {
    service: PlaywrightService,
    server: JoinHandle<()>,
    browser: Option<Child>,
    client: Client,
    base: String,
    _root: TempDir,
}
impl Harness {
    async fn open(browser: bool) -> anyhow::Result<Self> {
        let root = TempDir::new()?;
        let mut child = None;
        let mut cdp_port = 1;
        if browser {
            let binary = env::var_os("NEO_E2E_BROWSER")
                .ok_or_else(|| anyhow::anyhow!("browser required"))?;
            let profile = root.path().join("profile");
            let log = fs::File::create(root.path().join("browser.log"))?;
            child = Some(
                Command::new(binary)
                    .args([
                        "--headless=new",
                        "--disable-browseros-server",
                        "--remote-debugging-port=0",
                        "--no-first-run",
                    ])
                    .arg(format!("--user-data-dir={}", profile.display()))
                    .arg("about:blank")
                    .process_group(0)
                    .stdout(Stdio::null())
                    .stderr(log)
                    .spawn()?,
            );
            let deadline = time::Instant::now() + Duration::from_secs(20);
            loop {
                if let Ok(text) = fs::read_to_string(profile.join("DevToolsActivePort"))
                    && let Some(port) = text.lines().next().and_then(|v| v.parse().ok())
                {
                    cdp_port = port;
                    break;
                }
                if time::Instant::now() >= deadline {
                    if let Some(process) = &mut child {
                        let _ = process.kill();
                        let _ = process.wait();
                    }
                    anyhow::bail!("private browser did not start");
                }
                time::sleep(Duration::from_millis(30)).await;
            }
        }
        let service = PlaywrightService::new(
            cdp_port,
            CancellationToken::new(),
            root.path().join("state"),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let app = Router::new()
            .nest_service("/mcp", service.http_service())
            .route(
                "/login",
                get(|| async {
                    axum::response::Html(
                        "<input id=user><input id=pw type=password><p id=runs>0</p>",
                    )
                }),
            );
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            service,
            server,
            browser: child,
            client: Client::builder().timeout(Duration::from_secs(30)).build()?,
            base,
            _root: root,
        })
    }
    async fn rpc(&self, mut params: Value, capabilities: Value) -> anyhow::Result<Value> {
        params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28", "io.modelcontextprotocol/clientCapabilities":capabilities, "io.modelcontextprotocol/clientInfo":{"name":"host-test","version":"1"}});
        let response = self
            .client
            .post(format!("{}/mcp", self.base))
            .header("MCP-Protocol-Version", "2026-07-28")
            .header("Mcp-Method", "tools/call")
            .header("Mcp-Name", "run")
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params}))
            .send()
            .await?;
        let body = response.text().await?;
        if body.trim_start().starts_with('{') {
            return Ok(serde_json::from_str(&body)?);
        }
        body.lines()
            .filter_map(|v| v.strip_prefix("data:"))
            .filter_map(|v| serde_json::from_str::<Value>(v.trim()).ok())
            .find(|v| v["id"] == 1 && v.get("method").is_none())
            .ok_or_else(|| anyhow::anyhow!("missing reply: {body}"))
    }
    async fn run(&self, args: Value) -> anyhow::Result<Value> {
        self.rpc(json!({"name":"run", "arguments":args}), json!({}))
            .await
    }
    async fn legacy(&self, args: Value, capable: bool, answer: Value) -> anyhow::Result<Value> {
        let caps = if capable {
            json!({"elicitation":{}})
        } else {
            json!({})
        };
        let initialized = self.client.post(format!("{}/mcp",self.base))
            .header("Accept","application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":10,"method":"initialize","params":{
                "protocolVersion":"2025-06-18","capabilities":caps,"clientInfo":{"name":"legacy","version":"1"}}}))
            .send().await?;
        let session = initialized
            .headers()
            .get("mcp-session-id")
            .ok_or_else(|| anyhow::anyhow!("no session"))?
            .to_str()?
            .to_string();
        initialized.error_for_status()?.text().await?;
        let post = || {
            self.client
                .post(format!("{}/mcp", self.base))
                .header("Accept", "application/json, text/event-stream")
                .header("MCP-Protocol-Version", "2025-06-18")
                .header("Mcp-Session-Id", &session)
        };
        post()
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await?
            .error_for_status()?;
        let mut stream = post().json(&json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"run","arguments":args}}))
            .send().await?.error_for_status()?;
        let mut buffer = String::new();
        while let Some(chunk) = stream.chunk().await? {
            buffer.push_str(std::str::from_utf8(&chunk)?);
            while let Some(end) = buffer.find("\n\n") {
                let event: String = buffer.drain(..end + 2).collect();
                for data in event.lines().filter_map(|line| line.strip_prefix("data:")) {
                    if data.trim().is_empty() {
                        continue;
                    }
                    let value: Value = serde_json::from_str(data.trim())?;
                    if value["method"] == "elicitation/create" {
                        assert!(capable);
                        assert_eq!(
                            value["params"]["_meta"]["com.browseros.neo/host"]["name"],
                            "vault.login"
                        );
                        let response = if let Some(error) = answer.get("error") {
                            json!({"jsonrpc":"2.0","id":value["id"],"error":error})
                        } else {
                            json!({"jsonrpc":"2.0","id":value["id"],"result":answer})
                        };
                        post().json(&response).send().await?.error_for_status()?;
                    } else if value["id"] == 11 {
                        return Ok(value);
                    }
                }
            }
        }
        anyhow::bail!("legacy stream ended without a result")
    }
    async fn close(self) {
        self.service.disconnect().await;
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(child) = &mut self.browser {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.wait();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_hook_arguments_require_code_and_object_meta() -> anyhow::Result<()> {
    let h = Harness::open(false).await?;
    for args in [
        json!({"code":"return 1", "meta":{}}),
        json!({"result":"r", "hooks":""}),
        json!({"result":"r", "meta":{}}),
        json!({"code":"return 1", "hooks":1}),
        json!({"code":"return 1", "hooks":"", "meta":[]}),
        json!({"code":"return 1", "hooks":"", "extra":true}),
    ] {
        assert_eq!(h.run(args).await?["error"]["code"], -32602);
    }
    h.close().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tests_that_hooks_use_host_answers_without_replaying_the_run_or_leaking_secrets()
-> anyhow::Result<()> {
    if env::var_os("NEO_E2E_BROWSER").is_none() {
        return Ok(());
    }
    let h = Harness::open(true).await?;
    let first = h.run(json!({"code":format!("await page.goto({}); return 17", serde_json::to_string(&format!("{}/login",h.base))?)})).await?;
    assert_eq!(first["result"]["structuredContent"]["value"], 17, "{first}");
    let page = first["result"]["structuredContent"]["pages"][0]["id"].clone();
    let caps = json!({"elicitation":{"form":{}}});
    let args = json!({"page":page, "meta":{"origin":h.base},
        "code":"await page.evaluate(() => document.querySelector('#runs').textContent++); return 23;",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { const login = await ctx.host('vault.login', {origin:ctx.meta.origin}); await e.page.locator('#user').fill(login.username); await e.page.locator('#pw').fill(login.password); ctx.trace(login.password); }); neo.on('afterRun', () => ({note:'signed in'}));"});
    let initial = h
        .rpc(json!({"name":"run","arguments":args}), caps.clone())
        .await?;
    assert_eq!(
        initial["result"]["resultType"], "input_required",
        "{initial}"
    );
    let req = &initial["result"]["inputRequests"]["host"];
    assert_eq!(req["method"], "elicitation/create");
    assert_eq!(
        req["params"]["_meta"]["com.browseros.neo/host"],
        json!({"name":"vault.login","args":{"origin":h.base}})
    );
    let retry = json!({"name":"run","arguments":args,"requestState":initial["result"]["requestState"],"inputResponses":{"host":{"action":"accept","content":{"json":json!({"username":"alice","password":"host-only-secret"}).to_string()}}}});
    let mut changed = retry.clone();
    changed["arguments"]["code"] = json!("return 'changed'");
    assert_eq!(h.rpc(changed, caps.clone()).await?["error"]["code"], -32602);
    let answer = h.rpc(retry.clone(), caps.clone()).await?;
    assert_eq!(
        answer["result"]["structuredContent"]["status"], "ok",
        "{answer}"
    );
    assert_eq!(answer["result"]["structuredContent"]["value"], 23);
    assert_eq!(
        answer["result"]["structuredContent"]["notes"],
        json!(["signed in"])
    );
    assert!(!answer.to_string().contains("host-only-secret"));
    assert!(
        answer["result"]["structuredContent"]
            .get("hook_trace")
            .is_none()
    );
    assert_eq!(h.rpc(retry, caps.clone()).await?["error"]["code"], -32602);
    let read = h.run(json!({"page":page, "code":"return [await page.locator('#user').inputValue(), await page.locator('#pw').inputValue(), await page.locator('#runs').textContent()]"})).await?;
    assert_eq!(
        read["result"]["structuredContent"]["value"],
        json!(["alice", "host-only-secret", "1"])
    );
    for response in [
        json!({"action":"decline"}),
        json!({"action":"cancel"}),
        json!({"action":"accept"}),
        json!({"action":"accept","content":{"json":"bad"}}),
        json!({"action":"accept","content":{"json":1}}),
        json!({"error":{"code":-32603,"message":"host-only-secret"}}),
    ] {
        let initial = h
            .rpc(json!({"name":"run","arguments":args}), caps.clone())
            .await?;
        let answer = h.rpc(json!({"name":"run","arguments":args,"requestState":initial["result"]["requestState"],"inputResponses":{"host":response}}),caps.clone()).await?;
        assert_eq!(
            answer["result"]["structuredContent"]["status"], "blocked",
            "{answer}"
        );
        assert!(!answer.to_string().contains("host-only-secret"));
    }
    let missing = h.run(args.clone()).await?;
    assert_eq!(
        missing["result"]["structuredContent"]["status"], "blocked",
        "{missing}"
    );
    let read = h
        .run(json!({"page":page,"code":"return await page.locator('#runs').textContent()"}))
        .await?;
    assert_eq!(read["result"]["structuredContent"]["value"], "1");
    let mut plain = json!({"page":page,"code":"return 7"});
    let legacy = json!({"page":page, "code":"return 31",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { const answer = await ctx.host('vault.login', {}); if (answer !== 7) return {block:'wrong answer'}; });"});
    assert_eq!(
        h.legacy(
            legacy.clone(),
            true,
            json!({"action":"accept","content":{"json":"7"}})
        )
        .await?["result"]["structuredContent"]["value"],
        31
    );
    for capable in [false, true] {
        assert_eq!(
            h.legacy(legacy.clone(), capable, json!({"action":"decline"}))
                .await?["result"]["structuredContent"]["status"],
            "blocked"
        );
    }
    let error = h
        .legacy(
            legacy,
            true,
            json!({"error":{"code":-32603,"message":"host-only-secret"}}),
        )
        .await?;
    assert_eq!(error["result"]["structuredContent"]["status"], "blocked");
    assert!(!error.to_string().contains("host-only-secret"));
    let without = h.run(plain.clone()).await?;
    plain["hooks"] = json!("");
    assert_eq!(without, h.run(plain).await?);

    // Concurrent host calls retain the same realm; only the agent script runs once.
    let parallel = json!({"page":page,"code":"return 29",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { const answers = await Promise.all([ctx.host('first', 1), ctx.host('second', 2)]); if (answers[0] !== 11 || answers[1] !== 22) return {block:'wrong answers'}; });"});
    let mut next = h
        .rpc(json!({"name":"run","arguments":parallel}), caps.clone())
        .await?;
    let mut states = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for _ in 0..2 {
        let host =
            &next["result"]["inputRequests"]["host"]["params"]["_meta"]["com.browseros.neo/host"];
        let name = host["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing host name"))?
            .to_string();
        assert!(names.insert(name.clone()));
        assert!(name == "first" || name == "second");
        assert_eq!(host["args"], if name == "first" { 1 } else { 2 });
        states.push(next["result"]["requestState"].clone());
        next = h.rpc(json!({"name":"run","arguments":parallel,"requestState":next["result"]["requestState"],
            "inputResponses":{"host":{"action":"accept","content":{"json":if name == "first" {"11"} else {"22"}}}}}),caps.clone()).await?;
    }
    assert_ne!(states[0], states[1]);
    assert_eq!(next["result"]["structuredContent"]["value"], 29, "{next}");

    let abandoned = json!({"page":page,"timeout":150,"code":"await page.evaluate(() => document.querySelector('#runs').textContent++)",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { await ctx.host('never', {}); });"});
    let parked = h
        .rpc(json!({"name":"run","arguments":abandoned}), caps.clone())
        .await?;
    assert_eq!(parked["result"]["resultType"], "input_required");
    time::sleep(Duration::from_millis(300)).await;
    let late = h.rpc(json!({"name":"run","arguments":abandoned,"requestState":parked["result"]["requestState"],
        "inputResponses":{"host":{"action":"accept","content":{"json":"null"}}}}),caps.clone()).await?;
    assert_eq!(late["error"]["code"], -32602);
    let read = h
        .run(json!({"page":page,"code":"return await page.locator('#runs').textContent()"}))
        .await?;
    assert_eq!(read["result"]["structuredContent"]["value"], "1");

    let many = json!({"page":page,"code":"return 'must not run'",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { for (let i=0;i<10;i++) await ctx.host('round', i); });"});
    let mut next = h
        .rpc(json!({"name":"run","arguments":many}), caps.clone())
        .await?;
    for _ in 0..9 {
        assert_eq!(next["result"]["resultType"], "input_required", "{next}");
        next = h
            .rpc(
                json!({"name":"run","arguments":many,"requestState":next["result"]["requestState"],
            "inputResponses":{"host":{"action":"accept","content":{"json":"null"}}}}),
                caps.clone(),
            )
            .await?;
    }
    assert_eq!(
        next["result"]["structuredContent"]["status"], "blocked",
        "{next}"
    );
    // Dropping an active HTTP response still cancels a hook-enabled run.
    let request = h.client.post(format!("{}/mcp",h.base))
        .header("Accept","application/json, text/event-stream").header("MCP-Protocol-Version","2026-07-28")
        .header("Mcp-Method","tools/call").header("Mcp-Name","run")
        .json(&json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{
            "name":"run","arguments":{"page":page,"code":"await page.evaluate(() => document.querySelector('#runs').textContent++)",
                "hooks":"neo.on('beforeRun', async (e) => { await e.page.evaluate(() => document.querySelector('#runs').dataset.waiting = 'yes'); await new Promise(r => setTimeout(r, 500)); });"},
            "_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}));
    let pending =
        tokio::spawn(async move { request.send().await?.error_for_status()?.text().await });
    let waiting = h.run(json!({"page":page,"code":"await page.waitForFunction(() => document.querySelector('#runs').dataset.waiting === 'yes'); return true"})).await?;
    assert_eq!(waiting["result"]["structuredContent"]["value"], true);
    pending.abort();
    let _ = pending.await;
    time::sleep(Duration::from_millis(600)).await;
    let read = h
        .run(json!({"page":page,"code":"return await page.locator('#runs').textContent()"}))
        .await?;
    assert_eq!(read["result"]["structuredContent"]["value"], "1");
    let pending = h
        .rpc(
            json!({"name":"run","arguments":{"page":page,"code":"return 1",
        "hooks":"neo.on('beforeRun', async (e, ctx) => { await ctx.host('shutdown', {}); });"}}),
            caps,
        )
        .await?;
    assert_eq!(pending["result"]["resultType"], "input_required");
    time::timeout(Duration::from_secs(2), h.close()).await?;
    Ok(())
}
