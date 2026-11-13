//! The one route table, the list a reader checks against the contract (`openapi.yaml`). Each
//! handler lives in its area's module. Any other path is a 404 in the contract's `Error`
//! shape, and the request-id stamp wraps every route and that fallback.
//!
//! Every request must carry [`Authenticated`], which only [`host_service`](crate::host_service)'s
//! auth inserts. Served without that auth, the router refuses everything: the routes can never
//! be exposed by forgetting the wrapper.

use crate::api_error::ApiError;
use crate::host_service::Authenticated;
use crate::service::ServiceState;
use crate::{
    display_route, events_route, fit_routes, live_tabs, part_routes, record_routes, response_id,
    settings_routes, tab_routes, takeover_routes,
};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Router, middleware};
use std::sync::Arc;

pub(crate) fn router(state: Arc<ServiceState>) -> Router {
    Router::new()
        .route("/v1/runs", get(record_routes::list_runs))
        .route("/v1/runs/{run_id}", get(record_routes::get_run))
        .route(
            "/v1/runs/{run_id}/downloads/{index}",
            get(record_routes::get_download),
        )
        .route(
            "/v1/runs/{run_id}/blobs/{blob_id}",
            get(record_routes::get_blob),
        )
        .route(
            "/v1/operations/{operation_id}",
            get(record_routes::get_operation),
        )
        .route(
            "/v1/operations/{operation_id}/screenshot",
            get(record_routes::get_screenshot),
        )
        .route("/v1/trace", get(record_routes::get_trace))
        .route("/v1/events", get(events_route::stream_events))
        .route("/v1/tabs", get(tab_routes::list_tabs))
        .route("/v1/tabs/{tab_id}/screencast", get(live_tabs::screencast))
        .route("/v1/tabs/{tab_id}/video", get(live_tabs::video))
        .route("/v1/locks", post(part_routes::create_lock))
        .route(
            "/v1/locks/{lock_id}",
            get(part_routes::get_lock).delete(part_routes::release_lock),
        )
        .route("/v1/tabs/{tab_id}/fit", post(fit_routes::fit_tab))
        .route("/v1/fits/{fit_id}", get(fit_routes::get_fit))
        .route("/v1/fits/{fit_id}/navigate", post(fit_routes::navigate_fit))
        .route("/v1/fits/{fit_id}/restore", post(fit_routes::restore_fit))
        .route("/v1/display", get(display_route::open_display))
        .route("/v1/takeovers", post(takeover_routes::start_takeover))
        .route(
            "/v1/takeovers/{takeover_id}",
            get(takeover_routes::get_takeover),
        )
        .route(
            "/v1/takeovers/{takeover_id}/release",
            post(takeover_routes::release_takeover),
        )
        .route(
            "/v1/settings",
            get(settings_routes::get_settings)
                .put(settings_routes::put_settings)
                .patch(settings_routes::patch_settings),
        )
        .fallback(not_found)
        .with_state(state)
        .layer(middleware::from_fn(require_authenticated))
        .layer(middleware::from_fn(response_id::stamp))
}

async fn require_authenticated(request: Request, next: Next) -> Response {
    if request.extensions().get::<Authenticated>().is_some() {
        next.run(request).await
    } else {
        tracing::error!("a host route was served without host_service; refusing it");
        ApiError::unauthorized("these routes are served only behind the host auth").into_response()
    }
}

async fn not_found() -> ApiError {
    ApiError::not_found("no route matches this path")
}
