//! Prompt text: the system prompt (8 sections of TS `prompt.ts`, verbatim, rendered after tools
//! are known), the user-message wrapper (`format-message.ts`), and the `[Context: …]` notices
//! emitted when a session's MCP set, workspace or mode changes (`chat-service.helpers.ts`).
//! Implemented by P6; S0 stubs return empty/unchanged text.

use std::{collections::BTreeSet, path::PathBuf};

/// The browser state a turn runs against, with page ids already resolved by the server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BrowserContextView {
    pub window_id: Option<i64>,
    pub active_tab: Option<TabView>,
    pub selected_tabs: Vec<TabView>,
    pub enabled_mcp_servers: Vec<String>,
    pub custom_mcp_urls: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TabView {
    pub id: i64,
    pub page_id: Option<u32>,
    pub url: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    #[default]
    Sidepanel,
    Newtab,
}

#[derive(Debug, Clone, Default)]
pub struct PromptInputs {
    pub chat_mode: bool,
    pub is_scheduled_task: bool,
    pub origin: Origin,
    pub workspace_dir: Option<PathBuf>,
    pub connected_apps: Vec<String>,
    pub declined_apps: Vec<String>,
    pub user_system_prompt: Option<String>,
    pub active_tab_page_id: Option<u32>,
}

/// `"<AGENT_PROMPT>\n" + sections.join("\n\n") + "\n</AGENT_PROMPT>"`. S0 stub: no sections.
pub fn system_prompt(inputs: &PromptInputs, tool_names: &BTreeSet<String>) -> String {
    let _ = (inputs, tool_names);
    "<AGENT_PROMPT>\n\n</AGENT_PROMPT>".to_owned()
}

/// `formatUserMessage`: browser-context wrapper + selected text + `<USER_QUERY>`. S0 stub:
/// the message unchanged.
pub fn format_user_message(
    message: &str,
    ctx: Option<&BrowserContextView>,
    selected_text: Option<&str>,
    selected_text_source: Option<(&str, &str)>,
) -> String {
    let _ = (ctx, selected_text, selected_text_source);
    message.to_owned()
}

/// The inputs whose change forces a session rebuild, and the notices describing that change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionInputs {
    pub mcp_server_key: String,
    pub working_dir: Option<PathBuf>,
    pub chat_mode: bool,
}

/// S0 stub: no notices.
pub fn context_notices(prev: &SessionInputs, next: &SessionInputs) -> Vec<String> {
    let _ = (prev, next);
    Vec::new()
}

/// Cache key of the session's MCP set (enabled servers, custom URLs, Klavis state). S0 stub.
pub fn mcp_server_key(ctx: Option<&BrowserContextView>, klavis_state: Option<&str>) -> String {
    let _ = (ctx, klavis_state);
    String::new()
}
