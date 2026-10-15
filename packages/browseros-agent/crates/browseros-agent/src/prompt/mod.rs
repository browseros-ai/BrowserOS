//! Prompt rendering follows the TS server's eight ordered sections and per-turn wrappers.
//! Session-change notices describe rebuilt inputs without rewriting the durable user text;
//! browser page ids arrive resolved by the server before rendering begins.
mod sections;
use crate::messages::js_trim;
use std::{collections::BTreeSet, path::PathBuf};

/// Browser state for a turn, with Chromium tab ids and MCP page ids kept distinct.
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
/// All user/session inputs needed to render instructions, after the actual toolset is known.
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

/// Sections and whitespace are copied verbatim. The actual toolset gates workspace
/// instructions and determines whether output-only filesystem reads are available.
pub fn system_prompt(inputs: &PromptInputs, tool_names: &BTreeSet<String>) -> String {
    let workspace = inputs
        .workspace_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty() && tool_names.contains("filesystem_write"));
    let has_workspace = workspace.is_some() && !inputs.chat_mode;
    let mut role = if has_workspace {
        sections::ROLE_WORKSPACE
    } else {
        sections::ROLE_NO_WORKSPACE
    }
    .to_owned();
    if inputs.is_scheduled_task {
        role.push_str("\n\nYou are running as a scheduled background task on a system-managed page opened in the background. Complete the task autonomously and report results.");
    } else if inputs.chat_mode {
        role.push_str("\n\nYou are in read-only chat mode. You can observe pages but cannot interact with them or modify files.");
    }
    let role = format!("<role>\n{role}\n</role>");
    let execution = format!(
        "{}{}{}",
        sections::EXECUTION_START,
        if inputs.origin == Origin::Newtab {
            sections::EXECUTION_NEWTAB
        } else {
            ""
        },
        sections::EXECUTION_END
    );
    let connected = if inputs.connected_apps.is_empty() {
        "No apps are currently connected via Strata.".into()
    } else {
        format!(
            "Connected apps (use Strata for these): {}.",
            inputs.connected_apps.join(", ")
        )
    };
    let declined = if inputs.declined_apps.is_empty() {
        String::new()
    } else {
        format!(
            " Declined apps (use browser automation, never Strata): {}.",
            inputs.declined_apps.join(", ")
        )
    };
    let integrations = sections::INTEGRATIONS.replace(
        "${connectedList}${declinedNote}",
        &format!("{connected}{declined}"),
    );
    let mut parts = vec![role, sections::SECURITY.into(), execution, integrations];
    if has_workspace && let Some(path) = workspace {
        parts.push(sections::WORKSPACE.replace("${options.workspaceDir}", &path.to_string_lossy()));
    }
    // TS excludes the nudge section at agent construction for scheduled/read-only turns.
    if !inputs.is_scheduled_task && !inputs.chat_mode {
        parts.push(sections::NUDGES.into());
    }
    let mut style = sections::STYLE.to_owned();
    if !has_workspace {
        style.push_str(if tool_names.contains("filesystem_read") {
            sections::STYLE_OUTPUT_READ
        } else {
            sections::STYLE_NO_WORKSPACE
        });
    }
    style.push_str("\n</style>");
    parts.push(style);
    let mut user_context = Vec::new();
    if let Some(text) = &inputs.user_system_prompt {
        // The JS regexp removes template lines containing 'your', case-insensitively,
        // including surrounding ECMAScript whitespace, then trims the remaining text.
        let cleaned = text
            .split('\n')
            .filter(|line| {
                let trimmed = js_trim(line);
                !(trimmed.starts_with('[')
                    && trimmed.ends_with(']')
                    && !trimmed.contains(['\r', '\u{2028}', '\u{2029}'])
                    && trimmed.to_lowercase().contains("your"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cleaned = js_trim(&cleaned);
        if !cleaned.is_empty() {
            user_context.push(format!(
                "<user_preferences>\n{cleaned}\n</user_preferences>"
            ));
        }
    }
    if !inputs.chat_mode {
        let mut page = "<page_context>\nUse the page ID from the Browser Context directly as your starting page; do not call `tabs` action=\"list\" to find it.".to_owned();
        if inputs.is_scheduled_task {
            let page_ref = inputs
                .active_tab_page_id
                .filter(|id| *id != 0)
                .map(|id| format!("`{id}`"))
                .unwrap_or_else(|| "the page ID from the Browser Context".into());
            page.push_str(&format!("\nThis is a scheduled background task on a system-managed page. Use starting page ID {page_ref} directly; for extra browsing use `tabs` action=\"new\" (background=true). Do NOT close your starting page or create windows. Close extra background pages when done. Complete the task end-to-end and report results."));
        }
        page.push_str("\n</page_context>");
        user_context.push(page);
    }
    if !user_context.is_empty() {
        parts.push(user_context.join("\n\n"));
    }
    format!("<AGENT_PROMPT>\n{}\n</AGENT_PROMPT>", parts.join("\n\n"))
}

pub fn format_user_message(
    message: &str,
    ctx: Option<&BrowserContextView>,
    selected_text: Option<&str>,
    selected_text_source: Option<(&str, &str)>,
) -> String {
    let mut output = String::new();
    if let Some(ctx) = ctx.filter(|ctx| ctx.active_tab.is_some() || !ctx.selected_tabs.is_empty()) {
        output.push_str("## Browser Context\n");
        if let Some(id) = ctx.window_id {
            output.push_str(&format!("**Window ID:** {id}\n"));
        }
        if let Some(tab) = &ctx.active_tab {
            output.push_str(&format!("**Active Tab:** {}\n", format_tab(tab)));
        }
        if !ctx.selected_tabs.is_empty() {
            output.push_str(&format!(
                "**Selected Tabs ({}):**\n",
                ctx.selected_tabs.len()
            ));
            for (index, tab) in ctx.selected_tabs.iter().enumerate() {
                output.push_str(&format!("  {}. {}\n", index + 1, format_tab(tab)));
            }
        }
        output.push_str("\n---\n\n");
    }
    if let Some(text) = selected_text.filter(|text| !text.is_empty()) {
        let (url, title) = selected_text_source.unwrap_or(("", ""));
        let title = sanitize(title).replace('"', "'");
        let url = sanitize(url);
        let source = if title.is_empty() {
            String::new()
        } else {
            format!(
                " (from \"{title}\"{})",
                if url.is_empty() {
                    String::new()
                } else {
                    format!(" — {url}")
                }
            )
        };
        output.push_str(&format!(
            "<selected_text{source}>\n{}\n</selected_text>\n\n",
            sanitize(text)
        ));
    }
    // User query and browser labels are deliberately not sanitized: TS only fences selected
    // text and its source. Expanding sanitization would silently change the stored contract.
    output.push_str(&format!("<USER_QUERY>\n{message}\n</USER_QUERY>"));
    output
}
fn format_tab(tab: &TabView) -> String {
    let mut output = format!("Tab {}", tab.id);
    if let Some(id) = tab.page_id {
        output.push_str(&format!(" (Page ID: {id})"));
    }
    if let Some(title) = tab.title.as_deref().filter(|title| !title.is_empty()) {
        output.push_str(&format!(" - \"{title}\""));
    }
    if let Some(url) = tab.url.as_deref().filter(|url| !url.is_empty()) {
        output.push_str(&format!(" ({url})"));
    }
    output
}
fn sanitize(text: &str) -> String {
    let mut output = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let body = candidate[1..].strip_prefix('/').unwrap_or(&candidate[1..]);
        let matches = [
            "selected_text",
            "USER_QUERY",
            "page_context",
            "AGENT_PROMPT",
            "soul",
            "security",
            "workspace",
        ]
        .iter()
        .any(|tag| {
            body.get(..tag.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(tag))
        });
        if matches && let Some(end) = candidate.find('>') {
            rest = &candidate[end + 1..];
        } else {
            output.push('<');
            rest = &candidate[1..];
        }
    }
    output.push_str(rest);
    output
}

/// Rebuild-trigger inputs; notices compare the snapshot before rebuild with the new inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionInputs {
    pub mcp_server_key: String,
    pub working_dir: Option<PathBuf>,
    pub chat_mode: bool,
}
pub fn context_notices(prev: &SessionInputs, next: &SessionInputs) -> Vec<String> {
    let mut notices = Vec::new();
    if prev.mcp_server_key != next.mcp_server_key {
        notices.push(mcp_change(&prev.mcp_server_key, &next.mcp_server_key));
    }
    // PathBuf equality normalizes repeated separators and '.', whereas TS compares strings.
    // Compare OS strings so a change triggers the same rebuild notice on either server.
    if prev.working_dir.as_ref().map(|path| path.as_os_str())
        != next.working_dir.as_ref().map(|path| path.as_os_str())
    {
        notices.push(workspace_change(
            prev.working_dir.as_ref(),
            next.working_dir.as_ref(),
            next.chat_mode,
        ));
    }
    if prev.chat_mode != next.chat_mode {
        notices.push(
            mode_change(
                next.chat_mode,
                next.working_dir
                    .as_ref()
                    .is_some_and(|path| !path.as_os_str().is_empty()),
            )
            .into(),
        );
    }
    notices
}
fn mcp_change(before: &str, after: &str) -> String {
    let old = before
        .split(',')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let new = after
        .split(',')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    // JS Set retains insertion order; a BTreeSet alone would reorder notice text.
    let servers = |parts: &[&str]| {
        let mut seen = BTreeSet::new();
        parts
            .iter()
            .filter(|part| !part.starts_with("klavis:") && seen.insert(**part))
            .map(|part| (*part).to_owned())
            .collect::<Vec<_>>()
    };
    let old_servers = servers(&old);
    let new_servers = servers(&new);
    let removed = old_servers
        .iter()
        .filter(|part| !new_servers.contains(part))
        .cloned()
        .collect::<Vec<_>>();
    let added = new_servers
        .iter()
        .filter(|part| !old_servers.contains(part))
        .cloned()
        .collect::<Vec<_>>();
    let mut parts = Vec::new();
    if !removed.is_empty() {
        parts.push(format!("The following app integrations were disconnected: {}. Their tools are no longer available.", removed.join(", ")));
    }
    if !added.is_empty() {
        parts.push(format!(
            "The following app integrations were connected: {}. Their tools are now available.",
            added.join(", ")
        ));
    }
    if parts.is_empty() {
        if old.iter().find(|part| part.starts_with("klavis:")) != Some(&"klavis:ready")
            && new.iter().find(|part| part.starts_with("klavis:")) == Some(&"klavis:ready")
            && !new_servers.is_empty()
        {
            parts.push(format!("Klavis app integration tools are now available for the following connected apps: {}.", new_servers.join(", ")));
        } else {
            parts.push("Connected app integrations changed during this conversation. Use only tools that are currently registered.".into());
        }
    }
    parts.join(" ")
}
fn workspace_change(before: Option<&PathBuf>, after: Option<&PathBuf>, chat_mode: bool) -> String {
    let after = after.filter(|path| !path.as_os_str().is_empty());
    let before = before.filter(|path| !path.as_os_str().is_empty());
    match (before, after, chat_mode) {
        (_, None, _) => "The user disconnected the workspace during this conversation. Workspace filesystem tools (filesystem_write, filesystem_edit, filesystem_bash, filesystem_grep, filesystem_find, filesystem_ls, and workspace file reads) are no longer available. filesystem_read can only read BrowserOS-generated output files returned in this session. Return other output directly in chat. If the user asks for file operations, suggest they select a working directory from the chat toolbar.".into(),
        (None, Some(_), true) => "The user connected a workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools. filesystem_read can only read BrowserOS-generated output files returned in this session.".into(),
        (Some(_), Some(_), true) => "The user switched workspace during this conversation, but read-only chat mode cannot use workspace filesystem tools. filesystem_read can only read BrowserOS-generated output files returned in this session.".into(),
        (None, Some(path), false) => format!("The user connected a workspace during this conversation. Filesystem tools are now available. Working directory: {}", path.to_string_lossy()),
        (Some(_), Some(path), false) => format!("The user switched workspace during this conversation. Filesystem tools now use the new working directory: {}", path.to_string_lossy()),
    }
}
fn mode_change(chat_mode: bool, workspace: bool) -> &'static str {
    match (chat_mode, workspace) {
        (true, true) => {
            "The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them, and workspace filesystem tools other than reading BrowserOS output files are no longer available."
        }
        (true, false) => {
            "The user switched to read-only chat mode during this conversation. You can observe pages but can no longer interact with them or modify files."
        }
        (false, true) => {
            "The user switched to agent mode during this conversation. You can interact with pages and perform browser actions again, and the workspace filesystem tools are available again. Any earlier statement that read-only chat mode blocked them no longer applies."
        }
        (false, false) => {
            "The user switched to agent mode during this conversation. You can interact with pages and perform browser actions again."
        }
    }
}
/// Sorting uses UTF-16 units to match JavaScript Array.sort, including non-BMP app names.
pub fn mcp_server_key(ctx: Option<&BrowserContextView>, klavis_state: Option<&str>) -> String {
    let Some(ctx) = ctx else {
        return String::new();
    };
    let mut managed = ctx.enabled_mcp_servers.clone();
    let mut custom = ctx.custom_mcp_urls.clone();
    managed.sort_by_cached_key(|text| text.encode_utf16().collect::<Vec<_>>());
    custom.sort_by_cached_key(|text| text.encode_utf16().collect::<Vec<_>>());
    let mut parts = Vec::new();
    if !managed.is_empty() {
        parts.push(format!("klavis:{}", klavis_state.unwrap_or("disabled")));
    }
    parts.extend(managed);
    parts.extend(custom);
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}
