//! Page-result acceptance through the real MCP server and QuickJS bridge. `test:browser` owns
//! the private browser; this test owns only its temporary SDK app and task's about:blank page.

use std::error::Error;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, fs};

use neo_trace::{Outcome, RunId, TraceStore};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, RequestMetaObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time;

use crate::App;
use crate::run::RUN_META_KEY;
use crate::test_support::test_config;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Client = RunningService<RoleClient, ClientInfo>;

#[test]
#[ignore = "requires bun run test:browser to supply a private BROWSEROS_CDP_PORT"]
fn surrogate_page_values_live() -> TestResult {
    let port = env::var("BROWSEROS_CDP_PORT")?.parse()?;
    App::runtime()?.block_on(drive(port))
}

async fn drive(cdp_port: u16) -> TestResult {
    use rmcp::ServiceExt;

    // Create lone units inside the page, so serializing the function's source cannot fail at
    // the earlier QuickJS input boundary and disguise which reader rejects the page's reply.
    let cases = [
        (
            "raw-high",
            r"return await page.evaluate(() => 'x' + String.fromCharCode(0xd83d));",
            json!("x�"),
        ),
        (
            "raw-low",
            r"return await page.evaluate(() => String.fromCharCode(0xde00) + 'y');",
            json!("�y"),
        ),
        (
            "nested",
            r"return await page.evaluate(() => ({
                nested: ['x' + String.fromCharCode(0xd83d), String.fromCharCode(0xde00) + 'y'],
                ['key' + String.fromCharCode(0xd83d)]: { low: String.fromCharCode(0xdc00) }
            }));",
            json!({"nested": ["x�", "�y"], "key�": {"low": "�"}}),
        ),
        (
            "slice-3000",
            r"return await page.evaluate(() => {
            document.body.textContent = 'x'.repeat(2999) + '\ud83d\ude00tail';
            return document.body.innerText.slice(0, 3000);
        });",
            json!(format!("{}�", "x".repeat(2999))),
        ),
        (
            "preserved",
            r"return await page.evaluate(() => ({
            text: 'plain é 中 😀 𐀀 \ud7ff\ue000\ufffd',
            literal: '\\ud83d|\\ude00|\\\\ud83d', controls: '\u0000\t\n'
        }));",
            json!({"text": "plain é 中 😀 𐀀 \u{d7ff}\u{e000}�",
                "literal": r"\ud83d|\ude00|\\ud83d", "controls": "\0\t\n"}),
        ),
    ];
    let directory = tempfile::tempdir()?;
    let mut config = test_config(directory.path());
    config.launch.cdp_port = cdp_port;
    let app = App::start(config).await?;
    let address = app.local_addr()?;
    let store = app.trace_store();
    let app_task = tokio::spawn(app.serve());
    let measured = async {
        ready(address).await?;
        let client = ClientInfo::default()
            .serve(StreamableHttpClientTransport::from_uri(format!(
                "http://{address}/mcp"
            )))
            .await?;
        let mut observations = Vec::new();
        // Do not assert or propagate a case's error: a red baseline must retain every MCP
        // reply, including the preservation control and any transport/trace failure.
        for (name, code, _) in &cases {
            observations.push(observe(&client, &store, name, code).await);
        }
        let cleanup = observe(
            &client,
            &store,
            "cleanup",
            "await page.close(); return true;",
        )
        .await;
        let cancellation = client.cancel().await.err().map(|error| error.to_string());
        Ok::<_, Box<dyn Error>>(json!({"cases": observations, "cleanup": cleanup,
            "cancellationError": cancellation}))
    }
    .await;
    // Request app cancellation before inspecting results, including failed value assertions.
    // The runtime owns task teardown; the outer launcher owns its browser process and profile.
    app_task.abort();
    let evidence = measured?;
    if let Some(path) = env::var_os("NEO_SURROGATE_EVIDENCE") {
        fs::write(&path, serde_json::to_vec_pretty(&evidence)?)?;
    }
    // Expectations always describe the fixed contract. Baseline classification belongs to the
    // remote job, which reads the same evidence instead of changing this test's expectations.
    for ((name, _, expected), observed) in cases
        .iter()
        .zip(evidence["cases"].as_array().ok_or("case evidence")?)
    {
        assert_eq!(observed["outcome"]["status"], "ok", "{name}: {observed}");
        assert_eq!(&observed["value"], expected, "{name}: {observed}");
        assert!(observed.get("traceError").is_none(), "{name}: {observed}");
    }
    assert_eq!(evidence["cleanup"]["outcome"]["status"], "ok", "{evidence}");
    assert_eq!(evidence["cleanup"]["value"], true, "{evidence}");
    assert!(
        evidence["cleanup"].get("traceError").is_none(),
        "{evidence}"
    );
    assert!(evidence["cancellationError"].is_null(), "{evidence}");
    Ok(())
}

async fn ready(address: SocketAddr) -> TestResult {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    .await?
}

async fn observe(client: &Client, store: &TraceStore, name: &str, code: &str) -> Value {
    match time::timeout(Duration::from_secs(40), run(client, code)).await {
        Ok(Ok(reply)) => {
            let trace = trace(store, &reply).await;
            let mut observed = json!({"name": name, "code": code,
                "outcome": reply.meta.as_ref().and_then(|meta| meta.get(RUN_META_KEY)),
                "value": reply.structured_content.as_ref().and_then(|content| content.get("value")),
                "reply": reply});
            match trace {
                Ok(trace) => observed["trace"] = trace,
                Err(error) => observed["traceError"] = json!(error.to_string()),
            }
            observed
        }
        Ok(Err(error)) => json!({"name": name, "code": code, "transportError": error.to_string()}),
        Err(error) => json!({"name": name, "code": code, "transportError": error.to_string()}),
    }
}

async fn trace(store: &TraceStore, reply: &CallToolResult) -> TestResult<Value> {
    let id: RunId = reply
        .meta
        .as_ref()
        .and_then(|meta| meta.get(RUN_META_KEY))
        .and_then(|outcome| outcome.get("id"))
        .and_then(Value::as_str)
        .ok_or("missing run id")?
        .parse()?;
    time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(record) = store.run(id).await?
                && record.summary.complete
            {
                let operations: Vec<_> = record
                    .operations
                    .iter()
                    .map(|op| {
                        json!({"name": op.name, "delivery": op.delivery, "error": op.error,
                        "outcome": op.outcome.map(Outcome::as_str),
                        "mainWorld": op.main_world})
                    })
                    .collect();
                return Ok::<_, Box<dyn Error>>(json!({"complete": true,
                    "error": record.summary.error, "operations": operations}));
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?
}

async fn run(client: &Client, code: &str) -> TestResult<CallToolResult> {
    let mut meta = MetaObject::new();
    meta.insert(
        RUN_META_KEY.to_owned(),
        json!({"taskId": "surrogate-page-values"}),
    );
    let mut request = CallToolRequestParams::new("run").with_arguments(
        json!({"code": code, "timeout": 30000})
            .as_object()
            .ok_or("arguments")?
            .clone(),
    );
    request.meta = Some(RequestMetaObject(meta));
    Ok(client.call_tool(request).await?)
}
