//! Read-only acceptance against the private browser supplied by `bun run test:browser`.
//! The launcher owns its browser; this test starts only a temporary SDK app and fixture server.

use std::env;
use std::error::Error;
use std::path::Path;
use std::time::Duration;

use axum::response::Html;
use axum::{Router, routing};
use neo_trace::{Delivery, RunAccess, RunDetail, RunId, TraceStore};
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, RequestMetaObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time;

use crate::App;
use crate::run::RUN_META_KEY;
use crate::test_support::test_config;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Client = RunningService<RoleClient, ClientInfo>;

const FIXTURE: &str = r"<!doctype html><title>Read boundary</title><h1>Read boundary</h1>
<p>Visible text</p><input id=field value=untouched><button>Go</button><input type=file>
<script>
window.effects = { click: 0, input: 0, keydown: 0, custom: 0, code: 0 };
for (const type of ['click', 'input', 'keydown', 'custom'])
  document.addEventListener(type, () => window.effects[type]++);
</script>";

#[test]
#[ignore = "requires bun run test:browser to supply a private BROWSEROS_CDP_PORT"]
fn read_only_live() -> TestResult {
    let port = env::var("BROWSEROS_CDP_PORT")?.parse()?;
    App::runtime()?.block_on(async { time::timeout(Duration::from_secs(180), drive(port)).await? })
}

async fn drive(cdp_port: u16) -> TestResult {
    let fixture = TcpListener::bind(("127.0.0.1", 0)).await?;
    let url = format!("http://{}/", fixture.local_addr()?);
    let fixture_task = tokio::spawn(async move {
        axum::serve(
            fixture,
            Router::new().route("/", routing::get(|| async { Html(FIXTURE) })),
        )
        .await
    });
    let directory = tempfile::tempdir()?;
    let mut config = test_config(directory.path());
    config.launch.cdp_port = cdp_port;
    let app = App::start(config).await?;
    let address = app.local_addr()?;
    let store = app.trace_store();
    let app_task = tokio::spawn(app.serve());
    time::timeout(Duration::from_secs(60), async {
        loop {
            let mut socket = TcpStream::connect(address).await?;
            socket
                .write_all(
                    b"GET /system/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await?;
            let mut reply = String::new();
            socket.read_to_string(&mut reply).await?;
            if reply.starts_with("HTTP/1.1 200") {
                return Ok::<_, Box<dyn Error>>(());
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await??;
    let client = ClientInfo::default()
        .serve(StreamableHttpClientTransport::from_uri(format!(
            "http://{address}/mcp"
        )))
        .await?;
    let initialized = run(
        &client,
        &format!("await page.goto({url:?}); return await page.title();"),
        None,
    )
    .await?;
    assert_eq!(value(&initialized)?, "Read boundary");
    read_buffers_and_snapshots(&client).await?;
    refuse_page_changes(&client, &store).await?;
    refuse_file_paths(&client, directory.path(), &store).await?;
    let cleanup = run(&client, "await page.close(); return true;", None).await?;
    assert_eq!(value(&cleanup)?, true);
    client.cancel().await?;
    app_task.abort();
    fixture_task.abort();
    Ok(())
}

async fn read_buffers_and_snapshots(client: &Client) -> TestResult {
    let reads = run(
        client,
        r"
        await page.getByText('Visible text', { exact: true }).waitFor({ state: 'visible' });
        const snapshot = await page.ariaSnapshot({ text: true });
        const full = await page.ariaSnapshot({ mode: 'full' });
        const screenshot = await page.screenshot();
        const pdf = await page.pdf();
        return { snapshot, full, screenshot: Buffer.isBuffer(screenshot) && screenshot.length > 0,
            pdf: Buffer.isBuffer(pdf) && pdf.length > 0 };
    ",
        Some(RunAccess::Read),
    )
    .await?;
    let read_value = value(&reads)?;
    assert!(
        read_value["snapshot"]
            .as_str()
            .is_some_and(|text| text.contains("Visible text"))
    );
    assert!(
        read_value["full"]
            .as_str()
            .is_some_and(|text| text.contains("Visible text"))
    );
    assert_eq!(read_value["screenshot"], true);
    assert_eq!(read_value["pdf"], true);
    assert_eq!(outcome(&reads)?["access"], "read");
    let delta = run(
        client,
        "return await page.ariaSnapshot({ text: true, delta: true });",
        Some(RunAccess::Read),
    )
    .await?;
    assert!(value(&delta)?.is_string(), "snapshot delta stays admitted");

    Ok(())
}

async fn refuse_page_changes(client: &Client, store: &TraceStore) -> TestResult {
    let blocked = run(client, r"
        let chooserCalls = 0;
        page.on('filechooser', () => chooserCalls++);
        const failures = [];
        // Reload comes first so a broken gate cannot erase later input effects.
        for (const action of [() => page.reload(), () => page.click('button'), () => page.fill('#field', 'changed'),
            () => page.press('#field', 'A'), () => page.mouse.click(20, 20),
            () => page.evaluate(() => { window.effects.code++; }),
            () => page.dispatchEvent('button', 'custom'),
            () => page.waitForEvent('filechooser'), () => page.setInputFiles('input[type=file]', []),
            () => page.goto('data:text/html,x'), () => page.goto('about:blank')]) {
            try { await action(); failures.push('allowed'); }
            catch (e) { failures.push([e.owner, e.code, e.delivery]); }
        }
        await page.title();
        return { failures, chooserCalls };
    ", Some(RunAccess::Read)).await?;
    let blocked_value = value(&blocked)?;
    assert_eq!(
        blocked_value["failures"],
        json!(vec![json!(["script", "read_only_run", "not_sent"]); 11])
    );
    assert_eq!(
        blocked_value["chooserCalls"], 0,
        "failed asynchronous listener setup does not fire a callback"
    );
    let inspected = run(client, "return await page.evaluate(() => ({ effects: window.effects, input: document.querySelector('#field').value }));", None).await?;
    assert_eq!(
        value(&inspected)?,
        &json!({"effects":{"click":0,"input":0,"keydown":0,"custom":0,"code":0},"input":"untouched"}),
        "page listeners observe zero effects from refused operations"
    );

    let id = outcome(&blocked)?["id"].as_str().ok_or("run id")?.parse()?;
    let record = recorded(store, id).await?;
    assert!(record.operations.iter().any(|op| {
        op.name == "events.interceptFileChooser"
            && op.delivery == Some(Delivery::NotSent)
            && op
                .error
                .as_ref()
                .is_some_and(|error| error.code == "read_only_run")
    }));
    assert!(
        !record.operations.iter().any(|op| op.name == "events.listen"
            && op
                .args
                .as_ref()
                .and_then(|args| args.get("event"))
                .and_then(Value::as_str)
                == Some("fileChooser")),
        "quiet listener setup failure must not install the file chooser listener"
    );
    Ok(())
}

async fn refuse_file_paths(client: &Client, directory: &Path, store: &TraceStore) -> TestResult {
    let image = directory.join("blocked.png");
    let screenshot = run(
        client,
        &format!(
            "await page.screenshot({{path:{:?}}});",
            image.to_string_lossy()
        ),
        Some(RunAccess::Read),
    )
    .await?;
    assert_eq!(outcome(&screenshot)?["error"]["code"], "read_only_run");
    assert_eq!(outcome(&screenshot)?["error"]["delivery"], "not_sent");
    assert!(
        !image.exists(),
        "capture may happen first, but the path write is never sent"
    );
    let id: RunId = outcome(&screenshot)?["id"]
        .as_str()
        .ok_or("run id")?
        .parse()?;
    let recorded = recorded(store, id).await?;
    assert!(
        recorded
            .operations
            .iter()
            .any(|op| op.name == "page.screenshot" && op.error.is_none())
    );
    assert!(
        recorded
            .operations
            .iter()
            .any(|op| op.name == "page.saveScreenshot" && op.delivery == Some(Delivery::NotSent))
    );
    assert_eq!(recorded.summary.access, Some(RunAccess::Read));
    let pdf_path = directory.join("blocked.pdf");
    let pdf_code = format!("await page.pdf({{path:{:?}}});", pdf_path.to_string_lossy());
    for access in [None, Some(RunAccess::Read)] {
        let pdf = run(client, &pdf_code, access).await?;
        assert_eq!(
            outcome(&pdf)?["error"]["code"],
            "method_unavailable",
            "PDF path keeps its prelude refusal"
        );
    }
    assert!(!pdf_path.exists());
    Ok(())
}

async fn recorded(store: &TraceStore, id: RunId) -> TestResult<RunDetail> {
    time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(recorded) = store.run(id).await?
                && recorded.summary.complete
            {
                return Ok::<_, Box<dyn Error>>(recorded);
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

async fn run(client: &Client, code: &str, access: Option<RunAccess>) -> TestResult<CallToolResult> {
    let mut metadata = json!({"taskId":"read-boundary"});
    if let Some(access) = access {
        metadata["access"] = json!(access);
    }
    let mut meta = MetaObject::new();
    meta.insert(RUN_META_KEY.to_owned(), metadata);
    let mut request = CallToolRequestParams::new("run").with_arguments(
        json!({"code":code, "timeout":30000})
            .as_object()
            .ok_or("arguments")?
            .clone(),
    );
    request.meta = Some(RequestMetaObject(meta));
    Ok(client.call_tool(request).await?)
}

fn outcome(result: &CallToolResult) -> TestResult<&Value> {
    result
        .meta
        .as_ref()
        .and_then(|meta| meta.get(RUN_META_KEY))
        .ok_or_else(|| "run metadata".into())
}

fn value(result: &CallToolResult) -> TestResult<&Value> {
    assert_eq!(outcome(result)?["status"], "ok", "{result:?}");
    result
        .structured_content
        .as_ref()
        .and_then(|content| content.get("value"))
        .ok_or_else(|| "script value".into())
}
