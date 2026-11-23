//! The whole app, observed through both telemetry doors at once: Neo started from its config the
//! way `main` starts it (telemetry from the `OTEL_*` variables, the host API from its token file),
//! on the fake browser, driven over MCP like an agent drives it. What the mock collector decodes
//! and what `GET /v1/metrics` serves must tell the same story: the startup config, the run and op
//! hooks, the caller's trace context, the scrape route and the push all working together.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue};
use neo_telemetry::TelemetryConfig;
use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, ProtocolVersion,
    RequestMetaObject,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};
use tokio::time;

use super::{ExportedSpan, MockCollector, Scrape, Wire};
use crate::app::App;
use crate::run::RUN_META_KEY;
use crate::test_rig::{TestError, on_neo_runtime, within};
use crate::test_support::{FakeCdp, test_config};

/// The host's bearer token: at least 32 bytes of visible ASCII, as neo-http requires.
const TOKEN: &str = "telemetry-rig-token-0123456789abcdef";
/// A caller's trace, sent as the request's HTTP header.
const HEADER: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
/// Another caller's trace, sent in `_meta`.
const META: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
/// Generous for a debug build on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(20);

/// One HTTP exchange's status line, headers and body.
struct Reply {
    head: String,
    body: String,
}

impl Reply {
    fn status(&self) -> Option<&str> {
        self.head.split(' ').nth(1)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

/// Neo serving on a port the OS picked, its host API on a socket in its state directory.
struct Neo {
    agent: String,
    host_socket: PathBuf,
    _dir: TempDir,
}

impl Neo {
    /// Started with `env` as its telemetry environment, read as `EnvConfig` reads it, and its
    /// host API on; returns once both listeners answer.
    async fn start(env: &[(&'static str, String)]) -> Result<Self, TestError> {
        Self::start_on(env, FakeCdp::new()).await
    }

    /// [`Neo::start`] on `fake`, a browser the test set up first (a launch tab, say).
    async fn start_on(
        env: &[(&'static str, String)],
        fake: Arc<FakeCdp>,
    ) -> Result<Self, TestError> {
        let dir = tempfile::tempdir()?;
        let token_file = dir.path().join("host-token");
        fs::write(&token_file, TOKEN)?;
        let mut config = test_config(dir.path());
        config.env.telemetry = TelemetryConfig::from_env(&|name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        })?;
        config.env.host_api.token_file = Some(token_file);
        let host_socket = config.env.host_api.socket.clone();
        let app = App::start_with_fake_browser(config, fake).await?;
        let agent = app.local_addr()?.to_string();
        tokio::spawn(app.serve());
        let neo = Self {
            agent,
            host_socket,
            _dir: dir,
        };
        within(PATIENCE, async {
            while !neo.agent_ready().await || !neo.host_ready().await {
                time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await?;
        Ok(neo)
    }

    async fn agent_ready(&self) -> bool {
        let Ok(mut stream) = TcpStream::connect(&self.agent).await else {
            return false;
        };
        exchange(
            &mut stream,
            "GET /system/ready HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        )
        .await
        .is_ok_and(|reply| reply.status() == Some("200"))
    }

    async fn host_ready(&self) -> bool {
        self.metrics(Some(TOKEN))
            .await
            .is_ok_and(|reply| reply.status() == Some("200"))
    }

    /// `GET /v1/metrics` on the host socket, with `token` as the bearer when given.
    async fn metrics(&self, token: Option<&str>) -> Result<Reply, TestError> {
        let auth = token.map_or_else(String::new, |token| {
            format!("Authorization: Bearer {token}\r\n")
        });
        let mut stream = UnixStream::connect(&self.host_socket).await?;
        exchange(
            &mut stream,
            &format!(
                "GET /v1/metrics HTTP/1.1\r\nHost: localhost\r\n{auth}Connection: close\r\n\r\n"
            ),
        )
        .await
    }

    /// `run { code }` as a legacy (2025-11-25) MCP client sends it, with these HTTP headers and,
    /// when given, this `_meta["com.browseros.neo/run"]`.
    async fn run(
        &self,
        code: &str,
        headers: &[(&'static str, &str)],
        meta: Option<Value>,
    ) -> Result<CallToolResult, TestError> {
        let mut custom = HashMap::new();
        for (name, value) in headers {
            custom.insert(HeaderName::from_static(name), HeaderValue::from_str(value)?);
        }
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", self.agent))
                .custom_headers(custom),
        );
        let client = info.serve(transport).await?;
        let Value::Object(arguments) = json!({ "code": code }) else {
            return Err("arguments are an object".into());
        };
        let mut params = CallToolRequestParams::new("run").with_arguments(arguments);
        if let Some(meta) = meta {
            let mut object = MetaObject::new();
            object.insert(RUN_META_KEY.to_owned(), meta);
            params.meta = Some(RequestMetaObject(object));
        }
        let result = client.call_tool(params).await?;
        client.cancel().await?;
        Ok(result)
    }
}

/// Writes `request` and reads the reply to the end (`Connection: close`).
async fn exchange<S>(stream: &mut S, request: &str) -> Result<Reply, TestError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| format!("no HTTP head in {response:?}"))?;
    Ok(Reply {
        head: head.to_owned(),
        body: body.to_owned(),
    })
}

/// A result's `_meta["com.browseros.neo/run"]` at `pointer`.
fn outcome<'a>(result: &'a CallToolResult, pointer: &str) -> Option<&'a Value> {
    result.meta.as_ref()?.get(RUN_META_KEY)?.pointer(pointer)
}

/// The `neo.run` span of the run a result answered.
fn run_span<'a>(
    spans: &'a [ExportedSpan],
    result: &CallToolResult,
) -> Result<&'a ExportedSpan, TestError> {
    let id = outcome(result, "/id")
        .and_then(Value::as_str)
        .ok_or("the result names its run")?;
    spans
        .iter()
        .find(|span| span.name == "neo.run" && span.text("neo.run.id") == Some(id))
        .ok_or_else(|| format!("no neo.run span for run {id}").into())
}

/// The trace id and span id of a W3C `traceparent`.
fn ids(traceparent: &str) -> (&str, &str) {
    let mut parts = traceparent.split('-').skip(1);
    (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    )
}

/// Two runs over MCP, one ok with an unregistered op and the caller's trace in its HTTP headers,
/// one that throws with the caller's trace in `_meta`: the collector gets both `neo.run` spans,
/// each a child of its caller's span, with the op a child of its run under its raw name; the
/// authenticated scrape counts both runs, the op as `unknown` and each error once, with nothing
/// left in flight; and the push carries the metrics too, under OpenTelemetry's names.
async fn both_doors_tell_the_same_story(wire: Wire) -> Result<(), TestError> {
    let collector = MockCollector::start(wire)?;
    let neo = Neo::start(&collector.env(100)).await?;

    let refused = neo.metrics(None).await?;
    assert_eq!(
        refused.status(),
        Some("401"),
        "{wire:?}: the scrape needs the host's token"
    );

    let ok = neo
        .run(
            "try { await __neo.invoke('no.such.method', {}); } catch (error) {} return 1;",
            &[("traceparent", HEADER), ("tracestate", "edge=1")],
            None,
        )
        .await?;
    assert_eq!(
        outcome(&ok, "/status"),
        Some(&json!("ok")),
        "{wire:?}: {ok:?}"
    );
    let failed = neo
        .run(
            "throw new Error('boom');",
            &[],
            Some(json!({ "traceparent": META })),
        )
        .await?;
    assert_eq!(
        outcome(&failed, "/status"),
        Some(&json!("error")),
        "{wire:?}: {failed:?}"
    );

    assert_exported(wire, &collector, &ok, &failed).await?;
    assert_scraped(wire, &neo).await?;
    within(PATIENCE, async {
        while !collector
            .metric_names()
            .is_ok_and(|names| names.iter().any(|name| name == "neo.runs"))
        {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|error| format!("{wire:?}: the push carries the metrics: {error}"))?;
    Ok(())
}

/// What the collector decoded of the two runs [`both_doors_tell_the_same_story`] drove.
async fn assert_exported(
    wire: Wire,
    collector: &MockCollector,
    ok: &CallToolResult,
    failed: &CallToolResult,
) -> Result<(), TestError> {
    let spans = collector
        .spans_until(PATIENCE, |spans| {
            spans.iter().filter(|span| span.name == "neo.run").count() >= 2
                && spans
                    .iter()
                    .any(|span| span.text("neo.span.type") == Some("op"))
        })
        .await?;
    let ok_run = run_span(&spans, ok)?;
    assert_eq!(
        (
            ok_run.trace_id.as_str(),
            ok_run.parent_span_id.as_deref(),
            ok_run.trace_state.as_str()
        ),
        (ids(HEADER).0, Some(ids(HEADER).1), "edge=1"),
        "{wire:?}: the header's trace is the run span's parent"
    );
    let failed_run = run_span(&spans, failed)?;
    assert_eq!(
        (
            failed_run.trace_id.as_str(),
            failed_run.parent_span_id.as_deref()
        ),
        (ids(META).0, Some(ids(META).1)),
        "{wire:?}: with no header, _meta's trace is the run span's parent"
    );
    assert_eq!(
        (
            failed_run.text("neo.run.status"),
            failed_run.text("neo.error.owner"),
            failed_run.text("neo.error.code")
        ),
        (Some("error"), Some("script"), Some("script_error")),
        "{wire:?}: the failed run's span carries its error"
    );
    let ops: Vec<&ExportedSpan> = spans
        .iter()
        .filter(|span| {
            span.text("neo.span.type") == Some("op")
                && span.parent_span_id.as_ref() == Some(&ok_run.span_id)
        })
        .collect();
    let [op] = ops.as_slice() else {
        return Err(format!("{wire:?}: one op span under the ok run: {spans:#?}").into());
    };
    assert_eq!(
        (op.trace_id.as_str(), op.text("neo.op.name")),
        (ids(HEADER).0, Some("no.such.method")),
        "{wire:?}: the op is a child of its run, named as the script called it"
    );

    Ok(())
}

/// Label pairs of one series.
type Labels<'a> = &'a [(&'a str, &'a str)];

/// What the authenticated scrape serves after the two runs [`both_doors_tell_the_same_story`]
/// drove: Prometheus text, both runs, the op as `unknown`, each error once, nothing in flight.
async fn assert_scraped(wire: Wire, neo: &Neo) -> Result<(), TestError> {
    let reply = neo.metrics(Some(TOKEN)).await?;
    assert_eq!(
        (reply.status(), reply.header("content-type")),
        (
            Some("200"),
            Some("text/plain; version=0.0.4; charset=utf-8")
        ),
        "{wire:?}: the host's scrape is Prometheus text"
    );
    let scrape = Scrape::parse(&reply.body)?;
    let expected: [(&str, Labels<'_>, u64); 9] = [
        (
            "neo_runs_total",
            &[("neo_run_status", "ok"), ("neo_error_owner", "none")],
            1,
        ),
        (
            "neo_runs_total",
            &[("neo_run_status", "error"), ("neo_error_owner", "script")],
            1,
        ),
        (
            "neo_run_duration_seconds_count",
            &[("neo_run_status", "ok")],
            1,
        ),
        (
            "neo_run_duration_seconds_count",
            &[("neo_run_status", "error")],
            1,
        ),
        (
            "neo_operations_total",
            &[
                ("neo_op_name", "unknown"),
                ("neo_op_outcome", "error"),
                ("neo_error_owner", "script"),
            ],
            1,
        ),
        (
            "neo_operation_duration_seconds_count",
            &[("neo_op_name", "unknown"), ("neo_op_outcome", "error")],
            1,
        ),
        (
            "neo_errors_total",
            &[
                ("neo_error_owner", "script"),
                ("error_type", "method_unavailable"),
            ],
            1,
        ),
        (
            "neo_errors_total",
            &[
                ("neo_error_owner", "script"),
                ("error_type", "script_error"),
            ],
            1,
        ),
        ("neo_run_active", &[], 0),
    ];
    for (name, labels, value) in expected {
        assert_eq!(
            scrape.whole(name, labels),
            Some(value),
            "{wire:?}: {name}{labels:?} in the scrape"
        );
    }

    Ok(())
}

#[test]
fn both_doors_tell_the_same_story_over_protobuf_and_tcp() -> Result<(), TestError> {
    on_neo_runtime(both_doors_tell_the_same_story(Wire::ProtobufTcp))
}

#[test]
fn both_doors_tell_the_same_story_over_protobuf_and_a_unix_socket() -> Result<(), TestError> {
    on_neo_runtime(both_doors_tell_the_same_story(Wire::ProtobufUnix))
}

#[test]
fn both_doors_tell_the_same_story_over_json() -> Result<(), TestError> {
    on_neo_runtime(both_doors_tell_the_same_story(Wire::JsonTcp))
}

/// On the whole app: the launch tab of a fresh browser is the blank
/// tab Neo adopts at attach, and the scrape counts it born `neo`, not `found`, since the app takes
/// Neo's open intent before the link's first listing.
async fn the_launch_tab_neo_adopts_is_counted_neo() -> Result<(), TestError> {
    let fake = FakeCdp::new();
    fake.open_tab("about:blank", "");
    let neo = Neo::start_on(&[], Arc::clone(&fake)).await?;
    let opened = || async {
        let reply = neo.metrics(Some(TOKEN)).await?;
        Ok::<Vec<String>, TestError>(
            Scrape::parse(&reply.body)?.lines_of(&["neo_tab_opened_total", "neo_tab_count"]),
        )
    };
    // Sorted as `lines_of` sorts them.
    let expected = [
        r#"neo_tab_count{neo_tab_origin="neo"} 1"#.to_owned(),
        r#"neo_tab_opened_total{neo_tab_origin="neo"} 1"#.to_owned(),
    ];
    within(PATIENCE, async {
        while opened().await.ok().as_deref() != Some(&expected[..]) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert_eq!(
        fake.count("Browser.createTab"),
        0,
        "the launch tab was adopted, not a new one opened"
    );
    Ok(())
}

#[test]
fn the_launch_tab_neo_adopts_at_attach_is_counted_neo() -> Result<(), TestError> {
    on_neo_runtime(the_launch_tab_neo_adopts_is_counted_neo())
}
