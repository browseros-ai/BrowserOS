//! The chunk → `UIMessage` reducer: a port of the AI SDK's `processUIMessageStream`
//! (`ai/src/ui/process-ui-message-stream.ts`). The run registry feeds it every chunk of a turn,
//! from either engine, and persists the result. Implemented by P1; S0 ships the signature only.

use crate::{chunk::UiChunk, message::UiMessage};

/// Accumulates one assistant message: the current message plus open text/reasoning parts by id
/// and tool parts by call id.
#[derive(Debug, Default)]
pub struct UiMessageAssembler {
    message: Option<UiMessage>,
}

impl UiMessageAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one chunk. S0 stub: records nothing until P1 lands the reducer, so `message()`
    /// stays `None` and nothing can be persisted from a stub.
    pub fn push(&mut self, chunk: &UiChunk) {
        let _ = chunk;
    }

    /// `None` before the first `start` chunk.
    pub fn message(&self) -> Option<&UiMessage> {
        self.message.as_ref()
    }

    pub fn into_message(self) -> Option<UiMessage> {
        self.message
    }
}
