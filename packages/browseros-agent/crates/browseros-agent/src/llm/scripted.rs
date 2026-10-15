//! `ScriptedModel`: a deterministic `LanguageModel` that replays one scripted step per `stream`
//! call. Backs unit tests across the workspace and `BROWSEROS_USE_MOCK_LLM=true`
//! (port of `mock-language-model.ts`).

use super::{
    LanguageModel, ModelError, ModelEvent, ModelFinishReason, ModelRequest, ModelStream,
    ProviderKind, Usage,
};
use std::{collections::VecDeque, sync::Mutex};
use tokio_util::sync::CancellationToken;

pub struct ScriptedModel {
    steps: Mutex<VecDeque<Vec<Result<ModelEvent, ModelError>>>>,
}

impl ScriptedModel {
    /// One step that streams `Mock BrowserOS test response.` and stops.
    pub fn mock() -> Self {
        Self::new(vec![vec![
            Ok(ModelEvent::TextStart {
                id: "mock-text".to_owned(),
                provider_metadata: None,
            }),
            Ok(ModelEvent::TextDelta {
                id: "mock-text".to_owned(),
                delta: "Mock BrowserOS test response.".to_owned(),
            }),
            Ok(ModelEvent::TextEnd {
                id: "mock-text".to_owned(),
                provider_metadata: None,
            }),
            Ok(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
                usage: Usage::default(),
            }),
        ]])
    }

    pub fn new(steps: Vec<Vec<Result<ModelEvent, ModelError>>>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
        }
    }
}

#[async_trait::async_trait]
impl LanguageModel for ScriptedModel {
    fn provider(&self) -> ProviderKind {
        ProviderKind::OpenAiCompatible
    }

    fn model_id(&self) -> &str {
        "scripted"
    }

    fn supports_media_in_tool_results(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelStream, ModelError> {
        let _ = request;
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let step = self
            .steps
            .lock()
            .map_err(|_| ModelError::Config("scripted model lock poisoned".to_owned()))?
            .pop_front()
            .ok_or_else(|| ModelError::Config("scripted model has no steps left".to_owned()))?;
        Ok(Box::pin(futures_util::stream::iter(step)))
    }
}
