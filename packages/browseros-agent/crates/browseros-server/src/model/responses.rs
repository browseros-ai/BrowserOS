//! The OpenAI Responses wire, written once for both of its callers: the `openai` provider and
//! the ChatGPT sign-in backend, which differ only in a `ResponsesEndpoint`. The Codex dialect's
//! body rules live here, beside the wire. Owned by chunk `models`; chunk `chatgpt` builds the
//! endpoint (`SignIn::endpoint`) and never touches the wire (CORRECTIONS-2 section 2).

use super::{AssistantTurn, StepRequest};
use crate::turn::{ChatError, EventSink};

/// Where and how to speak the OpenAI Responses wire. The wire is written once; the `openai`
/// provider and the ChatGPT sign-in backend differ only in this value.
#[expect(
    dead_code,
    reason = "built and read once chunks `models` and `chatgpt` write the wire"
)]
pub(super) struct ResponsesEndpoint {
    /// `…/v1/responses`, or `https://chatgpt.com/backend-api/codex/responses`.
    pub url: url::Url,
    /// Authorization and any fixed headers.
    pub headers: Vec<(String, String)>,
    pub dialect: ResponsesDialect,
}

#[expect(
    dead_code,
    reason = "built once chunks `models` and `chatgpt` write the wire"
)]
pub(super) enum ResponsesDialect {
    /// api.openai.com: the wire as documented.
    OpenAi,
    /// The ChatGPT Codex backend: `stream: true`, `store: false`; no `previous_response_id`,
    /// temperature or token limits; `instructions` never empty; no `id` on input items;
    /// encrypted reasoning requested and replayed.
    ChatGptCodex,
}

/// One Responses call. Same contract as `Model::step`.
#[expect(dead_code, reason = "called once chunk `models` writes the dispatch")]
pub(super) async fn step(
    _http: &reqwest::Client,
    _endpoint: &ResponsesEndpoint,
    _model: &str,
    _request: &StepRequest<'_>,
    _sink: &mut dyn EventSink,
) -> Result<AssistantTurn, ChatError> {
    Err(ChatError::not_implemented("The OpenAI Responses wire"))
}
