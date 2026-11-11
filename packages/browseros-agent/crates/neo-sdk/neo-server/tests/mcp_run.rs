//! `run` over stateless MCP end to end: the app's real router and wiring on the fake browser,
//! rmcp clients on the legacy protocol (2025-11-25) and on 2026-07-28, and raw HTTP where the
//! headers themselves are the point.

use std::error::Error;
use std::fs;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future;
use neo_server::App;
use neo_server::test_support::{FakeCdp, test_config};
use neo_trace::{
    Delivery, Effect, ErrorOwner, Outcome, RunQuery, RunStatus, RunSummary, TaskId, TraceStore,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, ProtocolVersion,
    RequestMetaObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt, ServiceExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{self, Instant};

type TestResult = Result<(), Box<dyn Error>>;
type Client = RunningService<RoleClient, ClientInfo>;

/// The `_meta` key a host's ids and the run's outcome ride under.
const RUN_META: &str = "com.browseros.neo/run";
/// Generous for a debug build on a loaded machine; a passing test takes a fraction of it.
const PATIENCE: Duration = Duration::from_secs(20);

/// Runs a test on Neo's own runtime: the app's tasks get the 8 MiB worker stacks they get in
/// production.
fn on_neo_runtime(test: impl Future<Output = TestResult>) -> TestResult {
    App::runtime()?.block_on(test)
}

/// Neo serving on a port the OS picked, over a fake browser.
struct Neo {
    address: SocketAddr,
    store: TraceStore,
    fake: Arc<FakeCdp>,
    _dir: TempDir,
}

impl Neo {
    /// Started, with the browser linked.
    async fn start() -> Result<Self, Box<dyn Error>> {
        let neo = Self::start_with(FakeCdp::new()).await?;
        neo.until_ready().await?;
        Ok(neo)
    }

    async fn start_with(fake: Arc<FakeCdp>) -> Result<Self, Box<dyn Error>> {
        Self::start_in(tempfile::tempdir()?, fake).await
    }

    /// Started on a state directory the test prepared.
    async fn start_in(dir: TempDir, fake: Arc<FakeCdp>) -> Result<Self, Box<dyn Error>> {
        let app = App::start_with_fake_browser(test_config(dir.path()), Arc::clone(&fake)).await?;
        let address = app.local_addr()?;
        let store = app.trace_store();
        tokio::spawn(app.serve());
        Ok(Self {
            address,
            store,
            fake,
            _dir: dir,
        })
    }

    async fn until_ready(&self) -> TestResult {
        within(PATIENCE, async {
            while !self.get("/system/ready").await?.starts_with("HTTP/1.1 200") {
                time::sleep(Duration::from_millis(20)).await;
            }
            Ok(())
        })
        .await?
    }

    fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    /// A client on the legacy protocol, as most hosts speak it today.
    async fn legacy_client(&self) -> Result<Client, Box<dyn Error>> {
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        let transport = StreamableHttpClientTransport::from_config(self.transport());
        Ok(info.serve(transport).await?)
    }

    /// A client on 2026-07-28, which has no `initialize` and no sessions.
    async fn modern_client(&self) -> Result<Client, Box<dyn Error>> {
        let lifecycle = ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        };
        let transport = StreamableHttpClientTransport::from_config(self.transport());
        Ok(ClientInfo::default()
            .serve_with_lifecycle(transport, lifecycle)
            .await?)
    }

    fn transport(&self) -> StreamableHttpClientTransportConfig {
        // rmcp's client queues past 16 concurrent requests by default.
        StreamableHttpClientTransportConfig::with_uri(self.url()).max_concurrent_requests(64)
    }

    async fn get(&self, path: &str) -> Result<String, Box<dyn Error>> {
        self.raw(&format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        ))
        .await
    }

    /// One raw HTTP/1.1 exchange on a fresh connection, read to the end.
    async fn raw(&self, request: &str) -> Result<String, Box<dyn Error>> {
        let mut stream = TcpStream::connect(self.address).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        Ok(response)
    }

    /// The task's latest run once the trace has it with `status`.
    async fn until_status(
        &self,
        task: &str,
        status: RunStatus,
    ) -> Result<RunSummary, Box<dyn Error>> {
        let query = RunQuery {
            task_id: Some(TaskId::parse(task)?),
            ..RunQuery::default()
        };
        within(PATIENCE, async {
            loop {
                if let Some(run) = self
                    .store
                    .runs(query.clone())
                    .await?
                    .items
                    .into_iter()
                    .next()
                    && run.status == status
                {
                    return Ok::<_, Box<dyn Error>>(run);
                }
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?
    }
}

async fn within<T>(limit: Duration, work: impl Future<Output = T>) -> Result<T, Box<dyn Error>> {
    time::timeout(limit, work)
        .await
        .map_err(|error| format!("not within {limit:?}: {error}").into())
}

/// `run` with these arguments and, when given, this `_meta["com.browseros.neo/run"]`.
async fn run(
    client: &Client,
    arguments: Value,
    meta: Option<Value>,
) -> Result<CallToolResult, Box<dyn Error>> {
    let Value::Object(arguments) = arguments else {
        return Err("arguments are an object".into());
    };
    let mut params = CallToolRequestParams::new("run").with_arguments(arguments);
    if let Some(meta) = meta {
        let mut object = MetaObject::new();
        object.insert(RUN_META.to_owned(), meta);
        params.meta = Some(RequestMetaObject(object));
    }
    Ok(client.call_tool(params).await?)
}

/// `_meta["com.browseros.neo/run"]` of a result.
fn outcome(result: &CallToolResult) -> Result<&Value, Box<dyn Error>> {
    result
        .meta
        .as_ref()
        .and_then(|meta| meta.get(RUN_META))
        .ok_or_else(|| "every result carries the run's _meta".into())
}

#[test]
fn both_protocols_list_one_tool_and_run_plain_javascript() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        for client in [neo.legacy_client().await?, neo.modern_client().await?] {
            let tools = client.list_tools(None).await?.tools;
            let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
            assert_eq!(names, ["run"], "exactly one tool");
            let schema = tools
                .first()
                .map(|tool| Value::Object((*tool.input_schema).clone()));
            assert_eq!(
                schema.as_ref().and_then(|schema| schema.get("required")),
                Some(&json!(["code"])),
                "the schema comes from the arguments type"
            );

            let result = run(&client, json!({ "code": "return 6 * 7;" }), None).await?;
            assert_eq!(result.structured_content, Some(json!({ "value": 42 })));
            assert_eq!(result.is_error, Some(false));
            let meta = outcome(&result)?;
            assert_eq!(meta.get("status"), Some(&json!("ok")));
            assert_eq!(meta.get("effects"), Some(&json!("none")));
            assert_eq!(meta.get("unknown"), Some(&json!([])));
            let id = meta
                .get("id")
                .and_then(Value::as_str)
                .ok_or("an id")?
                .parse()?;
            // The writer commits in 5 ms batches, so the record can trail the reply a little.
            within(PATIENCE, async {
                while neo.store.run(id).await?.is_none() {
                    time::sleep(Duration::from_millis(10)).await;
                }
                Ok::<_, Box<dyn Error>>(())
            })
            .await??;
            client.cancel().await?;
        }
        Ok(())
    })
}

#[test]
fn no_session_id_is_issued() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-11-25", "capabilities": {},
                        "clientInfo": { "name": "probe", "version": "1" } },
        })
        .to_string();
        let response = neo
            .raw(&format!(
                "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
                 Accept: application/json, text/event-stream\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            ))
            .await?;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(
            !response.to_ascii_lowercase().contains("mcp-session-id"),
            "stateless on the legacy protocol too: {response}"
        );
        Ok(())
    })
}

#[test]
fn meta_goes_into_the_trace_and_a_repeat_replays_the_first_id() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let client = neo.modern_client().await?;
        let meta =
            json!({ "requestId": "call-1", "taskId": "task-1", "hostData": { "ownerRun": "r9" } });
        let code = json!({ "code": "return Math.random();" });
        let first = run(&client, code.clone(), Some(meta.clone())).await?;
        let repeat = run(&client, code, Some(meta)).await?;
        assert_eq!(
            outcome(&repeat)?.get("id"),
            outcome(&first)?.get("id"),
            "a repeated requestId is the first run"
        );
        assert_eq!(
            repeat.structured_content, first.structured_content,
            "and its value"
        );
        let recorded = neo.until_status("task-1", RunStatus::Ok).await?;
        assert_eq!(
            recorded.request_id.map(|id| id.to_string()),
            Some("call-1".to_owned())
        );
        client.cancel().await?;
        Ok(())
    })
}

#[test]
fn a_script_drives_a_page_through_the_playwright_api() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let client = neo.modern_client().await?;
        let tools = client.list_tools(None).await?.tools;
        let description = tools
            .first()
            .and_then(|tool| tool.description.as_deref())
            .ok_or("run has a description")?;
        assert!(
            description.contains("`getByRole`") && description.contains("page.getByRef('e5')"),
            "the description lists the Playwright API and its ref hint: {description}"
        );

        // The fake browser has no isolated world for neo-playwright's reads, so this drive
        // opens and closes a tab; tests/e2e_browseros.rs reads pages on a real BrowserOS.
        let code = r"
            const p = await context.newPage();
            await p.close();
            return p.isClosed();";
        let result = run(
            &client,
            json!({ "code": code }),
            Some(json!({ "taskId": "task-pages" })),
        )
        .await?;
        assert_eq!(
            result.structured_content,
            Some(json!({ "value": true })),
            "the page knows it closed from the reply's page facts"
        );
        assert_eq!(outcome(&result)?.get("status"), Some(&json!("ok")));
        assert!(
            neo.fake
                .calls()
                .iter()
                .any(|call| call.method == "Browser.closeTab"),
            "the browser closed the tab"
        );

        let recorded = neo.until_status("task-pages", RunStatus::Ok).await?;
        let detail = neo
            .store
            .run(recorded.id)
            .await?
            .ok_or("the run is stored")?;
        let ops: Vec<_> = detail
            .operations
            .iter()
            // Worker bookkeeping can run read-only subscriptions beside the foreground writes.
            .filter(|op| op.effect == Effect::Write)
            .map(|op| (op.name.as_str(), op.effect, op.outcome, op.delivery))
            .collect();
        assert_eq!(
            ops,
            [
                (
                    "context.newPage",
                    Effect::Write,
                    Some(Outcome::Ok),
                    Some(Delivery::Done)
                ),
                (
                    "page.close",
                    Effect::Write,
                    Some(Outcome::Ok),
                    Some(Delivery::Done)
                ),
            ],
            "writes are done once sent"
        );
        client.cancel().await?;
        Ok(())
    })
}

#[test]
fn without_a_provider_key_solve_captcha_is_unlisted_and_throws_for_the_host() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let client = neo.modern_client().await?;
        let tools = client.list_tools(None).await?.tools;
        let description = tools
            .first()
            .and_then(|tool| tool.description.as_deref())
            .ok_or("run has a description")?;
        assert!(
            !description.contains("solveCaptcha"),
            "a Neo without a key does not offer it: {description}"
        );

        // `neo` reaches the script as a parameter, like `page`.
        let code = r"
            try {
              await neo.solveCaptcha(page);
              return 'solved without a key';
            } catch (e) {
              return { name: e.name, code: e.code, owner: e.owner, delivery: e.delivery };
            }";
        let result = run(&client, json!({ "code": code }), None).await?;
        assert_eq!(
            result.structured_content,
            Some(json!({ "value": {
                "name": "NeoError",
                "code": "captcha_unavailable",
                "owner": "host",
                "delivery": "not_sent",
            } })),
            "the deployment adds the key, so the host owns the failure: {result:?}"
        );
        client.cancel().await?;
        Ok(())
    })
}

#[test]
fn a_malformed_call_is_refused_with_its_owner_and_still_has_an_id() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let client = neo.legacy_client().await?;
        let typo = run(
            &client,
            json!({ "code": "return 1;" }),
            Some(json!({ "requestID": "x" })),
        )
        .await?;
        assert_eq!(typo.is_error, Some(true));
        let meta = outcome(&typo)?;
        assert_eq!(
            (meta.pointer("/error/owner"), meta.pointer("/error/code")),
            (Some(&json!("host")), Some(&json!("invalid_meta"))),
            "the host sent it"
        );
        assert!(
            meta.get("id").and_then(Value::as_str).is_some(),
            "an id on every result"
        );

        for arguments in [
            json!({ "code": "return 1;", "timeout": null }),
            json!({ "code": "return 1;", "timout": 5000 }),
            json!({ "code": "return 1;", "timeout": 0 }),
        ] {
            let refused = run(&client, arguments.clone(), None).await?;
            let meta = outcome(&refused)?;
            assert_eq!(
                (meta.pointer("/error/owner"), meta.pointer("/error/code")),
                (Some(&json!("script")), Some(&json!("invalid_input"))),
                "{arguments}: the model sent it"
            );
        }
        let unknown = client.call_tool(CallToolRequestParams::new("eval")).await;
        assert!(unknown.is_err(), "only `run` exists");
        client.cancel().await?;
        Ok(())
    })
}

#[test]
fn a_dropped_stream_stops_a_run_without_a_request_id() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {
                "name": "run",
                "arguments": { "code": "await new Promise((resolve) => setTimeout(resolve, 600000));" },
                "_meta": { "com.browseros.neo/run": { "taskId": "task-dropped" } },
            },
        })
        .to_string();
        let mut stream = TcpStream::connect(neo.address).await?;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
             Accept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await?;
        let mut head = [0_u8; 256];
        let read = stream.read(&mut head).await?;
        assert!(
            String::from_utf8_lossy(head.get(..read).unwrap_or_default())
                .starts_with("HTTP/1.1 200"),
            "the SSE response opens before the run ends"
        );
        neo.until_status("task-dropped", RunStatus::Running).await?;
        let dropped = Instant::now();
        drop(stream);
        let ended = neo
            .until_status("task-dropped", RunStatus::Cancelled)
            .await?;
        assert_eq!(
            ended.error.map(|error| (error.owner, error.code)),
            Some((ErrorOwner::Host, "cancelled".to_owned())),
            "stateless rmcp's only cancel is the stream going away"
        );
        assert!(
            dropped.elapsed() < Duration::from_secs(12),
            "noticed on the next 5 s keep-alive, not at the run's deadline"
        );
        Ok(())
    })
}

#[test]
fn fifty_runs_at_once_all_execute() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let client = neo.legacy_client().await?;
        let started = Instant::now();
        let calls = (0..50).map(|index| {
            let code = format!(
                "await new Promise((resolve) => setTimeout(resolve, 1000)); return {index};"
            );
            run(&client, json!({ "code": code }), None)
        });
        let results = future::try_join_all(calls).await?;
        let values: Vec<Option<Value>> = results
            .into_iter()
            .map(|result| result.structured_content)
            .collect();
        let expected: Vec<Option<Value>> = (0..50)
            .map(|index| Some(json!({ "value": index })))
            .collect();
        assert_eq!(values, expected);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no concurrency limit: 50 one-second runs overlap ({:?})",
            started.elapsed()
        );
        client.cancel().await?;
        Ok(())
    })
}

#[test]
fn request_hygiene_keeps_browser_pages_out() -> TestResult {
    on_neo_runtime(async {
        let neo = Neo::start().await?;
        let post = |headers: &str| {
            format!(
                "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
        };
        let from_page = neo
            .raw(&post(
                "Content-Type: application/json\r\nOrigin: https://evil.example\r\n",
            ))
            .await?;
        assert!(
            from_page.starts_with("HTTP/1.1 403"),
            "a page's fetch carries Origin: {from_page}"
        );
        let fetched = neo
            .raw(&post(
                "Content-Type: application/json\r\nSec-Fetch-Site: cross-site\r\n",
            ))
            .await?;
        assert!(fetched.starts_with("HTTP/1.1 403"), "{fetched}");
        let form = neo.raw(&post("Content-Type: text/plain\r\n")).await?;
        assert!(
            form.starts_with("HTTP/1.1 415"),
            "a form post is not JSON: {form}"
        );
        let preflight = neo
            .raw("OPTIONS /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .await?;
        assert!(preflight.starts_with("HTTP/1.1 204"), "{preflight}");
        Ok(())
    })
}

#[test]
fn ready_waits_for_the_browser_and_health_reports_it() -> TestResult {
    on_neo_runtime(async {
        let fake = FakeCdp::new();
        fake.set_reachable(false);
        let neo = Neo::start_with(fake).await?;
        let starting = neo.get("/system/ready").await?;
        assert!(
            starting.starts_with("HTTP/1.1 503"),
            "no runs before the browser: {starting}"
        );
        let health = neo.get("/system/health").await?;
        assert!(
            health.starts_with("HTTP/1.1 200"),
            "health never waits on the browser: {health}"
        );
        assert!(
            health.contains(r#""status":"ok""#) && health.contains(r#""connected":false"#),
            "{health}"
        );

        neo.fake.set_reachable(true);
        neo.until_ready().await?;
        let health = neo.get("/system/health").await?;
        assert!(
            health.contains(r#""connected":true"#) && health.contains(r#""epoch":1"#),
            "{health}"
        );
        Ok(())
    })
}

/// Replacing an unreadable trace must not block capture operations on tabs that outlived it.
#[test]
fn screenshots_and_pdfs_work_after_a_trace_store_is_replaced() -> TestResult {
    on_neo_runtime(async {
        let dir = tempfile::tempdir()?;
        let trace_dir = dir.path().join("claw-sdk");
        fs::create_dir_all(&trace_dir)?;
        fs::write(trace_dir.join("trace.db"), "not a trace store")?;
        let fake = FakeCdp::new();
        let tab = fake.open_tab("https://bank.example/", "Bank");
        fake.answer(
            "Page.getFrameTree",
            json!({ "frameTree": { "frame": {
            "id": format!("target-{}", tab.0), "loaderId": "bank-loader",
            "url": "https://bank.example/",
        } } }),
        );
        fake.answer("Page.captureScreenshot", json!({ "data": "cGl4ZWxz" }));
        fake.answer("Page.printToPDF", json!({ "data": "JVBERi0xLjQ=" }));
        let neo = Neo::start_in(dir, fake).await?;
        neo.until_ready().await?;
        let client = neo.legacy_client().await?;
        let result = run(
            &client,
            json!({ "code": r"
                const tab = context.pages().find(p => p.url() === 'https://bank.example/');
                return [(await tab.screenshot()).length, (await tab.pdf()).length];
            " }),
            None,
        )
        .await?;
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|reply| reply.get("value")),
            Some(&json!([6, 8])),
            "both capture ops stay served after trace replacement: {result:?}"
        );
        client.cancel().await?;
        Ok(())
    })
}
