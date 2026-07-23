pub mod agent_session_ends;
pub mod agent_session_starts;
pub mod recording_batches;
pub mod recording_payloads;
pub mod recording_streams;
pub mod session_efficiency_stats;
pub mod session_tabs;
pub mod tab_claims;
pub mod tab_recordings;
pub mod tasks;
pub mod tool_dispatches;

pub mod prelude {
    pub use super::agent_session_ends::Entity as AgentSessionEnds;
    pub use super::agent_session_starts::Entity as AgentSessionStarts;
    pub use super::recording_batches::Entity as RecordingBatches;
    pub use super::recording_payloads::Entity as RecordingPayloads;
    pub use super::recording_streams::Entity as RecordingStreams;
    pub use super::session_efficiency_stats::Entity as SessionEfficiencyStats;
    pub use super::session_tabs::Entity as SessionTabs;
    pub use super::tab_claims::Entity as TabClaims;
    pub use super::tab_recordings::Entity as TabRecordings;
    pub use super::tasks::Entity as Tasks;
    pub use super::tool_dispatches::Entity as ToolDispatches;
}
