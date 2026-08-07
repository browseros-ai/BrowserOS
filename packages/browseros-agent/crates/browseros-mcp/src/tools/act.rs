use crate::framework::{
    ToolCtx, ToolExecResult, ToolResult, error_result, parse_args, pending_dialog_result,
    text_result,
};
use browseros_core::{
    PageId, Ref,
    input::{ClickOptions, Point, PublicMouseButton as MouseButton, ScrollDirection},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const DESCRIPTION: &str = "\
Act on the page using refs from the last snapshot. \
kinds: click, type (into focused element), fill (ref+value, or many via fields[]), \
press (key/combo), hover, focus, check, uncheck, select (option value), scroll, drag. \
dialog_accept/dialog_dismiss handle pending JavaScript dialogs. \
ALWAYS fill a whole form in one call via fields[], never field-by-field. \
Reads back a post-settle diff - no follow-up diff/snapshot needed; \
re-snapshot only for fresh refs.";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ActKind {
    Click,
    ClickAt,
    Type,
    TypeAt,
    Fill,
    Press,
    Hover,
    HoverAt,
    Focus,
    Check,
    Uncheck,
    Select,
    Scroll,
    Drag,
    DragAt,
    DialogAccept,
    DialogDismiss,
}

impl ActKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::ClickAt => "click_at",
            Self::Type => "type",
            Self::TypeAt => "type_at",
            Self::Fill => "fill",
            Self::Press => "press",
            Self::Hover => "hover",
            Self::HoverAt => "hover_at",
            Self::Focus => "focus",
            Self::Check => "check",
            Self::Uncheck => "uncheck",
            Self::Select => "select",
            Self::Scroll => "scroll",
            Self::Drag => "drag",
            Self::DragAt => "drag_at",
            Self::DialogAccept => "dialog_accept",
            Self::DialogDismiss => "dialog_dismiss",
        }
    }

    fn is_dialog(&self) -> bool {
        matches!(self, Self::DialogAccept | Self::DialogDismiss)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ActDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ActMouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FillField {
    r#ref: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ActArgs {
    page: u32,
    kind: ActKind,
    /// Target element ref, e.g. "e12".
    r#ref: Option<String>,
    /// Text for kind=type.
    text: Option<String>,
    /// Value for kind=fill/select.
    value: Option<String>,
    /// Multiple fields for kind=fill, filled in order.
    fields: Option<Vec<FillField>>,
    /// Key/combo for kind=press, e.g. "Enter", "Control+a".
    key: Option<String>,
    direction: Option<ActDirection>,
    /// Scroll amount (wheel notches), default 3.
    amount: Option<f64>,
    /// Viewport x coordinate for *_at kinds.
    x: Option<f64>,
    /// Viewport y coordinate for *_at kinds.
    y: Option<f64>,
    /// Target ref for kind=drag.
    #[serde(rename = "targetRef")]
    target_ref: Option<String>,
    /// Drag start x coordinate.
    #[serde(rename = "startX")]
    start_x: Option<f64>,
    /// Drag start y coordinate.
    #[serde(rename = "startY")]
    start_y: Option<f64>,
    /// Drag end x coordinate.
    #[serde(rename = "endX")]
    end_x: Option<f64>,
    /// Drag end y coordinate.
    #[serde(rename = "endY")]
    end_y: Option<f64>,
    button: Option<ActMouseButton>,
    #[serde(rename = "clickCount")]
    click_count: Option<i64>,
    clear: Option<bool>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<ActArgs>("act", DESCRIPTION, None, handler)
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: ActArgs = parse_args(raw)?;
        let page_id = PageId(args.page);
        if !args.kind.is_dialog()
            && let Some(result) = pending_dialog_result(ctx, page_id.clone())
        {
            return Ok(Some(result));
        }
        let console_start = ctx.session.page_signals.console_mark(&page_id);
        let input = ctx.session.input(page_id.clone()).await;
        if let Some(err) = run_kind(&args, &input).await? {
            return Ok(Some(err));
        }
        if args.kind.is_dialog() {
            ctx.session.page_signals.clear_dialog(&page_id);
        }
        response.data(json!({ "kind": args.kind.as_str() }));
        response.include_diff(args.page, true);
        response.include_console_summary(args.page, console_start);
        Ok(Some(text_result(
            format!("ok ({})", args.kind.as_str()),
            None,
        )))
    })
}

async fn run_kind(
    args: &ActArgs,
    input: &browseros_core::input::Input,
) -> ToolExecResult<Option<ToolResult>> {
    match args.kind {
        ActKind::Click => {
            let Some(ref_id) = args.r#ref.as_deref() else {
                return Ok(Some(error_result("act click: ref is required.")));
            };
            input
                .click(&Ref(ref_id.to_string()), click_options(args))
                .await?;
        }
        ActKind::ClickAt => {
            let Some(point) = point_from_args(args, "click_at")? else {
                return Ok(Some(error_result("act click_at: x and y are required.")));
            };
            input
                .click_at(point.x, point.y, click_options(args))
                .await?;
        }
        ActKind::Type => {
            let Some(text) = args.text.as_deref() else {
                return Ok(Some(error_result("act type: text is required.")));
            };
            input.type_text(text).await?;
        }
        ActKind::TypeAt => {
            let Some(point) = point_from_args(args, "type_at")? else {
                return Ok(Some(error_result("act type_at: x and y are required.")));
            };
            let Some(text) = args.text.as_deref() else {
                return Ok(Some(error_result("act type_at: text is required.")));
            };
            input
                .type_at(point.x, point.y, text, args.clear.unwrap_or(false))
                .await?;
        }
        ActKind::Fill => {
            if let Some(fields) = &args.fields {
                for field in fields {
                    input
                        .fill(
                            &Ref(field.r#ref.clone()),
                            &field.value,
                            args.clear.unwrap_or(false),
                        )
                        .await?;
                }
            } else if let (Some(ref_id), Some(value)) =
                (args.r#ref.as_deref(), args.value.as_deref())
            {
                input
                    .fill(&Ref(ref_id.to_string()), value, args.clear.unwrap_or(false))
                    .await?;
            } else {
                return Ok(Some(error_result(
                    "act fill: provide fields[] or both ref and value.",
                )));
            }
        }
        ActKind::Press => {
            let Some(key) = args.key.as_deref() else {
                return Ok(Some(error_result("act press: key is required.")));
            };
            input.press(key).await?;
        }
        ActKind::Hover => {
            let Some(ref_id) = args.r#ref.as_deref() else {
                return Ok(Some(error_result("act hover: ref is required.")));
            };
            input.hover(&Ref(ref_id.to_string())).await?;
        }
        ActKind::HoverAt => {
            let Some(point) = point_from_args(args, "hover_at")? else {
                return Ok(Some(error_result("act hover_at: x and y are required.")));
            };
            input.hover_at(point.x, point.y).await?;
        }
        ActKind::Focus => {
            let Some(ref_id) = args.r#ref.as_deref() else {
                return Ok(Some(error_result("act focus: ref is required.")));
            };
            input.focus(&Ref(ref_id.to_string())).await?;
        }
        ActKind::Check => {
            let Some(ref_id) = args.r#ref.as_deref() else {
                return Ok(Some(error_result("act check: ref is required.")));
            };
            input.check(&Ref(ref_id.to_string())).await?;
        }
        ActKind::Uncheck => {
            let Some(ref_id) = args.r#ref.as_deref() else {
                return Ok(Some(error_result("act uncheck: ref is required.")));
            };
            input.uncheck(&Ref(ref_id.to_string())).await?;
        }
        ActKind::Select => {
            let (Some(ref_id), Some(value)) = (args.r#ref.as_deref(), args.value.as_deref()) else {
                return Ok(Some(error_result(
                    "act select: ref and value are required.",
                )));
            };
            input.select_option(&Ref(ref_id.to_string()), value).await?;
        }
        ActKind::Scroll => {
            let ref_id = args.r#ref.as_ref().map(|value| Ref(value.clone()));
            input
                .scroll(
                    scroll_direction(args.direction.as_ref()),
                    args.amount.unwrap_or(3.0).round() as i64,
                    ref_id.as_ref(),
                )
                .await?;
        }
        ActKind::Drag => {
            let (Some(ref_id), Some(target_ref)) =
                (args.r#ref.as_deref(), args.target_ref.as_deref())
            else {
                return Ok(Some(error_result(
                    "act drag: ref and targetRef are required.",
                )));
            };
            input
                .drag(&Ref(ref_id.to_string()), &Ref(target_ref.to_string()))
                .await?;
        }
        ActKind::DragAt => {
            let (Some(start_x), Some(start_y), Some(end_x), Some(end_y)) =
                (args.start_x, args.start_y, args.end_x, args.end_y)
            else {
                return Ok(Some(error_result(
                    "act drag_at: startX, startY, endX, and endY are required.",
                )));
            };
            input
                .drag_at(
                    Point {
                        x: start_x,
                        y: start_y,
                    },
                    Point { x: end_x, y: end_y },
                )
                .await?;
        }
        ActKind::DialogAccept => {
            input.handle_dialog(true, args.text.as_deref()).await?;
        }
        ActKind::DialogDismiss => {
            input.handle_dialog(false, None).await?;
        }
    }
    Ok(None)
}

fn click_options(args: &ActArgs) -> ClickOptions {
    ClickOptions {
        button: args.button.as_ref().map(|button| match button {
            ActMouseButton::Left => MouseButton::Left,
            ActMouseButton::Middle => MouseButton::Middle,
            ActMouseButton::Right => MouseButton::Right,
        }),
        click_count: args.click_count,
    }
}

fn point_from_args(args: &ActArgs, _kind: &str) -> ToolExecResult<Option<Point>> {
    Ok(match (args.x, args.y) {
        (Some(x), Some(y)) => Some(Point { x, y }),
        _ => None,
    })
}

fn scroll_direction(direction: Option<&ActDirection>) -> ScrollDirection {
    match direction.unwrap_or(&ActDirection::Down) {
        ActDirection::Up => ScrollDirection::Up,
        ActDirection::Down => ScrollDirection::Down,
        ActDirection::Left => ScrollDirection::Left,
        ActDirection::Right => ScrollDirection::Right,
    }
}
