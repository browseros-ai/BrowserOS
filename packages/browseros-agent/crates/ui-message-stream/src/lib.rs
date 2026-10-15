//! AI SDK v7 UI message stream: the one output vocabulary of both chat engines, the replay
//! buffer's element type, and the reducer that turns a turn's chunks into the persisted assistant
//! `UIMessage`.
//!
//! Engines (`browseros-agent`, `acp-agents`) emit [`UiChunk`]s; `apps/server-rust` buffers them per
//! run, frames them as SSE ([`sse`]) and folds them into a [`UiMessage`] ([`UiMessageAssembler`])
//! for persistence. This crate knows nothing about agents, providers or HTTP routing.

pub mod assemble;
pub mod chunk;
pub mod message;
pub mod sse;

pub use assemble::UiMessageAssembler;
pub use chunk::{DataChunk, FinishReason, UiChunk};
pub use message::{
    FilePart, PartState, ReasoningPart, Role, TextPart, ToolPart, ToolPartKind, ToolState,
    UiMessage, UiPart,
};

/// A turn's chunks in emission order. Ends when the turn ends; failures inside a turn are
/// `error` chunks, never stream errors, so consumers never need a `Result` item.
pub type ChunkStream =
    std::pin::Pin<Box<dyn futures_core::Stream<Item = UiChunk> + Send + 'static>>;
