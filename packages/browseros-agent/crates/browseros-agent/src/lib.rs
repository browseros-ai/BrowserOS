//! The built-in BrowserOS agent: a BrowserOS-owned tool loop over providers normalised by
//! rig-core (adapter only; no rig type is public). Input is the conversation's display history;
//! output is the turn's UI chunk stream (`ui_message_stream::ChunkStream`).
//!
//! ```text
//!  apps/server-rust BrowserOsEngine ──AgentConfig──► Agent::build ──► tools::ToolSource (MCP over
//!        loopback /mcp + lease, custom MCP, filesystem, nudges)
//!                                   ──TurnInput────► Agent::run_turn ──► messages::ui_to_model
//!                                                     │  per step: normalize_for_model → compaction::prepare_step
//!                                                     └► llm::LanguageModel::stream ──► UiChunk stream
//! ```

mod agent;
pub mod compaction;
pub mod llm;
pub mod messages;
pub mod prompt;
pub mod tools;

pub use agent::{Agent, AgentBuildError, AgentConfig, ModelInfo, TurnInput};
