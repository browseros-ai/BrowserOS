//! Maps the MCP host's lifecycle events onto claw's single-source event catalog, so the
//! wire names stay in `events.rs` and every host event is validated like any other.

use super::{AnalyticsSink, events};
use browseros_mcp_host::seams::{HostAnalytics, HostEvent};
use serde_json::Value;
use std::sync::Arc;

pub struct HostAnalyticsBridge {
    sink: Arc<dyn AnalyticsSink>,
}

impl HostAnalyticsBridge {
    #[must_use]
    pub fn new(sink: Arc<dyn AnalyticsSink>) -> Arc<Self> {
        Arc::new(Self { sink })
    }
}

impl HostAnalytics for HostAnalyticsBridge {
    fn capture(&self, event: HostEvent, properties: Value) {
        let definition = match event {
            HostEvent::SessionStarted => events::AGENT_SESSION_STARTED,
            HostEvent::SessionTaskDeclared => events::AGENT_SESSION_TASK_DECLARED,
            HostEvent::SessionToolUsage => events::AGENT_SESSION_TOOL_USAGE,
            HostEvent::SessionEnded => events::AGENT_SESSION_ENDED,
        };
        self.sink.capture(definition, properties);
    }
}
