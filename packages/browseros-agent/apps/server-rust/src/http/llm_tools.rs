//! `/test-provider` and `/refine-prompt`: one-shot `browseros_agent::llm::generate_text` calls with
//! TS's provider resolution order and `[<provider>] <err>` messages. Owned by P13.

use axum::response::Response;

pub async fn test_provider() -> Response {
    super::not_implemented()
}

pub async fn refine_prompt() -> Response {
    super::not_implemented()
}
