//! `http`: the extension-facing surface and the contract. Every route is registered once, in
//! a `utoipa-axum` `routes!` list, so the router and the OpenAPI document come from the same
//! list. Handlers parse a wire type, call one module, and map the result; nothing else.
//!
//! ```text
//!   127.0.0.1:<port>
//!     ├─ system routes        /system/health, /status, …   no origin check: the browser's C++ calls them
//!     ├─ contract routes      /chat, /providers, …          behind `trust::guard` (design section 4.3)
//!     └─ /mcp                 the private MCP endpoint      NOT behind `trust::guard`: `browser`
//!                                                           refuses `Origin` / `Sec-Fetch-Site`
//!                                                           and unarmed tasks itself
//! ```

mod agents;
mod chat;
mod dto;
mod extract;
mod history;
mod probe;
mod providers;
mod signin;
mod system;
mod test_provider;
mod trust;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use utoipa::openapi::response::ResponseBuilder;
use utoipa::openapi::{Content, Ref};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;

pub use trust::parse_trusted_origins;

use crate::app::App;
use crate::store::StoreError;
use crate::turn::ChatError;

/// `{"error": "<one sentence>"}`, or the chat-error envelope on the chat path.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

#[derive(Debug)]
enum ErrorBody {
    Message(String),
    Chat(Box<ChatError>),
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody::Message(message.into()),
        }
    }

    /// The chat path's refusals (`400`, `409`) carry the envelope an `error` chunk carries, so
    /// the error card renders both the same way.
    pub fn chat(status: StatusCode, error: ChatError) -> Self {
        Self {
            status,
            body: ErrorBody::Chat(Box::new(error)),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "Forbidden")
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    /// A route whose handler the skeleton has not filled in yet.
    pub fn not_implemented() -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "Not implemented")
    }

    /// A failure the caller cannot fix: logged here with its cause, answered with one sentence.
    pub fn internal(cause: &dyn std::fmt::Display) -> Self {
        tracing::error!(%cause, "request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        Self::internal(&error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self.body {
            ErrorBody::Message(error) => {
                (self.status, Json(dto::ApiErrorBody { error })).into_response()
            }
            ErrorBody::Chat(error) => {
                (self.status, Json(dto::ChatErrorEnvelope::from(*error))).into_response()
            }
        }
    }
}

/// The document's header: title, description, tags and server.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "BrowserOS server API",
        description = "The contract between the BrowserOS extension and its local server. Generated from \
            the server's Rust handlers and wire types; do not edit by hand.\n\n\
            Trust model, for every route except the `system` tag: the server binds `127.0.0.1` only; \
            the `Host` header must name loopback; a request that carries an `Origin` header must carry \
            the extension's origin (or one listed in `BROWSEROS_TRUSTED_ORIGINS`); a request with no \
            `Origin` may only use `GET`, `HEAD` or `OPTIONS`. A refused request gets \
            `403 {\"error\":\"Forbidden\"}`. A preflight `OPTIONS` from an allowed origin is \
            answered by the trust layer itself (`204`, methods `GET, POST, PUT, DELETE, OPTIONS`, \
            headers `Content-Type, Accept`).\n\nTimes are epoch milliseconds. Ids are strings."
    ),
    servers((url = "http://127.0.0.1:{port}", description = "The local server. The port comes from the browser pref `browseros.server.mcp_port`.",
        variables(("port" = (default = "9200", description = "The server port"))))),
    tags(
        (name = "chat", description = "Turns, server-owned runs, replay and side-panel routing."),
        (name = "conversations", description = "History. The server is the only writer."),
        (name = "providers", description = "LLM providers, the selected target, connection test."),
        (name = "agents", description = "Coding agents run through acpx (Claude Code, Codex)."),
        (name = "signin", description = "Sign in with ChatGPT."),
        (name = "system", description = "The browser's supervisor contract and diagnostics. No origin check."),
    )
)]
struct ApiDoc;

/// Lists the trust layer's `403` on every operation outside the `system` tag, so the document
/// says what the middleware does without each annotation repeating it.
struct GuardedOperations;

impl Modify for GuardedOperations {
    fn modify(&self, document: &mut utoipa::openapi::OpenApi) {
        let forbidden = ResponseBuilder::new()
            .description("The request did not come from the extension on this machine.")
            .content(
                "application/json",
                Content::new(Some(Ref::from_schema_name("ApiError"))),
            )
            .build();
        for item in document.paths.paths.values_mut() {
            for operation in [
                &mut item.get,
                &mut item.put,
                &mut item.post,
                &mut item.delete,
                &mut item.patch,
            ]
            .into_iter()
            .flatten()
            {
                let guarded = !operation
                    .tags
                    .as_ref()
                    .is_some_and(|tags| tags.iter().any(|tag| tag == "system"));
                if guarded {
                    operation
                        .responses
                        .responses
                        .entry("403".to_owned())
                        .or_insert_with(|| forbidden.clone().into());
                }
            }
        }
    }
}

/// The routes behind the trust rule: everything the extension calls.
fn contract_routes() -> OpenApiRouter<App> {
    OpenApiRouter::new()
        .merge(chat::routes())
        .merge(history::routes())
        .merge(providers::routes())
        .merge(test_provider::routes())
        .merge(agents::routes())
        .merge(probe::routes())
        .merge(signin::routes())
}

/// Three routers merged:
///   system routes    -> no origin check (the browser's C++ calls them)
///   contract routes  -> under `trust::guard` (DESIGN 4.3 rules 1 to 4)
///   browser.mcp_router(), nested at `/mcp` -> NOT under `trust::guard`. Its own rule: refuse
///     any request with `Origin` or `Sec-Fetch-Site`; the task in the path must be armed.
pub fn router(app: App) -> axum::Router {
    let (system, _) = system::routes().split_for_parts();
    let (contract, _) = contract_routes().split_for_parts();
    let contract = contract.layer(axum::middleware::from_fn_with_state(
        app.launch.clone(),
        trust::guard,
    ));
    let mcp = app.browser.mcp_router();
    system
        .merge(contract)
        .with_state(app)
        .nest_service("/mcp", mcp)
}

/// The document committed as `contracts/server-api/openapi.json`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    let (_, mut document) = OpenApiRouter::<App>::with_openapi(ApiDoc::openapi())
        .merge(system::routes())
        .merge(contract_routes())
        .split_for_parts();
    // After the merge: a modifier on `ApiDoc` would run before the routes are in the document.
    GuardedOperations.modify(&mut document);
    document
}
