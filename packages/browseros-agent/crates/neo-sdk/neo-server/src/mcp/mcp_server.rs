//! rmcp's `ServerHandler`: the one tool, `run`.

use std::borrow::Cow;
use std::future::{self, Future};
use std::sync::Arc;

use axum::http::HeaderMap;
use axum::http::request::Parts;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, JsonObject, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerInfo, SubscriptionFilter, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler};
use serde_json::Value;

use super::{RunArgs, RunTool};
use crate::dispatch::{OperationError, Registry};
use crate::run::{RUN_META_KEY, RunMeta, RunRequest, RunService, RunTimeout};

/// The server's name in `initialize` and `server/discover`.
const SERVER_NAME: &str = "browseros-claw-sdk";

/// The stateless 2026-07-28 revision first, then the legacy ones. rmcp picks per request from
/// what a client negotiates, and serves all of them without a session.
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2024_11_05,
];

/// The handler. Stateless rmcp builds one per request (the service factory runs each time), so
/// it is two `Arc`s, and the tool's definition is built once, at startup.
#[derive(Clone)]
pub(crate) struct McpServer {
    runs: Arc<RunService>,
    tool: Arc<RunTool>,
}

/// A call refused at the boundary, with what is known of it, so the refusal is recorded.
struct Refused {
    code: String,
    meta: RunMeta,
    error: OperationError,
}

/// The W3C trace context a call's HTTP request carried (`traceparent`, `tracestate`), which wins
/// over `_meta`'s (`RunMeta::parse`). rmcp's streamable HTTP service hands each request's
/// `http::request::Parts` to the handler in its context's extensions.
#[derive(Debug, PartialEq, Eq)]
struct TraceHeaders {
    traceparent: String,
    tracestate: Option<String>,
}

impl TraceHeaders {
    /// `None` unless exactly one `traceparent` arrived as visible ASCII: W3C reads a repeated one
    /// as invalid, so the run falls back to `_meta`'s, else starts its own trace. A `tracestate`
    /// split over several lines is joined with commas, as HTTP combines a list header; one that
    /// is not visible ASCII is dropped and its `traceparent` kept, as W3C asks.
    fn read(headers: &HeaderMap) -> Option<Self> {
        let mut traceparents = headers.get_all("traceparent").iter();
        let traceparent = traceparents.next()?.to_str().ok()?;
        if traceparents.next().is_some() {
            return None;
        }
        let states: Option<Vec<&str>> = headers
            .get_all("tracestate")
            .iter()
            .map(|state| state.to_str().ok())
            .collect();
        Some(Self {
            traceparent: traceparent.to_owned(),
            tracestate: states
                .filter(|states| !states.is_empty())
                .map(|states| states.join(",")),
        })
    }

    /// The pair as `RunMeta::parse` takes it.
    fn pair(&self) -> (&str, Option<&str>) {
        (&self.traceparent, self.tracestate.as_deref())
    }
}

impl McpServer {
    /// The server, with `run`'s description built from the frozen registry.
    #[must_use]
    pub(crate) fn new(runs: Arc<RunService>, registry: &Registry) -> Self {
        Self {
            runs,
            tool: Arc::new(RunTool::new(registry)),
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        let result = ListToolsResult::with_all_items(vec![self.tool.tool()]);
        // 2026-07-28 clients require a cache hint on list results (SEP-2549); older peers keep
        // the old wire shape. The one tool is the same for every caller.
        let modern = context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28);
        future::ready(Ok(if modern {
            result.with_ttl_ms(0).with_cache_scope(CacheScope::Public)
        } else {
            result
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        // rmcp validates 2026-07-28's per-parameter headers against this schema.
        (name == RunTool::NAME).then(|| self.tool.tool())
    }

    // A 2026-07-28 client may open `subscriptions/listen`; rmcp's default refuses it with a
    // 404 that ends the transport before `tools/list` runs. Accept what the capabilities
    // offer (nothing but tools), so the stream opens and stays quiet.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.supported_by(&self.get_info().capabilities))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != RunTool::NAME {
            return Err(ErrorData::invalid_params(
                format!(
                    "unknown tool `{}`; claw-sdk serves one tool, `run`",
                    request.name
                ),
                None,
            ));
        }
        // `_meta` arrives on the context: rmcp moves it out of the request params. So do the
        // HTTP request's parts, whose trace headers link the run to its caller's trace; a
        // transport without HTTP has none.
        let headers = context
            .extensions
            .get::<Parts>()
            .and_then(|parts| TraceHeaders::read(&parts.headers));
        let meta = context.meta.get(RUN_META_KEY);
        let reply = match parse(request.arguments, meta, headers.as_ref()) {
            Ok(run) => tokio::select! {
                reply = self.runs.run(run) => reply,
                // The client's response stream is gone, which is the only cancel stateless rmcp
                // delivers (it drops `notifications/cancelled`). Dropping the run's future is
                // leaving it: a run without a requestId stops at once, one with a requestId
                // waits out its orphan grace for a retry. Nobody reads this result.
                () = context.ct.cancelled() => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text("the caller went away")]).into());
                }
            },
            Err(refused) => {
                let Refused { code, meta, error } = *refused;
                self.runs.refuse(code, meta, &error)
            }
        };
        Ok(RunTool::result(&reply).into())
    }
}

/// The call in domain types. The host's `_meta` is read first, so a call refused for its
/// arguments still records the task it came from.
fn parse(
    arguments: Option<JsonObject>,
    meta: Option<&Value>,
    headers: Option<&TraceHeaders>,
) -> Result<RunRequest, Box<Refused>> {
    let meta = match RunMeta::parse(meta, headers.map(TraceHeaders::pair)) {
        Ok(meta) => meta,
        Err(error) => {
            let code = RunArgs::parse(arguments)
                .map(|args| args.code)
                .unwrap_or_default();
            return Err(Box::new(Refused {
                code,
                meta: RunMeta::default(),
                error,
            }));
        }
    };
    let args = match RunArgs::parse(arguments) {
        Ok(args) => args,
        Err(error) => {
            return Err(Box::new(Refused {
                code: String::new(),
                meta,
                error,
            }));
        }
    };
    match RunTimeout::from_millis(args.timeout) {
        Ok(timeout) => Ok(RunRequest {
            code: args.code,
            timeout,
            meta,
        }),
        Err(error) => Err(Box::new(Refused {
            code: args.code,
            meta,
            error,
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::IntoFuture;
    use std::io;
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use axum::http::{HeaderMap, HeaderName, HeaderValue};
    use neo_telemetry::{Probe, Telemetry, TelemetryConfig, TraceContext};
    use rmcp::ServiceExt;
    use rmcp::model::{
        CallToolRequestParams, CallToolResult, ClientInfo, MetaObject, ProtocolVersion,
        RequestMetaObject,
    };
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::{McpServer, TraceHeaders};
    use crate::config::Defaults;
    use crate::http::agent_router;
    use crate::run::{RUN_META_KEY, RunService, RunServiceParts};
    use crate::test_rig::{TestError, TestRig, on_neo_runtime};

    /// Two callers' traces (W3C `traceparent`): one sent as the request's HTTP header, one in
    /// `_meta`.
    const HEADER: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    const META: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

    fn headers(lines: &[(&'static str, &[u8])]) -> Result<HeaderMap, TestError> {
        let mut headers = HeaderMap::new();
        for (name, value) in lines {
            headers.append(*name, HeaderValue::from_bytes(value)?);
        }
        Ok(headers)
    }

    fn pair(traceparent: &str, tracestate: Option<&str>) -> TraceHeaders {
        TraceHeaders {
            traceparent: traceparent.to_owned(),
            tracestate: tracestate.map(str::to_owned),
        }
    }

    #[test]
    fn one_traceparent_is_read_with_its_tracestate_lines_joined() -> Result<(), TestError> {
        assert_eq!(TraceHeaders::read(&headers(&[])?), None, "no header, none");
        assert_eq!(
            TraceHeaders::read(&headers(&[("traceparent", HEADER.as_bytes())])?),
            Some(pair(HEADER, None))
        );
        assert_eq!(
            TraceHeaders::read(&headers(&[
                ("traceparent", HEADER.as_bytes()),
                ("tracestate", b"a=1"),
                ("tracestate", b"b=2"),
            ])?),
            Some(pair(HEADER, Some("a=1,b=2"))),
            "tracestate is a list header, whose lines HTTP joins with commas"
        );
        assert_eq!(
            TraceHeaders::read(&headers(&[("traceparent", b"garbage")])?),
            Some(pair("garbage", None)),
            "a malformed value is still read: deciding validity is TraceContext's alone"
        );
        Ok(())
    }

    #[test]
    fn a_repeated_or_unreadable_traceparent_is_none_and_a_bad_tracestate_is_dropped()
    -> Result<(), TestError> {
        assert_eq!(
            TraceHeaders::read(&headers(&[
                ("traceparent", HEADER.as_bytes()),
                ("traceparent", META.as_bytes()),
            ])?),
            None,
            "W3C reads a repeated traceparent as invalid, so the run falls back to _meta's"
        );
        assert_eq!(
            TraceHeaders::read(&headers(&[("traceparent", b"00-\xff")])?),
            None,
            "a traceparent that is not visible ASCII cannot be one"
        );
        assert_eq!(
            TraceHeaders::read(&headers(&[
                ("traceparent", HEADER.as_bytes()),
                ("tracestate", b"a=1"),
                ("tracestate", b"b=\xff"),
            ])?),
            Some(pair(HEADER, None)),
            "W3C drops a tracestate it cannot read and keeps its traceparent"
        );
        Ok(())
    }

    /// The caller's trace each run's exported `neo.run` span joined, in the order the runs
    /// ended: the caller's context (trace id, parent span id, tracestate) when the span's parent
    /// is remote, else `None` (a trace of the run's own).
    fn parents(probe: &Probe) -> Vec<Option<TraceContext>> {
        probe
            .spans()
            .into_iter()
            .filter(|span| span.name == "neo.run")
            .map(|span| {
                let parent = span.parent_span_id.filter(|_| span.parent_is_remote)?;
                let state = Some(span.trace_state.as_str()).filter(|state| !state.is_empty());
                TraceContext::from_w3c(&format!("00-{}-{parent}-01", span.trace_id), state)
            })
            .collect()
    }

    /// `run { code: "return 1" }` as a legacy (2025-11-25) MCP client sends it, with these HTTP
    /// headers and, when given, this `_meta["com.browseros.neo/run"]`.
    async fn call(
        url: &str,
        lines: &[(&'static str, &str)],
        meta: Option<Value>,
    ) -> Result<CallToolResult, TestError> {
        let mut custom = HashMap::new();
        for (name, value) in lines {
            custom.insert(HeaderName::from_static(name), HeaderValue::from_str(value)?);
        }
        let mut info = ClientInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url.to_owned()).custom_headers(custom),
        );
        let client = info.serve(transport).await?;
        let Value::Object(arguments) = json!({ "code": "return 1" }) else {
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

    /// `_meta["com.browseros.neo/run"]` of a result, at `pointer`.
    fn outcome<'a>(result: &'a CallToolResult, pointer: &str) -> Option<&'a Value> {
        result.meta.as_ref()?.get(RUN_META_KEY)?.pointer(pointer)
    }

    /// Neo's agent router over a run service reporting to the live pipeline, its spans kept in
    /// memory, on a port the OS picked.
    struct Served {
        url: String,
        probe: Probe,
        server: JoinHandle<io::Result<()>>,
        rig: TestRig,
    }

    impl Served {
        async fn start() -> Result<Self, TestError> {
            let rig = TestRig::new().await?;
            let (telemetry, probe) = Telemetry::in_memory(&TelemetryConfig::default());
            let runs = RunService::new(RunServiceParts {
                dispatcher: Arc::clone(&rig.dispatcher),
                browser: Arc::clone(&rig.link),
                tabs: Arc::clone(&rig.tabs),
                store: rig.store.clone(),
                trace: rig.sink.clone(),
                settings: rig.settings.clone(),
                shutdown: rig.shutdown.clone(),
                orphan_grace: Defaults::ORPHAN_GRACE,
                blank_tab: None,
                holders: None,
                telemetry,
            });
            let router = agent_router(
                McpServer::new(runs, rig.dispatcher.registry()),
                Arc::clone(&rig.link),
                None,
            );
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
            let url = format!("http://{}/mcp", listener.local_addr()?);
            let server = tokio::spawn(axum::serve(listener, router).into_future());
            Ok(Self {
                url,
                probe,
                server,
                rig,
            })
        }

        fn stop(&self) {
            self.server.abort();
            self.rig.shutdown.cancel();
        }
    }

    /// One call of [`trace_context_reaches_the_run_service_from_the_header_or_the_meta`]: what
    /// it sends, and the parent its run must start with.
    struct Case {
        headers: &'static [(&'static str, &'static str)],
        meta: Option<Value>,
        parent: Option<TraceContext>,
        why: &'static str,
    }

    /// The caller's trace context crosses the MCP boundary into the run's exported span, over
    /// Neo's real agent router and rmcp's HTTP transport: the header's, else `_meta`'s, else
    /// none, and the run runs either way.
    #[test]
    fn trace_context_reaches_the_run_service_from_the_header_or_the_meta() -> Result<(), TestError>
    {
        on_neo_runtime(async {
            let neo = Served::start().await?;
            let cases = [
                Case {
                    headers: &[("traceparent", HEADER), ("tracestate", "edge=1")],
                    meta: None,
                    parent: TraceContext::from_w3c(HEADER, Some("edge=1")),
                    why: "the header's pair",
                },
                Case {
                    headers: &[],
                    meta: Some(json!({ "traceparent": META, "tracestate": "host=1" })),
                    parent: TraceContext::from_w3c(META, Some("host=1")),
                    why: "no header: _meta's pair",
                },
                Case {
                    headers: &[("traceparent", HEADER)],
                    meta: Some(json!({ "traceparent": META, "tracestate": "host=1" })),
                    parent: TraceContext::from_w3c(HEADER, None),
                    why: "both: the header wins, with no tracestate of _meta's",
                },
                Case {
                    headers: &[("traceparent", "garbage")],
                    meta: None,
                    parent: None,
                    why: "an invalid header: a trace of the run's own",
                },
                Case {
                    headers: &[("traceparent", "garbage")],
                    meta: Some(json!({ "traceparent": META })),
                    parent: TraceContext::from_w3c(META, None),
                    why: "an invalid header falls back to _meta's",
                },
            ];
            assert!(
                TraceContext::from_w3c(HEADER, None).is_some()
                    && TraceContext::from_w3c(META, None).is_some(),
                "the fixture's traceparents are valid, so a parent missing below is Neo's doing"
            );
            for Case {
                headers,
                meta,
                parent,
                why,
            } in cases
            {
                let started = parents(&neo.probe).len();
                let result = call(&neo.url, headers, meta).await?;
                assert_eq!(
                    outcome(&result, "/status"),
                    Some(&json!("ok")),
                    "{why}: trace context never stops a run: {result:?}"
                );
                let after = parents(&neo.probe);
                assert_eq!(after.len(), started + 1, "{why}: one run span");
                assert_eq!(after.last(), Some(&parent), "{why}");
            }
            neo.stop();
            Ok(())
        })
    }

    #[test]
    fn an_unknown_meta_key_is_still_refused_beside_trace_context() -> Result<(), TestError> {
        on_neo_runtime(async {
            let neo = Served::start().await?;
            let typo = call(
                &neo.url,
                &[("traceparent", HEADER)],
                Some(json!({ "traceparent": META, "traceParent": META })),
            )
            .await?;
            assert_eq!(
                (
                    outcome(&typo, "/error/owner"),
                    outcome(&typo, "/error/code")
                ),
                (Some(&json!("host")), Some(&json!("invalid_meta"))),
                "an unknown _meta key is still refused, trace headers or not: {typo:?}"
            );
            assert!(
                parents(&neo.probe).is_empty(),
                "a refused call exports no run span"
            );
            neo.stop();
            Ok(())
        })
    }
}
