//! The HTTP surface. Every route is registered in exactly one place, [`ROUTES`], which
//! `app::build_router` mounts verbatim and `tests/contract_routes.rs` compares against the
//! OpenAPI contract (`contracts/server-api`). Handlers live one module per contract path group.
//!
//! Cross-cutting behaviour lives in one middleware, `security::ingress`, in Hono's order:
//! CORS (no PUT in preflight) → trusted-origin guard → trusted-app prefix guard → `/mcp`
//! browser-fetch guard → handler; unmatched routes are a plain-text 404. The per-route [`Auth`]
//! column documents that middleware and is held to it by a test. On the way out the same
//! middleware reports every response marked [`errors::Unhandled`] to Sentry, as TS's
//! `app.onError` does.

pub mod acpx_probe;
pub mod agents;
pub mod chat;
pub mod conversations;
pub mod errors;
pub mod extract;
pub mod klavis;
pub mod llm_tools;
pub mod mcp;
pub mod mcp_manager;
pub mod oauth;
pub mod providers;
pub mod schedules;
pub mod security;
pub mod system;
pub mod wire;
pub mod zod;

use crate::app::AppState;
use axum::{
    response::Response,
    routing::{MethodFilter, MethodRouter, on},
};
use http::{Method, StatusCode};

/// Which request-trust check a route runs. `security::ingress` enforces these by path prefix;
/// `tests/contract_routes.rs` holds the two in agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// `O`: trusted-origin guard only.
    Origin,
    /// `O+App`: also `require_trusted_app` before the handler.
    TrustedApp,
    /// `O+App*`: the handler decides (e.g. only ACP conversations need the app check on DELETE).
    TrustedAppInHandler,
    /// `O+NoBrowser`: `/mcp` rejects browser fetches (`Sec-Fetch-Site`) instead.
    NoBrowserFetch,
}

/// One registered route. `mount` builds the method router for `method`, so the method appears
/// once per entry and the table cannot disagree with the router it produces.
pub struct Route {
    pub method: Method,
    pub path: &'static str,
    pub auth: Auth,
    pub mount: fn(MethodFilter) -> MethodRouter<AppState>,
}

/// The single route table. Paths use axum's `{param}` syntax, identical to OpenAPI's.
/// The OAuth PKCE callback listener (`127.0.0.1:1455`) is a separate server, not listed here.
pub static ROUTES: &[Route] = &[
    // system.yaml (root aliases kept for older callers)
    Route {
        method: Method::GET,
        path: "/system/health",
        auth: Auth::Origin,
        mount: |m| on(m, system::health),
    },
    Route {
        method: Method::GET,
        path: "/health",
        auth: Auth::Origin,
        mount: |m| on(m, system::health),
    },
    Route {
        method: Method::GET,
        path: "/status",
        auth: Auth::Origin,
        mount: |m| on(m, system::status),
    },
    Route {
        method: Method::GET,
        path: "/system/diagnostics",
        auth: Auth::Origin,
        mount: |m| on(m, system::diagnostics),
    },
    Route {
        method: Method::POST,
        path: "/system/shutdown",
        auth: Auth::Origin,
        mount: |m| on(m, system::shutdown),
    },
    Route {
        method: Method::POST,
        path: "/shutdown",
        auth: Auth::Origin,
        mount: |m| on(m, system::shutdown),
    },
    // chat.yaml
    Route {
        method: Method::POST,
        path: "/chat",
        auth: Auth::TrustedAppInHandler,
        mount: |m| on(m, chat::post_chat),
    },
    Route {
        method: Method::GET,
        path: "/chat/panels",
        auth: Auth::TrustedAppInHandler,
        mount: |m| on(m, chat::panels),
    },
    Route {
        method: Method::DELETE,
        path: "/chat/panels/{tabId}",
        auth: Auth::TrustedAppInHandler,
        mount: |m| on(m, chat::delete_panel),
    },
    Route {
        method: Method::GET,
        path: "/chat/{conversationId}/state",
        auth: Auth::Origin,
        mount: |m| on(m, chat::state),
    },
    Route {
        method: Method::GET,
        path: "/chat/{conversationId}/stream",
        auth: Auth::Origin,
        mount: |m| on(m, chat::resume_stream),
    },
    Route {
        method: Method::POST,
        path: "/chat/{conversationId}/stop",
        auth: Auth::Origin,
        mount: |m| on(m, chat::stop),
    },
    Route {
        method: Method::DELETE,
        path: "/chat/{conversationId}",
        auth: Auth::TrustedAppInHandler,
        mount: |m| on(m, chat::delete_conversation_session),
    },
    // conversations.yaml
    Route {
        method: Method::GET,
        path: "/conversations",
        auth: Auth::TrustedApp,
        mount: |m| on(m, conversations::list),
    },
    Route {
        method: Method::GET,
        path: "/conversations/{conversationId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, conversations::get),
    },
    Route {
        method: Method::PUT,
        path: "/conversations/{conversationId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, conversations::import),
    },
    Route {
        method: Method::DELETE,
        path: "/conversations/{conversationId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, conversations::delete),
    },
    // agents.yaml
    Route {
        method: Method::GET,
        path: "/agents",
        auth: Auth::TrustedApp,
        mount: |m| on(m, agents::list),
    },
    Route {
        method: Method::POST,
        path: "/agents",
        auth: Auth::TrustedApp,
        mount: |m| on(m, agents::create),
    },
    Route {
        method: Method::GET,
        path: "/agents/{agentId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, agents::get),
    },
    Route {
        method: Method::PUT,
        path: "/agents/{agentId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, agents::update),
    },
    Route {
        method: Method::DELETE,
        path: "/agents/{agentId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, agents::delete),
    },
    // providers.yaml
    Route {
        method: Method::GET,
        path: "/providers",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::list),
    },
    Route {
        method: Method::GET,
        path: "/providers/default",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::get_default),
    },
    Route {
        method: Method::PUT,
        path: "/providers/default",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::set_default),
    },
    Route {
        method: Method::GET,
        path: "/providers/{providerId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::get),
    },
    Route {
        method: Method::PUT,
        path: "/providers/{providerId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::upsert),
    },
    Route {
        method: Method::DELETE,
        path: "/providers/{providerId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, providers::delete),
    },
    Route {
        method: Method::POST,
        path: "/test-provider",
        auth: Auth::TrustedApp,
        mount: |m| on(m, llm_tools::test_provider),
    },
    Route {
        method: Method::POST,
        path: "/refine-prompt",
        auth: Auth::TrustedApp,
        mount: |m| on(m, llm_tools::refine_prompt),
    },
    // scheduled-jobs.yaml, scheduled-job-runs.yaml
    Route {
        method: Method::GET,
        path: "/scheduled-jobs",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::list_jobs),
    },
    Route {
        method: Method::GET,
        path: "/scheduled-jobs/{jobId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::get_job),
    },
    Route {
        method: Method::PUT,
        path: "/scheduled-jobs/{jobId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::upsert_job),
    },
    Route {
        method: Method::DELETE,
        path: "/scheduled-jobs/{jobId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::delete_job),
    },
    Route {
        method: Method::GET,
        path: "/scheduled-job-runs",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::list_runs),
    },
    Route {
        method: Method::GET,
        path: "/scheduled-job-runs/{runId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::get_run),
    },
    Route {
        method: Method::PUT,
        path: "/scheduled-job-runs/{runId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::upsert_run),
    },
    Route {
        method: Method::DELETE,
        path: "/scheduled-job-runs/{runId}",
        auth: Auth::TrustedApp,
        mount: |m| on(m, schedules::delete_run),
    },
    // acpx-probe.yaml
    Route {
        method: Method::POST,
        path: "/acpx/probe",
        auth: Auth::TrustedApp,
        mount: |m| on(m, acpx_probe::probe),
    },
    // klavis.yaml
    Route {
        method: Method::GET,
        path: "/klavis/servers",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::servers),
    },
    Route {
        method: Method::GET,
        path: "/klavis/oauth-urls",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::oauth_urls),
    },
    Route {
        method: Method::GET,
        path: "/klavis/user-integrations",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::user_integrations),
    },
    Route {
        method: Method::POST,
        path: "/klavis/servers/add",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::add_server),
    },
    Route {
        method: Method::POST,
        path: "/klavis/servers/submit-api-key",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::submit_api_key),
    },
    Route {
        method: Method::DELETE,
        path: "/klavis/servers/remove",
        auth: Auth::Origin,
        mount: |m| on(m, klavis::remove_server),
    },
    // oauth.yaml
    Route {
        method: Method::GET,
        path: "/oauth/{provider}/start",
        auth: Auth::Origin,
        mount: |m| on(m, oauth::start),
    },
    Route {
        method: Method::POST,
        path: "/oauth/{provider}/token",
        auth: Auth::Origin,
        mount: |m| on(m, oauth::store_token),
    },
    Route {
        method: Method::GET,
        path: "/oauth/{provider}/status",
        auth: Auth::Origin,
        mount: |m| on(m, oauth::status),
    },
    Route {
        method: Method::DELETE,
        path: "/oauth/{provider}",
        auth: Auth::Origin,
        mount: |m| on(m, oauth::logout),
    },
    // mcp-manager.yaml
    Route {
        method: Method::GET,
        path: "/mcp-manager/tools",
        auth: Auth::Origin,
        mount: |m| on(m, mcp_manager::tools),
    },
    Route {
        method: Method::GET,
        path: "/mcp-manager/agents",
        auth: Auth::Origin,
        mount: |m| on(m, mcp_manager::agents),
    },
    Route {
        method: Method::POST,
        path: "/mcp-manager/agents/{id}/install",
        auth: Auth::Origin,
        mount: |m| on(m, mcp_manager::install),
    },
    Route {
        method: Method::POST,
        path: "/mcp-manager/agents/{id}/uninstall",
        auth: Auth::Origin,
        mount: |m| on(m, mcp_manager::uninstall),
    },
    // mcp.yaml: POST/DELETE are the shared host's rmcp service behind the lease admission.
    Route {
        method: Method::GET,
        path: "/mcp",
        auth: Auth::NoBrowserFetch,
        mount: |m| on(m, mcp::status_json),
    },
    Route {
        method: Method::POST,
        path: "/mcp",
        auth: Auth::NoBrowserFetch,
        mount: |m| on(m, mcp::dispatch),
    },
    Route {
        method: Method::DELETE,
        path: "/mcp",
        auth: Auth::NoBrowserFetch,
        mount: |m| on(m, mcp::delete_session),
    },
];

/// `axum::Json` written as `JSON.stringify` writes it ([`ui_message_stream::sse::json_bytes`]).
/// The generated contract DTOs hold every number as `f64`, so serde_json alone would answer a
/// stored millisecond timestamp as `1792055977284.0` where TS answers `1792055977284`: equal to a
/// JS client, but not to a strict one decoding the contract's integer fields. Every handler
/// answers JSON through this type.
pub struct Json<T>(pub T);

impl<T: serde::Serialize> axum::response::IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match ui_message_stream::sse::json_bytes(&self.0) {
            Ok(bytes) => (
                [(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                )],
                bytes,
            )
                .into_response(),
            // axum::Json's own answer to an unserializable value.
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }
}

/// Response extension set only by the router fallback. It never reaches the wire; it lets tests
/// tell "no such route" apart from a handler's own 404 (e.g. `DELETE /mcp`'s plain-text 404).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnmatchedRoute;

/// Unmatched path or method: Hono's plain-text 404 (TS parity; axum's default would be 405).
pub async fn not_found() -> Response {
    let mut response = errors::text_plain(StatusCode::NOT_FOUND, "404 Not Found");
    response.extensions_mut().insert(UnmatchedRoute);
    response
}
