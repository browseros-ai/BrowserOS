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
use crate::{events_route, part_routes, record_routes, response_id, secret_routes, tab_routes};
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
            "/v1/operations/{operation_id}",
            get(record_routes::get_operation),
        )
        .route(
            "/v1/operations/{operation_id}/screenshot",
            get(record_routes::get_screenshot),
        )
        .route("/v1/trace", get(record_routes::get_trace))
        .route("/v1/events", get(events_route::stream_events))
        .route("/v1/tabs/{tab_id}/private", post(tab_routes::make_private))
        .route("/v1/locks", post(part_routes::create_lock))
        .route(
            "/v1/locks/{lock_id}",
            get(part_routes::get_lock).delete(part_routes::release_lock),
        )
        .route("/v1/secrets", post(secret_routes::register_secret))
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
