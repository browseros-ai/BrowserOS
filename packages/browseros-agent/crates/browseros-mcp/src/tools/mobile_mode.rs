use crate::framework::{
    ArgIssue, ToolCtx, ToolError, ToolExecResult, ToolResult, parse_args, text_result,
};
use browseros_core::{
    PageId,
    mobile_mode::{
        DESKTOP_HEIGHT, DESKTOP_WIDTH, MOBILE_HEIGHT, MOBILE_WIDTH, MobileMode, MobileModeArgError,
        MobileModeOutcome, MobileModeRequest, ModeKind, WindowBounds, WindowOutcome,
    },
};
use futures_util::future::BoxFuture;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use serde_json::{Value, json};
use std::borrow::Cow;

const DESCRIPTION: &str = "\
Set one existing tab's viewport mode. mobile: Android Chrome user agent and client hints, \
touch input, and a fixed 400x906 CSS px viewport at 2x density. desktop: a fixed 1920x1080 \
CSS px viewport at 1x with the browser's own identity and no touch. freesize: none of these \
overrides; the tab uses its window's own viewport. Tabs this server attaches to start in \
desktop. In mobile or desktop, viewport commands sent through page.cdp or browser.cdp are \
refused; switch to freesize first to change the viewport yourself. \
zoom (mobile only, 25-400, default 100) is the DevTools device-preview scale and only changes \
how large the tab draws the page; snapshots, screenshots, refs and act coordinates stay in \
CSS px. The mode survives reloads, navigation and tab switches in this tab. \
It affects everyone using this tab, including a tab the user is viewing. \
It never reloads: the loaded document keeps its server-chosen content until you reload it \
(reloadRequired). New tabs, popups, cross-site iframes and workers are not covered by mobile. \
Raw page.cdp Input coordinates are in zoomed view space (CSS px x zoom/100). \
resizeWindow:true (mobile only, default false) also resizes this tab's whole Chrome window, \
shared by every tab in it, so its content area fits the zoomed view. The window keeps its tab \
strip, toolbar and frame, so its size is not the viewport size; the result reports the actual \
bounds and any axis Chrome's minimum size or the screen limited. A maximized or fullscreen \
window is left alone. Leaving mobile restores the window unless something resized it since.";

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ModeArg {
    Mobile,
    Desktop,
    Freesize,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ToggleModeArgs {
    /// Page id from `tabs`.
    #[schemars(range(min = 1))]
    page: u32,
    /// mobile: fixed 400x906 at 2x; desktop: fixed 1920x1080 at 1x; freesize: the tab's own.
    mode: ModeArg,
    /// Device-preview zoom in percent (default 100): 150 draws the page at 1.5x. Mobile only.
    #[schemars(range(min = 25, max = 400))]
    zoom: Option<f64>,
    /// Also fit this tab's whole Chrome window to the zoomed view. Default false. Mobile only.
    #[serde(rename = "resizeWindow")]
    resize_window: Option<bool>,
}

/// The wire shape of `structured`. Hosts keep structured data only for tools that declare an
/// output schema, and callers need `changed`, `reloadRequired` and the window bounds as data.
struct ToggleModeOutput;

impl JsonSchema for ToggleModeOutput {
    fn schema_name() -> Cow<'static, str> {
        "ToggleModeOutput".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        let size = json!({
            "type": "object",
            "properties": { "width": { "type": "integer" }, "height": { "type": "integer" } },
            "required": ["width", "height"]
        });
        json_schema!({
            "type": "object",
            "properties": {
                "page": { "type": "integer", "minimum": 1 },
                "mode": { "type": "string", "enum": ["mobile", "desktop", "freesize"] },
                "zoom": { "type": "number", "minimum": 25, "maximum": 400 },
                "viewport": {
                    "type": "object",
                    "properties": {
                        "width": { "type": "integer" },
                        "height": { "type": "integer" },
                        "deviceScaleFactor": { "type": "number" }
                    },
                    "required": ["width", "height", "deviceScaleFactor"]
                },
                "visibleSize": size,
                "changed": { "type": "boolean" },
                "reloadRequired": { "type": "boolean" },
                "window": {
                    "type": "object",
                    "properties": {
                        "windowId": { "type": "integer" },
                        "resized": { "type": "boolean" },
                        "restored": { "type": "boolean" },
                        "bounds": {
                            "type": "object",
                            "properties": {
                                "left": { "type": "integer" },
                                "top": { "type": "integer" },
                                "width": { "type": "integer" },
                                "height": { "type": "integer" }
                            },
                            "required": ["left", "top", "width", "height"]
                        },
                        "requestedContentSize": size,
                        "contentSizeExact": {
                            "type": "object",
                            "properties": {
                                "width": { "type": "boolean" },
                                "height": { "type": "boolean" }
                            },
                            "required": ["width", "height"]
                        },
                        "reason": { "type": "string" }
                    }
                }
            },
            "required": ["page", "mode", "changed", "reloadRequired"]
        })
    }
}

pub fn definition() -> crate::framework::ToolDef {
    super::def_with_output::<ToggleModeArgs, ToggleModeOutput>(
        "toggleMode",
        DESCRIPTION,
        None,
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: ToggleModeArgs = parse_args(raw)?;
        if args.page == 0 {
            return Err(invalid("page", "page ids start at 1".into()));
        }
        let kind = match args.mode {
            ModeArg::Mobile => ModeKind::Mobile,
            ModeArg::Desktop => ModeKind::Desktop,
            ModeArg::Freesize => ModeKind::Freesize,
        };
        let request =
            MobileModeRequest::new(kind, args.zoom, args.resize_window).map_err(|err| {
                let path = match err {
                    MobileModeArgError::ResizeWindowWithoutMobile => "resizeWindow",
                    _ => "zoom",
                };
                invalid(path, err.to_string())
            })?;
        let outcome = ctx
            .browser
            .toggle_mobile_mode(PageId(args.page), request)
            .await?;
        Ok(Some(text_result(
            summary(&outcome),
            Some(structured(&outcome)),
        )))
    })
}

fn invalid(path: &str, message: String) -> ToolError {
    ToolError::InvalidArguments(vec![ArgIssue {
        path: path.to_string(),
        message,
    }])
}

fn summary(outcome: &MobileModeOutcome) -> String {
    let page = &outcome.page;
    let mut text = match outcome.mode {
        MobileMode::Mobile(zoom) => {
            let (width, height) = zoom.visible_size();
            format!(
                "Mobile mode on for page {page} at {}% zoom: {MOBILE_WIDTH}x{MOBILE_HEIGHT} CSS px viewport at 2x, drawn in a {width}x{height} view.",
                number(zoom.percent())
            )
        }
        MobileMode::Desktop => format!(
            "Desktop mode on for page {page}: fixed {DESKTOP_WIDTH}x{DESKTOP_HEIGHT} CSS px viewport at 1x."
        ),
        MobileMode::Freesize => format!(
            "Freesize mode on for page {page}: the tab uses its window's own viewport, and viewport commands are allowed."
        ),
    };
    if !outcome.changed {
        text.push_str(" Settings were already in place and were re-sent.");
    }
    if outcome.reload_required {
        text.push_str(" Reload the page to get the server's content for this mode.");
    }
    if let Some(window) = &outcome.window {
        text.push_str(&format!(" {}.", capitalized(&window.to_string())));
    }
    text
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn structured(outcome: &MobileModeOutcome) -> Value {
    let mut value = json!({
        "page": outcome.page.0,
        "mode": outcome.mode.name(),
        "changed": outcome.changed,
        "reloadRequired": outcome.reload_required,
    });
    if let MobileMode::Mobile(zoom) = outcome.mode {
        value["zoom"] = number(zoom.percent());
    }
    if let (Some((width, height, density)), Some((visible_width, visible_height))) =
        (outcome.mode.viewport(), outcome.mode.visible_size())
    {
        value["viewport"] =
            json!({"width":width,"height":height,"deviceScaleFactor":number(density)});
        value["visibleSize"] = json!({"width":visible_width,"height":visible_height});
    }
    if let Some(window) = &outcome.window {
        value["window"] = window_json(window);
    }
    value
}

fn window_json(window: &WindowOutcome) -> Value {
    let bounds = |bounds: &WindowBounds| json!({"left":bounds.left,"top":bounds.top,"width":bounds.width,"height":bounds.height});
    match window {
        WindowOutcome::Fitted {
            window,
            bounds: actual,
            requested: (width, height),
            exact: (exact_width, exact_height),
        } => json!({
            "windowId": window.0,
            "resized": true,
            "bounds": bounds(actual),
            "requestedContentSize": {"width":width,"height":height},
            "contentSizeExact": {"width":exact_width,"height":exact_height},
        }),
        WindowOutcome::NotFitted { window, reason } => {
            let mut value = json!({"resized": false, "reason": reason});
            if let Some(window) = window {
                value["windowId"] = json!(window.0);
            }
            value
        }
        WindowOutcome::Restored {
            window,
            bounds: actual,
        } => json!({"windowId": window.0, "restored": true, "bounds": bounds(actual)}),
        WindowOutcome::NotRestored { window, reason } => {
            json!({"windowId": window.0, "restored": false, "reason": reason})
        }
    }
}

/// Whole percents read as integers (150, not 150.0); fractions are kept.
fn number(value: f64) -> Value {
    if value.fract() == 0.0 {
        json!(value as i64)
    } else {
        json!(value)
    }
}
