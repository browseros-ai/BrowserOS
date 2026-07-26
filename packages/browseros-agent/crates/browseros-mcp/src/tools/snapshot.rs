use crate::{
    format::snapshot::format_snapshot_result,
    framework::{
        ToolCtx, ToolExecResult, ToolResult, parse_args, pending_dialog_result, text_result,
    },
};
use browseros_core::{
    PageId,
    snapshot::{SnapshotMode, SnapshotOptions},
};
use futures_util::future::BoxFuture;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_DEPTH: usize = 100;
const DESCRIPTION: &str = "\
Capture the page as an indented accessibility tree. \
Each actionable element carries a stable [ref=eN] you pass to `act`. \
mode=\"interactive\" returns actionables plus headings and ancestor context; depth caps nesting. \
Default mode=\"full\" is unchanged; iframe content is stitched inline. \
Re-snapshot after navigation or large changes (refs are invalidated). \
This is the start of the loop: snapshot -> act -> (reads back a diff).";

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SnapshotModeArg {
    #[default]
    Full,
    Interactive,
}

impl SnapshotModeArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Interactive => "interactive",
        }
    }
}

impl From<SnapshotModeArg> for SnapshotMode {
    fn from(value: SnapshotModeArg) -> Self {
        match value {
            SnapshotModeArg::Full => Self::Full,
            SnapshotModeArg::Interactive => Self::Interactive,
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct SnapshotArgs {
    /// Page id from `tabs` or `navigate`.
    page: u32,
    /// Snapshot compactness mode. Defaults to full.
    #[serde(default)]
    mode: SnapshotModeArg,
    /// Maximum rendered tree depth. Values are floored and clamped to 1..=100.
    depth: Option<f64>,
}

pub fn definition() -> crate::framework::ToolDef {
    super::def::<SnapshotArgs>(
        "snapshot",
        DESCRIPTION,
        Some(super::read_only_annotations()),
        handler,
    )
}

fn handler<'a>(
    raw: Value,
    ctx: &'a ToolCtx,
    _response: &'a mut crate::response::ToolResponse,
) -> BoxFuture<'a, ToolExecResult<Option<ToolResult>>> {
    Box::pin(async move {
        let args: SnapshotArgs = parse_args(raw)?;
        let depth = args.depth.map(clamp_depth);
        if let Some(result) = pending_dialog_result(ctx, PageId(args.page)) {
            return Ok(Some(result));
        }
        let snapshot = ctx
            .session
            .observe(PageId(args.page))
            .await
            .snapshot_with_options(SnapshotOptions {
                mode: args.mode.into(),
                depth,
            })
            .await?;
        let formatted = format_snapshot_result(&snapshot.text, &snapshot.url, ctx).await;
        let mut structured = formatted.structured;
        if let Value::Object(object) = &mut structured {
            object.insert("page".to_string(), json!(args.page));
            object.insert("mode".to_string(), json!(args.mode.as_str()));
            if let Some(depth) = depth {
                object.insert("depth".to_string(), json!(depth));
            }
        }
        Ok(Some(text_result(formatted.text, Some(structured))))
    })
}

fn clamp_depth(depth: f64) -> usize {
    if !depth.is_finite() {
        return MAX_DEPTH;
    }
    depth.floor().clamp(1.0, MAX_DEPTH as f64) as usize
}
