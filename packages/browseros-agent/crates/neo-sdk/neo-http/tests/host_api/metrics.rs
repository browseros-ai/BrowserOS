//! `GET /v1/metrics`: the text the app's telemetry renders, served as it is at every scrape, as
//! Prometheus text exposition 0.0.4, and only behind the host token.

use crate::support::{BoxError, Harness, Reply, TOKEN, TestResult, harness};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::Response;
use neo_http::{HostAuth, HostToken, PeerInfo, host_router};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use tower::ServiceExt;

/// The content type the telemetry wire contract pins for the pull door, spelled out here rather
/// than taken from the crate, so a wrong constant there fails this test.
const PROMETHEUS_TEXT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// A scrape as the telemetry renders one: `# HELP` and `# TYPE` lines, a counter and a
/// histogram, with UTF-8 beyond ASCII in a help text.
const EXPOSITION: &str = "\
# HELP neo_runs_total Runs Neo admitted, by how they ended → ok, error, cancelled or timeout.
# TYPE neo_runs_total counter
neo_runs_total{neo_run_status=\"ok\",neo_error_owner=\"none\"} 3
# HELP neo_run_duration_seconds How long each run took.
# TYPE neo_run_duration_seconds histogram
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"0.5\"} 1
neo_run_duration_seconds_bucket{neo_run_status=\"ok\",le=\"+Inf\"} 3
neo_run_duration_seconds_sum{neo_run_status=\"ok\"} 4.25
neo_run_duration_seconds_count{neo_run_status=\"ok\"} 3
";

/// A later scrape, after the numbers moved.
const LATER: &str = "# TYPE neo_run_active gauge\nneo_run_active 1\n";

/// The app's telemetry as `ServiceInputs::metrics` reaches it: the text a test set (empty, as a
/// telemetry that recorded nothing renders), and how many times the service asked for it.
#[derive(Default)]
pub(crate) struct FakeMetrics {
    text: Mutex<String>,
    renders: AtomicUsize,
}

impl FakeMetrics {
    /// What every later render returns.
    fn set(&self, text: &str) {
        text.clone_into(&mut self.text.lock().unwrap_or_else(PoisonError::into_inner));
    }

    /// The text now, counted: the harness hands this to the service as its `MetricsText`.
    pub(crate) fn render(&self) -> String {
        self.renders.fetch_add(1, Ordering::SeqCst);
        self.text
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn renders(&self) -> usize {
        self.renders.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn each_scrape_serves_what_the_telemetry_renders_for_it() -> TestResult {
    let h = harness().await?;
    assert_eq!(
        scrape(&h).await?,
        "",
        "a telemetry that recorded nothing renders no families, which is a valid exposition"
    );
    h.metrics.set(EXPOSITION);
    assert_eq!(scrape(&h).await?, EXPOSITION, "served byte for byte");
    h.metrics.set(LATER);
    assert_eq!(
        scrape(&h).await?,
        LATER,
        "rendered at each scrape, never kept from an earlier one"
    );
    assert_eq!(h.metrics.renders(), 3, "one render per scrape");
    Ok(())
}

#[tokio::test]
async fn a_scrape_without_the_host_token_is_refused_before_anything_renders() -> TestResult {
    let h = harness().await?;
    h.metrics.set(EXPOSITION);
    // The real host auth with nothing added to the request, as a scraper without the token
    // reaches the host listener.
    let auth = HostAuth {
        token: HostToken::parse(TOKEN)?,
        peer: None,
    };
    let scraper = host_router(h.routes.clone(), auth).layer(MockConnectInfo(PeerInfo::tcp()));
    for authorization in [
        None,
        Some("Bearer not-the-token-0123456789-abcdefghij".to_owned()),
        Some(format!("Basic {TOKEN}")),
    ] {
        let response = get(&scraper, authorization.as_deref()).await?;
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer"),
            "{authorization:?}"
        );
        Reply::read(response)
            .await?
            .assert_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    assert_eq!(
        h.metrics.renders(),
        0,
        "the host auth answers before the route, so a refused scrape learns and costs nothing"
    );
    let response = get(&scraper, Some(&format!("Bearer {TOKEN}"))).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(content_types(response.headers()), [PROMETHEUS_TEXT]);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await?,
        EXPOSITION.as_bytes()
    );
    Ok(())
}

#[tokio::test]
async fn the_metrics_route_only_reads() -> TestResult {
    let h = harness().await?;
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        h.call(method, "/v1/metrics", Body::empty())
            .await?
            .assert_error(StatusCode::METHOD_NOT_ALLOWED, "invalid_request");
    }
    assert_eq!(h.metrics.renders(), 0, "only a GET renders the text");
    Ok(())
}

/// `GET /v1/metrics` as the host sends it, with its token: a 200 with the contract's content
/// type, whose body is returned as text.
async fn scrape(h: &Harness) -> Result<String, BoxError> {
    let (status, headers, body) = h.get_bytes("/v1/metrics").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_types(&headers),
        [PROMETHEUS_TEXT],
        "exactly the pinned content type, which a Prometheus scraper reads to pick its parser"
    );
    assert!(
        headers.contains_key("x-request-id"),
        "a scrape is stamped like every response"
    );
    Ok(String::from_utf8(body)?)
}

/// One `GET /v1/metrics` through `router`, with `authorization` as the request's only header.
async fn get(router: &Router, authorization: Option<&str>) -> Result<Response, BoxError> {
    let mut request = Request::builder().uri("/v1/metrics");
    if let Some(authorization) = authorization {
        request = request.header(header::AUTHORIZATION, authorization);
    }
    Ok(router.clone().oneshot(request.body(Body::empty())?).await?)
}

/// Every `Content-Type` the response carries.
fn content_types(headers: &HeaderMap) -> Vec<&str> {
    headers
        .get_all(header::CONTENT_TYPE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect()
}
