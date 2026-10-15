//! `ScriptedModel`: a deterministic `LanguageModel` that replays one scripted step per `stream`
//! call. Backs unit tests across the workspace and `BROWSEROS_USE_MOCK_LLM=true` (port of
//! `mock-language-model.ts`: every call streams `Mock BrowserOS test response.`).
//!
//! A step that ends without a terminal item (`Finish` or an error) then stalls until the call's
//! token is cancelled and yields `ModelError::Cancelled`, which models a provider that hangs
//! mid-stream for cancellation tests.

use super::{
    LanguageModel, ModelError, ModelEvent, ModelFinishReason, ModelRequest, ModelStream,
    ProviderKind, Usage,
};
use std::{collections::VecDeque, sync::Mutex};
use tokio_util::sync::CancellationToken;

/// `MOCK_LLM_RESPONSE_TEXT` in TS.
pub const MOCK_LLM_RESPONSE_TEXT: &str = "Mock BrowserOS test response.";

type Step = Vec<Result<ModelEvent, ModelError>>;

enum Script {
    /// One step per call, then an error once exhausted.
    Steps(VecDeque<Step>),
    /// The same step on every call (the mock LLM serves any number of turns).
    Repeat(Step),
}

pub struct ScriptedModel {
    script: Mutex<Script>,
    requests: Mutex<Vec<ModelRequest>>,
    model_id: &'static str,
}

impl ScriptedModel {
    /// The mock LLM: every call streams `Mock BrowserOS test response.` and stops.
    pub fn mock() -> Self {
        // TS's mock part is `text-1`; server tests pin `mock-text`. Stream diffs normalize
        // part ids, so the spelling is not wire-visible.
        let id = "mock-text".to_owned();
        let step = vec![
            Ok(ModelEvent::TextStart {
                id: id.clone(),
                provider_metadata: None,
            }),
            Ok(ModelEvent::TextDelta {
                id: id.clone(),
                delta: MOCK_LLM_RESPONSE_TEXT.to_owned(),
            }),
            Ok(ModelEvent::TextEnd {
                id,
                provider_metadata: None,
            }),
            Ok(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
                usage: Usage {
                    input_tokens: Some(1),
                    output_tokens: Some(4),
                    cached_input_tokens: None,
                    reasoning_tokens: None,
                },
            }),
        ];
        Self {
            script: Mutex::new(Script::Repeat(step)),
            requests: Mutex::new(Vec::new()),
            model_id: "browseros-test-mock",
        }
    }

    /// One step per `stream` call, in order; a call after the last step fails with
    /// `ModelError::Config`.
    pub fn new(steps: Vec<Vec<Result<ModelEvent, ModelError>>>) -> Self {
        Self {
            script: Mutex::new(Script::Steps(steps.into())),
            requests: Mutex::new(Vec::new()),
            model_id: "scripted",
        }
    }

    /// Every request this model received, in call order (what the loop actually sent).
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }

    fn next_step(&self) -> Result<Step, ModelError> {
        let mut script = self
            .script
            .lock()
            .map_err(|_| ModelError::Config("scripted model lock poisoned".to_owned()))?;
        match &mut *script {
            Script::Steps(steps) => steps
                .pop_front()
                .ok_or_else(|| ModelError::Config("scripted model has no steps left".to_owned())),
            Script::Repeat(step) => Ok(step.clone()),
        }
    }
}

fn is_terminal(item: &Result<ModelEvent, ModelError>) -> bool {
    matches!(item, Err(_) | Ok(ModelEvent::Finish { .. }))
}

#[async_trait::async_trait]
impl LanguageModel for ScriptedModel {
    fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn model_id(&self) -> &str {
        self.model_id
    }

    fn supports_media_in_tool_results(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let step = self.next_step()?;
        let terminated = step.iter().any(is_terminal);
        let replay = futures_util::stream::iter(step);
        if terminated {
            return Ok(Box::pin(replay));
        }
        let stall = futures_util::stream::once(async move {
            cancel.cancelled().await;
            Err(ModelError::Cancelled)
        });
        Ok(Box::pin(futures_util::StreamExt::chain(replay, stall)))
    }
}
