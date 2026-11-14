//! Pure reply shaping. Preserves the SDK's content, structured value and metadata
//! verbatim, keeping presentation separate from wire parsing.

use rmcp::model::CallToolResult;

use super::AgentReply;

pub(super) fn shape(reply: CallToolResult) -> AgentReply {
    reply
}
