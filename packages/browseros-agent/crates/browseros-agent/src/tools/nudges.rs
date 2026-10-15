//! `suggest_schedule` / `suggest_app_connection` (port of `agent/nudge-tools.ts`): UI-control
//! sentinel tools. Their output is `{content:[{type:'text', text:<JSON>}], isError:false}` and the
//! side panel renders a card from that JSON (`getMessageSegments.ts`), so the sentinel's key order
//! and texts are part of the app contract. `Agent::build` drops both tools for scheduled runs and
//! chat mode, exactly as TS deletes them from the merged tool set.

use super::{ToolCallError, ToolKind, ToolOutput, ToolSource, ToolSourceError, ToolSpec};
use crate::messages::ToolResultOutput;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const SUGGEST_SCHEDULE: &str = "suggest_schedule";
pub const SUGGEST_APP_CONNECTION: &str = "suggest_app_connection";

const SUGGEST_SCHEDULE_DESCRIPTION: &str = "Call this to suggest scheduling a task. Use in two cases: (1) MANDATORY after completing a task that could run on a recurring schedule (news, monitoring, reports, price tracking, data gathering). (2) Immediately when the user explicitly asks to schedule, automate, or repeat the current task - do NOT ask for clarification, infer all parameters from context. Do NOT call if the task requires real-time user interaction.";

/// `connector_names` is `getConnectorCatalog()` in catalog order; it is enumerated in the
/// `suggest_app_connection` description so the model picks an exact app name.
pub fn nudge_tools(connector_names: Vec<String>) -> Arc<dyn ToolSource> {
    Arc::new(NudgeTools {
        app_names: connector_names.join(", "),
    })
}

struct NudgeTools {
    app_names: String,
}

#[async_trait::async_trait]
impl ToolSource for NudgeTools {
    fn label(&self) -> &str {
        "nudges"
    }

    async fn tools(&self) -> Result<Vec<ToolSpec>, ToolSourceError> {
        Ok(vec![
            ToolSpec {
                name: SUGGEST_SCHEDULE.to_owned(),
                description: SUGGEST_SCHEDULE_DESCRIPTION.to_owned(),
                input_schema: json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The original user query to schedule",
                        },
                        "suggestedName": {
                            "type": "string",
                            "description": "A short, descriptive name for the scheduled task (e.g. \"Morning News Briefing\")",
                        },
                        "scheduleType": {
                            "type": "string",
                            "enum": ["daily", "hourly"],
                            "description": "How often the task should run",
                        },
                        "scheduleTime": {
                            "description": "Suggested time for daily tasks in HH:MM format (e.g. \"09:00\").",
                            "type": "string",
                        },
                    },
                    "required": ["query", "suggestedName", "scheduleType"],
                    "additionalProperties": false,
                }),
                kind: ToolKind::Static,
            },
            ToolSpec {
                name: SUGGEST_APP_CONNECTION.to_owned(),
                description: format!(
                    "BLOCKING DECISION - Call when the user's request relates to a Connect Apps service but you don't have MCP tools for it. Your response must contain ONLY this tool call with zero text. The appName must be one of: {}.",
                    self.app_names
                ),
                input_schema: json!({
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "object",
                    "properties": {
                        "appName": {
                            "type": "string",
                            "description": "The name of the app to connect (must match a supported app name exactly)",
                        },
                        "reason": {
                            "type": "string",
                            "description": "A brief, user-friendly explanation of why connecting this app would help",
                        },
                    },
                    "required": ["appName", "reason"],
                    "additionalProperties": false,
                }),
                kind: ToolKind::Static,
            },
        ])
    }

    /// Inputs arrive already validated against the schemas above (the loop validates static
    /// tools), so missing optional fields are the only shape variation left.
    async fn call(
        &self,
        name: &str,
        input: Value,
        _cancel: CancellationToken,
    ) -> Result<ToolOutput, ToolCallError> {
        let field = |key: &str| input.get(key).cloned().unwrap_or(Value::Null);
        let text = match name {
            SUGGEST_SCHEDULE => {
                let schedule_time = input
                    .get("scheduleTime")
                    .cloned()
                    .unwrap_or_else(|| Value::from("09:00"));
                super::ordered_json_object(&[
                    ("type", &Value::from("schedule_suggestion")),
                    ("query", &field("query")),
                    ("suggestedName", &field("suggestedName")),
                    ("scheduleType", &field("scheduleType")),
                    ("scheduleTime", &schedule_time),
                ])
            }
            SUGGEST_APP_CONNECTION => super::ordered_json_object(&[
                ("type", &Value::from("app_connection")),
                ("appName", &field("appName")),
                ("reason", &field("reason")),
            ]),
            other => return Err(ToolCallError(format!("Unknown nudge tool: {other}"))),
        };
        Ok(ToolOutput {
            ui: json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
            // `toModelOutput`: the sentinel text itself, not the CallToolResult wrapper.
            model: ToolResultOutput::Text { value: text },
        })
    }

    async fn close(&self) {}
}
