//! The SDK tool family for one MCP connection. Session admission and completion stay in the host.

mod after_run;
mod before_run;
mod prompt;
mod recipes;
pub(super) mod run;
pub(super) mod surface;

use super::{
    AGENT_NAME_ARG, AGENT_NAME_ARG_DESCRIPTION, AGENT_NAME_TASK_SENTENCE, AWAIT_HELP_TOOL_NAME,
    ClawMcpService, PURSUE_TOOL_NAME, REQUEST_HELP_TOOL_NAME, TOOL_LIST_WATCH_LIVENESS,
    await_help_tool, pursue, request_help_tool, session_handle, tab_grouping,
};
use crate::{
    AppState,
    services::neo::{EngineState, NeoEngine, RunTool},
};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool},
    service::{Peer, RoleServer},
};
use serde_json::{Value, json};
use std::{future::pending, sync::Arc};
use surface::{Admission, Surface};
use tokio::sync::watch;

const SESSION_DESCRIPTION: &str = "Opaque logical task handle. When a client has no transport session, the server returns it in every result. Pass it as session on later calls to keep the same SDK task. Find its pages with context.pages() and page.taskId. Omit it on the first call or to start a separate task after Stop; a stopped handle stays stopped.";
pub(in crate::api::mcp::service) const REQUEST_HELP_DESCRIPTION: &str = "Ask a human to take over a tab for sign-in, a one-time code, a captcha, an account choice, or an approval. Pass page.tabId from a run script as page, a short reason, and optionally details, resumeHint and kind. This waits briefly and returns the same tab id as page. If status is waiting, call await_human_help until resolved, cancelled or timed_out. Do nothing else on that tab while waiting.";
const AGENT_NAME_TASK_SENTENCE_PLAYWRIGHT: &str =
    "Name your task with neo.createTask(name) in run.";

const TOOLS: [&str; 4] = [
    "run",
    REQUEST_HELP_TOOL_NAME,
    AWAIT_HELP_TOOL_NAME,
    PURSUE_TOOL_NAME,
];

/// Offers the SDK run and the host tools that understand native tab ids.
pub(super) struct Playwright {
    request_help_tool: Tool,
    await_help_tool: Tool,
}

/// Admission runs before session resolution so a refusal creates no task.
pub(super) enum Admitted {
    Reply(CallToolResult),
    Run,
    Shared,
}

impl Playwright {
    pub(super) fn for_state(state: &AppState) -> Option<Self> {
        state.config.serves_playwright_run().then(|| {
            let mut request_help_tool = request_help_tool();
            request_help_tool.description = Some(REQUEST_HELP_DESCRIPTION.into());
            Arc::make_mut(&mut request_help_tool.input_schema)["properties"]["page"]["description"] =
                json!("The native tab id from page.tabId in a run script. The cockpit brings exactly this tab to the front.");
            let mut await_help_tool = await_help_tool();
            Arc::make_mut(&mut await_help_tool.input_schema)["properties"]["requestId"]["description"] =
                json!("The requestId returned by request_human_help. Optional; the server correlates this request by session.");
            Self {
                request_help_tool,
                await_help_tool,
            }
        })
    }

    /// Discovery can wait briefly for the definition while initialize stays independent of startup.
    pub(super) async fn ready(&self, state: &AppState) {
        let _ = state.neo.tool().await;
    }

    pub(super) fn listed_tools(
        &self,
        host: &ClawMcpService,
        declare_agent_name: bool,
    ) -> Vec<Tool> {
        self.listed_tools_for(host, declare_agent_name, Surface::for_host(host))
    }

    pub(in crate::api::mcp::service) fn listed_tools_for(
        &self,
        host: &ClawMcpService,
        declare_agent_name: bool,
        surface: Surface,
    ) -> Vec<Tool> {
        TOOLS
            .into_iter()
            .filter_map(|name| self.offered_tool(host, &surface, name))
            .map(|mut tool| {
                if declare_agent_name {
                    let description = AGENT_NAME_ARG_DESCRIPTION.replace(
                        AGENT_NAME_TASK_SENTENCE,
                        AGENT_NAME_TASK_SENTENCE_PLAYWRIGHT,
                    );
                    Arc::make_mut(&mut tool.input_schema)["properties"][AGENT_NAME_ARG] =
                        json!({ "type": "string", "description": description });
                }
                tab_grouping::tool(tool, host.tab_grouping())
            })
            .collect()
    }

    pub(super) fn get_tool(&self, host: &ClawMcpService, name: &str) -> Option<Tool> {
        self.offered_tool(host, &Surface::for_host(host), name)
    }

    fn offered_tool(&self, host: &ClawMcpService, surface: &Surface, name: &str) -> Option<Tool> {
        if !surface.listed(name) {
            return None;
        }
        let tool = match name {
            "run" => {
                let snapshot = host.state.neo.tool_snapshot();
                let run = Tool::new("run", snapshot.description, snapshot.input_schema);
                if surface.listed(PURSUE_TOOL_NAME) {
                    pursue::with_pursue_first(run)
                } else {
                    run
                }
            }
            REQUEST_HELP_TOOL_NAME => self.request_help_tool.clone(),
            AWAIT_HELP_TOOL_NAME => self.await_help_tool.clone(),
            PURSUE_TOOL_NAME => playwright_pursue_tool(),
            _ => return None,
        };
        Some(session_handle::offer_described(tool, SESSION_DESCRIPTION))
    }

    pub(super) fn admit(&self, host: &ClawMcpService, request: &CallToolRequestParams) -> Admitted {
        match Surface::for_host(host).admission(&request.name) {
            Admission::Call if request.name == "run" => Admitted::Run,
            Admission::Call => Admitted::Shared,
            Admission::Refuse(message) => {
                Admitted::Reply(CallToolResult::error(vec![ContentBlock::text(message)]))
            }
            Admission::PursueHandback => {
                let args = Value::Object(request.arguments.clone().unwrap_or_default());
                Admitted::Reply(surface::gate_off(&args).into_call_tool_result())
            }
        }
    }

    pub(super) fn watch_run(&self, state: &AppState, peer: Peer<RoleServer>) {
        let mut definition = RunDefinition::watch(Some(self), state);
        tokio::spawn(async move {
            let mut liveness = tokio::time::interval(TOOL_LIST_WATCH_LIVENESS);
            liveness.tick().await;
            loop {
                tokio::select! {
                    changed = definition.changed() => {
                        if changed.is_err()
                            || peer.is_transport_closed()
                            || peer.notify_tool_list_changed().await.is_err()
                        {
                            return;
                        }
                    }
                    _ = liveness.tick() => {
                        if peer.is_transport_closed() {
                            return;
                        }
                    }
                }
            }
        });
    }
}

fn playwright_pursue_tool() -> Tool {
    let mut tool = pursue::pursue_tool();
    Arc::make_mut(&mut tool.input_schema)["properties"]["page"]["description"] = json!(
        "The native tab id from page.tabId in a run script. The returned page uses this same id."
    );
    if let Some(schema) = &mut tool.output_schema {
        Arc::make_mut(schema)["properties"]["pagesRead"]["description"] =
            json!("Page states inspected by neo without sending their contents to you.");
    }
    tool
}

/// Advertise definition changes on both MCP transports. Readiness changes alone need no notice.
pub(super) struct RunDefinition(Option<WatchedDefinition>);

struct WatchedDefinition {
    engine: NeoEngine,
    state: watch::Receiver<EngineState>,
    tool: RunTool,
}

impl RunDefinition {
    pub(super) fn watch(playwright: Option<&Playwright>, state: &AppState) -> Self {
        Self(playwright.map(|_| WatchedDefinition {
            engine: state.neo.clone(),
            state: state.neo.watch(),
            tool: state.neo.tool_snapshot(),
        }))
    }

    pub(super) async fn changed(&mut self) -> Result<(), watch::error::RecvError> {
        let Some(watched) = &mut self.0 else {
            // Catalog subscriptions can wait for ultrafast without subscribing to the engine.
            return pending().await;
        };
        loop {
            watched.state.changed().await?;
            let current = watched.engine.tool_snapshot();
            if current.description != watched.tool.description
                || current.input_schema != watched.tool.input_schema
            {
                watched.tool = current;
                return Ok(());
            }
        }
    }
}
