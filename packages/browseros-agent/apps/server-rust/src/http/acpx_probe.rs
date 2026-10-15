//! `POST /acpx/probe`: initialize + `session/new` against an ACP agent to report its models and
//! modes (`acp_agents::probe_agent`). Owned by P16.

use axum::response::Response;

pub async fn probe() -> Response {
    super::not_implemented()
}
