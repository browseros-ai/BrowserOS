use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, text_result,
};
use browseros_core::WindowId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "Manage browser windows: list, create, or close.";
const PROFILE_NOTE: &str = "This is separate from browser_profiles discovery/loading and browser_target task selection. Calls include normal windows of one regular profile only: the browser_target selection, or the original Default profile (directory \"Default\") when none is set. create opens an inactive normal window in that profile (an explicit selection must be loaded first); a strict pin rejects creation with pinned_window_restricts.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WindowsAction {
    #[default]
    List,
    Create,
    Close,
    // Intentionally omit Activate so MCP callers cannot steal window focus.
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WindowsArgs {
    #[serde(default)]
    action: WindowsAction,
    /// Window id for close.
    #[serde(rename = "windowId")]
    window_id: Option<i64>,
}

pub fn definition() -> crate::framework::ToolDef {
    crate::framework::ToolDef {
        feature_note: super::profile_note(PROFILE_NOTE),
        ..super::def::<WindowsArgs>(
            "windows",
            DESCRIPTION,
            Some(super::open_world_annotations()),
            handler,
        )
    }
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: WindowsArgs = parse_args(raw)?;
        let result = match args.action {
            WindowsAction::List => {
                let windows = ctx.browser.list_windows().await?;
                text_result(
                    format_window_list(&windows),
                    Some(json!({ "action": "list", "windows": windows, "count": windows.len() })),
                )
            }
            WindowsAction::Create => {
                let window = ctx.browser.create_window().await?;
                text_result(
                    format!(
                        "created window {}{}",
                        window.window_id,
                        profile_suffix(&window)
                    ),
                    Some(json!({ "action": "create", "window": window })),
                )
            }
            WindowsAction::Close => {
                let Some(window_id) = args.window_id else {
                    return Ok(Some(error_result("windows close: windowId is required.")));
                };
                ctx.browser.close_window(WindowId(window_id)).await?;
                text_result(
                    format!("closed window {window_id}"),
                    Some(json!({ "action": "close", "windowId": window_id })),
                )
            }
        };
        Ok(Some(result))
    })
}

fn format_window_list(windows: &[browseros_core::windows::WindowInfo]) -> String {
    if windows.is_empty() {
        return "No windows found.".to_string();
    }
    let mut lines = vec![format!("Found {} windows:", windows.len()), String::new()];
    for window in windows {
        let mut markers = Vec::new();
        if !window.is_visible {
            markers.push("NOT VISIBLE");
        }
        if window.is_active {
            markers.push("ACTIVE");
        }
        let suffix = if markers.is_empty() {
            String::new()
        } else {
            format!(" [{}]", markers.join(", "))
        };
        let profile = profile_suffix(window);
        lines.push(format!(
            "Window {} ({}, {} tabs{profile}){suffix}",
            window.window_id, window.window_type, window.tab_count
        ));
    }
    lines.join("\n")
}

fn profile_suffix(window: &browseros_core::windows::WindowInfo) -> String {
    // Window metadata is native protocol JSON; never interpret the legacy
    // browserContextId label as the saved-profile identity.
    serde_json::to_value(window)
        .ok()
        .and_then(|value| {
            value
                .get("browserProfileId")
                .and_then(Value::as_str)
                .map(|id| format!(", profile {id}"))
        })
        .unwrap_or_default()
}
