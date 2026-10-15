//! Custom MCP server transport detection (port of `lib/mcp-transport-detect.ts`): POST an
//! `initialize` and decide between Streamable HTTP and the deprecated HTTP+SSE transport. Results
//! are cached per URL for an hour, except for answers that may be transient (5xx, network error),
//! which are re-probed next time.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

/// `TIMEOUTS.MCP_TRANSPORT_PROBE`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectedTransport {
    StreamableHttp,
    LegacySse,
}

fn cache() -> &'static Mutex<HashMap<String, (DetectedTransport, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (DetectedTransport, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

pub(super) async fn detect(url: &url::Url) -> DetectedTransport {
    let key = url.as_str().to_owned();
    if let Ok(cache) = cache().lock()
        && let Some((transport, expires)) = cache.get(&key)
        && Instant::now() < *expires
    {
        return *transport;
    }

    let (transport, cacheable) = probe(url).await;
    if cacheable && let Ok(mut cache) = cache().lock() {
        cache.insert(key, (transport, Instant::now() + CACHE_TTL));
    }
    tracing::debug!(url = %url, ?transport, cached = cacheable, "Detected MCP transport");
    transport
}

/// Returns the transport and whether the answer may be cached.
async fn probe(url: &url::Url) -> (DetectedTransport, bool) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "browseros-probe", "version": "1.0.0" },
        },
    });
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(client) => client,
        Err(_) => return (DetectedTransport::LegacySse, false),
    };
    let response = client
        .post(url.clone())
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(body.to_string())
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(url = %url, %error, "MCP transport probe failed");
            return (DetectedTransport::LegacySse, false);
        }
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    // Dropping the response releases the connection without reading an SSE body to its end.
    drop(response);

    let speaks_mcp =
        content_type.contains("application/json") || content_type.contains("text/event-stream");
    match status {
        200 if speaks_mcp => (DetectedTransport::StreamableHttp, true),
        401 => (DetectedTransport::StreamableHttp, true),
        500.. => (DetectedTransport::LegacySse, false),
        _ => (DetectedTransport::LegacySse, true),
    }
}
