use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, text_result,
};
use browseros_core::WindowId;
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "Manage browser windows: list, create, close, or activate windows.";

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WindowsAction {
    #[default]
    List,
    Create,
    Close,
    Activate,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WindowsArgs {
    #[serde(default)]
    action: WindowsAction,
    /// Window id for close and activate.
    #[serde(rename = "windowId")]
    window_id: Option<i64>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<WindowsArgs>(
        "windows",
        DESCRIPTION,
        Some(super::open_world_annotations()),
        handler,
    )
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
                let windows = ctx.session.windows.list().await?;
                text_result(
                    format_window_list(&windows),
                    Some(json!({ "action": "list", "windows": windows, "count": windows.len() })),
                )
            }
            WindowsAction::Create => {
                let window = ctx.session.windows.create().await?;
                text_result(
                    format!("created window {}", window.window_id),
                    Some(json!({ "action": "create", "window": window })),
                )
            }
            WindowsAction::Close => {
                let Some(window_id) = args.window_id else {
                    return Ok(Some(error_result("windows close: windowId is required.")));
                };
                ctx.session.windows.close(WindowId(window_id)).await?;
                text_result(
                    format!("closed window {window_id}"),
                    Some(json!({ "action": "close", "windowId": window_id })),
                )
            }
            WindowsAction::Activate => {
                let Some(window_id) = args.window_id else {
                    return Ok(Some(error_result(
                        "windows activate: windowId is required.",
                    )));
                };
                ctx.session.windows.activate(WindowId(window_id)).await?;
                text_result(
                    format!("activated window {window_id}"),
                    Some(json!({ "action": "activate", "windowId": window_id })),
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
        lines.push(format!(
            "Window {} ({}, {} tabs){suffix}",
            window.window_id, window.window_type, window.tab_count
        ));
    }
    lines.join("\n")
}
